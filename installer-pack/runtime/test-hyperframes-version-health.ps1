$ErrorActionPreference = 'Stop'
$runtimeRoot = $PSScriptRoot
$provisionerPath = Join-Path $runtimeRoot 'provision-runtime.ps1'
$provisioner = [IO.File]::ReadAllText($provisionerPath)
function Assert-Test([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

$package = Get-Content -LiteralPath (Join-Path $runtimeRoot 'bundled\hyperframes-package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$lockText = [IO.File]::ReadAllText((Join-Path $runtimeRoot 'bundled\hyperframes-package-lock.json'))
$lockRootMatch = [regex]::Match($lockText, '(?s)"packages"\s*:\s*\{\s*""\s*:\s*\{\s*"dependencies"\s*:\s*\{\s*"hyperframes"\s*:\s*"([^"]+)"')
$lockRootVersion = if ($lockRootMatch.Success) { $lockRootMatch.Groups[1].Value } else { '' }
$lockedVersion = [string]$package.dependencies.hyperframes
Assert-Test ($lockedVersion -eq '0.8.134') 'Unexpected HyperFrames package version in package.json.'
Assert-Test ($lockRootVersion -eq $lockedVersion) 'package-lock root version must match package.json.'
Assert-Test ($provisioner.Contains('$hyperframesVersion = ' + "'" + $lockedVersion + "'")) 'Runtime provisioner must use the package-locked HyperFrames version.'

$installer = [IO.File]::ReadAllText((Join-Path (Split-Path $runtimeRoot -Parent) 'install-to-codex.ps1'))
Assert-Test ($installer.Contains("'passed','requires-network','install-failed'")) 'The installer must allow optional HyperFrames install failure to remain partial instead of aborting core installation.'
Assert-Test ($installer.Contains("hyperframes.cliHealth -eq 'passed'")) 'The HyperFrames command shim must remain gated on a healthy CLI.'
$wizard = [IO.File]::ReadAllText((Join-Path (Split-Path $runtimeRoot -Parent) 'install-wizard.ps1'))
Assert-Test ($wizard.Contains('$hyperframes.cliHealth')) 'The installation wizard must display the runtime readiness details.'
Assert-Test ($wizard.Contains('Codex 启动时不会自动重试')) 'The wizard must not imply an automatic startup retry.'

Write-Output 'HyperFrames install-state focused tests passed (8 assertions).'
