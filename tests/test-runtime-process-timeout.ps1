$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$runtimeScript = Join-Path $repoRoot 'installer-pack\runtime\provision-runtime.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($runtimeScript, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) {
    throw "Runtime provisioner has PowerShell syntax errors: $($parseErrors[0].Message)"
}
$helperNames = @('ConvertTo-WindowsArgument', 'Stop-NativeProcessTree', 'Invoke-NativeCapture')
$helpers = @($ast.FindAll({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $helperNames -contains $node.Name
}, $true))
if ($helpers.Count -ne $helperNames.Count) {
    throw 'Could not extract all runtime process helpers from the installer source.'
}
. ([scriptblock]::Create(($helpers | ForEach-Object { $_.Extent.Text }) -join [Environment]::NewLine))

$powershell = (Get-Command powershell.exe -ErrorAction Stop).Source
$completed = Invoke-NativeCapture -Executable $powershell -Arguments @(
    '-NoProfile', '-NonInteractive', '-Command', '[Console]::WriteLine("quoted argument passed")'
) -TimeoutSeconds 10
if ($completed.TimedOut -or $completed.ExitCode -ne 0 -or $completed.StdOut -notmatch 'quoted argument passed') {
    throw "A completed child process did not preserve its quoted arguments/output: $($completed | ConvertTo-Json -Compress)"
}

$embeddedQuoteScript = '[Console]::WriteLine(''quoted: "value'')'
$embeddedQuote = Invoke-NativeCapture -Executable $powershell -Arguments @(
    '-NoProfile', '-NonInteractive', '-Command', $embeddedQuoteScript
) -TimeoutSeconds 10
if ($embeddedQuote.TimedOut -or $embeddedQuote.ExitCode -ne 0 -or $embeddedQuote.StdOut -notmatch 'quoted: "value') {
    throw "A child process did not preserve an embedded quote: $($embeddedQuote | ConvertTo-Json -Compress)"
}

$longRunning = Invoke-NativeCapture -Executable $powershell -Arguments @(
    '-NoProfile', '-NonInteractive', '-Command', '[Console]::Out.WriteLine("child-started"); [Console]::Error.WriteLine("child-stderr"); Start-Sleep -Seconds 8'
) -TimeoutSeconds 1
if (-not $longRunning.TimedOut -or $longRunning.ExitCode -eq 0) {
    throw 'A child process exceeding its deadline was not reported as timed out.'
}
if ($longRunning.StdOut -notmatch 'child-started' -or $longRunning.StdErr -notmatch 'child-stderr' -or $longRunning.Output -notmatch 'child-started' -or $longRunning.Output -notmatch 'child-stderr') {
    throw 'The timeout result did not preserve both stdout and stderr already written by the child.'
}

$pidFile = Join-Path ([IO.Path]::GetTempPath()) ("codex-runtime-inherited-pipe-$([guid]::NewGuid().ToString('N')).txt")
$parentScriptPath = Join-Path ([IO.Path]::GetTempPath()) ("codex-runtime-inherited-pipe-$([guid]::NewGuid().ToString('N')).ps1")
$parentScript = @'
param([string]$PidFile)
$si = [System.Diagnostics.ProcessStartInfo]::new()
$si.FileName = Join-Path $PSHOME 'powershell.exe'
$si.Arguments = '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'
$si.UseShellExecute = $false
$si.CreateNoWindow = $true
$child = [System.Diagnostics.Process]::Start($si)
[IO.File]::WriteAllText($PidFile, [string]$child.Id)
[Console]::Out.WriteLine('parent-finished')
'@
[IO.File]::WriteAllText($parentScriptPath, $parentScript, [Text.Encoding]::ASCII)
$drainWatch = [Diagnostics.Stopwatch]::StartNew()
$drainFailure = $null
try {
    [void](Invoke-NativeCapture -Executable $powershell -Arguments @(
        '-NoProfile', '-NonInteractive', '-File', $parentScriptPath, $pidFile
    ) -TimeoutSeconds 10)
} catch {
    $drainFailure = $_.Exception.Message
}
$drainWatch.Stop()
try {
    if ([string]::IsNullOrWhiteSpace($drainFailure) -or $drainFailure -notmatch 'redirected output pipes remained open') {
        throw "A descendant-held output pipe was not reported as a bounded drain failure: $drainFailure"
    }
    if ($drainWatch.Elapsed.TotalSeconds -gt 8) {
        throw "A descendant-held output pipe exceeded the bounded drain window: $([Math]::Round($drainWatch.Elapsed.TotalSeconds, 2)) seconds"
    }
    if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) {
        throw 'The pipe-holder test process did not record its descendant PID.'
    }
} finally {
    if (Test-Path -LiteralPath $pidFile -PathType Leaf) {
        $descendantPid = 0
        if ([int]::TryParse((Get-Content -LiteralPath $pidFile -Raw), [ref]$descendantPid)) {
            Stop-NativeProcessTree -ProcessId $descendantPid
        }
    }
    Remove-Item -LiteralPath $pidFile, $parentScriptPath -Force -ErrorAction SilentlyContinue
}

Write-Output 'runtime process timeout checks passed'
