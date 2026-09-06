[CmdletBinding()]
param(
    [string]$InstallerPath = "",
    [string]$ManagerPath = "",
    [string]$BundlePath = "",
    [string]$OutputPath = "",
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$releaseVersion = [string]((Get-Content -LiteralPath (Join-Path $repo "package.json") -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($releaseVersion)) {
    throw "package.json 缺少发布版本号"
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) { $InstallerPath = Join-Path $repo "release\setup\CodexUsbLauncher.exe" }
if ([string]::IsNullOrWhiteSpace($ManagerPath)) { $ManagerPath = Join-Path $repo "release\setup\CodexAppManager.exe" }
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = Join-Path $repo "release\usb-final" }
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $repo "release\usb-oneclick" }
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

& (Join-Path $PSScriptRoot "build-usb-layout.ps1") `
    -InstallerPath $installer `
    -BundlePath $bundle `
    -OutputPath $OutputPath `
    -SkipSignature:$SkipSignature
# The nested PowerShell script communicates failures through terminating
# errors.  `$LASTEXITCODE` is intentionally not inspected here because it can
# retain the exit code of a previous native command in the parent session.

$output = (Resolve-Path -LiteralPath $OutputPath).Path
if ((Get-Item -LiteralPath $manager).Name -ine "CodexAppManager.exe") {
    throw "ManagerPath 必须指向名为 CodexAppManager.exe 的 Tauri 管理器本体"
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
$payloadItem = Get-Item -LiteralPath (Join-Path $output "payload\OpenAI.Codex_x64.msix")
$ccswitchItem = Get-Item -LiteralPath (Join-Path $output "ccswitch\CodexSwitch.exe")
$vcRedistItem = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\vc_redist.x64.exe")
$webView2Item = Get-Item -LiteralPath (Join-Path $output "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
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
    schemaVersion = 1
    product = "codex-usb-oneclick"
    releaseVersion = $releaseVersion
    status = "oneclick"
    description = "Offline Codex desktop extracted from a verified MSIX into the authorized USB bundle. Codex account, session, cache and configuration data remain in the normal local Windows profile; no external AppX volume or WpSystem data layout is used."
    codexInstaller = [ordered]@{
        file = "CodexDesktopSetup.exe"
        size = [int64]$installerItem.Length
        sha256 = $installerHash
        payload = "payload/OpenAI.Codex_x64.msix"
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
        configLocation = "%USERPROFILE%/.codex/config.toml"
        secretsIncluded = $false
        status = "layout-ready-import-later"
    }
    workflow = @(
        "Double-click CodexDesktopSetup.exe; it is the USB-only requireAdministrator launcher and asks for one standard Windows UAC confirmation before starting CodexAppManager.exe.",
        "Before the UI opens, detect WebView2 and install the hash-pinned Microsoft x64 standalone runtime from the USB if missing; runtime failures use a native dialog.",
        "The installer verifies the manifest, fixed MSIX identity, SHA-256 and Authenticode signature, checks free USB space, and observes that CODEX_HOME/APPDATA/LOCALAPPDATA do not resolve onto the authorized USB.",
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
        "The content/skills and content/mcp folders define the USB payload boundary for the next import phase. This release does not yet auto-import them. Skill payloads may later be exposed through supported discovery links; MCP executables may live on USB, but MCP configuration and secrets remain local in %USERPROFILE%/.codex/config.toml.",
        "Ordinary uninstall preserves the user's independent .codex, Roaming Codex and Local Codex data. Advanced purge is destructive and may affect an official/CLI Codex installation sharing those paths.",
        "usb-license.json is manager-created mutable USB identity metadata, not an immutable SHA256SUMS entry. The detached finalizer validates it against the active lease and deletes it last; unrelated or modified files are listed and preserved.",
        "Unsigned development bundles auto-enroll on first run; strict anti-copy releases require a pre-provisioned signed usb-license.json."
    )
}
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText((Join-Path $output "usb-manifest.json"), ($usbManifest | ConvertTo-Json -Depth 8), $utf8NoBom)

$readme = Get-Content -LiteralPath (Join-Path $repo "docs\USB-INSTALL-USER-GUIDE.txt") -Raw -Encoding UTF8
[System.IO.File]::WriteAllText((Join-Path $output "README.txt"), $readme, $utf8NoBom)

$lines = Get-ChildItem -LiteralPath $output -File -Recurse |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    ForEach-Object {
        $relative = Get-RelativePathCompat $output $_.FullName
        $relative = $relative.Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)

Write-Output "已生成 Codex 一键 U 盘目录：$output"
Write-Output "入口：$(Join-Path $output 'CodexDesktopSetup.exe')"
