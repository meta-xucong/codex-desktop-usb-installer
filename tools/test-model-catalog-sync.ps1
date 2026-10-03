[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$syncScript = Join-Path $repo 'installer-pack\sync-model-catalog.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ('codex-model-policy-test-' + [guid]::NewGuid().ToString('N'))
$utf8 = [Text.UTF8Encoding]::new($false)
function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
try {
    New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
    $pack = Join-Path $root 'pack'
    New-Item -ItemType Directory -Path $pack | Out-Null
    $catalog = [ordered]@{
        schemaVersion = 1; preferredModel = 'synthetic-preferred'; retiredModelIds = @('synthetic-retired')
        models = @(
            [ordered]@{ id='synthetic-preferred'; displayName='Synthetic'; visibility='list'; supportedInApi=$true; priority=1; defaultReasoningLevel='low'; reasoningLevels=@('low') },
            [ordered]@{ id='synthetic-retired'; displayName='Retired'; visibility='hide'; supportedInApi=$false; priority=2; defaultReasoningLevel='low'; reasoningLevels=@('low') }
        )
    }
    [IO.File]::WriteAllText((Join-Path $pack 'model-catalog.json'), ($catalog | ConvertTo-Json -Depth 8), $utf8)
    $cases = @(
        @{ name='missing'; content=$null },
        @{ name='custom'; content="model_provider = 'third-party'`r`nmodel = 'user-chosen'`r`n[other]`r`nvalue = 'keep'`r`n" },
        @{ name='retired'; content="model = 'synthetic-retired'`n" },
        @{ name='managed'; content="# BEGIN CODEX-STARTER-KIT MODEL POLICY v1`nmodel = 'synthetic-preferred'`n# END CODEX-STARTER-KIT MODEL POLICY v1`n" }
    )
    foreach ($case in $cases) {
        $profile = Join-Path $root $case.name
        $codexHome = Join-Path $profile '.codex'
        New-Item -ItemType Directory -Path $codexHome -Force | Out-Null
        $config = Join-Path $codexHome 'config.toml'
        $kit = Join-Path $profile 'kit'
        if ($null -ne $case.content) { [IO.File]::WriteAllText($config, $case.content, $utf8) }
        $before = if (Test-Path -LiteralPath $config -PathType Leaf) { (Get-FileHash -LiteralPath $config -Algorithm SHA256).Hash } else { $null }
        $result = & $syncScript -PackRoot $pack -TargetUserProfile $profile -KitHome $kit -Confirm:$false | ConvertFrom-Json
        Assert-Condition ($result.configChanged -eq $false) 'Metadata sync must not change model selection.'
        Assert-Condition ($null -eq $result.preferredModel) 'Metadata sync must not recommend an unverified default.'
        Assert-Condition ($result.modelSelectionStatus -eq 'deferred-until-verified') 'Selection must remain deferred.'
        Assert-Condition (Test-Path -LiteralPath (Join-Path $kit 'model-catalog.json') -PathType Leaf) 'Discovery metadata should still synchronize.'
        if ($null -eq $before) {
            Assert-Condition (-not (Test-Path -LiteralPath $config)) 'No config may be invented for a missing model.'
        } else {
            Assert-Condition (((Get-FileHash -LiteralPath $config -Algorithm SHA256).Hash) -eq $before) 'Existing model configuration must remain byte-identical.'
        }
    }
    Write-Output 'Metadata-only model synchronization cases passed; this does not certify Code Agent compatibility.'
} finally {
    if (Test-Path -LiteralPath $root -PathType Container) { Remove-Item -LiteralPath $root -Recurse -Force }
}
