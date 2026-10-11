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
    $result = Invoke-NativeCapture -Executable $Executable -Arguments @('-c', $probe)
    return $result.ExitCode -eq 0 -and $result.Output -match '^3\.12\.\d+\|cpython-312\|win-amd64\|(AMD64|x86_64)$'
}

function Get-NodeVersion([string]$Executable) {
    if ([string]::IsNullOrWhiteSpace($Executable) -or -not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $null }
    $result = Invoke-NativeCapture -Executable $Executable -Arguments @('--version')
    if ($result.ExitCode -ne 0) { return $null }
    $text = $result.Output.TrimStart('v')
    $version = $null
    if ([version]::TryParse($text, [ref]$version)) { return $version }
    return $null
}

function Write-AsciiFile([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Set-Content -LiteralPath $Path -Value $Text -Encoding ASCII
}

function Invoke-NativeCapture([string]$Executable, [string[]]$Arguments) {
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = (& $Executable @Arguments 2>&1 | Out-String).Trim()
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    return [pscustomobject]@{ ExitCode = $exitCode; Output = $output }
}

$mediaManifest = Read-Json $mediaManifestPath
$pythonInstaller = Assert-Media $mediaManifest 'runtime.python312' $packRootFull
$nodeZip = Assert-Media $mediaManifest 'runtime.node22-common' $packRootFull
$ffmpegZip = Assert-Media $mediaManifest 'runtime.ffmpeg-windows-x64' $packRootFull
$hyperframesPackageFile = Assert-Media $mediaManifest 'runtime.hyperframes-package-json' $packRootFull
$hyperframesLockFile = Assert-Media $mediaManifest 'runtime.hyperframes-package-lock' $packRootFull
$feishuZip = $null
if ($EnableFeishu) {
    $feishuZip = Assert-Media $mediaManifest 'runtime.feishu-mcp-package' $packRootFull
}
$wheelManifest = Read-Json (Join-Path $mediaRoot 'python-wheelhouse-manifest.json')
$wheelDir = Join-Path $packRootFull 'runtime\bundled\python-wheels'
$lockPath = Join-Path $packRootFull 'runtime\bundled\python-requirements.lock'
$manimLockPath = Join-Path $packRootFull 'runtime\bundled\manim-requirements.lock'
if (-not (Test-Path -LiteralPath $wheelDir -PathType Container) -or -not (Test-Path -LiteralPath $lockPath -PathType Leaf) -or -not (Test-Path -LiteralPath $manimLockPath -PathType Leaf)) {
    throw 'Bundled Python wheelhouse or a hashed Python/Manim lock is missing.'
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
    $venvResult = Invoke-NativeCapture -Executable $basePython -Arguments @('-m','venv','--clear',$venvRoot)
    if ($venvResult.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $venvPython -PathType Leaf)) { throw "Could not create the per-user CPython 3.12 environment: $($venvResult.Output)" }
}

$pythonInstall = Invoke-NativeCapture -Executable $venvPython -Arguments @('-m','pip','install','--disable-pip-version-check','--no-input','--no-index','--find-links',$wheelDir,'--require-hashes','-r',$lockPath)
if ($pythonInstall.ExitCode -ne 0) { throw "Offline Python dependency installation failed: $($pythonInstall.Output)" }
$pythonHealthProbe = 'import akshare,pandas,numpy,matplotlib,openpyxl,docx,pptx,pypdf,pdfplumber,reportlab,PIL,scrapling,html2text,jsonpath'
$pythonHealthResult = Invoke-NativeCapture -Executable $venvPython -Arguments @('-c',$pythonHealthProbe)
if ($pythonHealthResult.ExitCode -ne 0) { throw "Python runtime health check failed: $($pythonHealthResult.Output)" }

$manimRoot = Join-Path $runtimeRoot 'manim-env'
$manimPython = Join-Path $manimRoot 'Scripts\python.exe'
if (-not (Test-ExactPython $manimPython)) {
    $manimVenvResult = Invoke-NativeCapture -Executable $basePython -Arguments @('-m','venv','--clear',$manimRoot)
    if ($manimVenvResult.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $manimPython -PathType Leaf)) { throw "Could not create the isolated Manim CPython 3.12 environment: $($manimVenvResult.Output)" }
}
$manimInstall = Invoke-NativeCapture -Executable $manimPython -Arguments @('-m','pip','install','--disable-pip-version-check','--no-input','--no-index','--find-links',$wheelDir,'--require-hashes','-r',$manimLockPath)
if ($manimInstall.ExitCode -ne 0) { throw "Offline Manim dependency installation failed: $($manimInstall.Output)" }
$manimHealthProbe = 'import importlib.metadata as m; import manim, av, manimpango; print(m.version(chr(109)+chr(97)+chr(110)+chr(105)+chr(109)))'
$previousManimErrorActionPreference = $ErrorActionPreference
try {
    # PowerShell 5.1 can promote native Python stderr into a terminating
    # NativeCommandError when ErrorActionPreference is Stop, hiding the actual
    # traceback from runtime-state.json. Capture it as text and retain the exit
    # code so the installer reports the real import failure.
    $manimProbe = Invoke-NativeCapture -Executable $manimPython -Arguments @('-c',$manimHealthProbe)
} finally {
    $ErrorActionPreference = $previousManimErrorActionPreference
}
$manimVersion = ($manimProbe.Output -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Last 1).Trim()
if ($manimProbe.ExitCode -ne 0 -or $manimVersion -ne '0.21.0') { throw "Manim runtime health check failed (exit=$($manimProbe.ExitCode)): $($manimProbe.Output)" }

$nodeRoot = Join-Path $runtimeRoot 'node-v22.22.0-win-x64'
$nodeExe = Join-Path $nodeRoot 'node.exe'
$nodeHealth = 'unknown'
$nodeVersion = $null
$feishuRoot = ''
$feishuHealth = 'disabled'
$nodeVersionObject = Get-NodeVersion $nodeExe
if ($nodeVersionObject -eq $null -or $nodeVersionObject.Major -lt 22) {
    Expand-Archive -LiteralPath $nodeZip -DestinationPath $runtimeRoot -Force
}
$nodeVersionObject = Get-NodeVersion $nodeExe
if ($nodeVersionObject -eq $null -or $nodeVersionObject.Major -lt 22) { throw 'Bundled Node.js 22 runtime is not healthy after extraction.' }
$nodeHealth = 'passed'
$nodeVersion = (Invoke-NativeCapture -Executable $nodeExe -Arguments @('--version')).Output

$ffmpegRoot = Join-Path $runtimeRoot 'ffmpeg'
$ffmpegPackageRoot = Join-Path $ffmpegRoot 'ffmpeg-n9.0-latest-win64-lgpl-shared-9.0'
$ffmpegBinRoot = Join-Path $ffmpegPackageRoot 'bin'
$ffmpegExe = Join-Path $ffmpegBinRoot 'ffmpeg.exe'
$ffprobeExe = Join-Path $ffmpegBinRoot 'ffprobe.exe'
$ffplayExe = Join-Path $ffmpegBinRoot 'ffplay.exe'
if (-not (Test-Path -LiteralPath $ffmpegExe -PathType Leaf) -or -not (Test-Path -LiteralPath $ffprobeExe -PathType Leaf)) {
    Expand-Archive -LiteralPath $ffmpegZip -DestinationPath $ffmpegRoot -Force
}
foreach ($requiredMediaExecutable in @($ffmpegExe, $ffprobeExe, $ffplayExe)) {
    if (-not (Test-Path -LiteralPath $requiredMediaExecutable -PathType Leaf)) { throw "Bundled FFmpeg executable is missing: $requiredMediaExecutable" }
}
$ffmpegVersionResult = Invoke-NativeCapture -Executable $ffmpegExe -Arguments @('-version')
$ffmpegVersionOutput = $ffmpegVersionResult.Output
$ffmpegExitCode = $ffmpegVersionResult.ExitCode
$ffmpegVersionText = ($ffmpegVersionOutput -split "`r?`n" | Select-Object -First 1).Trim()
if ($ffmpegExitCode -ne 0 -or $ffmpegVersionText -notmatch '^ffmpeg version (?:9\.|N-|n9\.)') { throw "Bundled FFmpeg health check failed (exit=$ffmpegExitCode): $ffmpegVersionText" }
$ffprobeVersionResult = Invoke-NativeCapture -Executable $ffprobeExe -Arguments @('-version')
$ffprobeVersionOutput = $ffprobeVersionResult.Output
$ffprobeExitCode = $ffprobeVersionResult.ExitCode
$ffprobeVersionText = ($ffprobeVersionOutput -split "`r?`n" | Select-Object -First 1).Trim()
if ($ffprobeExitCode -ne 0 -or $ffprobeVersionText -notmatch '^ffprobe version (?:9\.|N-|n9\.)') { throw "Bundled FFprobe health check failed (exit=$ffprobeExitCode): $ffprobeVersionText" }
$ffmpegHealth = 'passed'

$hyperframesVersion = '0.8.134'
$hyperframesRoot = Join-Path $runtimeRoot 'hyperframes'
$hyperframesEntry = Join-Path $hyperframesRoot 'node_modules\hyperframes\bin\hyperframes.mjs'
$hyperframesCmd = Join-Path $hyperframesRoot 'hyperframes.cmd'
$npmCli = Join-Path $nodeRoot 'node_modules\npm\bin\npm-cli.js'
$hyperframesHealth = 'requires-network'
$hyperframesCliHealth = 'requires-network'
$hyperframesBrowserHealth = 'unknown'
$hyperframesError = ''
$installedHyperframesVersion = ''
$hyperframesPackageJson = Join-Path $hyperframesRoot 'node_modules\hyperframes\package.json'
$hyperframesMarker = Join-Path $hyperframesRoot '.installed-lock-sha256'
$expectedHyperframesLockHash = (Get-FileHash -LiteralPath $hyperframesLockFile -Algorithm SHA256).Hash.ToLowerInvariant()
New-Item -ItemType Directory -Force -Path $hyperframesRoot | Out-Null
Copy-Item -LiteralPath $hyperframesPackageFile -Destination (Join-Path $hyperframesRoot 'package.json') -Force
Copy-Item -LiteralPath $hyperframesLockFile -Destination (Join-Path $hyperframesRoot 'package-lock.json') -Force
$installedLockHash = if (Test-Path -LiteralPath $hyperframesMarker -PathType Leaf) { (Get-Content -LiteralPath $hyperframesMarker -Raw -Encoding ASCII).Trim().ToLowerInvariant() } else { '' }
$npmCacheRoot = Join-Path $runtimeRoot 'npm-cache'
New-Item -ItemType Directory -Force -Path $npmCacheRoot | Out-Null
if ($installedLockHash -ne $expectedHyperframesLockHash -or -not (Test-Path -LiteralPath $hyperframesEntry -PathType Leaf)) {
    $npmResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @($npmCli,'ci','--prefix',$hyperframesRoot,'--cache',$npmCacheRoot,'--no-audit','--no-fund','--no-progress','--fetch-retries=2','--fetch-timeout=60000')
    $npmOutput = $npmResult.Output
    if ($npmResult.ExitCode -eq 0) {
        [IO.File]::WriteAllText($hyperframesMarker,$expectedHyperframesLockHash + [Environment]::NewLine,[Text.Encoding]::ASCII)
    } else {
        $hyperframesError = "HyperFrames CLI install requires network access: $npmOutput"
    }
}
if (Test-Path -LiteralPath $hyperframesEntry -PathType Leaf) {
    Write-AsciiFile $hyperframesCmd ('@echo off' + [Environment]::NewLine + '"%~dp0..\node-v22.22.0-win-x64\node.exe" "%~dp0node_modules\hyperframes\bin\hyperframes.mjs" %*' + $newline)
    $cliResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @($hyperframesEntry,'--version')
    $cliOutput = $cliResult.Output
    if ($cliResult.ExitCode -eq 0) {
        $hyperframesCliHealth = 'passed'
        $installedHyperframesVersion = $cliOutput
        $browserResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @($hyperframesEntry,'browser','ensure')
        $browserOutput = $browserResult.Output
        if ($browserResult.ExitCode -eq 0) {
            $hyperframesBrowserHealth = 'passed'
            $hyperframesHealth = 'passed'
        } else {
            $hyperframesBrowserHealth = 'requires-network'
            $hyperframesError = "HyperFrames browser setup requires network access or a supported local Chrome: $browserOutput"
        }
    } else {
        $hyperframesCliHealth = 'failed'
        $hyperframesError = "HyperFrames CLI health check failed: $cliOutput"
    }
} else {
    $hyperframesCliHealth = 'requires-network'
    if ([string]::IsNullOrWhiteSpace($hyperframesError)) { $hyperframesError = 'HyperFrames CLI is not installed; npm access is required during one-click setup.' }
}

if ($EnableFeishu) {
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
    $mcpHelpResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @($feishuCli,'mcp','--help')
    $mcpHelp = $mcpHelpResult.Output
    if ($mcpHelpResult.ExitCode -ne 0 -or $mcpHelp -notmatch 'Start Feishu/Lark MCP Service') { throw 'Offline Feishu MCP CLI health check failed.' }
    $feishuHealth = 'passed'
}

$binRoot = Join-Path $kitHome 'bin'
$newline = [Environment]::NewLine
Write-AsciiFile (Join-Path $binRoot 'python.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'pip.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" -m pip %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'manim.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\manim-env\Scripts\manim.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'node.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'npm.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" "%~dp0..\runtime\node-v22.22.0-win-x64\node_modules\npm\bin\npm-cli.js" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'npx.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" "%~dp0..\runtime\node-v22.22.0-win-x64\node_modules\npm\bin\npx-cli.js" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'ffmpeg.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\ffmpeg\ffmpeg-n9.0-latest-win64-lgpl-shared-9.0\bin\ffmpeg.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'ffprobe.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\ffmpeg\ffmpeg-n9.0-latest-win64-lgpl-shared-9.0\bin\ffprobe.exe" %*' + $newline)
Write-AsciiFile (Join-Path $binRoot 'ffplay.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\ffmpeg\ffmpeg-n9.0-latest-win64-lgpl-shared-9.0\bin\ffplay.exe" %*' + $newline)
if (Test-Path -LiteralPath $hyperframesCmd -PathType Leaf) {
    Write-AsciiFile (Join-Path $binRoot 'hyperframes.cmd') ('@echo off' + $newline + 'call "%~dp0..\runtime\hyperframes\hyperframes.cmd" %*' + $newline)
}

$actualUserProfile = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('USERPROFILE')).TrimEnd('\')
$environmentApplied = $profileFull.Equals($actualUserProfile, [StringComparison]::OrdinalIgnoreCase)
if ($environmentApplied) {
    [Environment]::SetEnvironmentVariable('CODEX_AGENT_KIT_HOME', $kitHome, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_PYTHON', $venvPython, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_MANIM_PYTHON', $manimPython, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_NODE', $nodeExe, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_FFMPEG', $ffmpegExe, 'User')
    [Environment]::SetEnvironmentVariable('CODEX_FFPROBE', $ffprobeExe, 'User')
    if ($hyperframesCliHealth -eq 'passed') { [Environment]::SetEnvironmentVariable('CODEX_HYPERFRAMES', $hyperframesEntry, 'User') } else { [Environment]::SetEnvironmentVariable('CODEX_HYPERFRAMES', $null, 'User') }
    if ($EnableFeishu) {
        [Environment]::SetEnvironmentVariable('CODEX_FEISHU_PACKAGE_CACHE', $feishuRoot, 'User')
    }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathItems = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (-not @($pathItems | Where-Object { $_.TrimEnd('\').Equals($binRoot.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase) }).Count) { $pathItems += $binRoot }
    [Environment]::SetEnvironmentVariable('Path', ($pathItems -join ';'), 'User')
    $env:CODEX_AGENT_KIT_HOME = $kitHome
    $env:CODEX_PYTHON = $venvPython
    $env:CODEX_MANIM_PYTHON = $manimPython
    $env:CODEX_NODE = $nodeExe
    $env:CODEX_FFMPEG = $ffmpegExe
    $env:CODEX_FFPROBE = $ffprobeExe
    if ($hyperframesCliHealth -eq 'passed') { $env:CODEX_HYPERFRAMES = $hyperframesEntry } else { Remove-Item Env:\CODEX_HYPERFRAMES -ErrorAction SilentlyContinue }
    if ($EnableFeishu) {
        $env:CODEX_FEISHU_PACKAGE_CACHE = $feishuRoot
    }
    $env:PATH = $binRoot + ';' + $env:PATH
}

$runtimeState = [ordered]@{
    schemaVersion = 1
    status = if ($hyperframesHealth -eq 'passed') { 'ready' } else { 'partial' }
    targetUserProfile = $profileFull
    environmentApplied = $environmentApplied
    python = [ordered]@{ executable = $venvPython; baseInterpreter = $basePython; version = '3.12.10'; wheelCount = $wheelCount; health = 'passed' }
    manim = [ordered]@{ executable = $manimPython; command = (Join-Path $binRoot 'manim.cmd'); version = $manimVersion; health = 'passed'; latex = 'optional-not-installed' }
    node = [ordered]@{ executable = $nodeExe; version = $nodeVersion; health = $nodeHealth }
    ffmpeg = [ordered]@{ executable = $ffmpegExe; ffprobe = $ffprobeExe; version = $ffmpegVersionText; health = $ffmpegHealth; license = 'LGPL shared build' }
    hyperframes = [ordered]@{ executable = $hyperframesEntry; command = if (Test-Path -LiteralPath $hyperframesCmd -PathType Leaf) { $hyperframesCmd } else { '' }; version = $hyperframesVersion; cliHealth = $hyperframesCliHealth; browserHealth = $hyperframesBrowserHealth; health = $hyperframesHealth; error = $hyperframesError }
    feishu = [ordered]@{ packageRoot = $feishuRoot; package = '@larksuiteoapi/lark-mcp@0.5.1'; health = $feishuHealth }
    paths = [ordered]@{ kitHome = $kitHome; runtimeRoot = $runtimeRoot; binRoot = $binRoot; wheelhouse = $wheelDir; manimEnvironment = $manimRoot; ffmpeg = $ffmpegRoot; hyperframes = $hyperframesRoot }
    secretsStored = $false
}
$statePath = Join-Path $kitHome 'runtime-state.json'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $statePath) | Out-Null
Write-JsonNoBom $statePath $runtimeState 8
$runtimeState | ConvertTo-Json -Depth 8
