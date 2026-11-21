[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackRoot,
    [Parameter(Mandatory = $true)][string]$TargetUserProfile,
    [string]$TargetCodexHome,
    [string]$ProgressFile = '',
    [switch]$EnableFeishu,
    [switch]$FeishuOnly
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Set-InstallProgress([string]$Stage) {
    if ([string]::IsNullOrWhiteSpace($ProgressFile)) { return }
    [IO.File]::WriteAllText($ProgressFile, $Stage, [Text.Encoding]::ASCII)
}

function Write-JsonNoBom([string]$Path, [object]$Value, [int]$Depth = 8) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $Value | ConvertTo-Json -Depth $Depth
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

$packRootFull = [IO.Path]::GetFullPath($PackRoot).TrimEnd('\')
$profileFull = [IO.Path]::GetFullPath($TargetUserProfile).TrimEnd('\')
$codexHome = if ([string]::IsNullOrWhiteSpace($TargetCodexHome)) { Join-Path $profileFull '.codex' } else { [IO.Path]::GetFullPath($TargetCodexHome) }
$kitHome = Join-Path $codexHome 'codex-agent-kit'
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
    $stdout = ([string]$result.StdOut).Trim()
    return $result.ExitCode -eq 0 -and $stdout -match '^3\.12\.\d+\|cpython-312\|win-amd64\|(AMD64|x86_64)$'
}

function Test-BasePython([string]$Executable) {
    if (-not (Test-ExactPython $Executable)) { return $false }
    # venv invokes ensurepip, which reads its bundled pip wheel. A version
    # probe alone accepts an interpreter left without these install files.
    $probe = 'import venv,ensurepip; from importlib.resources import files; assert files(ensurepip).joinpath("_bundled", "pip-"+ensurepip.version()+"-py3-none-any.whl").is_file()'
    $result = Invoke-NativeCapture -Executable $Executable -Arguments @('-c', $probe)
    return $result.ExitCode -eq 0
}

function Get-NodeVersion([string]$Executable) {
    if ([string]::IsNullOrWhiteSpace($Executable) -or -not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $null }
    $result = Invoke-NativeCapture -Executable $Executable -Arguments @('--version')
    if ($result.ExitCode -ne 0) { return $null }
    $text = $result.StdOut.TrimStart('v')
    $version = $null
    if ([version]::TryParse($text, [ref]$version)) { return $version }
    return $null
}

function Write-AsciiFile([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Set-Content -LiteralPath $Path -Value $Text -Encoding ASCII
}

function Set-UserEnvironmentIfChanged([string]$Name, $Value) {
    $current = [Environment]::GetEnvironmentVariable($Name, 'User')
    if ([string]$current -cne [string]$Value) {
        [Environment]::SetEnvironmentVariable($Name, $Value, 'User')
    }
}

function ConvertTo-WindowsArgument([string]$Argument) {
    if ([string]::IsNullOrEmpty($Argument)) { return '""' }
    if ($Argument -notmatch '[\s"]') { return $Argument }
    $builder = [System.Text.StringBuilder]::new()
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Argument.ToCharArray()) {
        if ($character -eq '\') { $backslashes++; continue }
        if ($character -eq '"') {
            if ($backslashes -gt 0) { [void]$builder.Append(('\' * (2 * $backslashes))) }
            [void]$builder.Append('\"')
            $backslashes = 0
            continue
        }
        if ($backslashes -gt 0) { [void]$builder.Append(('\' * $backslashes)) }
        [void]$builder.Append($character)
        $backslashes = 0
    }
    if ($backslashes -gt 0) { [void]$builder.Append(('\' * (2 * $backslashes))) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Stop-NativeProcessTree([int]$ProcessId) {
    $taskkill = Join-Path $env:SystemRoot 'System32\taskkill.exe'
    try {
        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $taskkill
        $startInfo.Arguments = "/PID $ProcessId /T /F"
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $killer = [System.Diagnostics.Process]::Start($startInfo)
        if ($null -ne $killer -and -not $killer.WaitForExit(10000)) { $killer.Kill() }
    } catch {
        # The direct process kill below remains the final best-effort action.
    }
}

function Invoke-NativeCapture([string]$Executable, [string[]]$Arguments, [ValidateRange(1,1800)][int]$TimeoutSeconds = 60) {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.Arguments = (($Arguments | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { return [pscustomobject]@{ ExitCode = -1; Output = 'Process did not start.'; StdOut = ''; StdErr = ''; TimedOut = $false } }
    } catch {
        return [pscustomobject]@{ ExitCode = -1; Output = $_.Exception.Message; StdOut = ''; StdErr = $_.Exception.Message; TimedOut = $false }
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $finished = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $finished) {
        Stop-NativeProcessTree -ProcessId $process.Id
        try { if (-not $process.HasExited) { $process.Kill() } } catch { }
        [void]$process.WaitForExit(10000)
        $partialStdOut = if ($stdoutTask.Wait(1000)) { $stdoutTask.Result } else { '' }
        $partialStdErr = if ($stderrTask.Wait(1000)) { $stderrTask.Result } else { '' }
        $process.Dispose()
        return [pscustomobject]@{ ExitCode = -1; Output = ($partialStdOut + $partialStdErr).Trim(); StdOut = $partialStdOut; StdErr = $partialStdErr; TimedOut = $true }
    }
    # A descendant can inherit either redirected pipe and keep it open after
    # this process exits. Bound the drain so that inherited handles cannot hold
    # capability installation indefinitely; the Rust supervisor owns the
    # process tree and will terminate any remaining descendants on script exit.
    $drainTimeoutMs = 5000
    $drainWatch = [System.Diagnostics.Stopwatch]::StartNew()
    $stdoutDrained = $stdoutTask.Wait($drainTimeoutMs)
    $remainingDrainMs = [Math]::Max(0, $drainTimeoutMs - [int]$drainWatch.ElapsedMilliseconds)
    $stderrDrained = $stderrTask.Wait($remainingDrainMs)
    if (-not $stdoutDrained -or -not $stderrDrained) {
        $partialStdOut = if ($stdoutTask.Status -eq [System.Threading.Tasks.TaskStatus]::RanToCompletion) { $stdoutTask.Result } else { '' }
        $partialStdErr = if ($stderrTask.Status -eq [System.Threading.Tasks.TaskStatus]::RanToCompletion) { $stderrTask.Result } else { '' }
        try { $process.StandardOutput.Dispose() } catch { }
        try { $process.StandardError.Dispose() } catch { }
        $exitCode = $process.ExitCode
        $process.Dispose()
        $partialOutput = ($partialStdOut + $partialStdErr).Trim()
        throw "Native process exited with code $exitCode, but redirected output pipes remained open beyond $drainTimeoutMs ms. Partial output: $partialOutput"
    }
    $stdOut = $stdoutTask.GetAwaiter().GetResult()
    $stdErr = $stderrTask.GetAwaiter().GetResult()
    $output = ($stdOut + $stdErr).Trim()
    $exitCode = $process.ExitCode
    $process.Dispose()
    return [pscustomobject]@{ ExitCode = $exitCode; Output = $output; StdOut = $stdOut; StdErr = $stdErr; TimedOut = $false }
}

function Test-VenvPython([string]$Executable, [string]$HealthProbe) {
    if (-not (Test-ExactPython $Executable)) { return $false }
    # A failed first-time ensurepip can leave Scripts\python.exe behind without
    # pip. Version-only detection would then reuse a broken venv on every retry.
    $pipProbe = Invoke-NativeCapture -Executable $Executable -Arguments @('-m','pip','--version')
    if ($pipProbe.ExitCode -ne 0 -or $pipProbe.StdOut -notmatch '^pip\s+\d+') { return $false }
    # pip can consider a wheel installed after METADATA is written, before its
    # modules exist. Reuse only when the existing runtime import check passes.
    $health = Invoke-NativeCapture -Executable $Executable -Arguments @('-c',$HealthProbe)
    return $health.ExitCode -eq 0
}

function Test-FeishuRuntime([string]$NodeExe, [string]$FeishuRoot) {
    $packageRoot = Join-Path $FeishuRoot 'node_modules\@larksuiteoapi\lark-mcp'
    $packageJson = Join-Path $packageRoot 'package.json'
    $cli = Join-Path $packageRoot 'dist\cli.js'
    if (-not (Test-Path -LiteralPath $packageJson -PathType Leaf) -or -not (Test-Path -LiteralPath $cli -PathType Leaf)) { return $false }
    try { $metadata = Read-Json $packageJson } catch { return $false }
    if ([string]$metadata.name -ne '@larksuiteoapi/lark-mcp' -or [string]$metadata.version -ne '0.5.1') { return $false }
    $helpResult = Invoke-NativeCapture -Executable $NodeExe -Arguments @($cli, 'mcp', '--help')
    return $helpResult.ExitCode -eq 0 -and $helpResult.Output -match 'Start Feishu/Lark MCP Service'
}

function Restore-FeishuRuntime([string]$Zip, [string]$NodeExe, [string]$FeishuRoot) {
    # A prior extraction can leave the CLI behind without its dependencies.
    Expand-Archive -LiteralPath $Zip -DestinationPath $FeishuRoot -Force
    if (-not (Test-FeishuRuntime $NodeExe $FeishuRoot)) {
        throw 'Offline Feishu MCP CLI health check failed after extraction.'
    }
}

if ($FeishuOnly) {
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    Set-InstallProgress 'runtime.feishu'
    $nodeExe = Join-Path $runtimeRoot 'node-v22.22.0-win-x64\node.exe'
    $feishuRoot = Join-Path $runtimeRoot 'feishu'
    $nodeVersion = Get-NodeVersion $nodeExe
    $nodeNeedsRepair = $null -eq $nodeVersion -or $nodeVersion.Major -lt 22
    $feishuNeedsRepair = $true
    if (-not $nodeNeedsRepair) {
        $feishuNeedsRepair = -not (Test-FeishuRuntime $nodeExe $feishuRoot)
    }
    if ($nodeNeedsRepair -or $feishuNeedsRepair) {
        $mediaManifest = Read-Json $mediaManifestPath
        if ($nodeNeedsRepair) {
            $nodeZip = Assert-Media $mediaManifest 'runtime.node22-common' $packRootFull
            Expand-Archive -LiteralPath $nodeZip -DestinationPath $runtimeRoot -Force
            $nodeVersion = Get-NodeVersion $nodeExe
            if ($null -eq $nodeVersion -or $nodeVersion.Major -lt 22) {
                throw 'Bundled Node.js 22 runtime is not healthy after extraction.'
            }
        }
        if ($feishuNeedsRepair) {
            $feishuZip = Assert-Media $mediaManifest 'runtime.feishu-mcp-package' $packRootFull
            Restore-FeishuRuntime $feishuZip $nodeExe $feishuRoot
        }
    }
    [ordered]@{
        repaired = $nodeNeedsRepair -or $feishuNeedsRepair
        feishu = [ordered]@{ packageRoot = $feishuRoot; package = '@larksuiteoapi/lark-mcp@0.5.1'; health = 'passed'; error = '' }
    } | ConvertTo-Json -Compress -Depth 4
    return
}

Set-InstallProgress 'runtime.verify'
$mediaManifest = Read-Json $mediaManifestPath
$pythonInstaller = Assert-Media $mediaManifest 'runtime.python312' $packRootFull
$nodeZip = Assert-Media $mediaManifest 'runtime.node22-common' $packRootFull
$ffmpegZip = Assert-Media $mediaManifest 'runtime.ffmpeg-windows-x64' $packRootFull
$hyperframesPackageFile = Assert-Media $mediaManifest 'runtime.hyperframes-package-json' $packRootFull
$hyperframesLockFile = Assert-Media $mediaManifest 'runtime.hyperframes-package-lock' $packRootFull
$hyperframesCliZip = Assert-Media $mediaManifest 'runtime.hyperframes-cli' $packRootFull
$hyperframesFfmpegZip = Assert-Media $mediaManifest 'runtime.hyperframes-ffmpeg' $packRootFull
$hyperframesBrowserZip = Assert-Media $mediaManifest 'runtime.hyperframes-browser' $packRootFull
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
Set-InstallProgress 'runtime.python'
# Use the same target-user path for discovery and installation. PrependPath=0
# means a healthy previous installation need not be discoverable on PATH.
$pythonRoot = Join-Path (Join-Path $profileFull 'AppData\Local') 'Programs\Python\Python312'
$kitPythonRoot = Join-Path $runtimeRoot 'cpython-3.12.10'
$pythonCandidates = [System.Collections.Generic.List[string]]::new()
foreach ($candidate in @(
    [Environment]::GetEnvironmentVariable('CODEX_PYTHON', 'Process'),
    [Environment]::GetEnvironmentVariable('CODEX_PYTHON', 'User'),
    $(try { (Get-Command python -ErrorAction Stop).Source } catch { $null }),
    (Join-Path $pythonRoot 'python.exe'),
    (Join-Path $kitPythonRoot 'python.exe')
)) {
    if (-not [string]::IsNullOrWhiteSpace([string]$candidate) -and -not $pythonCandidates.Contains([string]$candidate)) { $pythonCandidates.Add([string]$candidate) }
}
$basePython = $null
foreach ($candidate in $pythonCandidates) {
    if (Test-BasePython $candidate) { $basePython = [IO.Path]::GetFullPath($candidate); break }
}

if ([string]::IsNullOrWhiteSpace($basePython)) {
    # The manager may launch this script elevated, so the process environment's
    # LocalApplicationData can belong to the administrator rather than the
    # requested target user. Derive the per-user install path from the explicit
    # target profile instead.
    # CPython 3.12.10 converts /repair to install when its bundle is absent.
    # For a registered installation it restores missing files; plain /quiet
    # would only modify selected features and could leave the same failure.
    # Invoke-NativeCapture quotes each argument; embedded quotes become part
    # of MSI's TARGETDIR value and make a fresh installation fail with 1606.
    $argumentList = @('/repair', '/quiet', 'InstallAllUsers=0', 'PrependPath=0', 'Include_test=0', ('TargetDir={0}' -f $pythonRoot))
    $process = Invoke-NativeCapture -Executable $pythonInstaller -Arguments $argumentList -TimeoutSeconds 600
    if ($process.TimedOut) { throw 'Bundled Python installer exceeded its 600-second limit.' }
    if (@(0, 3010) -notcontains [int]$process.ExitCode) { throw "Python installer failed with exit code $($process.ExitCode): $($process.Output)" }
    $installedPython = $null
    # Repair preserves an existing registered install location, so inspect
    # the original candidates again as well as the target-user locations.
    foreach ($candidate in $pythonCandidates) {
        if (Test-BasePython $candidate) { $installedPython = [IO.Path]::GetFullPath($candidate); break }
    }
    if ([string]::IsNullOrWhiteSpace($installedPython)) {
        throw "Bundled Python installer completed but no healthy CPython 3.12 interpreter was found under $pythonRoot or $kitPythonRoot."
    }
    $basePython = $installedPython
}

$venvRoot = Join-Path $runtimeRoot 'python-env'
$venvPython = Join-Path $venvRoot 'Scripts\python.exe'
$pythonHealthProbe = 'import akshare,pandas,numpy,matplotlib,openpyxl,docx,pptx,pypdf,pdfplumber,reportlab,PIL,scrapling,html2text,jsonpath,requests'
$venvHealthy = Test-VenvPython $venvPython $pythonHealthProbe
if (-not $venvHealthy) {
    $venvResult = Invoke-NativeCapture -Executable $basePython -Arguments @('-m','venv','--clear',$venvRoot) -TimeoutSeconds 600
    if ($venvResult.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $venvPython -PathType Leaf)) { throw "Could not create the per-user CPython 3.12 environment: $($venvResult.Output)" }
}

$pythonInstall = Invoke-NativeCapture -Executable $venvPython -Arguments @('-m','pip','install','--disable-pip-version-check','--no-input','--no-index','--find-links',$wheelDir,'--require-hashes','-r',$lockPath) -TimeoutSeconds 600
if ($pythonInstall.ExitCode -ne 0) { throw "Offline Python dependency installation failed: $($pythonInstall.Output)" }
$pythonHealthResult = Invoke-NativeCapture -Executable $venvPython -Arguments @('-c',$pythonHealthProbe)
if ($pythonHealthResult.ExitCode -ne 0) { throw "Python runtime health check failed: $($pythonHealthResult.Output)" }

Set-InstallProgress 'runtime.manim'
$manimRoot = Join-Path $runtimeRoot 'manim-env'
$manimPython = Join-Path $manimRoot 'Scripts\python.exe'
$manimHealthProbe = 'import importlib.metadata as m; import manim, av, manimpango; print(m.version(chr(109)+chr(97)+chr(110)+chr(105)+chr(109)))'
if (-not (Test-VenvPython $manimPython $manimHealthProbe)) {
    $manimVenvResult = Invoke-NativeCapture -Executable $basePython -Arguments @('-m','venv','--clear',$manimRoot) -TimeoutSeconds 600
    if ($manimVenvResult.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $manimPython -PathType Leaf)) { throw "Could not create the isolated Manim CPython 3.12 environment: $($manimVenvResult.Output)" }
}
$manimInstall = Invoke-NativeCapture -Executable $manimPython -Arguments @('-m','pip','install','--disable-pip-version-check','--no-input','--no-index','--find-links',$wheelDir,'--require-hashes','-r',$manimLockPath) -TimeoutSeconds 600
if ($manimInstall.ExitCode -ne 0) { throw "Offline Manim dependency installation failed: $($manimInstall.Output)" }
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
$manimVersion = ($manimProbe.StdOut -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Last 1).Trim()
if ($manimProbe.ExitCode -ne 0 -or $manimVersion -ne '0.21.0') { throw "Manim runtime health check failed (exit=$($manimProbe.ExitCode)): $($manimProbe.Output)" }

Set-InstallProgress 'runtime.nodeMedia'
$nodeRoot = Join-Path $runtimeRoot 'node-v22.22.0-win-x64'
$nodeExe = Join-Path $nodeRoot 'node.exe'
$nodeHealth = 'unknown'
$nodeVersion = $null
$feishuRoot = ''
$feishuHealth = 'disabled'
$feishuError = ''
# node.exe can run before npm/npx have been extracted. Explicit capability
# setup restores the complete bundled Node distribution.
Expand-Archive -LiteralPath $nodeZip -DestinationPath $runtimeRoot -Force
$nodeVersionObject = Get-NodeVersion $nodeExe
if ($nodeVersionObject -eq $null -or $nodeVersionObject.Major -lt 22) { throw 'Bundled Node.js 22 runtime is not healthy after extraction.' }
$nodeHealth = 'passed'
$nodeVersion = (Invoke-NativeCapture -Executable $nodeExe -Arguments @('--version')).StdOut

$ffmpegRoot = Join-Path $runtimeRoot 'ffmpeg'
$ffmpegPackageRoot = Join-Path $ffmpegRoot 'ffmpeg-n9.0-latest-win64-lgpl-shared-9.0'
$ffmpegBinRoot = Join-Path $ffmpegPackageRoot 'bin'
$ffmpegExe = Join-Path $ffmpegBinRoot 'ffmpeg.exe'
$ffprobeExe = Join-Path $ffmpegBinRoot 'ffprobe.exe'
$ffplayExe = Join-Path $ffmpegBinRoot 'ffplay.exe'
# A partial archive can contain ffmpeg/ffprobe but lack ffplay or DLLs.
Expand-Archive -LiteralPath $ffmpegZip -DestinationPath $ffmpegRoot -Force
foreach ($requiredMediaExecutable in @($ffmpegExe, $ffprobeExe, $ffplayExe)) {
    if (-not (Test-Path -LiteralPath $requiredMediaExecutable -PathType Leaf)) { throw "Bundled FFmpeg executable is missing: $requiredMediaExecutable" }
}
$ffmpegVersionResult = Invoke-NativeCapture -Executable $ffmpegExe -Arguments @('-version')
$ffmpegVersionOutput = $ffmpegVersionResult.StdOut
$ffmpegExitCode = $ffmpegVersionResult.ExitCode
$ffmpegVersionText = ($ffmpegVersionOutput -split "`r?`n" | Select-Object -First 1).Trim()
if ($ffmpegExitCode -ne 0 -or $ffmpegVersionText -notmatch '^ffmpeg version (?:9\.|N-|n9\.)') { throw "Bundled FFmpeg health check failed (exit=$ffmpegExitCode): $ffmpegVersionText" }
$ffprobeVersionResult = Invoke-NativeCapture -Executable $ffprobeExe -Arguments @('-version')
$ffprobeVersionOutput = $ffprobeVersionResult.StdOut
$ffprobeExitCode = $ffprobeVersionResult.ExitCode
$ffprobeVersionText = ($ffprobeVersionOutput -split "`r?`n" | Select-Object -First 1).Trim()
if ($ffprobeExitCode -ne 0 -or $ffprobeVersionText -notmatch '^ffprobe version (?:9\.|N-|n9\.)') { throw "Bundled FFprobe health check failed (exit=$ffprobeExitCode): $ffprobeVersionText" }
$ffmpegHealth = 'passed'

$hyperframesVersion = '0.8.134'
$hyperframesRoot = Join-Path $runtimeRoot 'hyperframes'
$hyperframesEntry = Join-Path $hyperframesRoot 'node_modules\hyperframes\bin\hyperframes.mjs'
$hyperframesCmd = Join-Path $hyperframesRoot 'hyperframes.cmd'
$hyperframesBrowserVersion = '152.0.7977.30'
$hyperframesBrowserRoot = Join-Path $runtimeRoot ('hyperframes-browser\' + $hyperframesBrowserVersion)
$hyperframesBrowserExe = Join-Path $hyperframesBrowserRoot 'chrome-headless-shell-win64\chrome-headless-shell.exe'
$hyperframesHealth = 'unknown'
$hyperframesCliHealth = 'unknown'
$hyperframesBrowserHealth = 'unknown'
$hyperframesBrowserCheckMs = $null
$hyperframesError = ''
$installedHyperframesVersion = ''
try {
    $hyperframesMarker = Join-Path $hyperframesRoot '.installed-lock-sha256'
    $expectedHyperframesLockHash = (Get-FileHash -LiteralPath $hyperframesLockFile -Algorithm SHA256).Hash.ToLowerInvariant()
    New-Item -ItemType Directory -Force -Path $hyperframesRoot | Out-Null
    Copy-Item -LiteralPath $hyperframesPackageFile -Destination (Join-Path $hyperframesRoot 'package.json') -Force
    Copy-Item -LiteralPath $hyperframesLockFile -Destination (Join-Path $hyperframesRoot 'package-lock.json') -Force
    $installedLockHash = if (Test-Path -LiteralPath $hyperframesMarker -PathType Leaf) { (Get-Content -LiteralPath $hyperframesMarker -Raw -Encoding ASCII).Trim().ToLowerInvariant() } else { '' }
    Set-InstallProgress 'runtime.hyperframes'
    $deployHyperframes = $installedLockHash -ne $expectedHyperframesLockHash -or -not (Test-Path -LiteralPath $hyperframesEntry -PathType Leaf)
    if ($deployHyperframes) {
        # Keep the existing lock identity, but never reuse a partial extraction
        # as a completed deployment. No npm or lifecycle downloads run here.
        if (Test-Path -LiteralPath $hyperframesMarker -PathType Leaf) { Remove-Item -LiteralPath $hyperframesMarker }
        Expand-Archive -LiteralPath $hyperframesCliZip -DestinationPath $hyperframesRoot -Force
    }
    if (-not (Test-Path -LiteralPath $hyperframesEntry -PathType Leaf)) { throw 'Bundled HyperFrames CLI entry is missing after extraction.' }
    Set-InstallProgress 'runtime.hyperframesCliCheck'
    $cliResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @($hyperframesEntry,'--version')
    if ($cliResult.ExitCode -ne 0 -or $cliResult.StdOut.Trim() -ne $hyperframesVersion) { throw "HyperFrames CLI health check failed: $($cliResult.Output)" }
    $installedHyperframesVersion = $cliResult.StdOut
    # HyperFrames' default H.264 renderer requires libx264. Keep its GPL
    # build private; other skills continue using the original LGPL FFmpeg.
    Set-InstallProgress 'runtime.hyperframesFfmpeg'
    $hyperframesFfmpegRoot = Join-Path $runtimeRoot 'hyperframes-ffmpeg'
    $hyperframesFfmpegBin = Join-Path $hyperframesFfmpegRoot 'ffmpeg-n9.0-latest-win64-gpl-shared-9.0\bin'
    $hyperframesFfmpegExe = Join-Path $hyperframesFfmpegBin 'ffmpeg.exe'
    $hyperframesFfprobeExe = Join-Path $hyperframesFfmpegBin 'ffprobe.exe'
    # An interrupted extraction can leave both EXEs without their DLLs.
    # Explicit capability setup restores the complete private archive.
    Expand-Archive -LiteralPath $hyperframesFfmpegZip -DestinationPath $hyperframesFfmpegRoot -Force
    foreach ($tool in @($hyperframesFfmpegExe, $hyperframesFfprobeExe)) {
        $toolResult = Invoke-NativeCapture -Executable $tool -Arguments @('-version')
        if ($toolResult.ExitCode -ne 0) { throw "HyperFrames encoder health check failed (exit=$($toolResult.ExitCode)): $($toolResult.Output)" }
    }
    $hyperframesCliHealth = 'passed'
    if ($deployHyperframes) {
        [IO.File]::WriteAllText($hyperframesMarker,$expectedHyperframesLockHash + [Environment]::NewLine,[Text.Encoding]::ASCII)
    }

    Set-InstallProgress 'runtime.hyperframesBrowser'
    # --version can succeed with missing ICU/resources. Restore these files
    # during explicit setup even when the browser EXE already exists.
    Expand-Archive -LiteralPath $hyperframesBrowserZip -DestinationPath $hyperframesBrowserRoot -Force
    Set-InstallProgress 'runtime.hyperframesBrowserCheck'
    $browserProbe = @'
const [entry, executablePath, expectedVersion] = process.argv.slice(1);
const puppeteer = require('node:module').createRequire(entry)('puppeteer-core');
(async () => {
  const browser = await puppeteer.launch({ executablePath, headless: 'shell', pipe: true });
  try {
    const version = (await browser.version()).split('/').pop();
    if (version !== expectedVersion) throw new Error(`Expected browser ${expectedVersion}, got ${version}`);
    const page = await browser.newPage();
    await page.goto('about:blank');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
'@
    # The bundled Puppeteer launches the exact EXE over a local DevTools pipe.
    $browserWatch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $browserResult = Invoke-NativeCapture -Executable $nodeExe -Arguments @('--input-type=commonjs','-e',$browserProbe,$hyperframesEntry,$hyperframesBrowserExe,$hyperframesBrowserVersion)
    } finally {
        $hyperframesBrowserCheckMs = $browserWatch.ElapsedMilliseconds
    }
    if ($browserResult.TimedOut) { throw 'Bundled HyperFrames browser startup check timed out.' }
    if ($browserResult.ExitCode -ne 0) { throw "Bundled HyperFrames browser startup check failed: $($browserResult.Output)" }
    $hyperframesBrowserHealth = 'passed'
    $hyperframesHealth = 'passed'
} catch {
    # HyperFrames is optional for core setup. Preserve this component's failure
    # while letting the healthy core, config and readiness records finalize.
    $hyperframesHealth = 'install-failed'
    if ($hyperframesCliHealth -eq 'passed') { $hyperframesBrowserHealth = 'install-failed' } else { $hyperframesCliHealth = 'install-failed' }
    $hyperframesError = $_.Exception.Message
}
if ($hyperframesCliHealth -eq 'passed') {
    $newline = [Environment]::NewLine
    Write-AsciiFile $hyperframesCmd ('@echo off' + $newline + 'setlocal' + $newline + ('set "HYPERFRAMES_BROWSER_PATH=%~dp0..\hyperframes-browser\{0}\chrome-headless-shell-win64\chrome-headless-shell.exe"' -f $hyperframesBrowserVersion) + $newline + 'set "HYPERFRAMES_FFMPEG_PATH=%~dp0..\hyperframes-ffmpeg\ffmpeg-n9.0-latest-win64-gpl-shared-9.0\bin\ffmpeg.exe"' + $newline + 'set "HYPERFRAMES_FFPROBE_PATH=%~dp0..\hyperframes-ffmpeg\ffmpeg-n9.0-latest-win64-gpl-shared-9.0\bin\ffprobe.exe"' + $newline + 'if not exist "%HYPERFRAMES_BROWSER_PATH%" (echo Bundled HyperFrames browser is missing. Rerun capability setup. 1>&2 & exit /b 1)' + $newline + '"%~dp0..\node-v22.22.0-win-x64\node.exe" "%~dp0node_modules\hyperframes\bin\hyperframes.mjs" %*' + $newline + 'exit /b %errorlevel%' + $newline)
}

if ($EnableFeishu) {
    Set-InstallProgress 'runtime.feishu'
    $feishuRoot = Join-Path $runtimeRoot 'feishu'
    try {
        Restore-FeishuRuntime $feishuZip $nodeExe $feishuRoot
        $feishuHealth = 'passed'
    } catch {
        $feishuHealth = 'install-failed'
        $feishuError = $_.Exception.Message
    }
}

Set-InstallProgress 'runtime.environment'
$environmentWatch = [Diagnostics.Stopwatch]::StartNew()
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
    Set-UserEnvironmentIfChanged 'CODEX_AGENT_KIT_HOME' $kitHome
    Set-UserEnvironmentIfChanged 'CODEX_PYTHON' $venvPython
    Set-UserEnvironmentIfChanged 'CODEX_MANIM_PYTHON' $manimPython
    Set-UserEnvironmentIfChanged 'CODEX_NODE' $nodeExe
    Set-UserEnvironmentIfChanged 'CODEX_FFMPEG' $ffmpegExe
    Set-UserEnvironmentIfChanged 'CODEX_FFPROBE' $ffprobeExe
    if ($hyperframesCliHealth -eq 'passed') { Set-UserEnvironmentIfChanged 'CODEX_HYPERFRAMES' $hyperframesEntry } else { Set-UserEnvironmentIfChanged 'CODEX_HYPERFRAMES' $null }
    if ($feishuHealth -eq 'passed') {
        Set-UserEnvironmentIfChanged 'CODEX_FEISHU_PACKAGE_CACHE' $feishuRoot
    }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathItems = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (-not @($pathItems | Where-Object { $_.TrimEnd('\').Equals($binRoot.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase) }).Count) { $pathItems += $binRoot }
    Set-UserEnvironmentIfChanged 'Path' ($pathItems -join ';')
    $env:CODEX_AGENT_KIT_HOME = $kitHome
    $env:CODEX_PYTHON = $venvPython
    $env:CODEX_MANIM_PYTHON = $manimPython
    $env:CODEX_NODE = $nodeExe
    $env:CODEX_FFMPEG = $ffmpegExe
    $env:CODEX_FFPROBE = $ffprobeExe
    if ($hyperframesCliHealth -eq 'passed') { $env:CODEX_HYPERFRAMES = $hyperframesEntry } else { Remove-Item Env:\CODEX_HYPERFRAMES -ErrorAction SilentlyContinue }
    if ($feishuHealth -eq 'passed') {
        $env:CODEX_FEISHU_PACKAGE_CACHE = $feishuRoot
    }
    $env:PATH = $binRoot + ';' + $env:PATH
}
$environmentApplyMs = $environmentWatch.ElapsedMilliseconds

Set-InstallProgress 'runtime.complete'
$runtimeState = [ordered]@{
    schemaVersion = 1
    status = if ($hyperframesHealth -eq 'passed' -and $feishuHealth -ne 'install-failed') { 'ready' } else { 'partial' }
    targetUserProfile = $profileFull
    environmentApplied = $environmentApplied
    environmentApplyMs = $environmentApplyMs
    python = [ordered]@{ executable = $venvPython; baseInterpreter = $basePython; version = '3.12.10'; wheelCount = $wheelCount; health = 'passed' }
    manim = [ordered]@{ executable = $manimPython; command = (Join-Path $binRoot 'manim.cmd'); version = $manimVersion; health = 'passed'; latex = 'optional-not-installed' }
    node = [ordered]@{ executable = $nodeExe; version = $nodeVersion; health = $nodeHealth }
    ffmpeg = [ordered]@{ executable = $ffmpegExe; ffprobe = $ffprobeExe; version = $ffmpegVersionText; health = $ffmpegHealth; license = 'LGPL shared build' }
    hyperframes = [ordered]@{ executable = $hyperframesEntry; command = if (Test-Path -LiteralPath $hyperframesCmd -PathType Leaf) { $hyperframesCmd } else { '' }; version = $hyperframesVersion; cliHealth = $hyperframesCliHealth; browserHealth = $hyperframesBrowserHealth; browserExecutable = $hyperframesBrowserExe; browserVersion = $hyperframesBrowserVersion; browserCheckMs = $hyperframesBrowserCheckMs; health = $hyperframesHealth; error = $hyperframesError }
    feishu = [ordered]@{ packageRoot = $feishuRoot; package = '@larksuiteoapi/lark-mcp@0.5.1'; health = $feishuHealth; error = $feishuError }
    paths = [ordered]@{ kitHome = $kitHome; runtimeRoot = $runtimeRoot; binRoot = $binRoot; wheelhouse = $wheelDir; manimEnvironment = $manimRoot; ffmpeg = $ffmpegRoot; hyperframes = $hyperframesRoot }
    secretsStored = $false
}
$statePath = Join-Path $kitHome 'runtime-state.json'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $statePath) | Out-Null
Write-JsonNoBom $statePath $runtimeState 8
$runtimeState | ConvertTo-Json -Depth 8
