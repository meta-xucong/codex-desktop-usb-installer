$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
$tmpRoot = Join-Path $repo 'tmp'
New-Item -ItemType Directory -Force -Path $tmpRoot | Out-Null
$fixture = Join-Path $tmpRoot ('skills-search-installed-test-' + [guid]::NewGuid().ToString('N'))
$expectedRoot = [IO.Path]::GetFullPath($tmpRoot)
$fullFixture = [IO.Path]::GetFullPath($fixture)
if (-not $fullFixture.StartsWith($expectedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Unsafe fixture path: $fullFixture"
}

function Assert-Result {
    param([string]$RawJson, [string]$Directory, [string]$ExpectedStatus)
    $items = @($RawJson | ConvertFrom-Json)
    if ($items.Count -ne 1) { throw "Expected one match, got $($items.Count)." }
    if ($items[0].directory -ne $Directory) { throw "Unexpected directory: $($items[0].directory)" }
    if ($items[0].status -ne $ExpectedStatus) { throw "Unexpected status: $($items[0].status)" }
    if ($items[0].skillFile -match '[\\/]skills[\\/]skills[\\/]') { throw "Search incorrectly used a nested skills directory: $($items[0].skillFile)" }
}

try {
    $scriptSource = Join-Path $PSScriptRoot 'skills_search.ps1'
    $installedScripts = Join-Path $fixture '.codex\skills\skills-search\scripts'
    $installedTarget = Join-Path $fixture '.codex\skills\travel-planner'
    New-Item -ItemType Directory -Force -Path $installedScripts, $installedTarget | Out-Null
    $installedScript = Join-Path $installedScripts 'skills_search.ps1'
    Copy-Item -LiteralPath $scriptSource -Destination $installedScript
    Set-Content -LiteralPath (Join-Path $installedTarget 'SKILL.md') -Value "---`nname: Travel Planner`ndescription: Plan travel routes and itineraries.`n---`n" -Encoding UTF8

    $installedJson = (& $installedScript -Keyword travel -Json | Out-String)
    Assert-Result -RawJson $installedJson -Directory 'travel-planner' -ExpectedStatus 'unknown'
    $noMatch = & $installedScript -Keyword 'synthetic-no-match-token-918273'
    if ($noMatch -ne '未找到匹配的 Skill') { throw "Unexpected no-match result: $noMatch" }

    $packageRoot = Join-Path $fixture 'package'
    $packageScripts = Join-Path $packageRoot 'skills\skills-search\scripts'
    $packageTarget = Join-Path $packageRoot 'skills\travel-planner'
    $manifestDir = Join-Path $packageRoot 'manifest'
    New-Item -ItemType Directory -Force -Path $packageScripts, $packageTarget, $manifestDir | Out-Null
    $packageScript = Join-Path $packageScripts 'skills_search.ps1'
    Copy-Item -LiteralPath $scriptSource -Destination $packageScript
    Set-Content -LiteralPath (Join-Path $packageTarget 'SKILL.md') -Value "---`nname: Travel Planner`ndescription: Plan travel routes and itineraries.`n---`n" -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $manifestDir 'imported-skills.txt') -Value 'travel-planner' -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $manifestDir 'skill-audit.json') -Value '{"skills":[{"id":"travel-planner","status":"verified","afterRemediationStatus":"ready","dependencies":[{"id":"synthetic.runtime"}]}]}' -Encoding UTF8

    $packageJson = (& $packageScript -Keyword travel -Root $packageRoot -Json | Out-String)
    Assert-Result -RawJson $packageJson -Directory 'travel-planner' -ExpectedStatus 'verified'
    Write-Output 'PASS: installed fallback and packaged-root status behavior'
}
finally {
    if (Test-Path -LiteralPath $fullFixture) { Remove-Item -LiteralPath $fullFixture -Recurse -Force }
}
