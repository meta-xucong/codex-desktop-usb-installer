$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repo 'installer-pack\install-to-codex.ps1'
$tokens = $null; $parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Installer parse failed' }
foreach ($function in $ast.FindAll({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst]}, $false)) {
    . ([scriptblock]::Create($function.Extent.Text))
}
$start = $ast.EndBlock.Statements | Where-Object {
    $_ -is [System.Management.Automation.Language.AssignmentStatementAst] -and $_.Left.Extent.Text -eq '$wrapperTarget'
} | Select-Object -First 1
if (-not $start) { throw 'Actual installer finalization entry not found' }
$tail = [scriptblock]::Create($ast.Extent.Text.Substring($start.Extent.StartOffset))
$fixture = Join-Path (Join-Path $repo 'tmp') ('cap-finalize-' + [guid]::NewGuid().ToString('N'))
$utf8NoBom = [Text.UTF8Encoding]::new($false)
foreach ($case in @('media-failed','config-failed','model-failed','hyperframes-cli-failed','hyperframes-browser-failed','success')) {
    $caseRoot = Join-Path $fixture $case
    $PackRoot = Join-Path $caseRoot 'pack'
    $userProfile = Join-Path $caseRoot 'profile'
    $codexHome = Join-Path $userProfile 'codex-home'
    $ownershipRoot = Join-Path $codexHome 'codex-agent-kit'
    $ownershipFile = Join-Path $ownershipRoot 'install-manifest.json'
    $skillTarget = Join-Path $codexHome 'skills'
    $agentTarget = Join-Path $codexHome 'agents'
    $diagnosticsRoot = Join-Path $ownershipRoot 'diagnostics'
    $runtimeDiagnosticPath = Join-Path $diagnosticsRoot 'runtime-provision.log'
    $pdfSmokeDiagnosticPath = Join-Path $diagnosticsRoot 'pdf-smoke.log'
    $runtimeStatePath = Join-Path $ownershipRoot 'runtime-state.json'
    $ProgressFile = Join-Path $caseRoot 'progress.txt'
    New-Item -ItemType Directory -Force -Path $PackRoot,$skillTarget,$agentTarget,$diagnosticsRoot,(Join-Path $ownershipRoot 'bin'),(Join-Path $ownershipRoot 'config') | Out-Null
    # Isolate external stages; execute the actual installer orchestration and writers.
    [IO.File]::WriteAllText((Join-Path $ownershipRoot 'bin\start-media-generation-mcp.ps1'), @'
param([switch]$HealthCheck)
if ($case -eq 'media-failed') { throw 'fixture media protocol failure' }
$global:LASTEXITCODE = 0
'{"ready":true,"serverVersion":"1.3.11","healthScope":"local-runtime-protocol","runtimeStatus":"passed","protocolStatus":"passed","gatewayCatalogStatus":"not_checked","generationCapability":"not_tested","toolCount":6,"python":"fixture"}'
'@, $utf8NoBom)
    [IO.File]::WriteAllText((Join-Path $PackRoot 'render-codex-config.ps1'), @'
param($OutputPath)
[IO.File]::WriteAllText($OutputPath, "[mcp_servers.media_generation]`nenabled = true`n")
'{"status":"ready","mediaGenerationEnabled":true}'
'@, $utf8NoBom)
    [IO.File]::WriteAllText((Join-Path $PackRoot 'configure-codex.ps1'), @'
param($GeneratedFragment,$TargetCodexHome)
if ($case -eq 'config-failed') { throw 'fixture config failure' }
Copy-Item -LiteralPath $GeneratedFragment -Destination (Join-Path $TargetCodexHome 'config.toml')
'{"status":"merged","merged":true}'
'@, $utf8NoBom)
    [IO.File]::WriteAllText((Join-Path $PackRoot 'sync-model-catalog.ps1'), @'
param($KitHome)
[IO.File]::WriteAllText((Join-Path $KitHome 'model-stage-ran'), 'yes')
if ($case -eq 'model-failed') { throw 'fixture model sync failure' }
'{"status":"ready","preferredModel":"fixture","migration":"kept"}'
'@, $utf8NoBom)
    $runtimeState = [ordered]@{
        status = 'ready'; python = @{ health = 'passed'; executable = '' }
        manim = @{ health = 'passed' }; node = @{ health = 'passed' }; ffmpeg = @{ health = 'passed' }
        hyperframes = @{ cliHealth = 'passed'; browserHealth = 'passed' }
        feishu = @{ health = 'disabled'; packageRoot = '' }; mediaGeneration = @{}
    }
    if ($case -like 'hyperframes-*') {
        $runtimeState.status = 'partial'
        $runtimeState.hyperframes.error = 'fixture local HyperFrames failure'
        if ($case -eq 'hyperframes-cli-failed') { $runtimeState.hyperframes.cliHealth = 'install-failed' }
        else { $runtimeState.hyperframes.browserHealth = 'install-failed' }
    }
    $runtimeState = $runtimeState | ConvertTo-Json -Depth 6 | ConvertFrom-Json
    $runtimeState.mediaGeneration = @{ health = 'unknown'; protocol = 'not-checked' }
    $pdfSmokeStatus = @{ status = 'passed' }
    $installed = [System.Collections.Generic.List[object]]::new()
    $previousFiles = @(); $skillNames = @(); $agentFiles = @()
    $pack = @{ id = 'fixture'; version = '1'; sourceCommit = 'fixture' }
    $mediaGenerationEnabled = $true; $feishuEnabled = $false
    $MediaGatewayCatalogStatus = 'not-checked'; $MediaGatewayCatalogVerifiedAt = ''; $MediaGatewayModelCount = 0
    $FeishuAuthMode = 'tenant'; $FeishuDomain = 'https://open.feishu.cn'
    $EnabledRemoteMcps = @(); $EnabledApiServices = @(); $EnableSkills = @()
    $ReplaceExisting = $true; $Overwrite = $true; $EnableFullAccess = $false
    . $tail
    $report = Get-Content -LiteralPath (Join-Path $ownershipRoot 'readiness.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not (Test-Path -LiteralPath (Join-Path $ownershipRoot 'model-stage-ran'))) { throw "$case skipped model stage" }
    if (-not (Test-Path -LiteralPath $ownershipFile)) { throw "$case skipped ownership finalization" }
    if ((Get-Content -LiteralPath $ProgressFile -Raw) -ne 'pack.finalize') { throw "$case did not finish progress" }
    if ($case -eq 'success') {
        if ($report.overallStatus -ne 'ready' -or $report.connections.mediaGeneration.protocol -ne 'passed') { throw 'Success was not retained' }
    } elseif ($report.overallStatus -ne 'partial') { throw "$case was not reported partial" }
    if ($case -eq 'media-failed') {
        if (-not $report.config.merged -or $report.modelSync.status -ne 'ready') { throw 'Media failure blocked independent stages' }
        if ($report.connections.mediaGeneration.protocol -ne 'failed' -or $report.runtime.mediaGeneration.health -ne 'failed') { throw 'Media failure was not retained' }
        if ($report.runtime.mediaGeneration.reason -notmatch 'fixture media protocol failure') { throw 'Original media error missing' }
    }
    if ($case -eq 'config-failed' -and $report.config.reason -notmatch 'fixture config failure') { throw 'Original config error missing' }
    if ($case -like 'hyperframes-*') {
        if ($report.core.status -ne 'ready' -or -not $report.config.merged -or $report.connections.mediaGeneration.protocol -ne 'passed') { throw 'Optional HyperFrames failure blocked core/MCP finalization' }
        if ($report.runtime.hyperframes.error -ne 'fixture local HyperFrames failure') { throw 'Optional failure details were lost' }
    }
    if ($case -eq 'model-failed' -and $report.modelSync.reason -notmatch 'fixture model sync failure') { throw 'Original model error missing' }
    Write-Output "Finalization case passed: $case"
}
$resolved = (Get-Item -LiteralPath $fixture).FullName
$testRoot = [IO.Path]::GetFullPath((Join-Path $repo 'tmp')).TrimEnd('\') + '\'
if (-not $resolved.StartsWith($testRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notlike 'cap-finalize-*') { throw 'Unexpected test cleanup target' }
Remove-Item -LiteralPath $resolved -Recurse -Force
