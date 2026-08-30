[CmdletBinding()]
param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "x64",
    [string]$Repository = "meta-xucong/codex-app-mirror",
    [string]$ReleaseTag,
    [string]$DownloadDir = (Join-Path $PSScriptRoot "..\release\upstream"),
    [string]$BundleDir = (Join-Path $PSScriptRoot "..\release\usb"),
    [string]$OutputDir = (Join-Path $PSScriptRoot "..\release\usb-final"),
    [switch]$ForceDownload,
    [switch]$SkipLayout
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-GhJson {
    param([Parameter(Mandatory)][string[]]$Arguments)

    $raw = & gh @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') 执行失败：$($raw -join [Environment]::NewLine)"
    }
    return (($raw -join [Environment]::NewLine) | ConvertFrom-Json)
}

function Invoke-Gh {
    param([Parameter(Mandatory)][string[]]$Arguments)

    & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') 执行失败，退出码 $LASTEXITCODE。"
    }
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "未找到 GitHub CLI gh。请先安装 GitHub CLI 并完成 gh auth login。"
}

& gh auth status 2>&1 | Out-Host
if ($LASTEXITCODE -ne 0) {
    throw "GitHub CLI 未登录。请先执行 gh auth login。"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$downloadRoot = (New-Item -ItemType Directory -Force -Path $DownloadDir).FullName

if ([string]::IsNullOrWhiteSpace($ReleaseTag)) {
    $releases = Invoke-GhJson @("api", "repos/$Repository/releases?per_page=100")
    $release = @(
        $releases |
            Where-Object { -not $_.draft -and -not $_.prerelease } |
            Sort-Object { [DateTime]$_.published_at } -Descending |
            Select-Object -First 1
    ) | Select-Object -First 1
    if ($null -eq $release) {
        throw "仓库 $Repository 没有可用的稳定 Release。"
    }
    $ReleaseTag = [string]$release.tag_name
}
else {
    $release = Invoke-GhJson @("api", "repos/$Repository/releases/tags/$ReleaseTag")
    if ($release.draft -or $release.prerelease) {
        throw "Release $ReleaseTag 是草稿或预发布版本，已拒绝作为生产载荷。"
    }
}

$assetPattern = "^OpenAI\.Codex_.*_${Architecture}__.*\.(?i:Msix)$"
$msixAsset = @($release.assets | Where-Object { $_.name -match $assetPattern }) | Select-Object -First 1
if ($null -eq $msixAsset) {
    throw "Release $ReleaseTag 没有 $Architecture Windows MSIX 资产。"
}

Write-Host "GitHub 源：$Repository"
Write-Host "Release：$ReleaseTag"
Write-Host "MSIX：$($msixAsset.name)"

$downloadNames = @(
    [string]$msixAsset.name,
    "SHA256SUMS-windows.txt",
    "release-manifest.json"
)
$missingDownloads = @($downloadNames | Where-Object {
        $ForceDownload -or -not (Test-Path -LiteralPath (Join-Path $downloadRoot $_) -PathType Leaf)
    })
if ($missingDownloads.Count -gt 0) {
    $downloadArgs = @(
        "release", "download", $ReleaseTag,
        "--repo", $Repository,
        "--dir", $downloadRoot,
        "--clobber"
    )
    foreach ($name in $downloadNames) {
        $downloadArgs += @("--pattern", $name)
    }
    Invoke-Gh $downloadArgs
}
else {
    Write-Host "下载目录已包含此 Release 的三个载荷文件，跳过重复下载。"
}

$msixPath = Join-Path $downloadRoot $msixAsset.name
if (-not (Test-Path -LiteralPath $msixPath -PathType Leaf)) {
    throw "GitHub 下载后未找到 MSIX：$msixPath"
}

$upstreamManifestPath = Join-Path $downloadRoot "release-manifest.json"
if (-not (Test-Path -LiteralPath $upstreamManifestPath -PathType Leaf)) {
    throw "GitHub Release 缺少 release-manifest.json。"
}
$upstreamManifest = Get-Content -Raw -LiteralPath $upstreamManifestPath | ConvertFrom-Json
$architectureManifest = $upstreamManifest.sources.windows.architectures.$Architecture
$codexDesktopVersion = [string]$architectureManifest.appVersion
$packageVersion = [string]$architectureManifest.version
if ([string]::IsNullOrWhiteSpace($codexDesktopVersion)) {
    $codexDesktopVersion = [string]$upstreamManifest.appVersion
}
if ([string]::IsNullOrWhiteSpace($packageVersion)) {
    $packageVersion = [string]$architectureManifest.packageVersion
}
if ([string]::IsNullOrWhiteSpace($codexDesktopVersion) -or [string]::IsNullOrWhiteSpace($packageVersion)) {
    throw "release-manifest.json 缺少 $Architecture 的 appVersion/version。"
}

$collectScript = Join-Path $PSScriptRoot "collect-payload.ps1"
$verifyScript = Join-Path $PSScriptRoot "verify-payload.ps1"
& pwsh -NoProfile -File $collectScript `
    -SourcePath $msixPath `
    -CodexDesktopVersion $codexDesktopVersion `
    -PackageVersion $packageVersion `
    -Architecture $Architecture `
    -OutputDir $BundleDir
if ($LASTEXITCODE -ne 0) { throw "collect-payload.ps1 执行失败。" }

& pwsh -NoProfile -File $verifyScript -BundlePath $BundleDir
if ($LASTEXITCODE -ne 0) { throw "verify-payload.ps1 执行失败。" }

if (-not $SkipLayout) {
    $layoutScript = Join-Path $PSScriptRoot "build-usb-layout.ps1"
    $installerPath = Join-Path $repoRoot "release\setup\CodexDesktopSetup.exe"
    & pwsh -NoProfile -File $layoutScript `
        -InstallerPath $installerPath `
        -BundlePath $BundleDir `
        -OutputPath $OutputDir
    if ($LASTEXITCODE -ne 0) { throw "build-usb-layout.ps1 执行失败。" }
}

Write-Host "已从 GitHub Fork 更新 Codex $Architecture 载荷：$codexDesktopVersion / package $packageVersion"
Write-Host "下载缓存：$downloadRoot"
Write-Host "离线 bundle：$BundleDir"
if (-not $SkipLayout) {
    Write-Host "最终 U 盘目录：$OutputDir"
}
