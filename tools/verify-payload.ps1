[CmdletBinding()]
param(
    [string]$BundlePath = "",
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($BundlePath)) {
    $repoCandidate = Join-Path $PSScriptRoot "..\release\usb"
    $BundlePath = if (Test-Path -LiteralPath $repoCandidate -PathType Container) {
        $repoCandidate
    } else {
        $PSScriptRoot
    }
}

function Read-MsixManifest {
    param([string]$Path)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName -eq "AppxManifest.xml" } | Select-Object -First 1
        if ($null -eq $entry) { throw "MSIX 缺少 AppxManifest.xml" }
        $reader = [System.IO.StreamReader]::new($entry.Open())
        try { return [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    finally { $archive.Dispose() }
}

function Normalize-BundleRelativePath([string]$RelativePath) {
    $relative = ([string]$RelativePath).Trim().Replace('/', '\')
    if ([string]::IsNullOrWhiteSpace($relative) -or [System.IO.Path]::IsPathRooted($relative)) {
        throw "payload.path 必须是 bundle 内的相对路径"
    }
    foreach ($segment in $relative.Split('\')) {
        if ([string]::IsNullOrWhiteSpace($segment) -or $segment -eq '.' -or $segment -eq '..' -or $segment.Contains(':')) {
            throw "payload.path 必须是 bundle 内的相对路径"
        }
    }
    return $relative.Replace('\', '/')
}

function Resolve-BundleRelativeFile([string]$RootPath, [string]$RelativePath) {
    $relative = Normalize-BundleRelativePath $RelativePath
    $candidate = Join-Path $RootPath ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "未找到 payload：$relative"
    }
    $resolved = (Resolve-Path -LiteralPath $candidate).Path
    $rootFull = [System.IO.Path]::GetFullPath($RootPath).TrimEnd('\') + '\'
    $resolvedFull = [System.IO.Path]::GetFullPath($resolved)
    if (-not $resolvedFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "payload 不在 bundle 根目录内"
    }
    return $resolved
}

$root = (Resolve-Path -LiteralPath $BundlePath).Path
$manifestPath = Join-Path $root "manifest.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw "未找到 $manifestPath" }
$utf8 = [System.Text.UTF8Encoding]::new($false, $true)
$manifest = [System.IO.File]::ReadAllText($manifestPath, $utf8) | ConvertFrom-Json
if ($manifest.packageName -ne "OpenAI.Codex") { throw "manifest.packageName 必须为 OpenAI.Codex" }
if ($manifest.architecture -notin @("x64", "arm64")) { throw "manifest.architecture 无效" }

$relative = Normalize-BundleRelativePath ([string]$manifest.payload.path)
if ([System.IO.Path]::GetExtension($relative) -ine ".msix") {
    throw "payload.path 必须指向 .msix 文件"
}
if ([System.IO.Path]::IsPathRooted($relative) -or $relative -match '(^|\\)\.\.($|\\)') {
    throw "payload.path 必须是 bundle 内的相对路径"
}
$payload = Resolve-BundleRelativeFile $root $relative
$rootFull = [System.IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
$payloadFull = [System.IO.Path]::GetFullPath($payload)
if (-not $payloadFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "payload 不在 bundle 根目录内"
}
$item = Get-Item -LiteralPath $payload
if ($item.PSIsContainer -or $item.Extension -ine ".msix") { throw "payload 必须是 .msix 文件" }
if ([int64]$item.Length -ne [int64]$manifest.payload.size) { throw "MSIX 大小与 manifest 不一致" }
$hash = (Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash.ToLowerInvariant()
if ($hash -ne ([string]$manifest.payload.sha256).ToLowerInvariant()) { throw "MSIX SHA-256 不匹配" }

if (-not $SkipSignature) {
    $signature = Get-AuthenticodeSignature -LiteralPath $payload
    if ($signature.Status -ne "Valid") { throw "MSIX Authenticode 状态不是 Valid：$($signature.Status)" }
    $subject = [string]$signature.SignerCertificate.Subject
    if ($subject -notmatch "50bdfd77-8903-4850-9ffe-6e8522f64d5b") {
        throw "MSIX 签名主体不是当前锁定的 OpenAI Marketplace 证书：$subject"
    }
}

$xml = Read-MsixManifest -Path $payload
$identity = $xml.SelectSingleNode("//*[local-name()='Identity']")
$application = $xml.SelectSingleNode("//*[local-name()='Application']")
if ($null -eq $identity -or [string]$identity.Name -ne "OpenAI.Codex") { throw "MSIX Identity 不是 OpenAI.Codex" }
if ([string]$identity.ProcessorArchitecture -notin @($manifest.architecture, "neutral")) { throw "MSIX 架构与 manifest 不一致" }
if ($null -eq $application -or [string]::IsNullOrWhiteSpace($application.Executable)) { throw "MSIX 缺少桌面 Application 入口" }
if ($manifest.packageVersion -and [string]$identity.Version -ne [string]$manifest.packageVersion) { throw "MSIX package version 与 manifest 不一致" }

[pscustomobject]@{
    ready = $true
    payload = $payloadFull
    sha256 = $hash
    size = [int64]$item.Length
    packageName = [string]$identity.Name
    packageVersion = [string]$identity.Version
    architecture = [string]$identity.ProcessorArchitecture
    appEntry = [string]$application.Executable
} | ConvertTo-Json -Depth 4
