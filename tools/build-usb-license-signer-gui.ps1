[CmdletBinding()]
param(
    [string]$OutputPath = "",
    [string]$TargetDir = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
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
if (Test-Path -LiteralPath $outputFull) {
    if (-not (Get-Item -LiteralPath $outputFull).PSIsContainer) {
        throw "GUI 发布输出路径不是目录：$outputFull"
    }
    if (@(Get-ChildItem -LiteralPath $outputFull -Force).Count -gt 0) {
        throw "GUI 发布输出目录必须为空，请使用新的版本目录：$outputFull"
    }
} else {
    New-Item -ItemType Directory -Path $outputFull -Force | Out-Null
}

$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$publishedExe = Join-Path $outputFull "CodexUsbLicenseSigner.exe"
$publishedReadme = Join-Path $outputFull "README.txt"
$publishedDevelopment = Join-Path $outputFull "DEVELOPMENT.md"
$publishedManifest = Join-Path $outputFull "manifest.json"
$publishedSums = Join-Path $outputFull "SHA256SUMS.txt"
Copy-Item -LiteralPath $sourceExe -Destination $publishedExe
Copy-Item -LiteralPath $guiReadme -Destination $publishedReadme
Copy-Item -LiteralPath $guiDevelopmentDoc -Destination $publishedDevelopment

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
    privateKeyIncluded = $false
    files = [ordered]@{
        executable = Get-Sha256 $publishedExe
        readme = Get-Sha256 $publishedReadme
        development = Get-Sha256 $publishedDevelopment
    }
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
)
[System.IO.File]::WriteAllText($publishedSums, ($sumLines -join "`n") + "`n", $utf8NoBom)

Write-Output "GUI signer package: $outputFull"
Write-Output "Executable: $publishedExe"
