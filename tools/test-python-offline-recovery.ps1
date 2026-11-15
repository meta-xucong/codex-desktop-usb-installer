param([ValidateSet('first','recovery')][string]$Mode='first')
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$evidence=Join-Path $repo ('tmp/usb-python-recovery-1114/'+$Mode)
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$fixture=Join-Path $repo ('tmp/python-offline-'+$Mode+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
$runtimeRoot=Join-Path $fixture 'runtime'
New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
$env:USERPROFILE=Join-Path $fixture 'profile';$env:HOME=$env:USERPROFILE
$env:LOCALAPPDATA=Join-Path $env:USERPROFILE 'AppData/Local';$env:APPDATA=Join-Path $env:USERPROFILE 'AppData/Roaming'
$env:PATH=$env:SystemRoot+'\System32;'+$env:SystemRoot
$env:PYTHONNOUSERSITE='1';$env:PYTHONHOME=$null
$env:PIP_CONFIG_FILE='NUL';$env:PIP_CACHE_DIR=Join-Path $fixture 'empty-pip-cache'
$env:PIP_INDEX_URL=$null;$env:PIP_EXTRA_INDEX_URL=$null
$env:MPLCONFIGDIR=Join-Path $fixture 'matplotlib-config'
New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA,$env:APPDATA,$env:PIP_CACHE_DIR,$env:MPLCONFIGDIR | Out-Null
if(Get-Command node,chrome,ffmpeg -ErrorAction SilentlyContinue){throw 'Fixture unexpectedly resolves a system Node/Chrome/FFmpeg'}
$basePython='C:\Users\T14S\AppData\Local\Programs\Python\Python312\python.exe'
$wheelDir=Join-Path $repo 'installer-pack/runtime/bundled/python-wheels'
$lockPath=Join-Path $repo 'installer-pack/runtime/bundled/python-requirements.lock'
$manimLockPath=Join-Path $repo 'installer-pack/runtime/bundled/manim-requirements.lock'
$source=[IO.File]::ReadAllText((Join-Path $repo 'installer-pack/runtime/provision-runtime.ps1'))
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Provisioner parse failed'}
foreach($name in @('ConvertTo-WindowsArgument','Stop-NativeProcessTree','Invoke-NativeCapture','Test-ExactPython','Test-VenvPython')){
 . ([scriptblock]::Create($ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true).Extent.Text))
}
Set-Item Function:\Invoke-RealNativeCapture (Get-Item Function:\Invoke-NativeCapture).ScriptBlock
$script:nativeCalls=[Collections.Generic.List[object]]::new()
function Invoke-NativeCapture {
 param($Executable,[string[]]$Arguments,[int]$TimeoutSeconds=60)
 $result=Invoke-RealNativeCapture -Executable $Executable -Arguments $Arguments -TimeoutSeconds $TimeoutSeconds
 $script:nativeCalls.Add([pscustomobject]@{executable=$Executable;arguments=$Arguments;exit=$result.ExitCode})
 Add-Content -LiteralPath (Join-Path $evidence 'native.log') -Value (($Arguments -join ' ') + "`nexit="+$result.ExitCode+"`n"+$result.Output) -Encoding UTF8
 return $result
}
function Set-InstallProgress {param($Stage) Write-Host $Stage}
$start=$source.IndexOf('$venvRoot =');$split=$source.IndexOf("Set-InstallProgress 'runtime.manim'",$start);$end=$source.IndexOf("Set-InstallProgress 'runtime.nodeMedia'",$split)
$blocks=@{python=[scriptblock]::Create($source.Substring($start,$split-$start));manim=[scriptblock]::Create($source.Substring($split,$end-$split))}
$hook=Join-Path $fixture 'test-hook'
New-Item -ItemType Directory -Path $hook | Out-Null
$env:USB_PYTHON_NETWORK_LOG=Join-Path $fixture 'blocked-network.log'
[IO.File]::WriteAllText((Join-Path $hook 'sitecustomize.py'),@'
import os, socket, traceback
def denied(*args, **kwargs):
    with open(os.environ['USB_PYTHON_NETWORK_LOG'], 'a', encoding='utf-8') as log:
        log.write('blocked external network operation\n'+''.join(traceback.format_stack(limit=8)))
    raise RuntimeError('Test process prohibits external networking')
socket.socket.connect = denied
socket.create_connection = denied
socket.getaddrinfo = denied
if os.environ.get('USB_PYTHON_INTERRUPT'):
    try:
        from pip._internal.operations.install.wheel import ZipBackedFile
        original = ZipBackedFile.save
        def save(self):
            original(self)
            if self.src_record_path.startswith(os.environ['USB_PYTHON_INTERRUPT']+'-') and self.src_record_path.endswith('.dist-info/METADATA'):
                print('Injected interruption after METADATA', flush=True)
                os._exit(73)
        ZipBackedFile.save = save
    except ImportError:
        pass
'@,[Text.UTF8Encoding]::new($false))
$env:PYTHONPATH=$hook
$records=[Collections.Generic.List[object]]::new()
$watch=[Diagnostics.Stopwatch]::StartNew()
foreach($component in @('python','manim')){
 $runtimeEnv=Join-Path $runtimeRoot ($component+'-env');$runtimeExe=Join-Path $runtimeEnv 'Scripts/python.exe'
 $record=[ordered]@{component=$component;mode=$Mode;firstError=$null}
 if($Mode -eq 'recovery'){
  $env:USB_PYTHON_INTERRUPT=if($component -eq 'python'){'matplotlib'}else{'pycairo'}
  $firstError='';try{. $blocks[$component]}catch{$firstError=$_.Exception.Message}
  $env:USB_PYTHON_INTERRUPT=$null
  if($firstError -notmatch 'Injected interruption after METADATA'){throw "Wrong first failure: $component / $firstError"}
  $metadata=@(Get-ChildItem -LiteralPath (Join-Path $runtimeEnv 'Lib/site-packages') -Directory -Filter $(if($component -eq 'python'){'matplotlib-*.dist-info'}else{'pycairo-*.dist-info'}))
  $module=Join-Path $runtimeEnv ('Lib/site-packages/'+$(if($component -eq 'python'){'matplotlib'}else{'cairo'}))
  if($metadata.Count -ne 1 -or -not(Test-Path -LiteralPath (Join-Path $metadata[0].FullName 'METADATA')) -or (Test-Path -LiteralPath $module)){throw 'Expected metadata-only interruption state missing'}
  $pipCheck=Invoke-NativeCapture -Executable $runtimeExe -Arguments @('-m','pip','--version')
  if($pipCheck.ExitCode -ne 0){throw 'Pip itself should remain usable in the interrupted case'}
  $record.firstError=$firstError;$record.metadataOnly=$true;$record.pipStillHealthy=$true
 }
 $script:nativeCalls.Clear()
 . $blocks[$component]
 $rebuilds=@($nativeCalls|Where-Object{$_.arguments.Count -ge 3 -and $_.arguments[0] -eq '-m' -and $_.arguments[1] -eq 'venv' -and $_.arguments[2] -eq '--clear'}).Count
 if($rebuilds -ne 1){throw "Expected one rebuild for $component/$Mode, got $rebuilds"}
 $record.rebuilds=$rebuilds;$record.deployment='passed'
 # Healthy repeats must retain the actual existing venv and still finish.
 $sentinel=Join-Path $runtimeEnv 'healthy-reuse.txt';[IO.File]::WriteAllText($sentinel,'preserve')
 $script:nativeCalls.Clear()
 . $blocks[$component]
 if(@($nativeCalls|Where-Object{$_.arguments -contains '--clear'}).Count -ne 0 -or -not(Test-Path -LiteralPath $sentinel)){throw 'Healthy environment was unnecessarily rebuilt'}
 $record.healthyRepeatReused=$true;$records.Add($record)
 Write-Host "Real deployment passed: $component/$Mode"
}
$watch.Stop()
$chart=Join-Path $evidence 'chart.png';$chartScript=Join-Path $fixture 'chart.py'
[IO.File]::WriteAllText($chartScript,@'
import sys
import numpy as np
import pandas as pd
import matplotlib
matplotlib.use('Agg')
from matplotlib import pyplot as plt
from PIL import Image
frame=pd.DataFrame({'x':np.arange(4),'y':[1,4,2,5]})
plt.figure(figsize=(4,3));plt.plot(frame.x,frame.y);plt.savefig(sys.argv[1]);plt.close()
with Image.open(sys.argv[1]) as im:
    assert im.size == (400,300), im.size
print('Actual matplotlib chart: 400x300')
'@,[Text.UTF8Encoding]::new($false))
$chartResult=Invoke-NativeCapture -Executable $venvPython -Arguments @($chartScript,$chart)
if($chartResult.ExitCode -ne 0){throw $chartResult.Output}
$scene=Join-Path $fixture 'scene.py'
[IO.File]::WriteAllText($scene,@'
from manim import Scene, Square, BLUE
class LocalSmoke(Scene):
    def construct(self):
        self.add(Square(color=BLUE, fill_opacity=1))
        self.wait(0.5)
'@,[Text.UTF8Encoding]::new($false))
$manimHelp=Invoke-NativeCapture -Executable $manimPython -Arguments @('-m','manim','render','--help')
if($manimHelp.ExitCode -ne 0 -or $manimHelp.Output -notmatch '--media_dir' -or $manimHelp.Output -notmatch '--format' -or $manimHelp.Output -notmatch '--silent'){throw 'Manim render interface changed'}
# Manim's documented --silent only disables its optional post-render PyPI version notice.
$render=Invoke-NativeCapture -Executable $manimPython -Arguments @('-m','manim','render','--silent','--disable_caching','--media_dir',(Join-Path $fixture 'media'),'--format','mp4','-r','320,180','--fps','4',$scene,'LocalSmoke') -TimeoutSeconds 120
if($render.ExitCode -ne 0){throw $render.Output}
$videos=@(Get-ChildItem -LiteralPath (Join-Path $fixture 'media') -Recurse -File -Filter 'LocalSmoke.mp4')
if($videos.Count -ne 1){throw 'Expected one Manim render'}
$video=Join-Path $evidence 'manim.mp4';Copy-Item -LiteralPath $videos[0].FullName -Destination $video
$videoProbe=Join-Path $fixture 'probe_video.py'
[IO.File]::WriteAllText($videoProbe,@'
import av,json,sys
with av.open(sys.argv[1]) as video:
    frames=list(video.decode(video=0))
    assert len(frames)>=2 and frames[0].width==320 and frames[0].height==180
    print(json.dumps(dict(frames=len(frames),width=frames[0].width,height=frames[0].height)))
'@,[Text.UTF8Encoding]::new($false))
$probe=Invoke-NativeCapture -Executable $manimPython -Arguments @($videoProbe,$video)
if($probe.ExitCode -ne 0){throw $probe.Output}
$networkAttempts=if(Test-Path -LiteralPath $env:USB_PYTHON_NETWORK_LOG){@(Get-Content -LiteralPath $env:USB_PYTHON_NETWORK_LOG).Count}else{0}
if($networkAttempts -ne 0){throw 'Unexpected external networking during local deployment/render'}
$result=[ordered]@{mode=$Mode;fixture=$fixture;scope='actual full Python/Manim locked local dependencies in private venvs; existing base CPython, no global installation';results=$records;seconds=$watch.Elapsed.TotalSeconds;chart=$chart;chartBytes=(Get-Item -LiteralPath $chart).Length;video=$video;videoBytes=(Get-Item -LiteralPath $video).Length;videoProbe=($probe.StdOut|ConvertFrom-Json);externalNetworkAttempts=$networkAttempts;systemNodeChromeFfmpegOnPath=$false;paidCalls=0}
$json=$result|ConvertTo-Json -Depth 7
[IO.File]::WriteAllText((Join-Path $evidence 'result.json'),$json,[Text.UTF8Encoding]::new($false))
$json
