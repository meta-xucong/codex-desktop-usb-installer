[CmdletBinding()]
param(
    [Parameter(Position = 0, Mandatory = $true)][ValidateSet('has_city','get_city','set_city','get_config')][string]$Command,
    [Parameter(Position = 1)][string]$City
)

$ErrorActionPreference = 'Stop'
$base = if ($env:WEATHER_DATA_DIR) { $env:WEATHER_DATA_DIR } elseif ($env:CODEX_DATA_DIR) { Join-Path $env:CODEX_DATA_DIR 'weather-forecast' } else { Join-Path (Get-Location) 'codex-data\weather-forecast' }
$configFile = Join-Path $base 'config.json'
function Ensure-Config {
    New-Item -ItemType Directory -Force -Path $base | Out-Null
    if (-not (Test-Path -LiteralPath $configFile)) {
        [ordered]@{ initialized = $false; created_at = [DateTime]::Now.ToString('o'); default_city = $null; unit = 'metric'; format = 'compact' } |
            ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configFile -Encoding UTF8
    }
}
function Read-Config { Ensure-Config; return Get-Content -LiteralPath $configFile -Raw -Encoding UTF8 | ConvertFrom-Json }
function Write-Config($Value) { $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $configFile -Encoding UTF8 }
$config = Read-Config
switch ($Command) {
    'has_city' { if ($config.initialized -and $config.default_city) { 'true' } else { 'false' } }
    'get_city' { if ($config.default_city) { $config.default_city } else { '' } }
    'set_city' {
        if ([string]::IsNullOrWhiteSpace($City)) { throw 'city is required' }
        $config.default_city = $City; $config.initialized = $true
        $config | Add-Member -NotePropertyName updated_at -NotePropertyValue ([DateTime]::Now.ToString('o')) -Force
        Write-Config $config
        "Default city set to: $City"
    }
    'get_config' { $config | ConvertTo-Json -Depth 12 }
}
