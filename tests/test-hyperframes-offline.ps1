$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$source = [IO.File]::ReadAllText((Join-Path $repo 'installer-pack\runtime\provision-runtime.ps1'))
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Runtime script parse failed' }
$f = $ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Write-AsciiFile'}, $true)
. ([scriptblock]::Create($f.Extent.Text))
$start = $source.IndexOf('$hyperframesVersion =')
$end = $source.IndexOf('if ($EnableFeishu)', $start)
$blockText = $source.Substring($start, $end - $start)
$integrityRead = '(Get-FileHash -LiteralPath $hyperframesLockFile -Algorithm SHA256).Hash.ToLowerInvariant()'
if (-not $blockText.Contains($integrityRead)) { throw 'Expected integrity read not found' }
# Only the test block substitutes the existing integrity reader. Product
# integrity checks are unchanged. External operations are simulated below.
$block = [scriptblock]::Create($blockText.Replace($integrityRead, "'fixture-lock-identity'"))
$fixture = Join-Path $repo ('tmp\hyperframes-offline-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $fixture | Out-Null
$hyperframesPackageFile = Join-Path $fixture 'package.json'
$hyperframesLockFile = Join-Path $fixture 'package-lock.json'
[IO.File]::WriteAllText($hyperframesPackageFile, '{}')
[IO.File]::WriteAllText($hyperframesLockFile, '{}')
$hyperframesCliZip = 'fixture-cli.zip'
$hyperframesFfmpegZip = 'fixture-ffmpeg.zip'
$hyperframesBrowserZip = 'fixture-browser.zip'
$nodeExe = 'fixture-node.exe'
function Set-InstallProgress { param($Stage) $script:stages.Add($Stage) }
function Expand-Archive {
    param($LiteralPath,$DestinationPath,[switch]$Force)
    if ($LiteralPath -eq $hyperframesCliZip) {
        $script:calls.Add('extract-cli')
        New-Item -ItemType Directory -Force -Path (Split-Path $hyperframesEntry) | Out-Null
        [IO.File]::WriteAllText($hyperframesEntry, 'fixture installed entry')
        if ($case -eq 'extract-cli-error') { throw 'fixture CLI archive error' }
    } elseif ($LiteralPath -eq $hyperframesFfmpegZip) {
        $script:calls.Add('extract-encoder')
        New-Item -ItemType Directory -Force -Path $hyperframesFfmpegBin | Out-Null
        [IO.File]::WriteAllText($hyperframesFfmpegExe, 'fixture')
        [IO.File]::WriteAllText($hyperframesFfprobeExe, 'fixture')
        if ($case -eq 'extract-encoder-error') { throw 'fixture encoder archive error after EXEs' }
    } else {
        $script:calls.Add('extract-browser')
        $exe = Join-Path $DestinationPath 'chrome-headless-shell-win64\chrome-headless-shell.exe'
        New-Item -ItemType Directory -Force -Path (Split-Path $exe) | Out-Null
        [IO.File]::WriteAllText($exe, 'fixture')
        if ($case -eq 'extract-browser-error') { throw 'fixture browser archive error after EXE' }
    }
}
function Invoke-NativeCapture {
    param($Executable,$Arguments,$TimeoutSeconds=60)
    $kind = if ($Arguments -contains 'browser') { 'ensure' } elseif ($Executable -eq $nodeExe) { 'cli' } elseif ($Executable -match 'hyperframes-ffmpeg') { 'encoder-version' } else { 'browser-version' }
    $script:calls.Add($kind)
    if ($kind -eq 'ensure' -and $env:HYPERFRAMES_BROWSER_PATH -ne $hyperframesBrowserExe) { throw 'Ensure did not receive the local browser override' }
    if ($kind -eq 'ensure' -and $case -eq 'ensure-exception') { throw 'fixture browser process error' }
    $fail = ($case -eq 'encoder-failed' -and $kind -eq 'encoder-version') -or ($case -eq 'cli-failed' -and $kind -eq 'cli') -or ($case -eq 'browser-version-failed' -and $kind -eq 'browser-version')
    $version = if ($kind -eq 'browser-version') { 'Google Chrome for Testing 152.0.7977.30' } elseif ($case -eq 'cli-wrong-version') { '0.0.1' } else { '0.8.134' }
    return [pscustomobject]@{ ExitCode = $(if ($fail) { 1 } else { 0 }); TimedOut = $false; Output = 'fixture diagnostic'; StdOut = $version }
}
$savedOverride = [Environment]::GetEnvironmentVariable('HYPERFRAMES_BROWSER_PATH', 'Process')
try {
    $env:HYPERFRAMES_BROWSER_PATH = 'fixture-existing-user-setting'
    foreach ($case in @('clean','reuse','changed-lock','partial-tree','extract-cli-error','cli-failed','cli-wrong-version','extract-encoder-error','encoder-failed','extract-browser-error','browser-version-failed','ensure-exception')) {
        $runtimeRoot = Join-Path $fixture $case
        $entry = Join-Path $runtimeRoot 'hyperframes\node_modules\hyperframes\bin\hyperframes.mjs'
        if ($case -in @('reuse','changed-lock','partial-tree','extract-cli-error')) {
            New-Item -ItemType Directory -Force -Path (Split-Path $entry) | Out-Null
            [IO.File]::WriteAllText($entry, 'fixture prior entry')
        }
        if ($case -in @('reuse','changed-lock','extract-cli-error')) {
            $marker = if ($case -eq 'reuse') { 'fixture-lock-identity' } else { 'fixture-old-lock' }
            [IO.File]::WriteAllText((Join-Path $runtimeRoot 'hyperframes\.installed-lock-sha256'), $marker)
        }
        if ($case -eq 'reuse') {
            $existingBrowser = Join-Path $runtimeRoot 'hyperframes-browser\152.0.7977.30\chrome-headless-shell-win64\chrome-headless-shell.exe'
            New-Item -ItemType Directory -Force -Path (Split-Path $existingBrowser) | Out-Null
            [IO.File]::WriteAllText($existingBrowser, 'fixture browser')
        }
        $script:calls = [Collections.Generic.List[string]]::new()
        $script:stages = [Collections.Generic.List[string]]::new()
        . $block
        if ($env:HYPERFRAMES_BROWSER_PATH -ne 'fixture-existing-user-setting') { throw "$case changed the caller's browser setting" }
        if ($case -in @('clean','reuse','changed-lock','partial-tree')) {
            if ($hyperframesHealth -ne 'passed' -or $hyperframesBrowserHealth -ne 'passed') { throw "$case failed to become ready" }
            if ($case -eq 'reuse' -and $calls.Contains('extract-cli')) { throw 'Reuse reinstalled the healthy CLI' }
            if (-not $calls.Contains('extract-encoder') -or -not $calls.Contains('extract-browser')) { throw "$case failed to restore complete private archives" }
            if ($case -ne 'reuse' -and -not $calls.Contains('extract-cli')) { throw "$case did not deploy CLI" }
            if (-not (Test-Path -LiteralPath $hyperframesMarker)) { throw 'Successful deployment lost its marker' }
            if (-not $stages.Contains('runtime.hyperframesBrowserCheck')) { throw 'Missing local browser progress' }
            $shim = [IO.File]::ReadAllText($hyperframesCmd)
            if ($shim -notmatch 'setlocal' -or $shim -notmatch 'HYPERFRAMES_BROWSER_PATH=%~dp0') { throw 'Runtime shim does not retain the local browser' }
        } else {
            if ($hyperframesHealth -ne 'install-failed' -or -not $hyperframesError) { throw "$case lost its actual failure" }
            if ($case -eq 'extract-cli-error' -and $calls.Count -ne 1) { throw 'Partial CLI extraction was probed' }
            if ($case -in @('extract-cli-error','cli-failed','cli-wrong-version') -and (Test-Path -LiteralPath $hyperframesMarker)) { throw 'Incomplete CLI deployment retained a success marker' }
        }
        Write-Output "Source-branch regression passed: $case ($($calls -join ', '))"
        if ($case -in @('extract-cli-error','extract-encoder-error','extract-browser-error')) {
            $expectedExtraction = switch ($case) { 'extract-cli-error' { 'extract-cli' } 'extract-encoder-error' { 'extract-encoder' } 'extract-browser-error' { 'extract-browser' } }
            $case = 'retry-after-partial-extraction'
            $script:calls.Clear()
            . $block
            if ($hyperframesHealth -ne 'passed' -or -not $calls.Contains($expectedExtraction)) { throw "Explicit retry failed to replace the partial tree: $expectedExtraction" }
            Write-Output "Source-branch regression passed: explicit retry after partial extraction ($expectedExtraction)"
        }
    }
} finally { [Environment]::SetEnvironmentVariable('HYPERFRAMES_BROWSER_PATH', $savedOverride, 'Process') }

$installer = [IO.File]::ReadAllText((Join-Path $repo 'installer-pack\install-to-codex.ps1'))
$ast = [Management.Automation.Language.Parser]::ParseInput($installer, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Installer script parse failed' }
$f = $ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Test-RuntimeReady'}, $true)
. ([scriptblock]::Create($f.Extent.Text))
$start = $installer.IndexOf("`$runtimeOutput = ''")
$end = $installer.IndexOf('$runtimeDiagnostic = Write-Diagnostic', $start)
$receiver = [scriptblock]::Create($installer.Substring($start, $end - $start))
$runtimeScript = Join-Path $fixture 'runtime-fixture.ps1'
function Redact-Diagnostic { param($Text) return $Text }
$PackRoot=$fixture; $userProfile=$fixture; $codexHome=$fixture; $ProgressFile=''
$ownershipRoot=$fixture; $diagnosticsRoot=Join-Path $fixture 'diagnostics'
$feishuEnabled=$false; $mediaGenerationEnabled=$true; $MediaGatewayCatalogStatus='not-checked'
foreach ($case in @('optional-cli-failed','optional-browser-failed','core-failed','provisioner-exception')) {
    $payload = [ordered]@{ schemaVersion=1; status='partial'; python=@{health='passed';executable='fixture-python'}; manim=@{health='passed'}; node=@{health='passed'}; ffmpeg=@{health='passed'}; hyperframes=@{cliHealth='install-failed';browserHealth='unknown';error='fixture'}; feishu=@{health='disabled'} }
    if ($case -eq 'optional-browser-failed') { $payload.hyperframes.cliHealth='passed'; $payload.hyperframes.browserHealth='install-failed' }
    if ($case -eq 'core-failed') { $payload.python.health='failed' }
    $scriptText = "'" + ($payload | ConvertTo-Json -Depth 5 -Compress) + "'"
    if ($case -eq 'provisioner-exception') { $scriptText = "throw 'fixture provisioner exception'" }
    [IO.File]::WriteAllText($runtimeScript, $scriptText, [Text.Encoding]::ASCII)
    . $receiver
    if ($case -like 'optional-*') {
        if (-not (Test-RuntimeReady $runtimeState) -or $runtimeState.error -or -not $runtimeState.mediaGeneration) { throw "$case prevented core finalization" }
    } elseif (-not $runtimeState.error -or (Test-RuntimeReady $runtimeState)) { throw "$case lost the core failure" }
    Write-Output "Runtime receiver regression passed: $case"
}
