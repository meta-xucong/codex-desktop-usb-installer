[CmdletBinding()]
param(
    [string]$InstallerPath = "",
    [string]$ManagerPath = "",
    [string]$BundlePath = "",
    [string]$OutputPath = "",
    [string]$CapabilityPackSource = "",
    [string]$UsbPolicy = "",
    [string]$UsbSigningProfilePath = "",
    [switch]$InternalSigned,
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $repo "tools\usb-signing-profile.ps1")
$releaseVersion = [string]((Get-Content -LiteralPath (Join-Path $repo "package.json") -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($releaseVersion)) {
    throw "package.json 缺少发布版本号"
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) { $InstallerPath = Join-Path $repo "release\setup\CodexUsbLauncher.exe" }
if ([string]::IsNullOrWhiteSpace($ManagerPath)) { $ManagerPath = Join-Path $repo "release\setup\CodexAppManager.exe" }
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = Join-Path $repo "release\usb-final" }
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $repo "release\usb-oneclick" }
if ([string]::IsNullOrWhiteSpace($CapabilityPackSource)) {
    $CapabilityPackSource = Join-Path $repo "installer-pack"
}

if ([string]::IsNullOrWhiteSpace($UsbPolicy)) {
    $UsbPolicy = [string]$env:CODEX_USB_GUARD_POLICY
}
if ([string]::IsNullOrWhiteSpace($UsbPolicy)) {
    $UsbPolicy = if ($SkipSignature -and -not $InternalSigned) { "DevelopmentAutoEnroll" } else { "SignedRequired" }
}
if ($UsbPolicy -notin @("SignedRequired", "DevelopmentAutoEnroll")) {
    throw "UsbPolicy 必须是 SignedRequired 或 DevelopmentAutoEnroll：$UsbPolicy"
}
if ($InternalSigned -and $SkipSignature) {
    throw "InternalSigned 不能与 SkipSignature 同时使用"
}
if ($InternalSigned -and $UsbPolicy -ne "SignedRequired") {
    throw "InternalSigned 必须使用 SignedRequired 策略"
}
if ($SkipSignature -and $UsbPolicy -eq "SignedRequired" -and -not $InternalSigned) {
    throw "SignedRequired 静态发行包必须使用 -InternalSigned；-SkipSignature 仅适用于 DevelopmentAutoEnroll，不能生成正式签名包元数据"
}

if ($UsbPolicy -eq "SignedRequired") {
    $signingProfile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $UsbSigningProfilePath
    Initialize-CodexUsbSigningEnvironment -Profile $signingProfile
    $publicKeyId = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_ID
    $publicKeyB64 = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_B64
    if ([string]::IsNullOrWhiteSpace($publicKeyId) -or
        [string]::IsNullOrWhiteSpace($publicKeyB64)) {
        throw "SignedRequired U 盘包必须配置 CODEX_USB_GUARD_PUBLIC_KEY_ID 和 CODEX_USB_GUARD_PUBLIC_KEY_B64"
    }
    if ($publicKeyId -notmatch '^[A-Za-z0-9._-]{1,128}$') {
        throw "CODEX_USB_GUARD_PUBLIC_KEY_ID 不是合法 keyId：$publicKeyId"
    }
    try {
        $publicKeyBytes = [Convert]::FromBase64String($publicKeyB64.Trim())
    } catch {
        throw "CODEX_USB_GUARD_PUBLIC_KEY_B64 不是有效 Base64"
    }
    if ($publicKeyBytes.Length -ne 32) {
        throw "CODEX_USB_GUARD_PUBLIC_KEY_B64 必须解码为 32 字节 Ed25519 公钥"
    }
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $publicKeyDigestBytes = $hasher.ComputeHash($publicKeyBytes)
    } finally {
        $hasher.Dispose()
    }
    $publicKeySha256 = (($publicKeyDigestBytes | ForEach-Object { $_.ToString('x2') }) -join '')
    $signingBinding = [ordered]@{
        policy = "SignedRequired"
        keyId = $publicKeyId
        publicKeySha256 = $publicKeySha256
    }
} else {
    $signingBinding = [ordered]@{
        policy = "DevelopmentAutoEnroll"
        keyId = $null
        publicKeySha256 = $null
    }
}
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$manager = (Resolve-Path -LiteralPath $ManagerPath).Path
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path

# Windows PowerShell 5.1/.NET Framework lacks Path.GetRelativePath. Use a
# root-scoped compatibility helper so release packaging works on a stock
# Windows machine as well as PowerShell 7.
function Get-RelativePathCompat([string]$BasePath, [string]$FilePath) {
    $base = [System.IO.Path]::GetFullPath($BasePath).TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath($FilePath)
    if (-not $full.StartsWith($base, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "文件不在 U 盘输出目录内：$FilePath"
    }
    return $full.Substring($base.Length)
}

function Read-JsonFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "缺少 JSON 清单：$Path"
    }
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8) | ConvertFrom-Json
}

function Assert-AuthenticodePe([string]$Path, [string]$Label) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne "Valid") {
        throw "$Label Authenticode 状态不是 Valid：$($signature.Status)"
    }
    if ($null -eq $signature.SignerCertificate -or
        [string]::IsNullOrWhiteSpace([string]$signature.SignerCertificate.Subject)) {
        throw "$Label 缺少有效签名主体"
    }
}

function Assert-CompiledUsbManagerBinding([string]$Path, [string]$KeyId, [string]$PublicKeyB64, [string]$PublicKeySha256) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "USB 管理器不存在，无法验证编译期签名绑定：$Path"
    }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    # The Tauri manager embeds these compile-time option_env! values as
    # ordinary ASCII constants.  Checking the actual PE supplied through
    # -ManagerPath closes the stale-manager/new-manifest composition gap
    # before any release manifest is written.
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($ascii.IndexOf($KeyId, [System.StringComparison]::Ordinal) -lt 0) {
        throw "USB 管理器未嵌入预期 active keyId，拒绝打包：$KeyId"
    }
    $expectedPublicKey = $PublicKeyB64.Trim()
    if ($ascii.IndexOf($expectedPublicKey, [System.StringComparison]::Ordinal) -lt 0) {
        throw "USB 管理器未嵌入与发布配置一致的公钥，拒绝打包：publicKeySha256=$PublicKeySha256"
    }
    Write-Output "USB 管理器编译期签名绑定已验证：keyId=$KeyId；publicKeySha256=$PublicKeySha256"
}

function Assert-CompiledUsbManagerPolicy([string]$Path, [string]$ExpectedPolicy) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "USB 管理器不存在，无法验证编译期策略绑定：$Path"
    }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    $marker = "codex-usb-compiled-policy:$ExpectedPolicy"
    if ($ascii.IndexOf($marker, [System.StringComparison]::Ordinal) -lt 0) {
        throw "USB 管理器编译期策略与发布策略不一致：期望 $ExpectedPolicy；未找到标记 $marker"
    }
}

function Normalize-BundleRelativePath([string]$RelativePath) {
    $relative = ([string]$RelativePath).Trim().Replace('/', '\')
    if ([string]::IsNullOrWhiteSpace($relative) -or [System.IO.Path]::IsPathRooted($relative)) {
        throw "manifest 中的路径必须是 bundle 内的相对路径：$RelativePath"
    }
    foreach ($segment in $relative.Split('\')) {
        if ([string]::IsNullOrWhiteSpace($segment) -or $segment -eq '.' -or $segment -eq '..' -or $segment.Contains(':')) {
            throw "manifest 中的路径包含非法片段：$RelativePath"
        }
    }
    return $relative.Replace('\', '/')
}

function Resolve-BundleRelativeFile([string]$RootPath, [string]$RelativePath) {
    $relative = Normalize-BundleRelativePath $RelativePath
    $candidate = Join-Path $RootPath ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "manifest.payload.path 指向的文件不存在：$relative"
    }
    $resolved = (Resolve-Path -LiteralPath $candidate).Path
    $rootFull = [System.IO.Path]::GetFullPath($RootPath).TrimEnd('\') + '\'
    $resolvedFull = [System.IO.Path]::GetFullPath($resolved)
    if (-not $resolvedFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "manifest.payload.path 不得逃逸 bundle 根目录：$relative"
    }
    return $resolved
}

function Assert-NoSuccessfulStateResidue([string]$RootPath) {
    $rootLicensePath = Join-Path $RootPath "usb-license.json"
    if (Test-Path -LiteralPath $rootLicensePath) {
        throw "静态发行目录不得携带 usb-license.json；请在真实 U 盘复制完成后使用签名工具写入运行期授权"
    }
    foreach ($entry in Get-ChildItem -LiteralPath $RootPath -Recurse -Force) {
        $relative = (Get-RelativePathCompat $RootPath $entry.FullName).Replace('\', '/')
        if ($entry.Name -ieq "usb-license.json" -and $relative -ine "usb-license.json") {
            throw "usb-license.json 只能出现在 U 盘 bundle 根目录：$relative"
        }
        if ($entry.Name -ieq "usb-license.json.bak") {
            throw "U 盘 bundle 不得包含 usb-license.json.bak 残留：$relative"
        }
        $forbidden = $false
        foreach ($segment in $relative.Split('/')) {
            if ($segment -ieq '.staging' -or $segment -ieq 'staging' -or
                $segment -ieq '.transaction' -or $segment -ieq 'transaction' -or
                $segment -match '(?i)^transaction([.-]|$)' -or
                $segment -match '(?i)\.(bak|part|tmp)$') {
                $forbidden = $true
                break
            }
        }
        if ($forbidden) {
            throw "构建输出包含成功状态不允许的事务/临时残留：$relative"
        }
    }
}

& (Join-Path $PSScriptRoot "build-usb-layout.ps1") `
    -InstallerPath $installer `
    -BundlePath $bundle `
    -OutputPath $OutputPath `
    -CapabilityPackSource $CapabilityPackSource `
    -InternalSigned:$InternalSigned `
    -SkipSignature:$SkipSignature
# The nested PowerShell script communicates failures through terminating
# errors.  `$LASTEXITCODE` is intentionally not inspected here because it can
# retain the exit code of a previous native command in the parent session.

$output = (Resolve-Path -LiteralPath $OutputPath).Path
$manifestPath = Join-Path $output "manifest.json"
$manifest = Read-JsonFile $manifestPath
if ($manifest.schemaVersion -is [string] -or [int64]$manifest.schemaVersion -ne 2) {
    throw "输出 root manifest 必须声明 schemaVersion=2"
}
$architecture = [string]$manifest.architecture
if ($architecture -notin @("x64", "arm64")) {
    throw "输出 root manifest 的 architecture 无效：$architecture"
}
$payloadRelative = Normalize-BundleRelativePath ([string]$manifest.payload.path)
if (([System.IO.Path]::GetExtension($payloadRelative) -ine ".msix")) {
    throw "输出 root manifest 的 payload.path 必须指向 .msix：$payloadRelative"
}
$payload = Resolve-BundleRelativeFile $output $payloadRelative
$payloadItem = Get-Item -LiteralPath $payload
$payloadDirectoryRelative = $payloadRelative -replace '/[^/]+$', ''
$backupDirectory = if ([string]::IsNullOrWhiteSpace($payloadDirectoryRelative)) {
    "_backup"
} else {
    "$payloadDirectoryRelative/_backup"
}
$rootUpdateCapability = $manifest.updateCapability
if ($null -eq $rootUpdateCapability -or
    $rootUpdateCapability.codexPayload -ne $true -or
    $rootUpdateCapability.schemaVersion -is [string] -or
    [int64]$rootUpdateCapability.schemaVersion -ne 2) {
    throw "输出 root manifest 未声明 Codex payload update capability"
}
if ((Get-Item -LiteralPath $manager).Name -ine "CodexAppManager.exe") {
    throw "ManagerPath 必须指向名为 CodexAppManager.exe 的 Tauri 管理器本体"
}
if ($UsbPolicy -eq "SignedRequired") {
    Assert-CompiledUsbManagerPolicy $manager $UsbPolicy
    Assert-CompiledUsbManagerBinding $manager $publicKeyId $publicKeyB64 $publicKeySha256
}
if ($UsbPolicy -eq "DevelopmentAutoEnroll") {
    Assert-CompiledUsbManagerPolicy $manager $UsbPolicy
}
Copy-Item -LiteralPath $manager -Destination (Join-Path $output "CodexAppManager.exe") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-payload.ps1") -Destination (Join-Path $output "verify-payload.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-oneclick-usb.ps1") -Destination (Join-Path $output "verify-usb.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-usb-launcher.ps1") -Destination (Join-Path $output "verify-usb-launcher.ps1") -Force
$launcherVerifier = Join-Path $output "verify-usb-launcher.ps1"
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path (Join-Path $output "CodexAppManager.exe") -ExpectedLevel requireAdministrator
if ($LASTEXITCODE -ne 0) {
    throw "CodexAppManager.exe 未通过 USB requireAdministrator 清单验证"
}
# Tauri deliberately does not ship PowerShell files through the Windows
# resource bundle.  Keep the product verifier in the user-visible root
# ccswitch directory so offline diagnostics still work on a blank machine.
$ccswitchVerifier = Join-Path (Split-Path -Parent $repo) "release\ccswitch-product\verify-product.ps1"
if (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf) {
    Copy-Item -LiteralPath $ccswitchVerifier -Destination (Join-Path $output "ccswitch\verify-product.ps1") -Force
}

$installerItem = Get-Item -LiteralPath (Join-Path $output "CodexDesktopSetup.exe")
$managerItem = Get-Item -LiteralPath (Join-Path $output "CodexAppManager.exe")
if ($UsbPolicy -eq "SignedRequired") {
    Assert-CompiledUsbManagerPolicy $managerItem.FullName $UsbPolicy
    Assert-CompiledUsbManagerBinding $managerItem.FullName $publicKeyId $publicKeyB64 $publicKeySha256
}
if ($UsbPolicy -eq "DevelopmentAutoEnroll") {
    Assert-CompiledUsbManagerPolicy $managerItem.FullName $UsbPolicy
}
$ccswitchItem = Get-Item -LiteralPath (Join-Path $output "ccswitch\CodexSwitch.exe")
$embeddedCcswitchItem = Get-Item -LiteralPath (Join-Path $output "resources\ccswitch\CodexSwitch.exe")
$vcRedistItem = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\vc_redist.x64.exe")
$webView2Item = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
if (-not $SkipSignature -and -not $InternalSigned) {
    Assert-AuthenticodePe $ccswitchItem.FullName "CCSwitch"
    Assert-AuthenticodePe $embeddedCcswitchItem.FullName "embedded CCSwitch"
}
$installerHash = (Get-FileHash -LiteralPath $installerItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$managerHash = (Get-FileHash -LiteralPath $managerItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$payloadHash = (Get-FileHash -LiteralPath $payloadItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$ccswitchHash = (Get-FileHash -LiteralPath $ccswitchItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$vcRedistHash = (Get-FileHash -LiteralPath $vcRedistItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$webView2Hash = (Get-FileHash -LiteralPath $webView2Item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($webView2Hash -ine "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D") {
    throw "WebView2 离线安装包哈希与管理器内置值不符，拒绝生成发布包"
}

$usbManifest = [ordered]@{
    schemaVersion = 2
    product = "codex-usb-oneclick"
    releaseVersion = $releaseVersion
    status = "oneclick"
    updateCapability = [ordered]@{
        codexPayload = $true
        schemaVersion = 2
        mutableFields = @(
            "manifest.json/codexDesktopVersion",
            "manifest.json/packageVersion",
            "manifest.json/releaseTag",
            "manifest.json/payload/sha256",
            "manifest.json/payload/size",
            "codexInstaller/payloadSha256",
            "codexInstaller/payloadSize"
        )
        checksumFile = "SHA256SUMS.txt"
        backupDirectory = $backupDirectory
        backupPolicy = "single-msix-listed-in-SHA256SUMS"
        transactionDirectory = ".staging"
        successState = "clean"
    }
    description = "Offline Codex desktop extracted from a verified MSIX into the authorized USB bundle. Codex account, session, cache and configuration data remain in the normal local Windows profile; no external AppX volume or WpSystem data layout is used."
    codexInstaller = [ordered]@{
        file = "CodexDesktopSetup.exe"
        size = [int64]$installerItem.Length
        sha256 = $installerHash
        architecture = $architecture
        payload = $payloadRelative
        payloadSize = [int64]$payloadItem.Length
        payloadSha256 = $payloadHash
    }
    manager = [ordered]@{
        file = "CodexAppManager.exe"
        size = [int64]$managerItem.Length
        sha256 = $managerHash
        role = "requireAdministrator USB Tauri manager started by the launcher or directly"
    }
    ccswitch = [ordered]@{
        directory = "ccswitch"
        executable = "ccswitch/CodexSwitch.exe"
        size = [int64]$ccswitchItem.Length
        sha256 = $ccswitchHash
        embeddedExecutable = "resources/ccswitch/CodexSwitch.exe"
        launchMode = "usb-direct"
        graphicsMode = "child-scoped-webview2-software-rendering"
        successPostcondition = "stable-sizeable-window"
        foregroundPolicy = "restore-clamp-to-work-area-and-present"
        localInstallDirectory = "%LOCALAPPDATA%/Programs/CodexSwitch"
        dataDirectory = "%USERPROFILE%/.cc-switch (application-defined Windows profile path)"
    }
    vcRuntime = [ordered]@{
        file = "resources/prerequisites/vc_redist.x64.exe"
        size = [int64]$vcRedistItem.Length
        sha256 = $vcRedistHash
        architecture = "x64"
        installArgs = "/install /quiet /norestart"
        source = "Microsoft Visual C++ Redistributable (unmodified, signed)"
    }
    webView2Runtime = [ordered]@{
        file = "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
        size = [int64]$webView2Item.Length
        sha256 = $webView2Hash
        architecture = "x64"
        installArgs = "/silent /install"
        source = "Microsoft WebView2 Evergreen Standalone Installer (unmodified, signed)"
        sourceUrl = "https://go.microsoft.com/fwlink/p/?LinkId=2124701"
        beforeUi = $true
        installerTimeoutSeconds = 600
    }
    usbGuard = [ordered]@{
        helper = "CodexUsbGuard.exe"
        localDirectory = "%LOCALAPPDATA%/CodexUsbGuard"
        licenseFile = "usb-license.json"
        binding = "removable-volume + volume-serial + anchor-sha256 + bundle-relative-path"
        signing = $signingBinding
        verification = [ordered]@{
            runtimeMode = if ($InternalSigned -or $UsbPolicy -eq "SignedRequired") { "internal-signed-usb-license" } elseif ($SkipSignature) { "hash-only-development" } else { "strict-authenticode" }
            productAuthenticode = if ($InternalSigned -or $UsbPolicy -eq "SignedRequired") { "not-required-internal-license-mode" } else { "required" }
            externalAuthenticode = if ($SkipSignature) { "not-required-development-mode" } else { "required-openai-msix-and-microsoft-prerequisites" }
        }
        pollIntervalMs = 1000
        disconnectGraceMs = 5000
        startMenuInterception = $false
        localHelperRefresh = "length-and-sha256-before-new-lease"
    }
    sourceMediaCleanup = [ordered]@{
        planSchema = 3
        runtimeLicenseMode = "typed-manager-mutable-metadata"
        authorization = "active-lease-fingerprint+volume+immutable-manifest+license-snapshot"
        deleteOrder = "payload-first-license-and-retry-entry-last"
        unknownPolicy = "enumerate-and-preserve"
        usesRecycleBin = $false
    }
    startupElevation = [ordered]@{
        mode = "usb-launcher-requireAdministrator"
        appliesTo = "USB bundle entry only"
        mechanism = "embedded requireAdministrator manifest + standard Windows UAC consent"
        marker = "--codex-usb-elevated"
        onCancel = "exit-before-install"
        silentBypass = $false
        executionPolicy = "process-local-bypass"
        executionPolicyScope = "temporary PowerShell child only"
    }
    programLayout = [ordered]@{
        mode = "verified-msix-extraction"
        codexRoot = "apps/Codex"
        usesAppxVolume = $false
        createsWindowsApps = $false
        createsWpSystem = $false
        directShellLaunch = $false
    }
    codexLaunch = [ordered]@{
        initialMode = "software-rendering"
        initialArguments = @("--disable-gpu")
        fallbackMode = "software-rendering-no-sandbox"
        fallbackArguments = @("--disable-gpu", "--no-sandbox")
        fallbackOnlyAfterNoUsableWindow = $true
        successPostcondition = "stable-sizeable-window"
        compatibilityMemory = "local-exact-codex-version"
        selfUpdate = "disabled-for-usb-child"
        globalEnvironmentModified = $false
    }
    dataLayout = [ordered]@{
        codexHome = "%USERPROFILE%/.codex"
        roaming = "%APPDATA%/Codex"
        local = "%LOCALAPPDATA%/Codex"
        onUsb = $false
        observedBeforeInstall = $true
        placementCheck = "declared drive + canonical nearest existing ancestor must differ from the authorized USB volume"
    }
    managedContent = [ordered]@{
        skillsDirectory = "content/skills"
        mcpDirectory = "content/mcp"
        capabilityPackDirectory = "content/codex-starter-kit"
        configLocation = "%USERPROFILE%/.codex/config.toml"
        secretsIncluded = $false
        status = "bundled-auto-installable"
    }
    workflow = @(
        "Double-click CodexDesktopSetup.exe; it is the USB-only requireAdministrator launcher and asks for one standard Windows UAC confirmation before starting CodexAppManager.exe.",
        "Before the UI opens, detect WebView2 and install the hash-pinned Microsoft x64 standalone runtime from the USB if missing; runtime failures use a native dialog.",
        "The installer verifies the manifest, fixed MSIX identity and SHA-256. In the formal internal-signed USB mode it verifies the USB license/anchor plus OpenAI-signed MSIX and Microsoft-signed prerequisites; product-owned launcher, manager and CCSwitch files are protected by the immutable bundle hash and do not require Authenticode.",
        "The verified MSIX is treated as an archive and transactionally extracted to <bundle>/apps/Codex. The old tree is kept as a rollback directory until the new executable and manifest pass postconditions.",
        "This route never calls StagePackageAsync, RegisterPackageAsync, Add-AppxPackage, Add-AppxVolume or Move-AppxPackage and never creates <USB>/WindowsApps or <USB>/WpSystem.",
        "A legacy exact OpenAI.Codex MSIX left by an earlier product version may be removed through supported Windows APIs after the USB portable tree is ready; no WindowsApps ACL or private package database is edited.",
        "Before CCSwitch launch, it checks VCRUNTIME140.dll/VCRUNTIME140_1.dll/MSVCP140.dll; if missing, it verifies and runs the bundled Microsoft VC++ x64 installer offline, then launches CodexSwitch.exe directly from the USB with child-scoped WebView2 software rendering and requires a stable sizeable window.",
        "On first launch Codex Switch opens the 404TOKEN setup dialog; enter the user's API Key and click Save and enable. No key is included in this bundle.",
        "Codex is launched only by the authorized USB manager with CODEX_HOME fixed to %USERPROFILE%/.codex. It first uses software rendering and automatically retries the proven no-sandbox fallback only if no stable usable window appears. Removing the USB triggers the existing five-second guarded shutdown.",
        "Ordinary uninstall removes <bundle>/apps/Codex and preserves local user data. Advanced purge remains a separate explicit confirmation. Source-media deletion is allowed only after the runtime tree has been removed, immutable release files still match SHA256SUMS.txt, and usb-license.json plus its atomic backup match the active lease fingerprint and handoff snapshot."
    )
    secretsIncluded = $false
    automaticPostInstallLaunch = $true
    notes = @(
        "The root manifest.json is the Codex installer manifest and must not be replaced by usb-manifest.json.",
        "The CCSwitch launch is best-effort; a missing payload does not roll back a successful Codex installation.",
        "Executable bytes are installed only under apps/Codex on the removable drive. Mutable Codex state uses the normal Windows profile (normally C:); a declared or canonicalized profile path resolving to the product USB blocks installation.",
        "CodexDesktopSetup.exe is a tiny USB-only launcher with an embedded requireAdministrator manifest; after the user's standard UAC approval it starts the USB-specific CodexAppManager.exe, which also embeds requireAdministrator as a defence-in-depth boundary. The normal installed manager build remains asInvoker. Declining UAC, lacking administrator credentials, or an enterprise policy that blocks elevation cannot be bypassed.",
        "The manager entry point is statically linked to the MSVC CRT so it can start before the prerequisite is present. The bundled vc_redist.x64.exe is the unmodified Microsoft installer and is hash-pinned; no DLL is copied directly to System32.",
        "No AppX external-volume registry policy, developer mode or sideload policy is required for the portable route. Enterprise execution-control restrictions are diagnosed, not bypassed.",
        "The content/codex-starter-kit folder contains the complete offline capability pack. The manager automatically installs or repairs its Skills, Agents, runtimes and local MCP resources after Codex is verified; external connections remain opt-in and their secrets stay in the local Windows user environment.",
        "Ordinary uninstall preserves the user's independent .codex, Roaming Codex and Local Codex data. Advanced purge is destructive and may affect an official/CLI Codex installation sharing those paths.",
        "usb-license.json is manager-created mutable USB identity metadata, not an immutable SHA256SUMS entry. The detached finalizer validates it against the active lease and deletes it last; unrelated or modified files are listed and preserved.",
        "Unsigned development bundles auto-enroll on first run; strict anti-copy releases require a pre-provisioned signed usb-license.json."
    )
}
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText((Join-Path $output "usb-manifest.json"), ($usbManifest | ConvertTo-Json -Depth 8), $utf8NoBom)

$readme = Get-Content -LiteralPath (Join-Path $repo "docs\USB-INSTALL-USER-GUIDE.txt") -Raw -Encoding UTF8
[System.IO.File]::WriteAllText((Join-Path $output "README.txt"), $readme, $utf8NoBom)

Assert-NoSuccessfulStateResidue $output

$lines = Get-ChildItem -LiteralPath $output -File -Recurse -Force |
    Sort-Object -Property FullName |
    Where-Object {
        $relative = (Get-RelativePathCompat $output $_.FullName).Replace('\', '/')
        $_.Name -ne "SHA256SUMS.txt" -and $relative -ine "usb-license.json"
    } |
    ForEach-Object {
        $relative = Get-RelativePathCompat $output $_.FullName
        $relative = $relative.Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)

Write-Output "已生成 Codex 一键 U 盘目录：$output"
Write-Output "入口：$(Join-Path $output 'CodexDesktopSetup.exe')"
