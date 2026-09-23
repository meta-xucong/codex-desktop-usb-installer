[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackRoot,
    [Parameter(Mandatory = $true)][string]$TargetUserProfile,
    [switch]$EnableFeishu
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Write-JsonNoBom([string]$Path, [object]$Value, [int]$Depth = 8) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $Value | ConvertTo-Json -Depth $Depth
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

$packRootFull = [IO.Path]::GetFullPath($PackRoot).TrimEnd('\')
$profileFull = [IO.Path]::GetFullPath($TargetUserProfile).TrimEnd('\')
$kitHome = Join-Path $profileFull '.codex\codex-agent-kit'
$runtimeRoot = Join-Path $kitHome 'runtime'
$mediaRoot = Join-Path $packRootFull 'runtime\bundled'
$mediaManifestPath = Join-Path $mediaRoot 'media-manifest.json'

function Test-PathWithin([string]$Path, [string]$Root) {
    $pathFull = [IO.Path]::GetFullPath($Path)
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    return $pathFull.StartsWith(($rootFull + '\'), [StringComparison]::OrdinalIgnoreCase)
}

function Read-Json([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Required JSON file is missing: $Path" }
    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Assert-Media([object]$MediaManifest, [string]$Id, [string]$Root) {
    $entry = @($MediaManifest.media | Where-Object { [string]$_.id -eq $Id })[0]
    if (-not $entry) { throw "Media manifest entry is missing: $Id" }
    $relative = ([string]$entry.path).Replace('/', '\')
    $path = Join-Path $Root $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Bundled media is missing: $relative" }
    if (-not (Test-PathWithin $path $Root)) { throw "Bundled media path escapes pack root: $relative" }
    $actualBytes = (Get-Item -LiteralPath $path).Length
    if ([int64]$entry.bytes -ne $actualBytes) { throw "Bundled media byte count mismatch: $relative" }
    $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne ([string]$entry.sha256).ToLowerInvariant()) { throw "Bundled media SHA-256 mismatch: $relative" }
    return $path
}

function Test-ExactPython([string]$Executable) {
    if ([string]::IsNullOrWhiteSpace($Executable) -or -not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $false }
    # Keep the native argument free of embedded quotes: Windows PowerShell 5.1
    # can otherwise mangle a Python -c payload before it reaches python.exe.
    $probe = 'import platform,sys,sysconfig; print(str(sys.version_info[0])+chr(46)+str(sys.version_info[1])+chr(46)+str(sys.version_info[2])+chr(124)+sys.implementation.cache_tag+chr(124)+sysconfig.get_platform()+chr(124)+platform.machine())'
    $text = (& $Executable -c $probe 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    return $exitCode -eq 0 -and $text -match '^3\.12\.\d+\|cpython-312\|win-amd64\|(AMD64|x86_64)$'
}

function Get-NodeVersion([string]$Executable) {
    if ([string]::IsNullOrWhiteSpace($Executable) -or -not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $null }
    $text = (& $Executable --version 2>&1 | Out-String).Trim().TrimStart('v')
    $version = $null
    if ([version]::TryParse($text, [ref]$version)) { return $version }
    return $null
}

function Write-AsciiFile([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Set-Content -LiteralPath $Path -Value $Text -Encoding ASCII
}

$mediaManifest = Read-Json $mediaManifestPath
$pythonInstaller = Assert-Media $mediaManifest 'runtime.python312' $packRootFull
$nodeZip = $null
$feishuZip = $null
if ($EnableFeishu) {
    $nodeZip = Assert-Media $mediaManifest 'runtime.node20-feishu-optional' $packRootFull
    $feishuZip = Assert-Media $mediaManifest 'runtime.feishu-mcp-package' $packRootFull
}
$wheelManifest = Read-Json (Join-Path $mediaRoot 'python-wheelhouse-manifest.json')
$wheelDir = Join-Path $packRootFull 'runtime\bundled\python-wheels'
$lockPath = Join-Path $packRootFull 'runtime\bundled\python-requirements.lock'
if (-not (Test-Path -LiteralPath $wheelDir -PathType Container) -or -not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw 'Bundled Python wheelhouse or hashed lock is missing.'
}

$wheelCount = 0
foreach ($wheel in @($wheelManifest.wheels)) {
    $wheelCount++
    $relativeWheel = ([string]$wheel.relativePath).Replace('/', '\')
    $wheelPath = Join-Path $packRootFull $relativeWheel
    if (-not (Test-Path -LiteralPath $wheelPath -PathType Leaf) -or -not (Test-PathWithin $wheelPath $packRootFull)) {
        throw "Bundled wheel is missing or escapes the pack: $relativeWheel"
    }
    if ([int64]$wheel.bytes -ne (Get-Item -LiteralPath $wheelPath).Length) { throw "Bundled wheel byte count mismatch: $relativeWheel" }
    $wheelHash = (Get-FileHash -LiteralPath $wheelPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($wheelHash -ne ([string]$wheel.sha256).ToLowerInvariant()) { throw "Bundled wheel SHA-256 mismatch: $relativeWheel" }
}
if ($wheelCount -lt 1) { throw 'Bundled Python wheelhouse is empty.' }

$pythonSignature = Get-AuthenticodeSignature -LiteralPath $pythonInstaller
if ([string]$pythonSignature.Status -ne 'Valid') { throw "Python installer Authenticode validation failed: $($pythonSignature.Status)" }

New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
$pythonCandidates = [System.Collections.Generic.List[string]]::new()
foreach ($candidate in @(
    [Environment]::GetEnvironmentVariable('CODEX_PYTHON', 'Process'),
    [Environment]::GetEnvironmentVariable('CODEX_PYTHON', 'User'),
    $(try { (Get-Command python -ErrorAction Stop).Source } catch { $null }),
    (Join-Path $runtimeRoot 'cpython-3.12.10\python.exe')
)) {
    if (-not [string]::IsNullOrWhiteSpace([string]$candidate) -and -not $pythonCandidates.Contains([string]$candidate)) { $pythonCandidates.Add([string]$candidate) }
}
$basePython = $null
foreach ($candidate in $pythonCandidates) {
    if (Test-ExactPython $candidate) { $basePython = [IO.Path]::GetFullPath($candidate); break }
}

if ([string]::IsNullOrWhiteSpace($basePython)) {
    # The manager may launch this script elevated, so the process environment's
    # LocalApplicationData can belong to the administrator rather than the
    # requested target user. Derive the per-user install path from the explicit
    # target profile instead.
    $pythonRoot = Join-Path (Join-Path $profileFull 'AppData\Local') 'Programs\Python\Python312'
    $kitPythonRoot = Join-Path $runtimeRoot 'cpython-3.12.10'
    $argumentList = @('/quiet', 'InstallAllUsers=0', 'PrependPath=0', 'Include_test=0', ('TargetDir="{0}"' -f $pythonRoot))
    $process = Start-Process -FilePath $pythonInstaller -ArgumentList $argumentList -WindowStyle Hidden -Wait -PassThru
    if (@(0, 3010) -notcontains [int]$process.ExitCode) { throw "Python installer failed with exit code $($process.ExitCode)." }
    $installedPython = $null
    foreach ($candidate in @((Join-Path $pythonRoot 'python.exe'), (Join-Path $kitPythonRoot 'python.exe'))) {
        if (Test-ExactPython $candidate) { $installedPython = [IO.Path]::GetFullPath($candidate); break }
    }
    if ([string]::IsNullOrWhiteSpace($installedPython)) {
        throw "Bundled Python installer completed but no healthy CPython 3.12 interpreter was found under $pythonRoot or $kitPythonRoot."
    }
    $basePython = $installedPython
}

$venvRoot = Join-Path $runtimeRoot 'python-env'
$venvPython = Join-Path $venvRoot 'Scripts\python.exe'
$venvHealthy = Test-ExactPython $venvPython
if (-not $venvHealthy) {
    & $basePython -m venv --clear $venvRoot 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $venvPython -PathType Leaf)) { throw 'Could not create the per-user CPython 3.12 environment.' }
}

& $venvPython -m pip install --disable-pip-version-check --no-input --no-index --find-links $wheelDir --require-hashes -r $lockPath 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Offline Python dependency installation failed.' }
$pythonHealthProbe = 'import akshare,pandas,numpy,matplotlib,openpyxl,docx,pptx,pypdf,pdfplumber,reportlab,PIL,scrapling,html2text,jsonpath'
$pythonHealth = (& $venvPython -c $pythonHealthProbe 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "Python runtime health check failed: $pythonHealth" }

$nodeRoot = Join-Path $runtimeRoot 'node-v20.20.0-win-x64'
$nodeExe = Join-Path $nodeRoot 'node.exe'
$nodeHealth = 'skipped'
$nodeVersion = $null
$feishuRoot = ''
$feishuHealth = 'disabled'
if ($EnableFeishu) {
    if ((Get-NodeVersion $nodeExe) -eq $null -or (Get-NodeVersion $nodeExe).Major -lt 20) {
        Expand-Archive -LiteralPath $nodeZip -DestinationPath $runtimeRoot -Force
    }
    $nodeVersionObject = Get-NodeVersion $nodeExe
    if ($nodeVersionObject -eq $null -or $nodeVersionObject.Major -lt 20) { throw 'Bundled Node.js 20 runtime is not healthy after extraction.' }
    $nodeHealth = 'passed'
    $nodeVersion = (& $nodeExe --version 2>&1 | Out-String).Trim()

    $feishuRoot = Join-Path $runtimeRoot 'feishu'
    $feishuPackageJson = Join-Path $feishuRoot 'node_modules\@larksuiteoapi\lark-mcp\package.json'
    $feishuCli = Join-Path $feishuRoot 'node_modules\@larksuiteoapi\lark-mcp\dist\cli.js'
    $feishuHealthy = $false
    if ((Test-Path -LiteralPath $feishuPackageJson -PathType Leaf) -and (Test-Path -LiteralPath $feishuCli -PathType Leaf)) {
        $packageMetadata = Get-Content -LiteralPath $feishuPackageJson -Raw -Encoding UTF8 | ConvertFrom-Json
        $feishuHealthy = [string]$packageMetadata.name -eq '@larksuiteoapi/lark-mcp' -and [string]$packageMetadata.version -eq '0.5.1'
    }
    if (-not $feishuHealthy) {
        Expand-Archive -LiteralPath $feishuZip -DestinationPath $feishuRoot -Force
    }
    if (-not (Test-Path -LiteralPath $feishuPackageJson -PathType Leaf) -or -not (Test-Path -LiteralPath $feishuCli -PathType Leaf)) { throw 'Feishu npm closure is incomplete after extraction.' }
    $packageMetadata = Get-Content -LiteralPath $feishuPackageJson -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$packageMetadata.name -ne '@larksuiteoapi/lark-mcp' -or [string]$packageMetadata.version -ne '0.5.1') { throw 'Extracted Feishu package does not match the locked version.' }
    $mcpHelp = (& $nodeExe $feishuCli mcp --help 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $mcpHelp -notmatch 'Start Feishu/Lark MCP Service') { throw 'Offline Feishu MCP CLI health check failed.' }
    $feishuHealth = 'passed'
}

$binRoot = Join-Path $kitHome 'bin'
$newline = [Environment]::NewLine
Write-AsciiFile (Join-Path $binRoot 'python.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'pip.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" -m pip %*' + $newline)
if ($EnableFeishu) {
    Write-AsciiFile (Join-Path $binRoot 'node.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v20.20.0-win-x64\node.exe" %*' + $newline)
    Write-AsciiFile (Join-Path $binRoot 'npm.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v20.20.0-win-x64\node.exe" "%~dp0..\runtime\node-v20.20.0-win-x64\node_modules\npm\bin\npm-cli.js" %*' + $newline)
}

$actualUserProfile = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('USERPROFILE')).TrimEnd('\')
$environmentApplied = $profileFull.Equals($actualUserProfile, [StringComparison]::OrdinalIgnoreCase)
if ($environmentApplied) {
    [Environment]::SetEnvironmentVariable('CODEX_AGENT_KIT_HOME', $kitHome, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_PYTHON', $venvPython, 'User')
    if ($EnableFeishu) {
        [Environment]::SetEnvironmentVariable('CODEX_NODE', $nodeExe, 'User')
        [Environment]::SetEnvironmentVariable('CODEX_FEISHU_PACKAGE_CACHE', $feishuRoot, 'User')
    }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathItems = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (-not @($pathItems | Where-Object { $_.TrimEnd('\').Equals($binRoot.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase) }).Count) { $pathItems += $binRoot }
    [Environment]::SetEnvironmentVariable('Path', ($pathItems -join ';'), 'User')
    $env:CODEX_AGENT_KIT_HOME = $kitHome
    $env:CODEX_PYTHON = $venvPython
    if ($EnableFeishu) {
        $env:CODEX_NODE = $nodeExe
        $env:CODEX_FEISHU_PACKAGE_CACHE = $feishuRoot
    }
    $env:PATH = $binRoot + ';' + $env:PATH
}

$runtimeState = [ordered]@{
    schemaVersion = 1
    status = 'ready'
    targetUserProfile = $profileFull
    environmentApplied = $environmentApplied
    python = [ordered]@{ executable = $venvPython; baseInterpreter = $basePython; version = '3.12.10'; wheelCount = $wheelCount; health = 'passed' }
    node = [ordered]@{ executable = if ($EnableFeishu) { $nodeExe } else { '' }; version = $nodeVersion; health = $nodeHealth }
    feishu = [ordered]@{ packageRoot = $feishuRoot; package = '@larksuiteoapi/lark-mcp@0.5.1'; health = $feishuHealth }
    paths = [ordered]@{ kitHome = $kitHome; runtimeRoot = $runtimeRoot; binRoot = $binRoot; wheelhouse = $wheelDir }
    secretsStored = $false
}
$statePath = Join-Path $kitHome 'runtime-state.json'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $statePath) | Out-Null
Write-JsonNoBom $statePath $runtimeState 8
$runtimeState | ConvertTo-Json -Depth 8
