[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$PythonExe)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$fixture = Join-Path $repo ('tmp\usb-media-setup-1114\wrapper-' + [Guid]::NewGuid().ToString('N'))
$bin = Join-Path $fixture 'bin'
$connections = Join-Path $fixture 'connections'
New-Item -ItemType Directory -Path $bin, $connections -Force | Out-Null
$wrapper = Join-Path $bin 'start-media-generation-mcp.ps1'
Copy-Item -LiteralPath (Join-Path $repo 'installer-pack\mcp\start-media-generation-mcp.ps1') -Destination $wrapper
Copy-Item -LiteralPath (Join-Path $repo 'installer-pack\mcp\media-generation-mcp.py') -Destination (Join-Path $bin 'media-generation-mcp.py')
$python = (Resolve-Path -LiteralPath $PythonExe).Path
$utf8 = New-Object Text.UTF8Encoding($false)
$connectionPath = Join-Path $connections 'media-generation.connection.json'
$connection = '{"schemaVersion":1,"apiBaseUrl":"https://example.invalid","apiProfile":"responses"}'
[IO.File]::WriteAllText($connectionPath, $connection, $utf8)
Add-Type -AssemblyName System.Security
$plain = [Text.Encoding]::UTF8.GetBytes('synthetic-offline-media-fixture')
$protected = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
[Array]::Clear($plain, 0, $plain.Length)
$blob = [byte[]]([Text.Encoding]::ASCII.GetBytes('CDXMGDP1') + $protected)
$credentialPath = Join-Path $connections 'media-generation.credential.dpapi'
[IO.File]::WriteAllBytes($credentialPath, $blob)
$names = @('MEDIA_GENERATION_API_BASE_URL','MEDIA_GENERATION_API_PROFILE','MEDIA_GENERATION_API_KEY','MEDIA_GENERATION_SIGNED_LAUNCH','MEDIA_GENERATION_CONFIG_PATH','MEDIA_GENERATION_AGENT_ID','MEDIA_IMAGE2_MODEL','MEDIA_GROK_IMAGE_MODEL','MEDIA_GROK_VIDEO_MODEL','MEDIA_SEEDANCE_MODEL','MEDIA_MINIMAX_VIDEO_MODEL')
$global:MediaCredentialFixtureSettings = [pscustomobject]@{}
function Get-ItemProperty {
    param([string]$LiteralPath, [string]$ErrorAction)
    if ($LiteralPath -ne 'HKCU:\Environment') { throw 'Unexpected fixture registry read' }
    return $global:MediaCredentialFixtureSettings
}
function Invoke-Check {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    (& $wrapper -CredentialCheck -PythonExe $python | Out-String) | ConvertFrom-Json
}
function Assert-Ready($value) {
    if ($value.credentialSource -ne 'dpapi-current-user' -or -not $value.apiKeyConfigured -or -not $value.baseUrlConfigured -or $value.configuredModelCount -lt 1) { throw 'Expected ready credential check' }
}
$cases = New-Object System.Collections.Generic.List[object]
$value = Invoke-Check
Assert-Ready $value
if ($value.configuredModelCount -ne 1) { throw 'Default did not reach real model resolver' }
$cases.Add([ordered]@{case='no-history-model';passed=$true;models=$value.configuredModelCount})
$global:MediaCredentialFixtureSettings = [pscustomobject]@{MEDIA_IMAGE2_MODEL='existing-image-model';MEDIA_GROK_VIDEO_MODEL='existing-video-model'}
$value = Invoke-Check
Assert-Ready $value
if ($env:MEDIA_IMAGE2_MODEL -ne 'existing-image-model' -or $env:MEDIA_GROK_VIDEO_MODEL -ne 'existing-video-model') { throw 'Existing settings changed' }
$cases.Add([ordered]@{case='existing-models-preserved';passed=$true;models=$value.configuredModelCount})
$global:MediaCredentialFixtureSettings = [pscustomobject]@{}
$catalog = Join-Path $connections 'media-generation.json'
[IO.File]::WriteAllText($catalog, '{"schemaVersion":1,"gateway":"https://example.invalid","models":{"image-2":"saved-catalog-image","grok-video":"saved-catalog-video"}}', $utf8)
$value = Invoke-Check
Assert-Ready $value
if ($value.configuredModelCount -ne 2 -or -not [string]::IsNullOrEmpty($env:MEDIA_IMAGE2_MODEL)) { throw 'Saved catalog was overridden by process defaults' }
$cases.Add([ordered]@{case='saved-catalog-models-preserved';passed=$true;models=$value.configuredModelCount})
[IO.File]::WriteAllBytes($credentialPath, [byte[]]([Text.Encoding]::ASCII.GetBytes('CDXMGDP1') + [byte[]]@(0)))
$value = Invoke-Check
if ($value.apiKeyConfigured -or $value.credentialSource -ne 'missing' -or -not $value.credentialErrorType -or $null -eq $value.credentialErrorCode) { throw 'Credential failure did not retain safe diagnostics' }
$cases.Add([ordered]@{case='authorization-failure';passed=$true;exceptionType=$value.credentialErrorType;systemCode=$value.credentialErrorCode})
[IO.File]::WriteAllBytes($credentialPath, $blob)
$value = Invoke-Check
Assert-Ready $value
if ($value.credentialErrorType -or $null -ne $value.credentialErrorCode) { throw 'Old failure survived recovery' }
$cases.Add([ordered]@{case='authorization-recovered';passed=$true})
[IO.File]::WriteAllText($connectionPath, '{invalid-fixture', $utf8)
$value = Invoke-Check
if ($value.baseUrlConfigured -or -not $value.apiKeyConfigured -or -not $value.connectionErrorType) { throw 'Connection failure not distinguished' }
$cases.Add([ordered]@{case='connection-failure';passed=$true;exceptionType=$value.connectionErrorType})
[IO.File]::WriteAllText($connectionPath, $connection, $utf8)
$value = Invoke-Check
Assert-Ready $value
if ($value.connectionErrorType) { throw 'Old connection failure survived recovery' }
$cases.Add([ordered]@{case='connection-recovered';passed=$true})
$result = [ordered]@{scope='real-wrapper-real-python-PS5.1-isolated-DPAPI';mocked=@('HKCU Environment');networkRequests=0;userConfigOrRegistryModified=$false;cases=$cases.ToArray()}
$json = $result | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $fixture 'result.json'), $json, $utf8)
$json
