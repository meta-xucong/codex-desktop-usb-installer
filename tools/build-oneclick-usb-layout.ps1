[CmdletBinding()]
param(
    [string]$InstallerPath = "",
    [string]$ManagerPath = "",
    [string]$EntryShimPath = "",
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
$windowsPowerShell = Join-Path $env:WINDIR "System32\WindowsPowerShell\v1.0\powershell.exe"
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    $windowsPowerShellCommand = Get-Command powershell.exe -ErrorAction Stop
    $windowsPowerShell = $windowsPowerShellCommand.Source
}
. (Join-Path $repo "tools\usb-signing-profile.ps1")
$signingProfile = $null
$releaseVersion = [string]((Get-Content -LiteralPath (Join-Path $repo "package.json") -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($releaseVersion)) {
    throw "package.json 缺少发布版本号"
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) { $InstallerPath = Join-Path $repo "release\setup\CodexUsbLauncher.exe" }
if ([string]::IsNullOrWhiteSpace($ManagerPath)) { $ManagerPath = Join-Path $repo "release\setup\CodexAppManager.exe" }
if ([string]::IsNullOrWhiteSpace($EntryShimPath)) { $EntryShimPath = Join-Path $repo "release\setup\CodexEntryShim.exe" }
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
    if ([string]::IsNullOrWhiteSpace($UsbSigningProfilePath)) {
        throw "SignedRequired 正式 U 盘包必须显式传入当前版本 UsbSigningProfilePath；不会使用仓库中的固定 profile"
    }
    $signingProfile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $UsbSigningProfilePath -RequireExplicitPath
    Assert-CodexUsbSigningProfileVersion -Profile $signingProfile -ExpectedVersion $releaseVersion
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
        releaseVersion = [string]$signingProfile.ReleaseVersion
        releaseId = [string]$signingProfile.ReleaseId
        keyId = $publicKeyId
        publicKeySha256 = $publicKeySha256
    }
} else {
    $signingBinding = [ordered]@{
        policy = "DevelopmentAutoEnroll"
        releaseVersion = $null
        releaseId = $null
        keyId = $null
        publicKeySha256 = $null
    }
}
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$manager = (Resolve-Path -LiteralPath $ManagerPath).Path
$entryShim = (Resolve-Path -LiteralPath $EntryShimPath).Path
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path
if ((Get-Item -LiteralPath $entryShim).Name -ine "CodexEntryShim.exe") {
    throw "EntryShimPath 必须指向名为 CodexEntryShim.exe 的 direct-entry shim"
}

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

function Get-JsonPropertyValue($Object, [string]$Name) {
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Test-IntegerValue([object]$Value) {
    return $null -ne $Value -and (
        $Value -is [sbyte] -or $Value -is [byte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64]
    )
}

function Get-JsonInteger([object]$Object, [string]$Name) {
    if ($null -eq $Object) {
        return $null
    }
    if ($Object -is [System.Collections.IDictionary]) {
        if (-not $Object.Contains($Name)) {
            return $null
        }
        $value = $Object[$Name]
    } else {
        $property = $Object.PSObject.Properties[$Name]
        if ($null -eq $property) {
            return $null
        }
        $value = $property.Value
    }
    if (-not (Test-IntegerValue $value)) {
        return $null
    }
    return $value
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
    if (Test-Path -LiteralPath (Join-Path $RootPath "usb-license-revocation-registration.json")) {
        throw "静态发行目录不得携带登记声明 sidecar"
    }
    foreach ($entry in Get-ChildItem -LiteralPath $RootPath -Recurse -Force) {
        $relative = (Get-RelativePathCompat $RootPath $entry.FullName).Replace('\', '/')
        if ($entry.Name -ieq "usb-license-revocation-registration.json" -or $relative -imatch '^CodexUsbState(/|$)') {
            throw "静态发行目录不得携带登记声明或作废运行期状态：$relative"
        }
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
    -EntryShimPath $entryShim `
    -BundlePath $bundle `
    -OutputPath $OutputPath `
    -CapabilityPackSource $CapabilityPackSource `
    -InternalSigned:$InternalSigned `
    -SkipSignature:$SkipSignature
# The nested PowerShell script communicates failures through terminating
# errors.  `$LASTEXITCODE` is intentionally not inspected here because it can
# retain the exit code of a previous native command in the parent session.

$output = (Resolve-Path -LiteralPath $OutputPath).Path
# Keep the two packaged CCSwitch manifest copies aligned with the canonical
# repository resource. The launcher-side resource directory can be an older
# staging snapshot even when the executable and provider catalog are current.
$canonicalCcswitchManifest = Join-Path $repo "src-tauri\resources\ccswitch\manifest.json"
if (-not (Test-Path -LiteralPath $canonicalCcswitchManifest -PathType Leaf)) {
    throw "缺少当前 CCSwitch 资源清单：$canonicalCcswitchManifest"
}
foreach ($relativeCcswitchManifest in @("resources\ccswitch\manifest.json", "ccswitch\manifest.json")) {
    $destinationManifest = Join-Path $output $relativeCcswitchManifest
    if (-not (Test-Path -LiteralPath (Split-Path -Parent $destinationManifest) -PathType Container)) {
        throw "U 盘输出缺少 CCSwitch 清单目录：$destinationManifest"
    }
    Copy-Item -LiteralPath $canonicalCcswitchManifest -Destination $destinationManifest -Force
    $manifestBytes = [System.IO.File]::ReadAllBytes($destinationManifest)
    if ($manifestBytes.Length -ge 3 -and
        $manifestBytes[0] -eq 0xEF -and
        $manifestBytes[1] -eq 0xBB -and
        $manifestBytes[2] -eq 0xBF) {
        throw "CCSwitch manifest.json 必须使用无 BOM 的 UTF-8 编码：$destinationManifest"
    }
    $sourceHash = (Get-FileHash -LiteralPath $canonicalCcswitchManifest -Algorithm SHA256).Hash
    $destinationHash = (Get-FileHash -LiteralPath $destinationManifest -Algorithm SHA256).Hash
    if ($sourceHash -ine $destinationHash) {
        throw "U 盘包中的 CCSwitch 清单与当前资源不一致：$relativeCcswitchManifest"
    }
}
$manifestPath = Join-Path $output "manifest.json"
$manifest = Read-JsonFile $manifestPath
[void](Add-Member -InputObject $manifest -MemberType NoteProperty -Name "schemaVersion" -Value 3 -Force)
[void](Add-Member -InputObject $manifest -MemberType NoteProperty -Name "directEntryPolicy" -Value ([ordered]@{
    required = $true
    shim = "CodexEntryShim.exe"
    route = "same-user-session named pipe to USB manager"
    launchGate = "signed local license plus fresh signed revocation status when online"
    offline = "allow only when the local license is valid and no signed revoked state is stored"
    failure = "closed"
}) -Force)
[void](Add-Member -InputObject $manifest -MemberType NoteProperty -Name "revocationPolicy" -Value ([ordered]@{
    registrationRequired = $true
}) -Force)
if ((Get-JsonInteger $manifest "schemaVersion") -ne 3 -or
    $manifest.revocationPolicy.registrationRequired -ne $true) {
    throw "输出 root manifest 必须声明 schemaVersion=3 和 registrationRequired=true"
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
    (Get-JsonInteger $rootUpdateCapability "schemaVersion") -ne 3 -or
    (Get-JsonInteger $rootUpdateCapability "anchorSchema") -ne 3) {
    throw "输出 root manifest 未声明 Codex payload update capability"
}
[System.Text.UTF8Encoding]$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), $utf8NoBom)
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
Copy-Item -LiteralPath $entryShim -Destination (Join-Path $output "CodexEntryShim.exe") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-payload.ps1") -Destination (Join-Path $output "verify-payload.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-oneclick-usb.ps1") -Destination (Join-Path $output "verify-usb.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-usb-launcher.ps1") -Destination (Join-Path $output "verify-usb-launcher.ps1") -Force
$verificationOutput = Join-Path $output "verification"
New-Item -ItemType Directory -Force -Path $verificationOutput | Out-Null
Copy-Item -LiteralPath (Join-Path $repo "tools\usb-signing-profile.ps1") -Destination (Join-Path $verificationOutput "usb-signing-profile.ps1") -Force
Copy-Item -LiteralPath (Join-Path $repo "tools\verify-capability-pack-powershell.ps1") -Destination (Join-Path $verificationOutput "verify-capability-pack-powershell.ps1") -Force
if ($null -ne $signingProfile) {
    # Only emit the public verification fields. Never copy the source profile
    # verbatim: a future local signing profile may contain private or
    # operational metadata that must not enter a portable release directory.
    $verificationProfileJson = [ordered]@{
        schemaVersion = 1
        profile = "production"
        releaseVersion = [string]$signingProfile.ReleaseVersion
        releaseId = [string]$signingProfile.ReleaseId
        keyId = [string]$signingProfile.KeyId
        publicKeyB64 = [string]$signingProfile.PublicKeyB64
        publicKeySha256 = [string]$signingProfile.PublicKeySha256
    } | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText(
        (Join-Path $verificationOutput "usb-signing-release-profile.json"),
        $verificationProfileJson,
        $utf8NoBom
    )
}
$launcherVerifier = Join-Path $output "verify-usb-launcher.ps1"
& $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path (Join-Path $output "CodexAppManager.exe") -ExpectedLevel requireAdministrator
if ($LASTEXITCODE -ne 0) {
    throw "CodexAppManager.exe 未通过 USB requireAdministrator 清单验证"
}
& $windowsPowerShell -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path (Join-Path $output "CodexEntryShim.exe") -ExpectedLevel asInvoker
if ($LASTEXITCODE -ne 0) {
    throw "CodexEntryShim.exe 未通过 asInvoker 清单验证"
}
# Tauri deliberately does not ship PowerShell files through the Windows
# resource bundle. Keep the verifier in the user-visible root ccswitch
# directory so offline diagnostics still work on a blank machine. Prefer the
# current source/resource verifier; an older sibling release must not silently
# overwrite a newer CCSwitch contract.
$ccswitchVerifierCandidates = @(
    (Join-Path $repo "src-tauri\resources\ccswitch\verify-product.ps1"),
    (Join-Path $output "resources\ccswitch\verify-product.ps1"),
    (Join-Path (Split-Path -Parent $repo) "release\ccswitch-product\verify-product.ps1")
)
$ccswitchVerifier = $ccswitchVerifierCandidates |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if ($ccswitchVerifier) {
    $ccswitchVerifierDestination = Join-Path $output "ccswitch\verify-product.ps1"
    if ((Resolve-Path -LiteralPath $ccswitchVerifier).Path -ine (Resolve-Path -LiteralPath $ccswitchVerifierDestination).Path) {
        Copy-Item -LiteralPath $ccswitchVerifier -Destination $ccswitchVerifierDestination -Force
    }
}

$installerItem = Get-Item -LiteralPath (Join-Path $output "CodexDesktopSetup.exe")
$managerItem = Get-Item -LiteralPath (Join-Path $output "CodexAppManager.exe")
$entryShimItem = Get-Item -LiteralPath (Join-Path $output "CodexEntryShim.exe")
if ($UsbPolicy -eq "SignedRequired") {
    Assert-CompiledUsbManagerPolicy $managerItem.FullName $UsbPolicy
    Assert-CompiledUsbManagerBinding $managerItem.FullName $publicKeyId $publicKeyB64 $publicKeySha256
}
if ($UsbPolicy -eq "DevelopmentAutoEnroll") {
    Assert-CompiledUsbManagerPolicy $managerItem.FullName $UsbPolicy
}
# Compare both catalog copies against the canonical source before hashing
# the release. Binary/package hashes alone do not prove config consistency.
foreach ($relativeCatalog in @("ccswitch\providers.json", "resources\ccswitch\providers.json")) {
    & (Join-Path $repo "tools\verify-model-provider-resource.ps1") -ArtifactPath (Join-Path $output $relativeCatalog)
}
$ccswitchItem = Get-Item -LiteralPath (Join-Path $output "ccswitch\CodexSwitch.exe")
$embeddedCcswitchItem = Get-Item -LiteralPath (Join-Path $output "resources\ccswitch\CodexSwitch.exe")
$vcRedistItem = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\vc_redist.x64.exe")
$webView2Item = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
$skillHubSourcesItem = Get-Item -LiteralPath (Join-Path $output "resources\skill_hub_sources.json") -ErrorAction Stop
$skillHubSources = Read-JsonFile $skillHubSourcesItem.FullName
if ((Get-JsonInteger $skillHubSources "schemaVersion") -ne 1 -or [string]$skillHubSources.channel -cne "skill-hub") {
    throw "U 盘包中的 Skill Hub 来源目录 schema/channel 无效"
}
$skillHubRelay = @($skillHubSources.sources | Where-Object { [string]$_.id -ceq "ph-relay" -and [string]$_.kind -ceq "relay" })
$skillHubGithub = @($skillHubSources.sources | Where-Object { [string]$_.id -ceq "github" -and [string]$_.kind -ceq "github" })
if ($skillHubRelay.Count -ne 1 -or $skillHubRelay[0].enabled -ne $true -or [string]$skillHubRelay[0].base -cne "https://update.404token.xyz/__codex_update__/skill-hub/" -or
    $skillHubGithub.Count -ne 1 -or $skillHubGithub[0].enabled -ne $true -or $null -ne (Get-JsonPropertyValue $skillHubGithub[0] "base")) {
    throw "U 盘包中的 Skill Hub Relay/GitHub 来源身份无效"
}
$baseManifestItem = Get-Item -LiteralPath (Join-Path $output "resources\skill_repo_base_manifest.json") -ErrorAction Stop
$baseManifest = Read-JsonFile $baseManifestItem.FullName
$baseSourcePath = Join-Path $output "content\codex-starter-kit\manifest\skill-audit.json"
$baseSourceItem = Get-Item -LiteralPath $baseSourcePath -ErrorAction Stop
$baseSourceHash = (Get-FileHash -LiteralPath $baseSourceItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$baseSkills = @($baseManifest.skills)
$sourceManifest = Read-JsonFile $baseSourceItem.FullName
$sourceSkills = @($sourceManifest.skills)
if ((Get-JsonInteger $baseManifest "schemaVersion") -ne 1 -or
    [string]$baseManifest.source -cne "installer-pack/manifest/skill-audit.json" -or
    [string]$baseManifest.sourceSha256 -cne $baseSourceHash -or
    $baseSkills.Count -eq 0 -or $baseSkills.Count -ne $sourceSkills.Count) {
    throw "U 盘包中的基础 Skill ID 清单 schema/hash/数量无效"
}
for ($index = 0; $index -lt $sourceSkills.Count; $index++) {
    if ([string]$baseSkills[$index].id -cne [string]$sourceSkills[$index].id) {
        throw "U 盘包中的基础 Skill ID 顺序与源清单不一致：index=$index"
    }
}
Write-Output "基础 Skill ID 清单已随 U 盘包打包并通过身份/ID 顺序门禁：$($baseSkills.Count) 个"
if (-not $SkipSignature -and -not $InternalSigned) {
    Assert-AuthenticodePe $ccswitchItem.FullName "CCSwitch"
    Assert-AuthenticodePe $embeddedCcswitchItem.FullName "embedded CCSwitch"
}
$installerHash = (Get-FileHash -LiteralPath $installerItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$managerHash = (Get-FileHash -LiteralPath $managerItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$entryShimHash = (Get-FileHash -LiteralPath $entryShimItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$payloadHash = (Get-FileHash -LiteralPath $payloadItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$ccswitchHash = (Get-FileHash -LiteralPath $ccswitchItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$vcRedistHash = (Get-FileHash -LiteralPath $vcRedistItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$webView2Hash = (Get-FileHash -LiteralPath $webView2Item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($webView2Hash -ine "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D") {
    throw "WebView2 离线安装包哈希与管理器内置值不符，拒绝生成发布包"
}

$usbManifest = [ordered]@{
    schemaVersion = 3
    product = "codex-usb-oneclick"
    releaseVersion = $releaseVersion
    status = "oneclick"
    updateCapability = [ordered]@{
        codexPayload = $true
        schemaVersion = 3
        anchorSchema = 3
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
    entryShim = [ordered]@{
        file = "CodexEntryShim.exe"
        size = [int64]$entryShimItem.Length
        sha256 = $entryShimHash
        role = "asInvoker Codex manifest-entry shim; every launch is handed to manager authorization"
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
        # Keep the legacy fields above because the existing signed-license
        # contract validates them.  They are descriptive compatibility
        # history, not the command line used by the current manager.  The
        # effective USB runtime policy is recorded explicitly below.
        runtimeInitialMode = "portable"
        runtimeInitialArguments = @()
        runtimeFallbackMode = "software-rendering-no-sandbox"
        runtimeFallbackArguments = @("--disable-gpu", "--no-sandbox")
        runtimeFallbackOnlyAfterNoUsableWindow = $true
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
        "On first launch the manager passes the signed USB account binding to CCSwitch through the legacy startup environment. CCSwitch stores the 404TOKEN API key in its normal local provider database and activates the provider through its standard switch path; the manager does not wait for an acknowledgement file. After a usable window appears, CCSwitch is minimized once into the tray. Later use of the explicit Open CCSwitch button restores the window and does not minimize it again. A missing signed binding or launch failure is shown on the manager home page. The user can later switch providers and models in CCSwitch. Installation, Codex launch, and provider switching do not require a Manager model test. The Manager's Advanced gateway probe is optional and read-only: it does not apply a model or change configuration. A full matrix sends up to 12 generation requests per selected model (up to three models), may consume upstream quota, and requires separate user cost confirmation.",
        "If an older CCSwitch process already owns its single-instance localhost port, close that old process and use the manager retry entry; the manager reports the conflict instead of treating a refused localhost connection as a successful launch.",
        "In the CCSwitch Codex provider editor, Refresh models calls the current provider's /v1/models endpoint. It keeps every vendor, collapses only stable/latest/valid-date release aliases, and preserves the selected raw upstream model ID.",
        "Codex is launched only by the authorized USB manager with CODEX_HOME fixed to %USERPROFILE%/.codex. The effective USB runtime policy starts with normal hardware rendering (no --disable-gpu) and automatically retries the proven software-rendering/no-sandbox fallback only if no stable usable window appears. Removing the USB triggers the existing five-second guarded shutdown.",
        "Ordinary uninstall removes <bundle>/apps/Codex and preserves local user data. Advanced purge remains a separate explicit confirmation. Source-media deletion is allowed only after the runtime tree has been removed, immutable release files still match SHA256SUMS.txt, and usb-license.json plus its atomic backup match the active lease fingerprint and handoff snapshot."
    )
    secretsIncluded = $false
    automaticPostInstallLaunch = $true
    notes = @(
        "The root manifest.json is the Codex installer manifest and must not be replaced by usb-manifest.json.",
        "The CCSwitch launch is best-effort; a missing payload does not roll back a successful Codex installation.",
        "Executable bytes are installed only under apps/Codex on the removable drive. Mutable Codex state uses the normal Windows profile (normally C:); a declared or canonicalized profile path resolving to the product USB blocks installation.",
        "CodexDesktopSetup.exe is a tiny USB-only launcher with an embedded requireAdministrator manifest; after the user's standard UAC approval it starts the USB-specific CodexAppManager.exe, which also embeds requireAdministrator as a defence-in-depth boundary. The normal installed manager build remains asInvoker. Declining UAC, lacking administrator credentials, or an enterprise policy that blocks elevation cannot be bypassed.",
        "The legacy codexLaunch.initialMode/initialArguments fields remain for compatibility with the existing signed-license contract. The manager ignores those descriptive fields and follows codexLaunch.runtimeInitialMode/runtimeInitialArguments: normal GPU first, then the bounded software-rendering/no-sandbox fallback.",
        "The manager entry point is statically linked to the MSVC CRT so it can start before the prerequisite is present. The bundled vc_redist.x64.exe is the unmodified Microsoft installer and is hash-pinned; no DLL is copied directly to System32.",
        "No AppX external-volume registry policy, developer mode or sideload policy is required for the portable route. Enterprise execution-control restrictions are diagnosed, not bypassed.",
        "The content/codex-starter-kit folder contains the complete offline capability pack. The manager automatically installs or repairs its Skills, Agents, runtimes and local MCP resources after Codex is verified; external connections remain opt-in and their secrets stay in the local Windows user environment.",
        "Ordinary uninstall preserves the user's independent .codex, Roaming Codex and Local Codex data. Advanced purge is destructive and may affect an official/CLI Codex installation sharing those paths.",
        "usb-license.json is manager-created mutable USB identity metadata, not an immutable SHA256SUMS entry. The detached finalizer validates it against the active lease and deletes it last; unrelated or modified files are listed and preserved.",
        "Unsigned development bundles auto-enroll on first run; strict anti-copy releases require a pre-provisioned signed usb-license.json."
    )
}
if ((Get-JsonInteger $usbManifest "schemaVersion") -ne 3 -or
    (Get-JsonInteger $usbManifest.updateCapability "schemaVersion") -ne 3 -or
    (Get-JsonInteger $usbManifest.updateCapability "anchorSchema") -ne 3 -or
    (Get-JsonInteger $usbManifest.sourceMediaCleanup "planSchema") -ne 3) {
    throw "输出 usb-manifest 的 schema/capability 数字字段无效"
}
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

# The verifier is part of the deliverable. Run the copied version from the
# generated bundle, not the source-tree script, so a missing bundled helper or
# profile fails the build instead of surfacing only after the USB is moved.
$standaloneVerifier = Join-Path $output "verify-usb.ps1"
$standaloneVerifyArgs = @(
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", $standaloneVerifier,
    "-SkipNetwork"
)
if ($UsbPolicy -eq "SignedRequired") {
    $standaloneVerifyArgs += @(
        "-InternalSigned",
        "-UsbSigningProfilePath", (Join-Path $verificationOutput "usb-signing-release-profile.json")
    )
} elseif ($SkipSignature) {
    $standaloneVerifyArgs += "-SkipSignature"
}
$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $standaloneVerification = & $windowsPowerShell @standaloneVerifyArgs 2>&1 | Out-String
    $standaloneVerificationExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($standaloneVerificationExitCode -ne 0) {
    throw "生成后的随盘 verify-usb.ps1 自检失败：$standaloneVerification"
}

Write-Output "已生成 Codex 一键 U 盘目录：$output"
Write-Output "入口：$(Join-Path $output 'CodexDesktopSetup.exe')"
