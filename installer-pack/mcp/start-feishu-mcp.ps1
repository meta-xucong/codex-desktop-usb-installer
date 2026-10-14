[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackageCache,
    [ValidateSet('tenant','oauth','user')][string]$AuthMode = 'tenant',
    [string]$Domain = 'https://open.feishu.cn',
    [string]$ToolsManifest,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$allowedDomains = @('https://open.feishu.cn', 'https://open.larksuite.com')
if ($allowedDomains -notcontains $Domain.TrimEnd('/')) {
    throw "Unsupported Feishu/Lark domain: $Domain"
}
$Domain = $Domain.TrimEnd('/')

$kitRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($ToolsManifest)) {
    $ToolsManifest = Join-Path $kitRoot 'manifest\feishu-tools.json'
}
if (-not (Test-Path -LiteralPath $ToolsManifest -PathType Leaf)) {
    throw "Feishu tool manifest not found: $ToolsManifest"
}
$toolContract = Get-Content -LiteralPath $ToolsManifest -Raw -Encoding UTF8 | ConvertFrom-Json
if ([int]$toolContract.schemaVersion -ne 1 -or [string]$toolContract.serverId -ne 'feishu') {
    throw 'Unsupported Feishu tool manifest.'
}
if ([string]$toolContract.toolNameCase -ne 'dot') {
    throw 'Feishu tool manifest must use dot-case names.'
}
$tools = @($toolContract.readTools) + @($toolContract.writeTools)
if ($tools.Count -lt 1 -or @($tools | Select-Object -Unique).Count -ne $tools.Count) {
    throw 'Feishu tool manifest is empty or contains duplicate tools.'
}
if ([string]$toolContract.packageName -ne '@larksuiteoapi/lark-mcp' -or
    [string]$toolContract.packageVersion -ne '0.5.1' -or
    [string]$toolContract.cliRelativePath -ne 'node_modules/@larksuiteoapi/lark-mcp/dist/cli.js') {
    throw 'Feishu package name/version contract is invalid.'
}

$appId = [Environment]::GetEnvironmentVariable('FEISHU_APP_ID', 'Process')
$appSecret = [Environment]::GetEnvironmentVariable('FEISHU_APP_SECRET', 'Process')
$userAccessToken = [Environment]::GetEnvironmentVariable('FEISHU_USER_ACCESS_TOKEN', 'Process')
if ([string]::IsNullOrWhiteSpace($appId) -or $appId -notmatch '^cli_[A-Za-z0-9_-]+$') {
    throw 'FEISHU_APP_ID is missing or invalid.'
}
if ([string]::IsNullOrWhiteSpace($appSecret) -or $appSecret.Length -lt 8) {
    throw 'FEISHU_APP_SECRET is missing or invalid.'
}
if ($AuthMode -eq 'user' -and [string]::IsNullOrWhiteSpace($userAccessToken)) {
    throw 'FEISHU_USER_ACCESS_TOKEN is required when AuthMode=user.'
}

$cacheFull = [IO.Path]::GetFullPath($PackageCache)
$packageRoot = Join-Path $cacheFull 'node_modules\@larksuiteoapi\lark-mcp'
$packageJsonPath = Join-Path $packageRoot 'package.json'
$cliPath = Join-Path $packageRoot 'dist\cli.js'
$packagePresent = (Test-Path -LiteralPath $packageJsonPath -PathType Leaf) -and (Test-Path -LiteralPath $cliPath -PathType Leaf)
$packageValidated = $false
if ($packagePresent) {
    try {
        $installedPackage = Get-Content -LiteralPath $packageJsonPath -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        throw "Feishu package metadata is invalid: $packageJsonPath"
    }
    if ([string]$installedPackage.name -ne [string]$toolContract.packageName -or [string]$installedPackage.version -ne [string]$toolContract.packageVersion) {
        throw "Feishu package does not match the locked contract: $($installedPackage.name)@$($installedPackage.version)"
    }
    $packageValidated = $true
}

$nodePath = $null
$configuredNode = [Environment]::GetEnvironmentVariable('CODEX_NODE', 'Process')
foreach ($candidate in @(
        (Join-Path $kitRoot 'runtime\node-v22.22.0-win-x64\node.exe'),
        $configuredNode
    )) {
    if (-not [string]::IsNullOrWhiteSpace([string]$candidate) -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        $nodePath = [IO.Path]::GetFullPath([string]$candidate)
        break
    }
}
if ([string]::IsNullOrWhiteSpace($nodePath)) {
    $nodeCommand = Get-Command node -ErrorAction SilentlyContinue
    if ($nodeCommand) { $nodePath = [string]$nodeCommand.Source }
}
$nodeVersionText = $null
$nodeReady = $false
if (-not [string]::IsNullOrWhiteSpace($nodePath)) {
    $nodeVersionText = (& $nodePath --version 2>&1 | Out-String).Trim().TrimStart('v')
    $nodeVersion = $null
    $nodeReady = [version]::TryParse($nodeVersionText, [ref]$nodeVersion) -and $nodeVersion.Major -ge 20
}

$arguments = @(
    'mcp',
    '--app-id', $appId,
    '--app-secret', $appSecret,
    '--domain', $Domain,
    '--tool-name-case', 'dot',
    '--language', 'zh',
    '--tools', ($tools -join ',')
)
switch ($AuthMode) {
    'tenant' { $arguments += @('--token-mode', 'tenant_access_token') }
    'oauth' { $arguments += @('--oauth', '--token-mode', 'user_access_token') }
    'user' { $arguments += @('--user-access-token', $userAccessToken, '--token-mode', 'user_access_token') }
}

if ($DryRun) {
    [ordered]@{
        schemaVersion = 1
        command = if ($nodePath) { $nodePath } else { 'node' }
        entrypoint = $cliPath
        packageCache = $cacheFull
        packagePresent = [bool]$packagePresent
        packageValidated = [bool]$packageValidated
        package = [string]$toolContract.package
        nodeVersion = $nodeVersionText
        nodeReady = [bool]$nodeReady
        domain = $Domain
        authMode = $AuthMode
        toolNameCase = 'dot'
        tools = $tools
        credentialSources = @('FEISHU_APP_ID', 'FEISHU_APP_SECRET') + $(if ($AuthMode -eq 'user') { @('FEISHU_USER_ACCESS_TOKEN') } else { @() })
        credentialsPresent = $true
        secretsRendered = $false
    } | ConvertTo-Json -Depth 6
    return
}

if (-not $packagePresent -or -not $packageValidated) {
    throw "Locked Feishu MCP package is not installed under PackageCache: $cacheFull"
}
if ([string]::IsNullOrWhiteSpace($nodePath)) { throw 'Node.js >=20 is required for the Feishu MCP server.' }
if (-not $nodeReady) {
    throw "Node.js >=20 is required; detected: $nodeVersionText"
}

# Do not write to stdout before the child starts: stdout is reserved for MCP JSON-RPC.
# Invoke Node directly instead of the npm-generated .cmd shim so credential values
# cannot be reinterpreted by cmd.exe metacharacter parsing.
& $nodePath $cliPath @arguments
exit $LASTEXITCODE
