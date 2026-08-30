[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourcePath,
    [Parameter(Mandatory = $true)]
    [string]$CodexDesktopVersion,
    [string]$ProductVersion = "0.1.0",
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "x64",
    [string]$PackageVersion,
    [string]$OutputDir = (Join-Path $PSScriptRoot "..\release\usb"),
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"

function Read-MsixIdentity {
    param([string]$Path)

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName -eq "AppxManifest.xml" } | Select-Object -First 1
        if ($null -eq $entry) { throw "MSIX 缺少 AppxManifest.xml" }
        $reader = [System.IO.StreamReader]::new($entry.Open())
        try { $xml = [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
        $identity = $xml.SelectSingleNode("//*[local-name()='Identity']")
        $application = $xml.SelectSingleNode("//*[local-name()='Application']")
        if ($null -eq $identity) { throw "AppxManifest.xml 缺少 Identity" }
        [pscustomobject]@{
            Name = [string]$identity.Name
            Version = [string]$identity.Version
            ProcessorArchitecture = [string]$identity.ProcessorArchitecture
            ApplicationExecutable = if ($null -eq $application) { $null } else { [string]$application.Executable }
        }
    }
    finally { $archive.Dispose() }
}

$source = (Resolve-Path -LiteralPath $SourcePath).Path
$sourceItem = Get-Item -LiteralPath $source
if ($sourceItem.PSIsContainer -or $sourceItem.Extension -ine ".msix") {
    throw "SourcePath 必须是一个 .msix 文件"
}

if (-not $SkipSignature) {
    $signature = Get-AuthenticodeSignature -LiteralPath $source
    if ($signature.Status -ne "Valid") {
        throw "MSIX Authenticode 状态不是 Valid：$($signature.Status)"
    }
}

$identity = Read-MsixIdentity -Path $source
if ($identity.Name -ne "OpenAI.Codex") {
    throw "包身份不是 OpenAI.Codex，而是 $($identity.Name)"
}
if ([string]$identity.ProcessorArchitecture -notin @($Architecture, "neutral")) {
    throw "MSIX 架构 $($identity.ProcessorArchitecture) 与目标架构 $Architecture 不一致"
}
if ([string]::IsNullOrWhiteSpace($identity.ApplicationExecutable)) {
    throw "MSIX 缺少桌面 Application 入口"
}
if ([string]::IsNullOrWhiteSpace($CodexDesktopVersion)) {
    throw "必须显式提供 CodexDesktopVersion（例如 26.602.40724）"
}
if ([string]::IsNullOrWhiteSpace($PackageVersion)) {
    $PackageVersion = $identity.Version
} elseif ($PackageVersion -ne [string]$identity.Version) {
    throw "PackageVersion $PackageVersion 与 MSIX Identity 版本 $($identity.Version) 不一致"
}

$root = (New-Item -ItemType Directory -Force -Path $OutputDir).FullName
$payloadDir = Join-Path $root "payload"
New-Item -ItemType Directory -Force -Path $payloadDir | Out-Null
$destinationName = "OpenAI.Codex_$Architecture.msix"
$destination = Join-Path $payloadDir $destinationName
Copy-Item -LiteralPath $source -Destination $destination -Force

$payloadItem = Get-Item -LiteralPath $destination
$hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
$manifest = [ordered]@{
    productVersion = $ProductVersion
    codexDesktopVersion = $CodexDesktopVersion
    packageName = "OpenAI.Codex"
    architecture = $Architecture
    minWindowsBuild = "19041"
    packageVersion = $PackageVersion
    releaseTag = "local-signed-$CodexDesktopVersion"
    source = "fixed-build-input"
    payload = [ordered]@{
        path = "payload/$destinationName"
        sha256 = $hash
        size = [int64]$payloadItem.Length
    }
}
$json = $manifest | ConvertTo-Json -Depth 5
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText((Join-Path $root "manifest.json"), $json, $utf8NoBom)
$sumLines = @(
    "$hash  payload/$destinationName"
    "$((Get-FileHash -LiteralPath (Join-Path $root 'manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant())  manifest.json"
)
[System.IO.File]::WriteAllLines((Join-Path $root "SHA256SUMS.txt"), $sumLines, $utf8NoBom)

Write-Output ("已生成离线载荷：{0}`n版本：{1}`nSHA256：{2}`n输出：{3}" -f $destination, $CodexDesktopVersion, $hash, $root)
