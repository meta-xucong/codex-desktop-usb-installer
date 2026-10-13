[CmdletBinding()]
param([switch]$DryRun, [switch]$HealthCheck, [switch]$CredentialCheck, [string]$PythonExe)

$ErrorActionPreference = 'Stop'
$env:PYTHONIOENCODING = 'utf-8'
$env:PYTHONUTF8 = '1'
$kitHome = Split-Path -Parent $PSScriptRoot
$python = if ([string]::IsNullOrWhiteSpace($PythonExe)) { Join-Path $kitHome 'runtime\python-env\Scripts\python.exe' } else { [IO.Path]::GetFullPath($PythonExe) }
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
if ($HealthCheck) {
    $healthErrorPath = Join-Path ([IO.Path]::GetTempPath()) ("media-mcp-health-" + [guid]::NewGuid().ToString('N') + '.stderr.log')
    $healthOutput = ''
    $healthDiagnostic = ''
    $healthErrorPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $healthOutput = & $python $server --healthcheck 2> $healthErrorPath | Out-String
        $healthExitCode = $LASTEXITCODE
        if (Test-Path -LiteralPath $healthErrorPath -PathType Leaf) {
            $healthDiagnostic = Get-Content -LiteralPath $healthErrorPath -Raw -ErrorAction SilentlyContinue
        }
    } finally {
        $ErrorActionPreference = $healthErrorPreference
        Remove-Item -LiteralPath $healthErrorPath -Force -ErrorAction SilentlyContinue
    }
    if ($healthExitCode -ne 0) { throw "The bundled Python media MCP health check failed: $healthDiagnostic" }
    try { $health = $healthOutput | ConvertFrom-Json } catch { throw 'The bundled Python media MCP returned invalid health JSON.' }
    if (-not $health.ready -or [string]$health.serverVersion -ne '1.3.3' -or [string]$health.healthScope -ne 'local-runtime-protocol' -or [string]$health.runtimeStatus -ne 'passed' -or [string]$health.protocolStatus -ne 'passed' -or [string]$health.gatewayCatalogStatus -ne 'not_checked' -or [string]$health.generationCapability -ne 'not_tested' -or [int]$health.toolCount -ne 6) { throw 'The bundled Python media MCP local runtime/protocol check failed.' }
    $health | ConvertTo-Json -Compress
    exit 0
}
New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null
$env:MEDIA_GENERATION_DATA_DIR = $dataRoot

# Codex normally inherits the short-lived values from the USB launcher. This
# protected fallback also supports a Codex process started through another
# normal entry point after a signed USB install. The API key is never written
# into Codex TOML, a command line, or a plaintext connection file.
$connectionPath = Join-Path $kitHome 'connections\media-generation.connection.json'
$credentialPath = Join-Path $kitHome 'connections\media-generation.credential.dpapi'
$credentialSource = if (-not [string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_KEY)) { 'process' } else { 'missing' }
if (Test-Path -LiteralPath $connectionPath -PathType Leaf) {
    try {
        $connection = Get-Content -LiteralPath $connectionPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([int]$connection.schemaVersion -eq 1 -and [string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_BASE_URL)) {
            $base = [Uri]([string]$connection.apiBaseUrl)
            if ($base.Scheme -eq 'https' -and -not $base.UserInfo -and -not $base.Query -and -not $base.Fragment) {
                $env:MEDIA_GENERATION_API_BASE_URL = $base.AbsoluteUri.TrimEnd('/')
            }
        }
        if ([int]$connection.schemaVersion -eq 1 -and [string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_PROFILE)) {
            $env:MEDIA_GENERATION_API_PROFILE = [string]$connection.apiProfile
        }
    } catch {
        [Console]::Error.WriteLine('Media MCP connection settings could not be read; API calls remain disabled.')
    }
}
if ([string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_KEY) -and (Test-Path -LiteralPath $credentialPath -PathType Leaf)) {
    try {
        Add-Type -AssemblyName System.Security
        $blob = [IO.File]::ReadAllBytes($credentialPath)
        $magic = 'CDXMGDP1'
        if ($blob.Length -le 8 -or [Text.Encoding]::ASCII.GetString($blob, 0, 8) -cne $magic) {
            throw 'credential_format_invalid'
        }
        $protected = [byte[]]$blob[8..($blob.Length - 1)]
        $plain = [Security.Cryptography.ProtectedData]::Unprotect(
            $protected, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        $env:MEDIA_GENERATION_API_KEY = [Text.UTF8Encoding]::new().GetString($plain)
        [Array]::Clear($plain, 0, $plain.Length)
        $credentialSource = 'dpapi-current-user'
    } catch {
        $failureType = if ($_.Exception.InnerException) { $_.Exception.InnerException.GetType().Name } else { $_.Exception.GetType().Name }
        $failureCode = if ($_.Exception.InnerException) { $_.Exception.InnerException.HResult } else { $_.Exception.HResult }
        [Console]::Error.WriteLine(('Media MCP credential is unavailable for this Windows user ({0}, 0x{1:X8}); API calls remain disabled.' -f $failureType, $failureCode))
    }
}
try {
    $userEnvironment = Get-ItemProperty -LiteralPath 'HKCU:\Environment' -ErrorAction Stop
    foreach ($name in @('MEDIA_GENERATION_AGENT_ID','MEDIA_IMAGE2_MODEL','MEDIA_GROK_IMAGE_MODEL','MEDIA_GROK_VIDEO_MODEL','MEDIA_SEEDANCE_MODEL','MEDIA_MINIMAX_VIDEO_MODEL')) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, 'Process')) -and
            -not [string]::IsNullOrWhiteSpace([string]$userEnvironment.$name)) {
            [Environment]::SetEnvironmentVariable($name, [string]$userEnvironment.$name, 'Process')
        }
    }
} catch {
    # Models may still be supplied by the USB launcher environment.
}
if ($CredentialCheck) {
    $modelNames = @('MEDIA_IMAGE2_MODEL','MEDIA_GROK_IMAGE_MODEL','MEDIA_GROK_VIDEO_MODEL','MEDIA_SEEDANCE_MODEL','MEDIA_MINIMAX_VIDEO_MODEL')
    $configuredModels = @($modelNames | Where-Object { -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_, 'Process')) }).Count
    [ordered]@{
        baseUrlConfigured = -not [string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_BASE_URL)
        apiKeyConfigured = -not [string]::IsNullOrWhiteSpace($env:MEDIA_GENERATION_API_KEY)
        configuredModelCount = $configuredModels
        credentialSource = $credentialSource
    } | ConvertTo-Json -Compress
    exit 0
}
& $python $server
exit $LASTEXITCODE
