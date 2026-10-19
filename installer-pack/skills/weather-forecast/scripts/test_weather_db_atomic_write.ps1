[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$IsolatedRoot
)

$ErrorActionPreference = 'Stop'
$repo = [System.IO.Path]::GetFullPath($RepoRoot)
$isolated = [System.IO.Path]::GetFullPath($IsolatedRoot)
$source = Join-Path $repo 'installer-pack\skills\weather-forecast\scripts\weather_db.ps1'
$dataDir = Join-Path $isolated 'weather'
$config = Join-Path $dataDir 'config.json'

if (-not $isolated.StartsWith($repo + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "IsolatedRoot must be inside RepoRoot: $isolated"
}
if (Test-Path -LiteralPath $isolated) {
    throw "IsolatedRoot already exists; refusing to overwrite: $isolated"
}
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing source: $source" }
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null

$oldWeatherDataDir = $env:WEATHER_DATA_DIR
$oldCodexDataDir = $env:CODEX_DATA_DIR
$env:WEATHER_DATA_DIR = $dataDir
$env:CODEX_DATA_DIR = $null

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Convert-ConfigBytesToObject([byte[]]$Bytes) {
    $text = [System.Text.Encoding]::UTF8.GetString($Bytes).TrimStart([char]0xFEFF)
    return $text | ConvertFrom-Json -ErrorAction Stop
}

function Assert-NoStageFiles {
    $stageFiles = @(Get-ChildItem -LiteralPath $dataDir -Force -File -Filter '.config.*.tmp' -ErrorAction SilentlyContinue)
    if ($stageFiles.Count -ne 0) { throw "Staging files remain: $($stageFiles.FullName -join ', ')" }
}

try {
    # Successful set/get proves the atomic replace path produces readable JSON.
    & $source -Command set_city -City 'AtomicCity' | Out-Null
    $successBytes = [System.IO.File]::ReadAllBytes($config)
    $successConfig = Convert-ConfigBytesToObject $successBytes
    if (-not $successConfig.initialized -or $successConfig.default_city -ne 'AtomicCity') {
        throw 'Successful set_city did not persist the expected configuration.'
    }
    $readCity = (& $source -Command get_city | Out-String).Trim()
    if ($readCity -ne 'AtomicCity') { throw "get_city returned '$readCity' instead of AtomicCity." }
    Assert-NoStageFiles

    # Inject a partial write to the staging path, then throw. config.json must
    # remain byte-for-byte unchanged, and the owned stage must be removed.
    $beforeTempHash = Get-Sha256 $config
    $global:S051WeatherStageWriteReached = $false
    $global:S051WeatherInjectedStage = ''
    $global:S051WeatherExpectedConfig = [System.IO.Path]::GetFullPath($config)
    $global:S051WeatherExpectedDataDir = [System.IO.Path]::GetFullPath($dataDir)
    $previousGlobalSetContent = Get-Item Function:\global:Set-Content -ErrorAction SilentlyContinue
    function global:Set-Content {
        [CmdletBinding()]
        param(
            [Parameter(ValueFromPipeline = $true)][AllowNull()][object]$Value,
            [string]$LiteralPath,
            [string]$Encoding
        )
        process {
            $target = [System.IO.Path]::GetFullPath($LiteralPath)
            if ($target.Equals($global:S051WeatherExpectedConfig, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw 'Fault injector refused to write active config.json.'
            }
            if (-not $target.StartsWith($global:S051WeatherExpectedDataDir, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Fault injector refused non-isolated stage: $target"
            }
            [System.IO.File]::WriteAllText($target, '{"partial":', [System.Text.UTF8Encoding]::new($false))
            $global:S051WeatherStageWriteReached = $true
            $global:S051WeatherInjectedStage = $target
            throw [System.IO.IOException]::new('S051 injected stage-write failure')
        }
    }
    $tempFailureCaught = $false
    try { & $source -Command set_city -City 'MustNotPersist' | Out-Null }
    catch { $tempFailureCaught = $_.Exception.Message -match 'S051 injected stage-write failure' }
    if ($null -ne $previousGlobalSetContent) {
        Set-Item Function:\global:Set-Content -Value $previousGlobalSetContent.ScriptBlock
    } else {
        Remove-Item Function:\global:Set-Content -ErrorAction SilentlyContinue
    }
    if (-not $global:S051WeatherStageWriteReached -or -not $tempFailureCaught) {
        throw "Staging write failure injection was not reached and caught (reached=$global:S051WeatherStageWriteReached caught=$tempFailureCaught)."
    }
    if ((Get-Sha256 $config) -ne $beforeTempHash) {
        throw 'config.json changed after a staging write failure.'
    }
    if (Test-Path -LiteralPath $global:S051WeatherInjectedStage) { throw 'Partial staging file was not removed.' }
    Assert-NoStageFiles

    # A file handle without delete sharing forces File.Replace to fail on Windows.
    # The original config bytes must remain intact and the stage must be cleaned.
    $beforeReplaceHash = Get-Sha256 $config
    $lock = [System.IO.File]::Open(
        $config,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    $replaceFailureCaught = $false
    try {
        try { & $source -Command set_city -City 'MustAlsoNotPersist' | Out-Null }
        catch { $replaceFailureCaught = $true }
    } finally {
        $lock.Dispose()
    }
    if (-not $replaceFailureCaught) { throw 'Locked destination did not cause replacement failure.' }
    if ((Get-Sha256 $config) -ne $beforeReplaceHash) {
        throw 'config.json changed after replacement failure.'
    }
    Assert-NoStageFiles
    $finalConfig = Convert-ConfigBytesToObject ([System.IO.File]::ReadAllBytes($config))
    if ($finalConfig.default_city -ne 'AtomicCity') { throw 'Failed writes changed the saved city.' }

    [ordered]@{
        isolatedRoot = $isolated
        successfulSetGet = $true
        stageWriteInjectionReached = [bool]$global:S051WeatherStageWriteReached
        stageWriteFailureCaught = $tempFailureCaught
        configUnchangedAfterStageWriteFailure = $true
        partialStageRemoved = $true
        replacementFailureCaught = $replaceFailureCaught
        configUnchangedAfterReplacementFailure = $true
        noStageFilesRemain = $true
    } | ConvertTo-Json -Depth 4
} finally {
    $env:WEATHER_DATA_DIR = $oldWeatherDataDir
    $env:CODEX_DATA_DIR = $oldCodexDataDir
}
