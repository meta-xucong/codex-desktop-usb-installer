[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Keyword,
    [string]$Root,
    [switch]$Json
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Keyword)) { throw 'Keyword cannot be empty.' }

function Find-CatalogRoot {
    param([string]$ExplicitRoot)
    if ($ExplicitRoot) {
        $candidate = (Resolve-Path -LiteralPath $ExplicitRoot).Path
        if (Test-Path (Join-Path $candidate 'skills')) { return $candidate }
        if ((Split-Path -Leaf $candidate) -eq 'skills') { return (Split-Path -Parent $candidate) }
        throw "No skills directory found under $candidate"
    }
    $cursor = (Get-Item $PSScriptRoot).FullName
    while ($cursor) {
        if ((Test-Path (Join-Path $cursor 'manifest\imported-skills.txt')) -and (Test-Path (Join-Path $cursor 'skills'))) { return $cursor }
        $parent = Split-Path -Parent $cursor
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    $installedRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    return $installedRoot
}

function Get-FrontMatter([string]$Path) {
    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    $name = ([regex]::Match($text, '(?m)^name:\s*(.+?)\s*$')).Groups[1].Value.Trim(' "''')
    $description = ([regex]::Match($text, '(?m)^description:\s*(.+?)\s*$')).Groups[1].Value.Trim(' "''')
    return [ordered]@{ name = $name; description = $description }
}

$catalogRoot = Find-CatalogRoot $Root
$skillRoot = Join-Path $catalogRoot 'skills'
$audit = @{}
$auditPath = Join-Path $catalogRoot 'manifest\skill-audit.json'
if (Test-Path -LiteralPath $auditPath) {
    $auditDoc = Get-Content -LiteralPath $auditPath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($entry in $auditDoc.skills) { $audit[[string]$entry.id] = $entry }
}
$needle = $Keyword.Trim().ToLowerInvariant()
$results = [System.Collections.Generic.List[object]]::new()
foreach ($directory in Get-ChildItem -LiteralPath $skillRoot -Directory | Sort-Object Name) {
    $doc = Join-Path $directory.FullName 'SKILL.md'
    if (-not (Test-Path -LiteralPath $doc -PathType Leaf)) { continue }
    $meta = Get-FrontMatter $doc
    $haystack = "$($directory.Name) $($meta.name) $($meta.description)".ToLowerInvariant()
    if ($haystack.Contains($needle)) {
        $entry = $audit[$directory.Name]
        $results.Add([ordered]@{
            name = if ($meta.name) { $meta.name } else { $directory.Name }
            directory = $directory.Name
            description = $meta.description
            status = if ($entry) { $entry.status } else { 'unknown' }
            afterRemediationStatus = if ($entry) { $entry.afterRemediationStatus } else { 'unknown' }
            dependencies = if ($entry) { @($entry.dependencies) } else { @() }
            skillFile = $doc
        })
    }
}
if ($Json) { @($results) | ConvertTo-Json -Depth 12 } elseif ($results.Count -eq 0) { '未找到匹配的 Skill' } else {
    $index = 0
    foreach ($item in $results) {
        $index++
        Write-Output ("{0}. {1} ({2}) [{3}]" -f $index, $item.name, $item.directory, $item.status)
        Write-Output ("   {0}" -f $item.description)
        if ($item.dependencies.Count -gt 0) { Write-Output ("   依赖: {0}" -f (($item.dependencies | ForEach-Object { if ($_ -is [string]) { $_ } else { $_.id } }) -join ', ')) }
    }
}
