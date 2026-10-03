[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)][string]$PackRoot,
    [Parameter(Mandatory = $true)][string]$TargetUserProfile,
    [Parameter(Mandatory = $true)][string]$KitHome,
    [string]$PythonExe
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$profileFull = [IO.Path]::GetFullPath($TargetUserProfile).TrimEnd('\')
$catalogSource = Join-Path $PackRoot 'model-catalog.json'
$catalogTarget = Join-Path $KitHome 'model-catalog.json'
$configPath = Join-Path $profileFull '.codex\config.toml'

if (-not (Test-Path -LiteralPath $catalogSource -PathType Leaf)) { throw "Model catalog is missing: $catalogSource" }

function Test-IntegerValue([object]$Value) {
    return $Value -is [sbyte] -or $Value -is [byte] -or $Value -is [int16] -or $Value -is [uint16] -or $Value -is [int32] -or $Value -is [uint32] -or $Value -is [int64] -or $Value -is [uint64]
}

function Assert-StrictProperties([object]$Object, [string[]]$Required, [string[]]$Allowed, [string]$Context) {
    if ($null -eq $Object -or $Object -isnot [pscustomobject]) { throw "$Context must be an object." }
    $names = @($Object.PSObject.Properties.Name)
    foreach ($name in $Required) {
        if ($names -notcontains $name) { throw "$Context is missing required field: $name" }
    }
    foreach ($name in $names) {
        if ($Allowed -notcontains $name) { throw "$Context contains unknown field: $name" }
    }
}

function Assert-NonEmptyString([object]$Value, [string]$Context) {
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace($Value)) { throw "$Context must be a non-empty string." }
}

try {
    $catalog = Get-Content -LiteralPath $catalogSource -Raw -Encoding UTF8 | ConvertFrom-Json
} catch {
    throw "Model catalog is not valid JSON: $($_.Exception.Message)"
}

Assert-StrictProperties $catalog @('schemaVersion', 'preferredModel', 'retiredModelIds', 'models') @('schemaVersion', 'source', 'snapshotVersion', 'clientVersion', 'preferredModel', 'retiredModelIds', 'models') 'Model catalog'
if (-not (Test-IntegerValue $catalog.schemaVersion) -or [int64]$catalog.schemaVersion -ne 1) { throw 'Model catalog schemaVersion must be integer 1.' }
Assert-NonEmptyString $catalog.preferredModel 'Model catalog preferredModel'
foreach ($metadataField in @('source', 'snapshotVersion', 'clientVersion')) {
    if ($catalog.PSObject.Properties.Name -contains $metadataField) { Assert-NonEmptyString $catalog.$metadataField "Model catalog $metadataField" }
}
if ($catalog.retiredModelIds -isnot [array]) { throw 'Model catalog retiredModelIds must be an array.' }
$retiredIds = @()
$retiredSeen = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
foreach ($retired in @($catalog.retiredModelIds)) {
    Assert-NonEmptyString $retired 'Model catalog retiredModelIds entry'
    if (-not $retiredSeen.Add($retired)) { throw "Model catalog retiredModelIds contains a duplicate: $retired" }
    $retiredIds += $retired
}
if ($catalog.models -isnot [array] -or @($catalog.models).Count -eq 0) { throw 'Model catalog models must be a non-empty array.' }

$modelEntryFields = @('id', 'displayName', 'visibility', 'supportedInApi', 'priority', 'defaultReasoningLevel', 'reasoningLevels')
$modelIds = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
foreach ($model in @($catalog.models)) {
    Assert-StrictProperties $model $modelEntryFields $modelEntryFields 'Model catalog model entry'
    Assert-NonEmptyString $model.id 'Model catalog model id'
    if ([string]$model.id -notmatch '^[a-z0-9][a-z0-9.-]*$' -or -not $modelIds.Add([string]$model.id)) { throw "Invalid or duplicate model id: $($model.id)" }
    Assert-NonEmptyString $model.displayName 'Model catalog model displayName'
    if ([string]$model.visibility -notin @('list', 'hide')) { throw "Model catalog model $($model.id) visibility must be list or hide." }
    if ($model.supportedInApi -isnot [bool]) { throw "Model catalog model $($model.id) supportedInApi must be boolean." }
    if (-not (Test-IntegerValue $model.priority)) { throw "Model catalog model $($model.id) priority must be an integer." }
    Assert-NonEmptyString $model.defaultReasoningLevel "Model catalog model $($model.id) defaultReasoningLevel"
    if ($model.reasoningLevels -isnot [array] -or @($model.reasoningLevels).Count -eq 0) { throw "Model catalog model $($model.id) reasoningLevels must be a non-empty array." }
    $reasoningSeen = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    foreach ($level in @($model.reasoningLevels)) {
        Assert-NonEmptyString $level "Model catalog model $($model.id) reasoningLevels entry"
        if (-not $reasoningSeen.Add($level)) { throw "Model catalog model $($model.id) reasoningLevels contains a duplicate: $level" }
    }
    if (-not $reasoningSeen.Contains([string]$model.defaultReasoningLevel)) { throw "Model catalog model $($model.id) defaultReasoningLevel must be listed in reasoningLevels." }
}
foreach ($retiredId in $retiredIds) {
    $retiredEntry = @($catalog.models | Where-Object { [string]$_.id -eq [string]$retiredId })[0]
    if ($null -ne $retiredEntry -and [string]$retiredEntry.visibility -eq 'list' -and $retiredEntry.supportedInApi -eq $true) {
        throw "Retired model $retiredId cannot remain visible and active in the model catalog."
    }
}
if (-not $modelIds.Contains([string]$catalog.preferredModel)) { throw 'Preferred model is absent from the model catalog.' }
$preferredEntry = @($catalog.models | Where-Object { [string]$_.id -eq [string]$catalog.preferredModel })[0]
if ([string]$preferredEntry.visibility -ne 'list' -or $preferredEntry.supportedInApi -ne $true) { throw 'Model catalog preferredModel must be visible (list) and supportedInApi=true.' }
$activeIds = @($catalog.models | Where-Object {
        $_.supportedInApi -eq $true -and
        [string]$_.visibility -eq 'list' -and
        -not $retiredSeen.Contains([string]$_.id)
    } | ForEach-Object { [string]$_.id })
$catalogText = ($catalog | ConvertTo-Json -Depth 8) + [Environment]::NewLine
if ($catalogText -match '(?i)(model_messages|persistent_instructions|instructions_template|api[_-]?key|app[_-]?secret|access[_-]?token|password|bearer)') {
    throw 'Model catalog contains runtime instructions or credential-like fields.'
}
$plannedActions = @()
$whatIfMode = [bool]$WhatIfPreference

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    $temp = "$Path.tmp-$([guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllText($temp, $Text, $utf8NoBom)
        Move-Item -LiteralPath $temp -Destination $Path -Force
    } finally {
        if (Test-Path -LiteralPath $temp -PathType Leaf) { Remove-Item -LiteralPath $temp -Force }
    }
}

$catalogChanged = $true
if (Test-Path -LiteralPath $catalogTarget -PathType Leaf) {
    $existingCatalog = [IO.File]::ReadAllText($catalogTarget, [Text.Encoding]::UTF8)
    $catalogChanged = $existingCatalog -ne $catalogText
}
if ($catalogChanged) {
    # This is installer-owned discovery metadata, not capability evidence or
    # permission to select a model. User configuration is never changed here.
    $plannedActions += [ordered]@{ action = 'write'; kind = 'model-catalog'; path = $catalogTarget }
    if ($PSCmdlet.ShouldProcess($catalogTarget, 'Write the synchronized model catalog')) {
        Write-Utf8NoBom $catalogTarget $catalogText
    }
}

# The bundled snapshot is not proof that a model is usable by this account.
# Keep every existing user setting, including old managed blocks, unchanged.
if (Test-Path -LiteralPath $configPath -PathType Leaf) {
    $existing = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8
} else {
    $existing = ''
}
$lines = @($existing -split '\r?\n')
$firstTable = -1
for ($index = 0; $index -lt $lines.Count; $index++) {
    if ([string]$lines[$index] -match '^\s*\[') { $firstTable = $index; break }
}
$rootEnd = if ($firstTable -ge 0) { $firstTable } else { $lines.Count }
$modelLineIndex = -1
$currentModel = $null
for ($index = 0; $index -lt $rootEnd; $index++) {
    $match = [regex]::Match([string]$lines[$index], '^\s*model\s*=\s*"(?<id>[^"]+)"\s*(?:#.*)?$')
    if (-not $match.Success) {
        $match = [regex]::Match([string]$lines[$index], "^\s*model\s*=\s*'(?<id>[^']+)'\s*(?:#.*)?$")
    }
    if ($match.Success) { $modelLineIndex = $index; $currentModel = $match.Groups['id'].Value; break }
}
# No fallback, retirement migration, or default-model write is authorized by
# this metadata sync. Only the native verified-ready path may choose defaults.
$migration = if ($null -eq $currentModel) { 'default-selection-deferred' } else { 'preserved-existing-model' }

[ordered]@{
    schemaVersion = 1
    status = if ($whatIfMode) { 'planned' } else { 'ready' }
    preferredModel = $null
    modelSelectionStatus = 'deferred-until-verified'
    currentModel = $currentModel
    activeModelCount = @($activeIds).Count
    retiredModelIds = $retiredIds
    migration = $migration
    configChanged = $false
    catalogChanged = $catalogChanged
    catalogPath = $catalogTarget
    configPath = $configPath
    planned = @($plannedActions)
    source = 'bundled-sanitized-snapshot'
} | ConvertTo-Json -Depth 8
