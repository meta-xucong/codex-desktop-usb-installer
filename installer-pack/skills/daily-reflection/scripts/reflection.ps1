[CmdletBinding()]
param(
    [Parameter(Position = 0)][ValidateSet('health','template','log','summary')][string]$Command = 'health',
    [string]$Date = ([DateTime]::Now.ToString('yyyy-MM-dd')),
    [string]$Text,
    [ValidateSet('happy','neutral','sad','excited','tired','anxious')][string]$Mood = 'neutral',
    [int]$Energy = 0,
    [int]$Focus = 0
)

$ErrorActionPreference = 'Stop'
if ($Energy -lt 0 -or $Energy -gt 10) { throw 'Energy must be between 0 and 10.' }
if ($Focus -lt 0 -or $Focus -gt 10) { throw 'Focus must be between 0 and 10.' }
$base = if ($env:CODEX_DATA_DIR) { Join-Path $env:CODEX_DATA_DIR 'daily-reflection' } else { Join-Path (Get-Location) 'codex-data\daily-reflection' }
$logFile = Join-Path $base 'entries.jsonl'
function Ensure-Store { New-Item -ItemType Directory -Force -Path $base | Out-Null }

if ($Command -eq 'health') {
    [ordered]@{ ready = $true; storage = $logFile; runtime = 'PowerShell'; message = 'Codex-native reflection storage is available.' } | ConvertTo-Json
    exit 0
}
if ($Command -eq 'template') {
    $template = @"
# 日复盘

日期：$Date

## 今天做得好的
- 

## 今天学到的/可以改进的
- 

## 感恩
- 

## 明日一个行动
- 
"@
    $template.Trim()
    exit 0
}
if ($Command -eq 'log') {
    Ensure-Store
    $entry = [ordered]@{ date = $Date; text = $Text; mood = $Mood; energy = $Energy; focus = $Focus; created_at = [DateTime]::Now.ToString('o') }
    ($entry | ConvertTo-Json -Compress) | Add-Content -LiteralPath $logFile -Encoding UTF8
    $entry | ConvertTo-Json -Depth 8
    exit 0
}
if ($Command -eq 'summary') {
    Ensure-Store
    if (-not (Test-Path -LiteralPath $logFile)) { '[]'; exit 0 }
    @((Get-Content -LiteralPath $logFile -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })) | ConvertTo-Json -Depth 8
    exit 0
}
throw "Unsupported command: $Command"
