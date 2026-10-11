[CmdletBinding()]
param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$CcswitchArtifactDir = "",
    [string]$OutputDir = "",
    [ValidateSet("SignedRequired", "DevelopmentAutoEnroll")]
    [string]$UsbPolicy = "SignedRequired",
    [string]$UsbSigningProfilePath = "",
    [switch]$SkipNpmInstall,
    [switch]$SkipTests,
    [switch]$FullTests
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$defaultSetupDir = [string]::IsNullOrWhiteSpace($OutputDir)
if ($defaultSetupDir) {
    $setupDirPath = Join-Path $repo "release\setup"
} else {
    $outputCandidate = if ([System.IO.Path]::IsPathRooted($OutputDir)) { $OutputDir } else { Join-Path $repo $OutputDir }
    $setupDirPath = [System.IO.Path]::GetFullPath($outputCandidate)
    $releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $repo "release")).TrimEnd('\') + '\'
    if (-not $setupDirPath.StartsWith($releaseRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "OutputDir 必须位于仓库 release 目录内：$setupDirPath"
    }
    if (Test-Path -LiteralPath $setupDirPath) {
        throw "版本化输出目录已存在，为避免覆盖产物而停止：$setupDirPath"
    }
}
$previousUsbPolicy = $env:CODEX_USB_GUARD_POLICY
Push-Location $repo
try {
    . (Join-Path $repo "tools\usb-signing-profile.ps1")
    # This script produces the USB manager artifact.  Formal mode is the
    # default and must be selected explicitly as DevelopmentAutoEnroll when a
    # local unsigned fixture is intended; an inherited environment variable
    # must never silently downgrade a release build.
    $env:CODEX_USB_GUARD_POLICY = $UsbPolicy
    $signedRequired = $UsbPolicy -eq "SignedRequired"
    if ($signedRequired) {
        $packageVersion = [string]((Get-Content -LiteralPath (Join-Path $repo "package.json") -Raw | ConvertFrom-Json).version)
        if ([string]::IsNullOrWhiteSpace($UsbSigningProfilePath)) {
            throw "SignedRequired 正式构建必须显式传入当前版本 UsbSigningProfilePath；不会使用仓库中的固定 profile"
        }
        $signingProfile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $UsbSigningProfilePath -RequireExplicitPath
        Assert-CodexUsbSigningProfileVersion -Profile $signingProfile -ExpectedVersion $packageVersion
        Initialize-CodexUsbSigningEnvironment -Profile $signingProfile
        $publicKeyId = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_ID
        $publicKeyB64 = [string]$env:CODEX_USB_GUARD_PUBLIC_KEY_B64
        if ([string]::IsNullOrWhiteSpace($publicKeyId) -or
            [string]::IsNullOrWhiteSpace($publicKeyB64)) {
            throw "SignedRequired 构建必须配置 CODEX_USB_GUARD_PUBLIC_KEY_ID 和 CODEX_USB_GUARD_PUBLIC_KEY_B64"
        }
        try {
            $keyBytes = [Convert]::FromBase64String($publicKeyB64.Trim())
        } catch {
            throw "SignedRequired 构建的 CODEX_USB_GUARD_PUBLIC_KEY_B64 不是有效 Base64"
        }
        if ($keyBytes.Length -ne 32) {
            throw "SignedRequired 构建的 USB 公钥必须是 32 字节"
        }
        Write-Output "USB 签名强制策略已启用（keyId=$publicKeyId；公钥内容不输出）"
    }
    $ccswitchArtifactExplicit = -not [string]::IsNullOrWhiteSpace($CcswitchArtifactDir)
    if ([string]::IsNullOrWhiteSpace($CcswitchArtifactDir)) {
        # Prefer the resource set already staged and verified with this checkout.
        # A stale workspace-level release directory may still contain a valid EXE
        # that predates the USB handoff protocol required by this manager.
        $embeddedCcswitchDir = Join-Path $repo "src-tauri\resources\ccswitch"
        $embeddedCcswitchSource = Join-Path $embeddedCcswitchDir "CodexSwitch.exe"
        if ((Test-Path -LiteralPath $embeddedCcswitchSource -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $embeddedCcswitchDir "verify-product.ps1") -PathType Leaf)) {
            $CcswitchArtifactDir = $embeddedCcswitchDir
        } else {
            $workspace = (Split-Path -Parent $repo)
            $CcswitchArtifactDir = Join-Path $workspace "release\ccswitch-product"
        }
    }
    $ccswitchSource = Join-Path $CcswitchArtifactDir "CodexSwitch.exe"
    $ccswitchResourceDir = Join-Path $repo "src-tauri\resources\ccswitch"
    if (-not (Test-Path -LiteralPath $ccswitchSource -PathType Leaf) -and -not $ccswitchArtifactExplicit) {
        $embeddedCcswitchDir = Join-Path $repo "src-tauri\resources\ccswitch"
        $embeddedCcswitchSource = Join-Path $embeddedCcswitchDir "CodexSwitch.exe"
        if (Test-Path -LiteralPath $embeddedCcswitchSource -PathType Leaf) {
            $CcswitchArtifactDir = $embeddedCcswitchDir
            $ccswitchSource = $embeddedCcswitchSource
            Write-Warning "未找到外部 CCSwitch 产物，改用仓库内已校验的资源：$CcswitchArtifactDir"
        }
    }
    $vcRedistPath = Join-Path $repo "src-tauri\resources\prerequisites\vc_redist.x64.exe"
    $vcRedistExpectedSha256 = "843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C"
    if (-not (Test-Path -LiteralPath $vcRedistPath -PathType Leaf)) {
        throw "未找到离线 VC++ x64 运行库：$vcRedistPath。请将未修改的 Microsoft vc_redist.x64.exe 放入该目录。"
    }
    $vcRedistHash = (Get-FileHash -LiteralPath $vcRedistPath -Algorithm SHA256).Hash
    if ($vcRedistHash -ine $vcRedistExpectedSha256) {
        throw "VC++ x64 运行库哈希不匹配：$vcRedistHash != $vcRedistExpectedSha256"
    }
    $vcRedistSignature = Get-AuthenticodeSignature -LiteralPath $vcRedistPath
    if ($vcRedistSignature.Status -ne "Valid" -or
        $vcRedistSignature.SignerCertificate.Subject -notmatch "Microsoft Corporation") {
        throw "VC++ x64 运行库必须保持 Microsoft Authenticode 有效签名；当前状态=$($vcRedistSignature.Status)，签名者=$($vcRedistSignature.SignerCertificate.Subject)"
    }
    Write-Output "VC++ x64 离线运行库已校验：SHA256=$vcRedistExpectedSha256，签名者为 Microsoft Corporation"
    # The raw USB manager skips NSIS, so no online WebView2 bootstrapper will
    # ever run. Ship and verify the full Evergreen standalone x64 installer.
    $webView2Path = Join-Path $repo "src-tauri\resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
    $webView2ExpectedSha256 = "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D"
    if (-not (Test-Path -LiteralPath $webView2Path -PathType Leaf)) {
        throw "缺少 WebView2 x64 完整离线安装包：$webView2Path（不要使用约 2 MB 的在线 Bootstrapper）"
    }
    $webView2Hash = (Get-FileHash -LiteralPath $webView2Path -Algorithm SHA256).Hash
    if ($webView2Hash -ine $webView2ExpectedSha256) { throw "WebView2 离线安装包哈希不匹配：$webView2Hash" }
    $webView2Signature = Get-AuthenticodeSignature -LiteralPath $webView2Path
    if ($webView2Signature.Status -ne "Valid" -or $webView2Signature.SignerCertificate.Subject -notmatch "Microsoft Corporation") {
        throw "WebView2 离线安装包必须保持 Microsoft 有效签名：$($webView2Signature.Status)"
    }
    Write-Output "WebView2 x64 完整离线安装包已通过哈希和 Microsoft 签名校验"
    if (-not (Test-Path -LiteralPath $ccswitchSource -PathType Leaf)) {
        throw "未找到 CodexSwitch.exe：$ccswitchSource。请先构建 CCSwitch 产品版，或通过 -CcswitchArtifactDir 指定产物目录。"
    }
    $ccswitchBinaryText = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($ccswitchSource))
    foreach ($handoffMarker in @(
        "CODEX_USB_404TOKEN_HANDOFF_V1",
        "CCSWITCH_404TOKEN_HANDOFF_PATH",
        "CCSWITCH_404TOKEN_HANDOFF_NONCE",
        "CCSWITCH_404TOKEN_DEVICE_KEY"
    )) {
        if (-not $ccswitchBinaryText.Contains($handoffMarker)) {
            throw "CCSwitch EXE 缺少确认式 404token API Key 注入协议标记 $handoffMarker；拒绝继续打包：$ccswitchSource"
        }
    }
    $ccswitchVerifier = Join-Path $CcswitchArtifactDir "verify-product.ps1"
    if (-not (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf)) {
        throw "CCSwitch 产物缺少必需的 verify-product.ps1；拒绝打包未经确认式 API Key 注入校验的版本"
    }
    # This verifier checks the catalog, renderer, and the compiled persistence
    # acknowledgement marker before the offline installer consumes the EXE.
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ccswitchVerifier -ArtifactDir $CcswitchArtifactDir -SkipNetwork
    if ($LASTEXITCODE -ne 0) { throw "CCSwitch 产品版验证失败，拒绝打包未验证的版本" }
    New-Item -ItemType Directory -Force -Path $ccswitchResourceDir | Out-Null
    foreach ($fileName in @("CodexSwitch.exe", "providers.json", "manifest.json", "LICENSE", "SOURCE-NOTICE.txt", "verify-product.ps1")) {
        $sourceFile = Join-Path $CcswitchArtifactDir $fileName
        if (Test-Path -LiteralPath $sourceFile -PathType Leaf) {
            $destinationFile = Join-Path $ccswitchResourceDir $fileName
            # Allow a checkout's already-staged CCSwitch resource directory to
            # be used as the artifact source; Copy-Item rejects a self-copy.
            if ((Resolve-Path -LiteralPath $sourceFile).Path -ne (Resolve-Path -LiteralPath $destinationFile -ErrorAction SilentlyContinue).Path) {
                Copy-Item -LiteralPath $sourceFile -Destination $destinationFile -Force
            }
        }
    }
    Write-Output "CCSwitch 资源已暂存：$ccswitchResourceDir"
    if (-not $SkipNpmInstall) {
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci 失败" }
    }
    if (-not $SkipTests) {
        & npm.cmd run check
        if ($LASTEXITCODE -ne 0) { throw "TypeScript 检查失败" }
        & npm.cmd run lint
        if ($LASTEXITCODE -ne 0) { throw "前端 lint 失败" }
        # The product build only needs deterministic renderer tests by default.
        # The upstream release-workflow tests create WSL-backed git fixtures and
        # are available explicitly via -FullTests for release CI.
        & npm.cmd test -- --run src
        if ($LASTEXITCODE -ne 0) { throw "前端测试失败" }
        if ($FullTests) {
            & npm.cmd test -- --run
            if ($LASTEXITCODE -ne 0) { throw "完整前端测试失败" }
        }
        & cargo.exe test --manifest-path src-tauri\Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw "Rust 测试失败" }
    }
    # `cargo build` consumes the already-built Vite renderer from `dist`; make
    # that dependency explicit so a clean checkout cannot accidentally package
    # a stale development UI.  Vite's production mode enables the fixed USB
    # Windows screen, while the runtime dispatcher still keeps macOS online.
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw "前端生产构建失败" }
    # Build the Tauri application binary itself.  The USB manager is built as a
    # separate requireAdministrator artifact.  The ordinary source build keeps
    # CODEX_USB_MANAGER unset and therefore remains asInvoker for installed use.
    # next to manifest.json/payload at runtime; a normal NSIS setup EXE would
    # move the running process into Program Files before our offline probe.
    # `custom-protocol` is required for an offline release.  Without it,
    # Tauri deliberately keeps `cfg(dev)` enabled and opens build.devUrl
    # (127.0.0.1:1420), which only exists while `tauri dev` is running.
    $previousUsbManagerFlag = $env:CODEX_USB_MANAGER
    try {
        $env:CODEX_USB_MANAGER = "1"
        $usbFeatures = if ($signedRequired) { "custom-protocol,signed-required" } else { "custom-protocol" }
        & cargo.exe build --release --features $usbFeatures --manifest-path src-tauri\Cargo.toml --target $Target
        if ($LASTEXITCODE -ne 0) { throw "Tauri USB 管理器构建失败" }
    } finally {
        if ($null -eq $previousUsbManagerFlag) {
            Remove-Item Env:CODEX_USB_MANAGER -ErrorAction SilentlyContinue
        } else {
            $env:CODEX_USB_MANAGER = $previousUsbManagerFlag
        }
    }
    $releaseDir = Join-Path $repo "src-tauri\target\$Target\release"
    $binary = Join-Path $releaseDir "codex-app-manager.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "未找到 Tauri 应用二进制：$binary"
    }
    $setupDir = (New-Item -ItemType Directory -Force -Path $setupDirPath).FullName
    $entryShimProject = Join-Path $repo "tools\codex-entry-shim\Cargo.toml"
    $entryShimBinary = Join-Path $repo "tools\codex-entry-shim\target\$Target\release\codex-entry-shim.exe"
    & cargo.exe build --release --manifest-path $entryShimProject --target $Target
    if ($LASTEXITCODE -ne 0) { throw "USB Codex direct-entry shim 构建失败" }
    if (-not (Test-Path -LiteralPath $entryShimBinary -PathType Leaf)) {
        throw "未找到 USB Codex direct-entry shim：$entryShimBinary"
    }
    $entryShim = Join-Path $setupDir "CodexEntryShim.exe"
    Copy-Item -LiteralPath $entryShimBinary -Destination $entryShim -Force
    # Rust's MSVC linker does not consistently emit the shim's requested
    # execution level as an RT_MANIFEST resource.  Embed the checked-in
    # asInvoker manifest explicitly into the copied release artifact before
    # verifying it.  The shim itself must remain unelevated; it delegates
    # privileged USB work to CodexAppManager through the named pipe.
    $shimManifest = Join-Path $repo "tools\codex-entry-shim\as-invoker.manifest"
    if (-not (Test-Path -LiteralPath $shimManifest -PathType Leaf)) {
        throw "缺少 USB Codex direct-entry shim 的 asInvoker 清单：$shimManifest"
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    $programFiles = [Environment]::GetEnvironmentVariable("ProgramFiles")
    $windowsKitRoots = @(
        $(if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) { Join-Path $programFilesX86 "Windows Kits\10\bin" }),
        $(if (-not [string]::IsNullOrWhiteSpace($programFiles)) { Join-Path $programFiles "Windows Kits\10\bin" })
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and (Test-Path -LiteralPath $_) }
    $mtCandidates = foreach ($kitRoot in $windowsKitRoots) {
        Get-ChildItem -LiteralPath $kitRoot -Recurse -Filter "mt.exe" -File -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match "\\x64\\mt\.exe$" }
    }
    $mt = $mtCandidates | Sort-Object FullName -Descending | Select-Object -First 1
    if ($null -eq $mt) {
        throw "未找到 Windows SDK mt.exe，无法嵌入 USB Codex direct-entry shim 清单"
    }
    $shimManifestWritten = $false
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        & $mt.FullName -manifest $shimManifest ("-outputresource:{0};#1" -f $entryShim)
        if ($LASTEXITCODE -eq 0) {
            $shimManifestWritten = $true
            break
        }
        if ($attempt -lt 5) {
            Write-Warning "mt.exe 暂时无法写入 USB Codex direct-entry shim（尝试 $attempt/5），稍后重试"
            Start-Sleep -Milliseconds (500 * $attempt)
        }
    }
    if (-not $shimManifestWritten) {
        throw "无法把 asInvoker 清单写入 USB Codex direct-entry shim：$entryShim"
    }
    $launcherVerifier = Join-Path $repo "tools\verify-usb-launcher.ps1"
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path $entryShim -ExpectedLevel asInvoker
    if ($LASTEXITCODE -ne 0) { throw "USB Codex direct-entry shim 未通过 asInvoker 清单验证" }
    Write-Output "USB Codex direct-entry shim：$entryShim（asInvoker）"
    # The copied manager is the USB-specific requireAdministrator build.  The
    # tiny launcher remains the friendly entry point, but a direct/legacy launch
    # cannot accidentally run AppX operations with a filtered token.
    $managerForUsb = Join-Path $setupDir "CodexAppManager.exe"
    Copy-Item -LiteralPath $binary -Destination $managerForUsb -Force
    $launcherVerifier = Join-Path $repo "tools\verify-usb-launcher.ps1"
    if (-not (Test-Path -LiteralPath $launcherVerifier -PathType Leaf)) {
        throw "缺少 USB 入口/管理器清单验证脚本：$launcherVerifier"
    }
    # Tauri embeds its own RT_MANIFEST resource, so the linker cannot replace
    # its requestedExecutionLevel.  Patch only this copied USB artifact with
    # the Windows SDK manifest tool; the release source binary remains the
    # ordinary asInvoker manager used by local/installed launches.  Fail
    # closed when the SDK is unavailable instead of shipping a manager whose
    # elevation status is ambiguous.
    $managerManifest = Join-Path $repo "tools\codex-usb-launcher\usb-launcher.manifest"
    if (-not (Test-Path -LiteralPath $managerManifest -PathType Leaf)) {
        throw "缺少 USB requireAdministrator 清单：$managerManifest"
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    $programFiles = [Environment]::GetEnvironmentVariable("ProgramFiles")
    $windowsKitRoots = @(
        $(if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) { Join-Path $programFilesX86 "Windows Kits\10\bin" }),
        $(if (-not [string]::IsNullOrWhiteSpace($programFiles)) { Join-Path $programFiles "Windows Kits\10\bin" })
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and (Test-Path -LiteralPath $_) }
    $mtCandidates = foreach ($kitRoot in $windowsKitRoots) {
        Get-ChildItem -LiteralPath $kitRoot -Recurse -Filter "mt.exe" -File -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match "\\x64\\mt\.exe$" }
    }
    $mt = $mtCandidates | Sort-Object FullName -Descending | Select-Object -First 1
    if ($null -eq $mt) {
        throw "未找到 Windows SDK mt.exe（需要 Windows Kits\\10\\bin\\<version>\\x64\\mt.exe）"
    }
    # Copying a freshly linked PE can briefly trigger an on-access scanner.
    # `mt.exe` then reports access denied even though the file ACL is writable;
    # retry only this idempotent resource update with a short bounded delay.
    $managerManifestWritten = $false
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        & $mt.FullName -manifest $managerManifest ("-outputresource:{0};#1" -f $managerForUsb)
        if ($LASTEXITCODE -eq 0) {
            $managerManifestWritten = $true
            break
        }
        if ($attempt -lt 5) {
            Write-Warning "mt.exe 暂时无法写入 USB 管理器（尝试 $attempt/5），稍后重试"
            Start-Sleep -Milliseconds (500 * $attempt)
        }
    }
    if (-not $managerManifestWritten) {
        throw "无法把 requireAdministrator 清单写入 USB 管理器：$managerForUsb"
    }
    Write-Output "已为 USB 管理器写入 requireAdministrator 清单：$managerForUsb（$($mt.FullName)）"
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path $managerForUsb -ExpectedLevel requireAdministrator
    if ($LASTEXITCODE -ne 0) { throw "USB 管理器的嵌入式 requireAdministrator 清单验证失败" }
    $launcherProject = Join-Path $repo "tools\codex-usb-launcher\Cargo.toml"
    $launcherBinary = Join-Path $repo "tools\codex-usb-launcher\target\$Target\release\codex-usb-launcher.exe"
    & cargo.exe build --release --manifest-path $launcherProject --target $Target
    if ($LASTEXITCODE -ne 0) { throw "USB 管理员入口构建失败" }
    if (-not (Test-Path -LiteralPath $launcherBinary -PathType Leaf)) {
        throw "未找到 USB 管理员入口：$launcherBinary"
    }
    $usbLauncher = Join-Path $setupDir "CodexUsbLauncher.exe"
    Copy-Item -LiteralPath $launcherBinary -Destination $usbLauncher -Force
    # Verify the actual copied artifact, not only the Cargo source manifest.
    # This catches linker/resource regressions where a future build silently
    # falls back to the default asInvoker manifest.
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path $usbLauncher
    if ($LASTEXITCODE -ne 0) { throw "USB 管理员入口的嵌入式 requireAdministrator 清单验证失败" }
    # Keep the user-facing historical name aligned with the actual USB entry.
    # The normal Tauri manager is available separately as CodexAppManager.exe;
    # no asInvoker binary is published under the misleading setup name.
    Copy-Item -LiteralPath $usbLauncher -Destination (Join-Path $setupDir "CodexDesktopSetup.exe") -Force
    # cargo build does not guarantee a fresh target/release/resources copy.
    # Stage from the verified source resources and copy contents explicitly;
    # repeated builds must not nest resources/resources or retain old payloads.
    $resourceRoot = Join-Path $repo "src-tauri\resources"
    $setupResourceRoot = Join-Path $setupDir "resources"
    New-Item -ItemType Directory -Path $setupResourceRoot -Force | Out-Null
    Get-ChildItem -LiteralPath $resourceRoot | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $setupResourceRoot -Recurse -Force
    }
    $skillHubSourcesPath = Join-Path $setupResourceRoot "skill_hub_sources.json"
    if (-not (Test-Path -LiteralPath $skillHubSourcesPath -PathType Leaf)) {
        throw "打包资源缺少 Skill Hub 来源目录：$skillHubSourcesPath"
    }
    $skillHubSources = Get-Content -LiteralPath $skillHubSourcesPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not (Test-CodexUsbJsonInteger $skillHubSources.schemaVersion) -or
        $skillHubSources.schemaVersion -ne 1 -or [string]$skillHubSources.channel -cne "skill-hub") {
        throw "打包资源中的 Skill Hub 来源目录 schema/channel 无效"
    }
    $skillHubRelay = @($skillHubSources.sources | Where-Object { [string]$_.id -ceq "ph-relay" -and [string]$_.kind -ceq "relay" })
    $skillHubGithub = @($skillHubSources.sources | Where-Object { [string]$_.id -ceq "github" -and [string]$_.kind -ceq "github" })
    # PowerShell unwraps a one-item pipeline result during assignment.  Do
    # not read `.Count` from the result here: a single unexpected `base`
    # property must be detected just as reliably on Windows PowerShell 5.1
    # and PowerShell 7.  The GitHub source is intentionally origin-derived,
    # so any explicit base property remains a hard packaging failure.
    $skillHubGithubBase = $null
    if ($skillHubGithub.Count -eq 1) {
        $skillHubGithubBaseProperty = $skillHubGithub[0].PSObject.Properties["base"]
        if ($null -ne $skillHubGithubBaseProperty) {
            $skillHubGithubBase = $skillHubGithubBaseProperty.Value
        }
    }
    if ($skillHubRelay.Count -ne 1 -or $skillHubRelay[0].enabled -ne $true -or [string]$skillHubRelay[0].base -cne "https://update.404token.xyz/__codex_update__/skill-hub/" -or
        $skillHubGithub.Count -ne 1 -or $skillHubGithub[0].enabled -ne $true -or $null -ne $skillHubGithubBase) {
        throw "打包资源中的 Skill Hub Relay/GitHub 来源身份无效"
    }
    Write-Output "Skill Hub 来源目录已随管理器资源打包并通过身份门禁"
    $baseManifestPath = Join-Path $setupResourceRoot "skill_repo_base_manifest.json"
    if (-not (Test-Path -LiteralPath $baseManifestPath -PathType Leaf)) {
        throw "打包资源缺少基础 Skill ID 清单：$baseManifestPath"
    }
    $baseManifest = Get-Content -LiteralPath $baseManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $baseSourcePath = Join-Path $repo "installer-pack\manifest\skill-audit.json"
    if (-not (Test-Path -LiteralPath $baseSourcePath -PathType Leaf)) {
        throw "缺少基础 Skill 源清单，无法验证资源身份：$baseSourcePath"
    }
    $baseSourceHash = (Get-FileHash -LiteralPath $baseSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $baseSkills = @($baseManifest.skills)
    $sourceManifest = Get-Content -LiteralPath $baseSourcePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $sourceSkills = @($sourceManifest.skills)
    if (-not (Test-CodexUsbJsonInteger $baseManifest.schemaVersion) -or
        $baseManifest.schemaVersion -ne 1 -or
        [string]$baseManifest.source -cne "installer-pack/manifest/skill-audit.json" -or
        [string]$baseManifest.sourceSha256 -cne $baseSourceHash -or
        $baseSkills.Count -eq 0 -or $baseSkills.Count -ne $sourceSkills.Count) {
        throw "打包资源中的基础 Skill ID 清单 schema/hash/数量无效"
    }
    for ($index = 0; $index -lt $sourceSkills.Count; $index++) {
        if ([string]$baseSkills[$index].id -cne [string]$sourceSkills[$index].id) {
            throw "打包资源中的基础 Skill ID 顺序与源清单不一致：index=$index"
        }
    }
    Write-Output "基础 Skill ID 清单已随管理器资源打包并通过身份/ID 顺序门禁：$($baseSkills.Count) 个"
    Write-Output "USB 入口产物：$(Join-Path $setupDir 'CodexDesktopSetup.exe')（requireAdministrator 清单）"
    Write-Output "USB 入口别名：$usbLauncher"
    Write-Output "USB 管理器本体：$managerForUsb（requireAdministrator，入口启动或直接启动均安全）"
}
finally {
    if ($null -eq $previousUsbPolicy) {
        Remove-Item Env:CODEX_USB_GUARD_POLICY -ErrorAction SilentlyContinue
    } else {
        $env:CODEX_USB_GUARD_POLICY = $previousUsbPolicy
    }
    Pop-Location
}
