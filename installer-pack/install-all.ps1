[CmdletBinding()]
param(
    [string]$TargetUserProfile,
    [ValidateSet('tenant','oauth','user')][string]$FeishuAuthMode = 'tenant',
    [string]$FeishuDomain = 'https://open.feishu.cn',
    [string[]]$EnableSkills = @(),
    [string[]]$EnabledApiServices = @(),
    [switch]$EnableFeishu,
    [switch]$DisableFeishu
)

$ErrorActionPreference = 'Stop'

$packRoot = $PSScriptRoot
$installer = Join-Path $packRoot 'install-to-codex.ps1'
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "安装器不存在: $installer"
}

$arguments = @(
    '-NoProfile',
    '-ExecutionPolicy', 'Bypass',
    '-File', $installer,
    '-PackRoot', $packRoot,
    '-IncludeNonDefault',
    '-IncludeUnsupported',
    '-Overwrite'
)

if (-not [string]::IsNullOrWhiteSpace($TargetUserProfile)) {
    $arguments += @('-TargetUserProfile', $TargetUserProfile)
}
if ($EnableFeishu) { $arguments += '-EnableFeishu' }
if ($DisableFeishu) { $arguments += '-DisableFeishu' }
$arguments += @('-FeishuAuthMode', $FeishuAuthMode, '-FeishuDomain', $FeishuDomain)
if (@($EnableSkills).Count -gt 0) {
    $arguments += '-EnableSkills'
    $arguments += @($EnableSkills)
}
if (@($EnabledApiServices).Count -gt 0) {
    $arguments += '-EnabledApiServices'
    $arguments += @($EnabledApiServices)
}

$powershell = (Get-Command powershell.exe -ErrorAction Stop).Source
& $powershell @arguments
exit $LASTEXITCODE
