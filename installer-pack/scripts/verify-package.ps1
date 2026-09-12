[CmdletBinding()]
param(
    [switch]$SkipAudit
)

$ErrorActionPreference = 'Stop'
$packRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$textSuffixes = @('.cmd', '.env', '.in', '.json', '.lock', '.md', '.ps1', '.py', '.sha256', '.toml', '.txt', '.yaml', '.yml')
$generated = @('file-manifest.json', 'checksums.sha256')
$transientDirectories = @('.git', '.pytest_cache', '__pycache__')

function Get-CanonicalBytes([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $suffix = [System.IO.Path]::GetExtension($Path).ToLowerInvariant()
    if ($textSuffixes -notcontains $suffix) { return $bytes }
    $text = [System.Text.Encoding]::UTF8.GetString($bytes)
    $text = $text.Replace("`r`n", "`n").Replace("`r", "`n")
    return [System.Text.UTF8Encoding]::new($false).GetBytes($text)
}

function Get-HashBytes([byte[]]$Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { return (($sha.ComputeHash($Bytes) | ForEach-Object { $_.ToString('x2') }) -join '') }
    finally { $sha.Dispose() }
}

function Get-Relative([string]$Path) {
    return $Path.Substring($packRoot.Length + 1).Replace('\', '/')
}

function Assert-SafeRelative([string]$Relative) {
    if ([string]::IsNullOrWhiteSpace($Relative) -or [System.IO.Path]::IsPathRooted($Relative)) {
        throw "unsafe package path: $Relative"
    }
    $parts = $Relative.Replace('\', '/').Split('/')
    if ($parts -contains '..' -or $parts -contains '') { throw "unsafe package path: $Relative" }
}

$manifestPath = Join-Path $packRoot 'file-manifest.json'
$checksumsPath = Join-Path $packRoot 'checksums.sha256'
$packPath = Join-Path $packRoot 'pack.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'file-manifest.json is missing' }
if (-not (Test-Path -LiteralPath $checksumsPath -PathType Leaf)) { throw 'checksums.sha256 is missing' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$pack = Get-Content -LiteralPath $packPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.schemaVersion -ne 2) { throw 'file-manifest schemaVersion must be 2' }

$listed = @{}
foreach ($entry in @($manifest.files)) {
    $relative = [string]$entry.path
    Assert-SafeRelative $relative
    if ($generated -contains $relative -or $listed.ContainsKey($relative)) { throw "invalid or duplicate manifest path: $relative" }
    $target = Join-Path $packRoot ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "manifest target is missing: $relative" }
    $actual = Get-HashBytes (Get-CanonicalBytes $target)
    if ($actual -ne [string]$entry.sha256) { throw "manifest hash mismatch: $relative" }
    if (([System.IO.FileInfo]$target).Length -ne [int64]$entry.bytes) { throw "manifest byte-size mismatch: $relative" }
    $listed[$relative] = $actual
}

$actualPaths = @{}
foreach ($file in Get-ChildItem -LiteralPath $packRoot -Recurse -File -Force) {
    $relative = Get-Relative $file.FullName
    $parts = $relative.Split('/')
    if (($parts | Where-Object { $transientDirectories -contains $_ }).Count -gt 0 -or $relative.EndsWith('.pyc')) { continue }
    if ($generated -contains $relative) { continue }
    $actualPaths[$relative] = $true
}
$extra = @($actualPaths.Keys | Where-Object { -not $listed.ContainsKey($_) })
$missing = @($listed.Keys | Where-Object { -not $actualPaths.ContainsKey($_) })
if ($extra.Count -or $missing.Count) { throw "manifest closure mismatch; extra=$($extra -join ','); missing=$($missing -join ',')" }

$checksumMap = @{}
foreach ($line in (Get-Content -LiteralPath $checksumsPath -Encoding UTF8)) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    if ($line.Length -lt 66 -or $line.Substring(64, 2) -ne ' *') { throw 'checksums.sha256 has a malformed line' }
    $digest = $line.Substring(0, 64)
    $relative = $line.Substring(66)
    if ($checksumMap.ContainsKey($relative)) { throw "duplicate checksum path: $relative" }
    $checksumMap[$relative] = $digest
}
$expectedChecksumPaths = @('file-manifest.json') + @($listed.Keys)
$actualChecksumText = (@($checksumMap.Keys | Sort-Object) -join "`n")
$expectedChecksumText = (@($expectedChecksumPaths | Sort-Object) -join "`n")
if ($actualChecksumText -ne $expectedChecksumText) { throw 'checksums.sha256 closure mismatch' }
$manifestHash = Get-HashBytes (Get-CanonicalBytes $manifestPath)
if ($checksumMap['file-manifest.json'] -ne $manifestHash) { throw 'file-manifest checksum mismatch' }
foreach ($relative in $listed.Keys) { if ($checksumMap[$relative] -ne $listed[$relative]) { throw "checksum mismatch: $relative" } }

$sourceDigest = [System.Security.Cryptography.SHA256]::Create()
$sourceStream = [System.IO.MemoryStream]::new()
try {
    # The Python builder's manifest order is the canonical order.  Reusing it
    # avoids culture-dependent PowerShell sorting of mixed-case Windows paths.
    $sourceEntries = @($manifest.files | ForEach-Object { [string]$_.path } | Where-Object { $_ -notin @('pack.json', 'file-manifest.json', 'checksums.sha256') })
    foreach ($relative in $sourceEntries) {
        $nameBytes = [System.Text.Encoding]::UTF8.GetBytes($relative)
        $contentBytes = [System.Security.Cryptography.SHA256]::Create().ComputeHash((Get-CanonicalBytes (Join-Path $packRoot ($relative.Replace('/', '\')))))
        $lengthBytes = [BitConverter]::GetBytes([uint32]$nameBytes.Length)
        if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($lengthBytes) }
        $sourceStream.Write($lengthBytes, 0, $lengthBytes.Length)
        $sourceStream.Write($nameBytes, 0, $nameBytes.Length)
        $sourceStream.Write($contentBytes, 0, $contentBytes.Length)
    }
    $treeHash = (($sourceDigest.ComputeHash($sourceStream.ToArray()) | ForEach-Object { $_.ToString('x2') }) -join '')
} finally { $sourceStream.Dispose(); $sourceDigest.Dispose() }
if ([string]$pack.sourceTreeSha256 -ne $treeHash) { throw "pack.json sourceTreeSha256 mismatch: expected $treeHash, got $($pack.sourceTreeSha256)" }

if (-not $SkipAudit) {
    $python = Get-Command python.exe -ErrorAction SilentlyContinue
    if ($null -ne $python) {
        & $python.Source (Join-Path $packRoot 'audit-codex-compatibility.py') --root $packRoot --require-codex
        if ($LASTEXITCODE -ne 0) { throw "compatibility audit failed with exit code $LASTEXITCODE" }
    } else {
        Write-Warning 'Python not found; package closure checks passed, compatibility audit skipped.'
    }
}
Write-Output "verify-package: PASS ($($listed.Count) files; sourceTreeSha256=$treeHash)"
