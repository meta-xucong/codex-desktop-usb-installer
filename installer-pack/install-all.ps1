[CmdletBinding()]
param(
    [string]$TargetUserProfile,
    [ValidateSet('tenant','oauth','user')][string]$FeishuAuthMode = 'tenant',
    [string]$FeishuDomain = 'https://open.feishu.cn',
    [string[]]$EnableSkills = @(),
    [string[]]$EnabledApiServices = @(),
    [string[]]$EnabledRemoteMcps = @(),
    [switch]$EnableFeishu,
    [switch]$DisableFeishu,
    [switch]$EnableMediaGeneration,
    [switch]$DisableMediaGeneration,
    [ValidateSet('not-checked','verified','failed')][string]$MediaGatewayCatalogStatus = 'not-checked',
    [ValidateRange(0,100000)][int]$MediaGatewayModelCount = 0,
    [string]$MediaGatewayCatalogVerifiedAt = ''
)

$ErrorActionPreference = 'Stop'

$packRoot = $PSScriptRoot
$installer = Join-Path $packRoot 'install-to-codex.ps1'
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "安装器不存在: $installer"
}
if ($EnableMediaGeneration -and $DisableMediaGeneration) { throw '-EnableMediaGeneration and -DisableMediaGeneration are mutually exclusive.' }
if ($MediaGatewayCatalogStatus -eq 'verified' -and [string]::IsNullOrWhiteSpace($MediaGatewayCatalogVerifiedAt)) {
    throw 'A verified media gateway catalog must include its verification timestamp.'
}

$arguments = @(
    '-NoProfile',
    '-ExecutionPolicy', 'Bypass',
    '-WindowStyle', 'Hidden',
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
if ($EnableMediaGeneration) { $arguments += '-EnableMediaGeneration' }
if ($DisableMediaGeneration) { $arguments += '-DisableMediaGeneration' }
if ($EnableMediaGeneration) {
    $arguments += @('-MediaGatewayCatalogStatus', $MediaGatewayCatalogStatus, '-MediaGatewayModelCount', [string]$MediaGatewayModelCount)
    if (-not [string]::IsNullOrWhiteSpace($MediaGatewayCatalogVerifiedAt)) {
        $arguments += @('-MediaGatewayCatalogVerifiedAt', $MediaGatewayCatalogVerifiedAt)
    }
}
$arguments += @('-FeishuAuthMode', $FeishuAuthMode, '-FeishuDomain', $FeishuDomain)
if (@($EnableSkills).Count -gt 0) {
    $arguments += '-EnableSkills'
    $arguments += @($EnableSkills)
}
if (@($EnabledApiServices).Count -gt 0) {
    $arguments += '-EnabledApiServices'
    $arguments += @($EnabledApiServices)
}
if (@($EnabledRemoteMcps).Count -gt 0) {
    $arguments += '-EnabledRemoteMcps'
    $arguments += @($EnabledRemoteMcps)
}

$powershell = (Get-Command powershell.exe -ErrorAction Stop).Source
& $powershell @arguments
exit $LASTEXITCODE
