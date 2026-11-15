[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PythonExe,
    [Parameter(Mandatory=$true)][string]$EvidenceRoot,
    [string]$PackRoot
)
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($PackRoot)) { $PackRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'installer-pack' }
$fixture = Join-Path ([IO.Path]::GetFullPath($EvidenceRoot)) ('config-merge-' + [guid]::NewGuid().ToString('N'))
$utf8 = [Text.UTF8Encoding]::new($false)
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
$guard = Join-Path $fixture 'network-guard'
New-Item -ItemType Directory -Path $guard | Out-Null
[IO.File]::WriteAllText((Join-Path $guard 'sitecustomize.py'), @'
import os, sys
def reject_network(event, args):
    if event in ('socket.connect', 'socket.getaddrinfo', 'socket.sendto', 'socket.sendmsg'):
        with open(os.environ['DIAG_NETWORK_LOG'], 'a', encoding='ascii') as stream:
            stream.write(event + '\n')
        raise RuntimeError('test_network_blocked')
sys.addaudithook(reject_network)
'@, $utf8)
$env:PYTHONPATH = $guard
$env:PYTHONDONTWRITEBYTECODE = '1'
$env:DIAG_NETWORK_LOG = Join-Path $fixture 'network-attempts.txt'
Remove-Item Env:MEDIA_GENERATION_API_KEY -ErrorAction SilentlyContinue
$ErrorActionPreference = 'Continue'
$controlOutput = & $PythonExe -B -c "import socket; socket.create_connection(('127.0.0.1',9))" 2>&1
$ErrorActionPreference = 'Stop'
if ($LASTEXITCODE -eq 0 -or -not (Test-Path -LiteralPath $env:DIAG_NETWORK_LOG)) { throw 'Network control was not blocked' }
$controlCount = @(Get-Content -LiteralPath $env:DIAG_NETWORK_LOG).Count
$kit = Join-Path $fixture 'kit'
$bin = Join-Path $kit 'bin'
New-Item -ItemType Directory -Path $bin -Force | Out-Null
$env:MEDIA_GENERATION_DATA_DIR = Join-Path $kit 'data\media-generation'
& $PythonExe -B -m venv --without-pip (Join-Path $kit 'runtime\python-env')
if ($LASTEXITCODE -ne 0) { throw 'Fixture venv failed' }
Copy-Item -LiteralPath (Join-Path $PackRoot 'mcp\start-media-generation-mcp.ps1') -Destination $bin
Copy-Item -LiteralPath (Join-Path $PackRoot 'mcp\media-generation-mcp.py') -Destination $bin
$wrapper = Join-Path $bin 'start-media-generation-mcp.ps1'
$registeredPattern = '(?m)^\[mcp_servers\.media_generation\][ \t]*\r?\nenabled[ \t]*=[ \t]*true[ \t]*(?:#.*)?\r?$'
$preserved = @'
model = "fixture-model"
model_provider = "fixture-provider"
[desktop]
conversationDetailMode = "STEPS_COMMANDS"
[mcp_servers.user_local]
command = "fixture.exe"
args = ["keep"]
[plugins."latex@openai-bundled"]
enabled = false
'@
$verifyPython = @'
import pathlib,sys,tomllib
p=pathlib.Path(sys.argv[1]); cfg=tomllib.loads(p.read_text(encoding='utf-8-sig'))
assert cfg['mcp_servers']['media_generation']['enabled'] is True
if sys.argv[2]=='preserve':
    assert cfg['model']=='fixture-model' and cfg['model_provider']=='fixture-provider'
    assert cfg['desktop']['conversationDetailMode']=='STEPS_COMMANDS'
    assert cfg['mcp_servers']['user_local']=={'command':'fixture.exe','args':['keep']}
    assert cfg['plugins']['latex@openai-bundled']['enabled'] is False
'@
$cases = @('no-config','zero-byte','only-replaced-table','existing-settings','bom-crlf','repeat-install','invalid-toml','missing-fragment')
$rows = @()
foreach ($case in $cases) {
    $caseRoot = Join-Path $fixture ($case + ' with spaces')
    $caseHome = Join-Path $caseRoot 'home'
    New-Item -ItemType Directory -Path $caseHome -Force | Out-Null
    $config = Join-Path $caseHome 'config.toml'
    if ($case -eq 'zero-byte') { [IO.File]::WriteAllText($config, '', $utf8) }
    if ($case -eq 'only-replaced-table') { [IO.File]::WriteAllText($config, "[mcp_servers.media_generation]`nenabled = true`n", $utf8) }
    if ($case -in @('existing-settings','repeat-install','missing-fragment')) { [IO.File]::WriteAllText($config, $preserved, $utf8) }
    if ($case -eq 'bom-crlf') { [IO.File]::WriteAllText($config, $preserved.Replace("`r`n","`n").Replace("`n","`r`n"), [Text.UTF8Encoding]::new($true)) }
    if ($case -eq 'invalid-toml') { [IO.File]::WriteAllText($config, "model = [`n", $utf8) }
    $before = if(Test-Path -LiteralPath $config){[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))}else{$null}
    $generated = Join-Path $caseRoot 'generated.toml'
    $expectedFailure = $case -in @('invalid-toml','missing-fragment')
    $caught = $null
    $phase = 'render'
    try {
        $rounds = if ($case -eq 'repeat-install') { 2 } else { 1 }
        for($round=1; $round -le $rounds; $round++) {
            $phase = 'render'
            $renderText = & (Join-Path $PackRoot 'render-codex-config.ps1') -OutputPath $generated -TargetUserProfile $caseRoot -TargetCodexHome $caseHome -SkillRoot (Join-Path $caseRoot 'skills') -MediaMcpWrapperPath $wrapper -EnableMediaGeneration -PythonExe $PythonExe -Overwrite | Out-String
            $render = $renderText | ConvertFrom-Json
            if (-not $render.mediaGenerationEnabled) { throw 'Renderer did not enable media' }
            $inputFragment = if($case -eq 'missing-fragment'){Join-Path $caseRoot 'absent.toml'}else{$generated}
            $phase = 'merge'
            $mergeText = & (Join-Path $PackRoot 'configure-codex.ps1') -GeneratedFragment $inputFragment -TargetUserProfile $caseRoot -TargetCodexHome $caseHome -KitHome $kit -PythonExe $PythonExe -ReplaceExisting | Out-String
            $merge = $mergeText | ConvertFrom-Json
            if (-not $merge.merged) { throw 'Config did not merge' }
            $text = [IO.File]::ReadAllText($config)
            if (-not [regex]::IsMatch($text,$registeredPattern)) { throw 'Media registration did not pass' }
            foreach($marker in @('# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1','# END CODEX-STARTER-KIT MANAGED CONFIG v1')) {
                if ([regex]::Matches($text,'(?m)^'+[regex]::Escape($marker)+'\r?$').Count -ne 1) { throw 'Managed block count changed' }
            }
            $mode = if($case -in @('existing-settings','bom-crlf','repeat-install')){'preserve'}else{'new'}
            & $PythonExe -B -c $verifyPython $config $mode
            if ($LASTEXITCODE -ne 0) { throw 'TOML or retained settings verification failed' }
        }
    } catch { $caught = $_ }
    if ($expectedFailure) {
        if ($null -eq $caught) { throw "$case unexpectedly succeeded" }
        $expectedMessage = if($case -eq 'invalid-toml'){'Current-package configuration replacement failed.'}else{'Generated Codex fragment is missing:'}
        if ($phase -ne 'merge' -or -not $caught.Exception.Message.Contains($expectedMessage)) { throw "${case}: unrelated failure: $($caught.Exception.Message)" }
        $after = [Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
        if ($before -cne $after) { throw "$case changed existing configuration" }
        $rows += [pscustomobject]@{case=$case;passed=$true;outcome='rejected-with-existing-config-unchanged';errorType=$caught.Exception.GetType().Name}
    } else {
        if ($null -ne $caught) { throw $caught }
        $rows += [pscustomobject]@{case=$case;passed=$true;outcome='merged-registered-and-settings-verified'}
    }
}
$attempts = @(Get-Content -LiteralPath $env:DIAG_NETWORK_LOG).Count - $controlCount
if ($attempts -ne 0) { throw 'Configuration workflow attempted network access' }
$result = [ordered]@{time=[DateTimeOffset]::Now.ToString('o');fixture=$fixture;scope='real_config_scripts_and_media_protocol_not_full_install';networkControlBlocked=($controlCount -eq 1);testNetworkAttempts=$attempts;passed=$true;cases=$rows}
$json = $result | ConvertTo-Json -Depth 5
[IO.File]::WriteAllText((Join-Path $fixture 'result.json'), $json, $utf8)
$json
