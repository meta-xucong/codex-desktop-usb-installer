[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [string]$InputFile,
    [string]$PythonExe = '' ,
    [string]$IndexUrl,
    [string[]]$FindLinks = @()
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InputFile)) { $InputFile = Join-Path $repoRoot 'runtime\requirements-python-win-x64-py312.in' }
if ([string]::IsNullOrWhiteSpace($PythonExe)) {
    $candidate = Get-Command python -ErrorAction SilentlyContinue
    if (-not $candidate) { throw 'A CPython 3.12 build interpreter is required to materialize the wheelhouse.' }
    $PythonExe = $candidate.Source
}
$probe = 'import platform,sys,sysconfig; print(f"{sys.version_info[0]}.{sys.version_info[1]}.{sys.version_info[2]}|{sys.implementation.cache_tag}|{sysconfig.get_platform()}|{platform.machine()}")'
$probeOutput = (& $PythonExe -c $probe 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $probeOutput -notmatch '^3\.12\.\d+\|cpython-312\|win-amd64\|(AMD64|x86_64)$') {
    throw "Materialization requires Windows x64 CPython 3.12/cp312; detected: $probeOutput"
}
$script = Join-Path $PSScriptRoot 'materialize-python-wheelhouse.py'
$arguments = @($script, '--input', (Resolve-Path -LiteralPath $InputFile).Path, '--output', $OutputRoot, '--python', $PythonExe)
if (-not [string]::IsNullOrWhiteSpace($IndexUrl)) { $arguments += @('--index-url', $IndexUrl) }
foreach ($findLink in $FindLinks) {
    if ([string]::IsNullOrWhiteSpace($findLink)) { continue }
    if (-not (Test-Path -LiteralPath $findLink -PathType Container)) { throw "Find-links directory does not exist: $findLink" }
    $arguments += @('--find-links', (Resolve-Path -LiteralPath $findLink).Path)
}
& $PythonExe @arguments
if ($LASTEXITCODE -ne 0) { throw 'Wheelhouse materialization failed.' }
