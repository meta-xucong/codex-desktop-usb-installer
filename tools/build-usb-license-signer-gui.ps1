[CmdletBinding()]
param(
    [string]$OutputPath = "",
    [string]$TargetDir = "",
    [string]$SigningProfilePath = "",
    [string]$PortableVaultPath = "",
    [string]$PortableVaultDescriptorPath = "",
    [string]$Sub2ApiConfigPath = "",
    [switch]$SkipLocalKeyCheck,
    [switch]$TargetDesktopOnly,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $repo "tools\usb-signing-profile.ps1")
$packageVersion = [string]((Get-Content -LiteralPath (Join-Path $repo "package.json") -Raw | ConvertFrom-Json).version)
if ([string]::IsNullOrWhiteSpace($SigningProfilePath)) {
    throw "正式签名器构建必须显式传入当前版本 SigningProfilePath；不会使用仓库中的固定 profile"
}
$signingProfile = Get-CodexUsbSigningProfile -RepoRoot $repo -ProfilePath $SigningProfilePath -RequireExplicitPath
Assert-CodexUsbSigningProfileVersion -Profile $signingProfile -ExpectedVersion $packageVersion
$embeddedAdminApiKeyVariable = $null
foreach ($variable in @("CODEX_404TOKEN_ADMIN_API_KEY", "SUB2API_ADMIN_API_KEY")) {
    if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($variable))) {
        $embeddedAdminApiKeyVariable = $variable
        break
    }
}
$adminApiKeyEmbedded = $null -ne $embeddedAdminApiKeyVariable
$expectedAdminApiKeyLength = if ($adminApiKeyEmbedded) {
    ([Environment]::GetEnvironmentVariable($embeddedAdminApiKeyVariable, "Process") -replace "\s", "").Length
} else {
    0
}
if (-not [string]::IsNullOrWhiteSpace($Sub2ApiConfigPath)) {
    throw "正式签名器禁止通过 Sub2ApiConfigPath 旁置或复制管理员 Key；请使用 CODEX_404TOKEN_ADMIN_API_KEY 或 SUB2API_ADMIN_API_KEY 编译时内置"
}
if (-not $adminApiKeyEmbedded) {
    throw "正式签名器构建未检测到编译时管理员 Key；不会生成需要操作员手填或旁置 Key 的包"
}
$manifestPath = Join-Path $repo "tools\usb-license-signer\Cargo.toml"
$cargoTarget = Join-Path $repo "tools\usb-license-signer\target"
$buildTarget = if ([string]::IsNullOrWhiteSpace($TargetDir)) {
    $cargoTarget
} elseif ([System.IO.Path]::IsPathRooted($TargetDir)) {
    $TargetDir
} else {
    Join-Path $repo $TargetDir
}
$buildTarget = [System.IO.Path]::GetFullPath($buildTarget)
$sourceExe = Join-Path $buildTarget "release\usb-license-signer-gui.exe"
$guiReadme = Join-Path $repo "tools\usb-license-signer\README-GUI.txt"
$guiDevelopmentDoc = Join-Path $repo "docs\USB-LICENSE-SIGNER-GUI-DEVELOPMENT.md"

if (-not $SkipBuild) {
    $cargoArgs = @(
        "build",
        "--release",
        "--manifest-path",
        $manifestPath,
        "--bin",
        "usb-license-signer-gui"
    )
    if (-not [string]::IsNullOrWhiteSpace($TargetDir)) {
        $cargoArgs += @("--target-dir", $buildTarget)
    }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "签名 GUI release 构建失败，退出码：$LASTEXITCODE"
    }
}
if (-not (Test-Path -LiteralPath $sourceExe -PathType Leaf)) {
    throw "缺少 GUI 构建产物：$sourceExe"
}
foreach ($requiredFile in @($guiReadme, $guiDevelopmentDoc)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
        throw "缺少 GUI 发布文档：$requiredFile"
    }
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $repo ("release\usb-license-signer-gui-{0}" -f (Get-Date -Format "yyyyMMdd"))
}
$outputFull = [System.IO.Path]::GetFullPath($OutputPath)
$repoFull = [System.IO.Path]::GetFullPath($repo).TrimEnd('\')
$targetFull = $buildTarget.TrimEnd('\')
if ($outputFull.TrimEnd('\') -ieq $repoFull -or $outputFull.TrimEnd('\') -ieq $targetFull) {
    throw "GUI 发布输出目录不能覆盖仓库根目录或 target 目录：$outputFull"
}
$createdOutputDir = $false
if (Test-Path -LiteralPath $outputFull) {
    if (-not (Get-Item -LiteralPath $outputFull).PSIsContainer) {
        throw "GUI 发布输出路径不是目录：$outputFull"
    }
    if (@(Get-ChildItem -LiteralPath $outputFull -Force).Count -gt 0) {
        throw "GUI 发布输出目录必须为空，请使用新的版本目录：$outputFull"
    }
} else {
    New-Item -ItemType Directory -Path $outputFull -Force | Out-Null
    $createdOutputDir = $true
}

$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$publishedExe = Join-Path $outputFull "CodexUsbLicenseSigner.exe"
$publishedReadme = Join-Path $outputFull "README.txt"
$publishedDevelopment = Join-Path $outputFull "DEVELOPMENT.md"
$publishedManifest = Join-Path $outputFull "manifest.json"
$publishedSums = Join-Path $outputFull "SHA256SUMS.txt"
$publishedVault = Join-Path $outputFull "codex-usb-signing-key.vault"
$publishedVaultDescriptor = Join-Path $outputFull "codex-usb-signing-key.vault.public.json"
$publishedProfile = Join-Path $outputFull "usb-signing-release-profile.json"
try {
    if ($TargetDesktopOnly) {
        throw "正式签名器不再支持 TargetDesktopOnly 引导包；必须携带匹配的 portable vault，避免运行时依赖某台电脑的 DPAPI 公钥"
    }
    if ([string]::IsNullOrWhiteSpace($PortableVaultPath)) {
        throw "正式签名器必须携带与当前版本 profile 匹配的 portable vault；无 vault 的包不能交付"
    }
    Copy-Item -LiteralPath $sourceExe -Destination $publishedExe
    Copy-Item -LiteralPath $guiReadme -Destination $publishedReadme
    Copy-Item -LiteralPath $guiDevelopmentDoc -Destination $publishedDevelopment
    Copy-Item -LiteralPath $signingProfile.Path -Destination $publishedProfile
    $sub2ApiConfigIncluded = $false

$localKeyVerified = $false
Write-Warning "正式签名器使用同目录 portable vault；不读取或要求构建机/运行机的 DPAPI 公钥。实际密钥会在签名时由密码解密并与 Bundle 绑定复核。"

$portableVaultIncluded = $false
$portableVaultBindingVerified = $false
if (-not [string]::IsNullOrWhiteSpace($PortableVaultPath)) {
    $portableVaultFull = [System.IO.Path]::GetFullPath($PortableVaultPath)
    if (-not (Test-Path -LiteralPath $portableVaultFull -PathType Leaf)) {
        throw "Portable vault path does not point to a file: $portableVaultFull"
    }
    if ([string]::IsNullOrWhiteSpace($PortableVaultDescriptorPath)) {
        $PortableVaultDescriptorPath = "$portableVaultFull.public.json"
    }
    $portableVaultDescriptorFull = [System.IO.Path]::GetFullPath($PortableVaultDescriptorPath)
    if (-not (Test-Path -LiteralPath $portableVaultDescriptorFull -PathType Leaf)) {
        throw "Portable vault 缺少公共绑定描述文件：$portableVaultDescriptorFull；请使用 signer 的 export-portable 重新导出，不能用文件名代替密钥匹配"
    }
    try {
        $portableDescriptor = Get-Content -Raw -LiteralPath $portableVaultDescriptorFull | ConvertFrom-Json
    } catch {
        throw "Portable vault 公共绑定描述不是有效 JSON：$portableVaultDescriptorFull；$($_.Exception.Message)"
    }
    if ([string]$portableDescriptor.keyId -cne [string]$signingProfile.KeyId -or
        ([string]$portableDescriptor.publicKeySha256).ToLowerInvariant() -cne [string]$signingProfile.PublicKeySha256) {
        throw "Portable vault 与本版本 profile 不匹配：vault=$($portableDescriptor.keyId)/$($portableDescriptor.publicKeySha256)，profile=$($signingProfile.KeyId)/$($signingProfile.PublicKeySha256)"
    }
    Copy-Item -LiteralPath $portableVaultFull -Destination $publishedVault
    Copy-Item -LiteralPath $portableVaultDescriptorFull -Destination $publishedVaultDescriptor
    $portableVaultIncluded = $true
    $portableVaultBindingVerified = $true
}

# Verify the exact published executable with runtime credential overrides
# removed. A manifest flag based only on build-shell environment presence is
# insufficient: the exact GUI executable must load the generated credential.
$adminApiKeyRuntimeVerified = $false
$adminKeyDiagnosticPath = Join-Path $outputFull ".admin-key-diagnostic.json"
$previousAdminKey = [Environment]::GetEnvironmentVariable("CODEX_404TOKEN_ADMIN_API_KEY", "Process")
$previousFallbackAdminKey = [Environment]::GetEnvironmentVariable("SUB2API_ADMIN_API_KEY", "Process")
try {
    [Environment]::SetEnvironmentVariable("CODEX_404TOKEN_ADMIN_API_KEY", $null, "Process")
    [Environment]::SetEnvironmentVariable("SUB2API_ADMIN_API_KEY", $null, "Process")
    $diagnosticProcess = Start-Process -FilePath $publishedExe -ArgumentList @("--diagnose-admin-key", $adminKeyDiagnosticPath) -Wait -PassThru
    if ($diagnosticProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $adminKeyDiagnosticPath -PathType Leaf)) {
        throw "发布版签名器未能完成管理员 Key 自检；未生成可交付包"
    }
    $adminKeyDiagnostic = Get-Content -Raw -LiteralPath $adminKeyDiagnosticPath | ConvertFrom-Json
    if ($adminKeyDiagnostic.source -cne "embedded" -or
        [int]$adminKeyDiagnostic.resolvedLength -le 0 -or
        [int]$adminKeyDiagnostic.resolvedLength -ne $expectedAdminApiKeyLength -or
        [int]$adminKeyDiagnostic.embeddedPrimaryLength -ne $expectedAdminApiKeyLength) {
        throw "发布版签名器加载的内置管理员 Key 与本次提供的 Key 不一致；未生成可交付包"
    }
    $adminApiKeyRuntimeVerified = $true
} finally {
    [Environment]::SetEnvironmentVariable("CODEX_404TOKEN_ADMIN_API_KEY", $previousAdminKey, "Process")
    [Environment]::SetEnvironmentVariable("SUB2API_ADMIN_API_KEY", $previousFallbackAdminKey, "Process")
    Remove-Item -LiteralPath $adminKeyDiagnosticPath -Force -ErrorAction SilentlyContinue
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$manifest = [ordered]@{
    schemaVersion = 1
    product = "codex-usb-license-signer-gui"
    executable = "CodexUsbLicenseSigner.exe"
    platform = "windows-x64"
    signingCore = "shared usb-license-signer command pipeline"
    verificationMode = "usb-internal-license-only"
    systemAuthenticodeRequired = $false
    localKeyVerified = $localKeyVerified
    targetDesktopOnly = $false
    privateKeyIncluded = $false
    portableVaultIncluded = $portableVaultIncluded
    portableVaultBindingVerified = $portableVaultBindingVerified
    adminApiKeyEmbedded = $adminApiKeyEmbedded
    adminApiKeyRuntimeVerified = $adminApiKeyRuntimeVerified
    adminApiKeySource = if ($adminApiKeyEmbedded) {
        "compile-time-embedded"
    } else {
        "not-provisioned"
    }
    sub2ApiConfigIncluded = $sub2ApiConfigIncluded
    expectedSigningBinding = [ordered]@{
        keyId = $signingProfile.KeyId
        publicKeySha256 = $signingProfile.PublicKeySha256
        releaseVersion = $signingProfile.ReleaseVersion
        releaseId = $signingProfile.ReleaseId
        source = [System.IO.Path]::GetFileName($signingProfile.Path)
        note = "The signer package never contains a raw private key. The matching encrypted portable vault is the only signing-key source and may be copied to another Windows x64 computer and unlocked with its password."
    }
    files = [ordered]@{
        executable = Get-Sha256 $publishedExe
        readme = Get-Sha256 $publishedReadme
        development = Get-Sha256 $publishedDevelopment
        profile = Get-Sha256 $publishedProfile
    }
}
if ($portableVaultIncluded) {
    $manifest.files.portableVault = Get-Sha256 $publishedVault
    $manifest.files.portableVaultDescriptor = Get-Sha256 $publishedVaultDescriptor
}
[System.IO.File]::WriteAllText(
    $publishedManifest,
    ($manifest | ConvertTo-Json -Depth 5),
    $utf8NoBom
)

$sumLines = @(
    "$(Get-Sha256 $publishedExe)  CodexUsbLicenseSigner.exe"
    "$(Get-Sha256 $publishedDevelopment)  DEVELOPMENT.md"
    "$(Get-Sha256 $publishedManifest)  manifest.json"
    "$(Get-Sha256 $publishedReadme)  README.txt"
    "$(Get-Sha256 $publishedProfile)  usb-signing-release-profile.json"
)
if ($portableVaultIncluded) {
    $sumLines += "$(Get-Sha256 $publishedVault)  codex-usb-signing-key.vault"
    $sumLines += "$(Get-Sha256 $publishedVaultDescriptor)  codex-usb-signing-key.vault.public.json"
}
[System.IO.File]::WriteAllText($publishedSums, ($sumLines -join "`n") + "`n", $utf8NoBom)

    Write-Output "GUI signer package: $outputFull"
    Write-Output "Executable: $publishedExe"
} catch {
    if ($createdOutputDir -and (Test-Path -LiteralPath $outputFull -PathType Container)) {
        Remove-Item -LiteralPath $outputFull -Recurse -Force -ErrorAction SilentlyContinue
    }
    throw
}
