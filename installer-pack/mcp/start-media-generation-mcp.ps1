[CmdletBinding()]
param([switch]$DryRun)

$ErrorActionPreference = 'Stop'
$kitHome = Split-Path -Parent $PSScriptRoot
$python = Join-Path $kitHome 'runtime\python-env\Scripts\python.exe'
$server = Join-Path $PSScriptRoot 'media-generation-mcp.py'
$dataRoot = Join-Path $kitHome 'data\media-generation'

$plan = [ordered]@{
    pythonPath = $python
    pythonReady = (Test-Path -LiteralPath $python -PathType Leaf)
    serverPath = $server
    serverReady = (Test-Path -LiteralPath $server -PathType Leaf)
    dataRoot = $dataRoot
}
if ($DryRun) {
    $plan | ConvertTo-Json -Compress
    exit 0
}
if (-not $plan.pythonReady) { throw 'The bundled Python runtime required by the unified media MCP is missing.' }
if (-not $plan.serverReady) { throw 'The unified media MCP server script is missing.' }
New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null
$env:MEDIA_GENERATION_DATA_DIR = $dataRoot
& $python $server
exit $LASTEXITCODE
