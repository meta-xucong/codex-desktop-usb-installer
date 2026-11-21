# Real ZIP/native recovery regression, isolated from user settings and network.
param([string]$EvidenceDirectory='tmp\usb-runtime-extract-recovery-1114')
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$evidence=Join-Path $repo $EvidenceDirectory
$fixture=Join-Path $repo ('tmp\runtime-recovery-'+[guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $fixture | Out-Null
$env:USERPROFILE=Join-Path $fixture 'profile';$env:HOME=$env:USERPROFILE
$env:LOCALAPPDATA=Join-Path $env:USERPROFILE 'AppData\Local';$env:APPDATA=Join-Path $env:USERPROFILE 'AppData\Roaming'
$env:PATH=$env:SystemRoot+'\System32;'+$env:SystemRoot
$env:NPM_CONFIG_CACHE=Join-Path $fixture 'empty-npm-cache'
New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA,$env:APPDATA,$env:NPM_CONFIG_CACHE | Out-Null
if(Get-Command node,npm,npx -ErrorAction SilentlyContinue){throw 'System Node/npm/npx unexpectedly on PATH'}
$env:USB_TEST_NETWORK_LOG=Join-Path $fixture 'blocked-network.jsonl';$env:USB_TEST_STAGE='recovery'
$env:NODE_OPTIONS='--require "'+(Join-Path $evidence 'offline-network.cjs').Replace('\','/')+'"'
$source=[IO.File]::ReadAllText((Join-Path $repo 'installer-pack\runtime\provision-runtime.ps1'))
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Provisioner parse error'}
foreach($name in @('Test-PathWithin','Assert-Media','ConvertTo-WindowsArgument','Stop-NativeProcessTree','Invoke-NativeCapture','Get-NodeVersion','Write-AsciiFile')){
 . ([scriptblock]::Create($ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true).Extent.Text))
}
$packRootFull=Join-Path $repo 'installer-pack'
$media=Get-Content -LiteralPath (Join-Path $packRootFull 'runtime\bundled\media-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$nodeZip=Assert-Media $media 'runtime.node22-common' $packRootFull
$ffmpegZip=Assert-Media $media 'runtime.ffmpeg-windows-x64' $packRootFull
$feishuZip=Assert-Media $media 'runtime.feishu-mcp-package' $packRootFull
$start=$source.IndexOf("Set-InstallProgress 'runtime.nodeMedia'");$end=$source.IndexOf('$ffmpegRoot =',$start)
$nodeBlock=[scriptblock]::Create($source.Substring($start,$end-$start))
$start=$end;$end=$source.IndexOf('$hyperframesVersion =',$start)
$ffmpegBlock=[scriptblock]::Create($source.Substring($start,$end-$start))
$start=$source.IndexOf('if ($EnableFeishu)', $end);$end=$source.IndexOf('$binRoot = Join-Path $kitHome',$start)
$feishuBlock=[scriptblock]::Create($source.Substring($start,$end-$start))
function Set-InstallProgress {param($Stage) Write-Host $Stage}
Add-Type -AssemblyName System.IO.Compression.FileSystem
# Import before installing the test hook: module auto-loading would otherwise
# replace it when the first recovery calls the qualified real implementation.
Import-Module Microsoft.PowerShell.Archive
$script:inject=$false;$script:extractCalls=[Collections.Generic.List[string]]::new()
function Expand-Archive {
 param($LiteralPath,$DestinationPath,[switch]$Force)
 $script:extractCalls.Add([string]$LiteralPath)
 if(-not $script:inject){Microsoft.PowerShell.Archive\Expand-Archive -LiteralPath $LiteralPath -DestinationPath $DestinationPath -Force:$Force;return}
 $stopEntry=switch($case){
  'node' {'node-v22.22.0-win-x64/node.exe'}
  'ffmpeg' {'ffmpeg-n9.0-latest-win64-lgpl-shared-9.0/bin/ffprobe.exe'}
  'feishu' {'node_modules/@larksuiteoapi/lark-mcp/dist/cli.js'}
 }
 $archive=[IO.Compression.ZipFile]::OpenRead($LiteralPath)
 try{
  foreach($entry in $archive.Entries){
   if($entry.FullName.EndsWith('/')){continue}
   $target=Join-Path $DestinationPath $entry.FullName
   New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target)|Out-Null
   $inputStream=$entry.Open();$outputStream=[IO.File]::Create($target)
   try{$inputStream.CopyTo($outputStream)}finally{$outputStream.Dispose();$inputStream.Dispose()}
   if($entry.FullName -eq $stopEntry){throw "Injected interruption after $case entry"}
  }
 }finally{$archive.Dispose()}
}
function Invoke-CommandWrapper([string]$Wrapper,[string[]]$Arguments){
 $savedPreference=$ErrorActionPreference
 try{$ErrorActionPreference='Continue';$output=& $Wrapper @Arguments 2>&1|Out-String;$code=$LASTEXITCODE}
 finally{$ErrorActionPreference=$savedPreference}
 if($code -ne 0){throw "Wrapper failed ($code): $output"}
 return $output.Trim()
}
$results=[Collections.Generic.List[object]]::new()
# Disabled branch must leave an existing directory and its files untouched.
$runtimeRoot=Join-Path $fixture 'disabled\runtime';$feishuRoot=Join-Path $runtimeRoot 'feishu'
New-Item -ItemType Directory -Force -Path $feishuRoot | Out-Null
$sentinel=Join-Path $feishuRoot 'existing.txt';[IO.File]::WriteAllText($sentinel,'preserve existing disabled files')
$EnableFeishu=$false;$feishuHealth='disabled'
. $feishuBlock
if($extractCalls.Count -ne 0 -or [IO.File]::ReadAllText($sentinel) -ne 'preserve existing disabled files' -or $feishuHealth -ne 'disabled'){throw 'Disabled Feishu was modified or probed'}
$results.Add([ordered]@{case='feishu-disabled';extractions=0;existingFilesPreserved=$true;health=$feishuHealth})

foreach($case in @('node','ffmpeg','feishu')){
 $runtimeRoot=Join-Path $fixture ($case+'\runtime')
 New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
 $EnableFeishu=$case -eq 'feishu'
 $block=switch($case){'node'{$nodeBlock};'ffmpeg'{$ffmpegBlock};'feishu'{$feishuBlock}}
 $script:inject=$true;$script:extractCalls.Clear();$originalError=''
 try{. $block}catch{$originalError=$_.Exception.Message}
 if($case -eq 'feishu'){
  if($feishuHealth -ne 'install-failed'){throw 'Interrupted Feishu preparation did not mark the component unavailable'}
  $originalError=$feishuError
 }
 if(-not $originalError.StartsWith('Injected interruption')){throw "Expected original interruption missing: $case; actual=$originalError"}
 $entry=switch($case){'node'{$nodeExe};'ffmpeg'{$ffprobeExe};'feishu'{$feishuCli}}
 $dependency=switch($case){'node'{Join-Path $nodeRoot 'node_modules\npm\bin\npm-cli.js'};'ffmpeg'{$ffplayExe};'feishu'{Join-Path $feishuRoot 'node_modules\dotenv\package.json'}}
 if(-not(Test-Path -LiteralPath $entry) -or (Test-Path -LiteralPath $dependency)){throw 'Actual archive did not produce the intended partial state'}
 $script:inject=$false;$script:extractCalls.Clear()
 . $block
 if($extractCalls.Count -ne 1 -or -not(Test-Path -LiteralPath $dependency)){throw "Recovery skipped complete local extraction: $case"}
 $record=[ordered]@{case=$case;firstError=$originalError;entryExisted=$true;dependencyWasMissing=$true;recoveryExtractions=$extractCalls.Count;dependencyRestored=$true}
 if($case -eq 'node'){
  $binRoot=Join-Path (Split-Path -Parent $runtimeRoot) 'bin';$newline=[Environment]::NewLine
  $lines=@($source -split "`r?`n" | Where-Object { $_.StartsWith("Write-AsciiFile (Join-Path `$binRoot 'node.cmd')") -or $_.StartsWith("Write-AsciiFile (Join-Path `$binRoot 'npm.cmd')") -or $_.StartsWith("Write-AsciiFile (Join-Path `$binRoot 'npx.cmd')") })
  if($lines.Count -ne 3){throw 'Production wrapper anchors changed'}
  . ([scriptblock]::Create($lines -join [Environment]::NewLine))
  $record.nodeVersion=Invoke-CommandWrapper (Join-Path $binRoot 'node.cmd') @('--version')
  $record.npmVersion=Invoke-CommandWrapper (Join-Path $binRoot 'npm.cmd') @('--offline','--version')
  $record.npxVersion=Invoke-CommandWrapper (Join-Path $binRoot 'npx.cmd') @('--offline','--version')
  if($record.nodeVersion -ne 'v22.22.0' -or $record.npmVersion -notmatch '^\d+\.\d+\.\d+$' -or $record.npmVersion -ne $record.npxVersion){throw 'Restored Node/npm/npx wrappers returned unexpected versions'}
 } elseif($case -eq 'ffmpeg'){
  $play=Invoke-NativeCapture -Executable $ffplayExe -Arguments @('-version')
  if($play.ExitCode -ne 0 -or $play.Output -notmatch 'ffplay version'){throw 'Restored ffplay failed to load'}
  $video=Join-Path $fixture 'general-ffmpeg.mkv'
  $encode=Invoke-NativeCapture -Executable $ffmpegExe -Arguments @('-y','-f','lavfi','-i','color=c=blue:s=32x32:r=1','-frames:v','1','-c:v','ffv1','-threads','1',$video)
  if($encode.ExitCode -ne 0){throw $encode.Output}
  $probe=Invoke-NativeCapture -Executable $ffprobeExe -Arguments @('-v','error','-select_streams','v:0','-show_entries','stream=codec_name,width,height','-of','json',$video)
  if($probe.ExitCode -ne 0){throw $probe.Output}
  $stream=($probe.StdOut|ConvertFrom-Json).streams[0]
  if($stream.codec_name -ne 'ffv1' -or $stream.width -ne 32 -or $stream.height -ne 32){throw 'General FFmpeg render probe failed'}
  $record.health=$ffmpegHealth;$record.ffplay='passed';$record.videoCodec=$stream.codec_name;$record.video=$video
 } else {
  if($feishuHealth -ne 'passed'){throw 'Restored Feishu CLI not healthy'}
  $record.health=$feishuHealth;$record.cliHelp=$mcpHelpResult.Output;$record.nodeExecutable=$nodeExe;$record.cli=$feishuCli
 }
 $results.Add($record)
 Write-Host "Actual recovery passed: $case"
}
$env:USB_TEST_STAGE='feishu-fresh-default'
[Environment]::SetEnvironmentVariable('FEISHU_APP_ID',$null,'Process')
[Environment]::SetEnvironmentVariable('FEISHU_APP_SECRET',$null,'Process')
$runtimeRoot=Join-Path $fixture 'feishu-fresh-default\runtime'
$EnableFeishu=$true;$feishuHealth='disabled';$script:extractCalls.Clear()
. $feishuBlock
if($feishuHealth -ne 'passed' -or $extractCalls.Count -ne 1){throw 'Fresh Feishu deployment without credentials failed'}
$results.Add([ordered]@{case='feishu-fresh-default';extractions=$extractCalls.Count;health=$feishuHealth;credentialsProvided=$false})
Write-Host 'Fresh offline Feishu deployment without credentials passed'
if(Test-Path -LiteralPath (Join-Path $env:NPM_CONFIG_CACHE '_cacache')){throw 'npm package cache unexpectedly populated'}
$report=[ordered]@{scope='actual local ZIP recovery; production commands; private HOME, no system Node/npm/npx PATH, external Node requests blocked';fixture=$fixture;results=$results;packageCacheAbsent=$true;paidCalls=0}
$json=$report|ConvertTo-Json -Depth 7
[IO.File]::WriteAllText((Join-Path $evidence 'runtime-recovery-result.json'),$json,[Text.UTF8Encoding]::new($false))
$json
