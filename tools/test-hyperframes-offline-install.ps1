# Isolated real first-install regression. Uses existing integrity checks.
param(
    [ValidateSet('none','encoder','browser')][string]$InterruptComponent='none',
    [string]$EvidenceDirectory='tmp\usb-runtime-extract-recovery-1114'
)
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$evidenceRoot=Join-Path $repo $EvidenceDirectory
$evidence=Join-Path $evidenceRoot $InterruptComponent
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$fixture=Join-Path $repo ('tmp\hf-first-' + [guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $fixture | Out-Null
$env:USERPROFILE=Join-Path $fixture 'new user'
$env:HOME=$env:USERPROFILE
$env:LOCALAPPDATA=Join-Path $env:USERPROFILE 'AppData\Local'
$env:APPDATA=Join-Path $env:USERPROFILE 'AppData\Roaming'
$env:PATH=$env:SystemRoot+'\System32;'+$env:SystemRoot
$env:NPM_CONFIG_CACHE=Join-Path $fixture 'empty-npm-cache'
New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA,$env:APPDATA,$env:NPM_CONFIG_CACHE | Out-Null
if(Get-Command node,chrome -ErrorAction SilentlyContinue){throw 'Fixture unexpectedly resolves system Node/Chrome'}
$env:USB_TEST_NETWORK_LOG=Join-Path $fixture 'blocked-network.jsonl'
$env:USB_TEST_STAGE='first-install'
$env:NODE_OPTIONS='--require "'+(Join-Path $evidenceRoot 'offline-network.cjs').Replace('\','/')+'"'
$source=[IO.File]::ReadAllText((Join-Path $repo 'installer-pack\runtime\provision-runtime.ps1'))
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Provisioner parse error'}
foreach($name in @('Test-PathWithin','Assert-Media','ConvertTo-WindowsArgument','Stop-NativeProcessTree','Invoke-NativeCapture','Write-AsciiFile','Get-NodeVersion')){
    $f=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    . ([scriptblock]::Create($f.Extent.Text))
}
$start=$source.IndexOf('$hyperframesVersion ='); $end=$source.IndexOf('if ($EnableFeishu)',$start)
$block=[scriptblock]::Create($source.Substring($start,$end-$start))
$packRootFull=Join-Path $repo 'installer-pack'
$mediaManifest=Get-Content -LiteralPath (Join-Path $packRootFull 'runtime\bundled\media-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$nodeZip=Assert-Media $mediaManifest 'runtime.node22-common' $packRootFull
$ffmpegZip=Assert-Media $mediaManifest 'runtime.ffmpeg-windows-x64' $packRootFull
$hyperframesCliZip=Assert-Media $mediaManifest 'runtime.hyperframes-cli' $packRootFull
$hyperframesFfmpegZip=Assert-Media $mediaManifest 'runtime.hyperframes-ffmpeg' $packRootFull
$hyperframesPackageFile=Assert-Media $mediaManifest 'runtime.hyperframes-package-json' $packRootFull
$hyperframesLockFile=Assert-Media $mediaManifest 'runtime.hyperframes-package-lock' $packRootFull
$hyperframesBrowserZip=Assert-Media $mediaManifest 'runtime.hyperframes-browser' $packRootFull
$runtimeRoot=Join-Path $fixture 'codex-home\codex-agent-kit\runtime'
New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
$script:stages=[Collections.Generic.List[string]]::new()
function Set-InstallProgress { param($Stage) $script:stages.Add($Stage); $env:USB_TEST_STAGE=$Stage; Write-Host $Stage }
# Exercise the actual shared runtime installer before HyperFrames, rather
# than preparing Node/FFmpeg outside the modified production branches.
$sharedStart=$source.IndexOf("Set-InstallProgress 'runtime.nodeMedia'")
$sharedEnd=$source.IndexOf('$hyperframesVersion =',$sharedStart)
$setupWatch=[Diagnostics.Stopwatch]::StartNew()
. ([scriptblock]::Create($source.Substring($sharedStart,$sharedEnd-$sharedStart)))
# Inject interruption only in the test process. Preserve real EXEs from the
# actual archives; all subsequent deployment uses the real Expand-Archive.
$script:injectInterruption=$InterruptComponent -ne 'none'
$script:archiveCalls=[Collections.Generic.List[string]]::new()
function Expand-Archive {
    param($LiteralPath,$DestinationPath,[switch]$Force)
    $script:archiveCalls.Add([string]$LiteralPath)
    $isTarget=($InterruptComponent -eq 'encoder' -and $LiteralPath -eq $hyperframesFfmpegZip) -or ($InterruptComponent -eq 'browser' -and $LiteralPath -eq $hyperframesBrowserZip)
    if ($script:injectInterruption -and $isTarget) {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $names=if ($InterruptComponent -eq 'encoder') { @('ffmpeg-n9.0-latest-win64-gpl-shared-9.0/bin/ffmpeg.exe','ffmpeg-n9.0-latest-win64-gpl-shared-9.0/bin/ffprobe.exe') } else { @('chrome-headless-shell-win64/chrome-headless-shell.exe') }
        $archive=[IO.Compression.ZipFile]::OpenRead($LiteralPath)
        try {
            foreach ($name in $names) {
                $target=Join-Path $DestinationPath $name
                New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
                $inputStream=$archive.GetEntry($name).Open();$outputStream=[IO.File]::Create($target)
                try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose();$inputStream.Dispose() }
            }
        } finally { $archive.Dispose() }
        throw "Injected extraction interruption after $InterruptComponent EXE files"
    }
    Microsoft.PowerShell.Archive\Expand-Archive -LiteralPath $LiteralPath -DestinationPath $DestinationPath -Force:$Force
}
$env:HYPERFRAMES_BROWSER_PATH='fixture-caller-setting'
$watch=[Diagnostics.Stopwatch]::StartNew()
. $block
$interruptedState=$null
if ($InterruptComponent -ne 'none') {
    if ($hyperframesHealth -ne 'install-failed' -or $hyperframesError -notlike 'Injected extraction interruption*') { throw 'Interruption did not preserve its original failure' }
    $residualExe=if ($InterruptComponent -eq 'encoder') { $hyperframesFfmpegExe } else { $hyperframesBrowserExe }
    $missingDependency=if ($InterruptComponent -eq 'encoder') { Join-Path $hyperframesFfmpegBin 'avcodec-63.dll' } else { Join-Path (Split-Path -Parent $hyperframesBrowserExe) 'icudtl.dat' }
    if (-not (Test-Path -LiteralPath $residualExe) -or (Test-Path -LiteralPath $missingDependency)) { throw 'Fixture did not leave an EXE with missing dependencies' }
    $interruptedState=[ordered]@{component=$InterruptComponent;health=$hyperframesHealth;error=$hyperframesError;exePresent=$true;dependencyMissing=$true}
    $script:injectInterruption=$false
    $script:archiveCalls.Clear()
    . $block
    $targetZip=if ($InterruptComponent -eq 'encoder') { $hyperframesFfmpegZip } else { $hyperframesBrowserZip }
    if (-not $archiveCalls.Contains($targetZip) -or -not (Test-Path -LiteralPath $missingDependency)) { throw 'Explicit reinstall did not restore the incomplete component from local media' }
}
$watch.Stop()
$setupWatch.Stop()
if($hyperframesHealth -ne 'passed'){throw "Real first deployment failed: $hyperframesError"}
if($env:HYPERFRAMES_BROWSER_PATH -ne 'fixture-caller-setting'){throw 'Caller override changed'}
if($null -eq $hyperframesBrowserCheckMs){throw 'Browser startup duration was not recorded'}
$firstBrowserCheckMs=$hyperframesBrowserCheckMs
$firstStages=$stages.ToArray()
$repeatReused=$null
if ($InterruptComponent -eq 'none') {
    $markerTime=(Get-Item -LiteralPath $hyperframesMarker).LastWriteTimeUtc
    $entryTime=(Get-Item -LiteralPath $hyperframesEntry).LastWriteTimeUtc
    $script:stages.Clear()
    . $block
    if($hyperframesHealth -ne 'passed' -or (Get-Item -LiteralPath $hyperframesMarker).LastWriteTimeUtc -ne $markerTime -or (Get-Item -LiteralPath $hyperframesEntry).LastWriteTimeUtc -ne $entryTime){throw 'Repeat deployment did not reuse the healthy CLI'}
    $repeatReused=$true
}
$env:USB_TEST_STAGE='shim-browser-selection'
$shimOutput=& $hyperframesCmd browser ensure | Out-String
if($LASTEXITCODE -ne 0 -or $shimOutput -notmatch 'env'){throw 'Production shim did not use bundled browser'}
if($env:HYPERFRAMES_BROWSER_PATH -ne 'fixture-caller-setting'){throw 'Shim leaked its override'}
# Prove the Windows native dependency closure, and the network boundary itself.
$probe=Join-Path $runtimeRoot 'hyperframes\offline-probe.cjs'
[IO.File]::WriteAllText($probe,@'
const esbuild=require('esbuild'); const sharp=require('sharp');
(async()=>{
  if(!esbuild.transformSync('let value=1').code.includes('value')) throw Error('esbuild failed');
  const data=await sharp({create:{width:8,height:8,channels:3,background:'#ffffff'}}).png().toBuffer();
  if((await sharp(data).metadata()).width!==8) throw Error('sharp failed');
  let blocked=false; try{require('net').connect(443,'203.0.113.1');}catch(e){blocked=e.code==='ENETUNREACH';}
  if(!blocked) throw Error('External socket was not blocked');
  const browser=await require('puppeteer-core').launch({executablePath:process.env.USB_TEST_BROWSER_EXE,headless:true,args:['--no-sandbox']});
  try { const page=await browser.newPage(); await page.goto('about:blank'); if(page.url()!=='about:blank') throw Error('Browser did not load the local blank page'); }
  finally { await browser.close(); }
  console.log('native dependencies, offline boundary and real browser launch passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
'@,[Text.Encoding]::ASCII)
$env:USB_TEST_STAGE='native-dependency-probe'
$env:USB_TEST_BROWSER_EXE=$hyperframesBrowserExe
$probeResult=Invoke-NativeCapture -Executable $nodeExe -Arguments @($probe)
if($probeResult.ExitCode -ne 0){throw $probeResult.Output}
# Reproduce the installer's real user PATH (command shims), not the native
# FFmpeg directory. The production HyperFrames shim must bind the real EXEs.
$kitHome=Split-Path -Parent $runtimeRoot
$wrapperStart=$source.IndexOf('$binRoot = Join-Path $kitHome')
$wrapperEnd=$source.IndexOf('$actualUserProfile =',$wrapperStart)
. ([scriptblock]::Create($source.Substring($wrapperStart,$wrapperEnd-$wrapperStart)))
$env:PATH=$binRoot+';'+$env:PATH
$env:HYPERFRAMES_FFMPEG_PATH='fixture-caller-ffmpeg'
$env:HYPERFRAMES_FFPROBE_PATH='fixture-caller-ffprobe'
$project=Join-Path $fixture 'local composition'
New-Item -ItemType Directory -Path $project | Out-Null
[IO.File]::WriteAllText((Join-Path $project 'index.html'),@'
<!doctype html><html><head><meta charset="utf-8"><title>Offline CLI test</title><style>html,body{margin:0}#test{width:800px;height:450px;background:#edf4ff;font:36px sans-serif;display:flex;align-items:center;justify-content:center}</style></head><body><div id="test" data-composition-id="offline-cli" data-no-timeline data-duration="1" data-width="800" data-height="450" data-fps="1">USB HYPERFRAMES LOCAL INSTALL PASS</div></body></html>
'@,[Text.Encoding]::ASCII)
$out=Join-Path $fixture 'frames'
$env:USB_TEST_STAGE='render'
$ErrorActionPreference='Continue'
& $hyperframesCmd render $project --format png-sequence --fps 1 --workers 1 -o $out 2>&1 | Out-File -LiteralPath (Join-Path $evidence 'render.log') -Encoding utf8
$exit=$LASTEXITCODE; $ErrorActionPreference='Stop'
if($exit -ne 0){throw "Render failed: $exit"}
$frames=@(Get-ChildItem -LiteralPath $out -Filter '*.png' -File)
if($frames.Count -ne 1){throw 'Expected one local frame'}
$mp4=Join-Path $fixture 'offline.mp4'
$ErrorActionPreference='Continue'
& $hyperframesCmd render $project --format mp4 --fps 1 --workers 1 -o $mp4 2>&1 | Out-File -LiteralPath (Join-Path $evidence 'render-mp4.log') -Encoding utf8
$mp4Exit=$LASTEXITCODE; $ErrorActionPreference='Stop'
if($mp4Exit -ne 0 -or -not (Test-Path -LiteralPath $mp4)){throw "Local MP4 render failed: $mp4Exit"}
$probeExe=$hyperframesFfprobeExe
$video=(& $probeExe -v error -select_streams v:0 -show_entries stream=codec_name,width,height -of json $mp4 | Out-String) | ConvertFrom-Json
if($LASTEXITCODE -ne 0 -or $video.streams[0].width -ne 800 -or $video.streams[0].height -ne 450){throw 'Rendered video dimensions are incorrect'}
if($env:HYPERFRAMES_FFMPEG_PATH -ne 'fixture-caller-ffmpeg' -or $env:HYPERFRAMES_FFPROBE_PATH -ne 'fixture-caller-ffprobe'){throw 'Production shim leaked FFmpeg overrides'}
$network=@(if(Test-Path -LiteralPath $env:USB_TEST_NETWORK_LOG){Get-Content -LiteralPath $env:USB_TEST_NETWORK_LOG | ForEach-Object{$_ | ConvertFrom-Json}})
if(@($network | Where-Object{$_.stage -eq 'runtime.hyperframesBrowserCheck'}).Count){throw 'Local browser startup check attempted to use the network'}
if(@($network | Where-Object{$_.host -match 'googleapis|gvt1|googleusercontent'}).Count){throw 'Browser download attempted'}
if(@(Get-ChildItem -LiteralPath $env:NPM_CONFIG_CACHE -Force).Count){throw 'Target installation used the npm cache'}
$report=[ordered]@{scope='fresh private runtime from bundled media; external Node networking blocked and browser external DNS disabled; not a clean Windows VM';fixture=$fixture;interruptedState=$interruptedState;recovered=($InterruptComponent -ne 'none');firstInstallSeconds=$watch.Elapsed.TotalSeconds;sharedAndHyperframesSeconds=$setupWatch.Elapsed.TotalSeconds;sharedNodeHealth=$nodeHealth;sharedFfmpegHealth=$ffmpegHealth;firstStages=$firstStages;cliHealth=$hyperframesCliHealth;browserHealth=$hyperframesBrowserHealth;shimBrowserSource='env';repeatReused=$repeatReused;systemNodeOnPath=$false;systemChromeOnPath=$false;targetNpmCacheEmpty=$true;nativeEsbuildAndSharp='passed';realBrowserLaunch='passed';remoteSocketBoundary='passed';blockedHosts=@($network.host | Sort-Object -Unique);browserDownloadAttempts=0;renderExitCode=$exit;png=$frames[0].FullName;pngBytes=$frames[0].Length;mp4=$mp4;mp4Bytes=(Get-Item -LiteralPath $mp4).Length;videoCodec=$video.streams[0].codec_name;productionCommandPath=$true;ffmpegCallerSettingsPreserved=$true;paidCalls=0}
$report.browserCheckMs=$firstBrowserCheckMs
$report.browserCheckNetworkAttempts=0
$json=$report|ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $evidence 'offline-install-result.json'),$json,[Text.UTF8Encoding]::new($false))
$json
