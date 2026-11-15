$ErrorActionPreference='Stop'
if($env:USERNAME -ne 'WDAGUtilityAccount' -or -not(Test-Path -LiteralPath 'C:\InputPack\runtime\provision-runtime.ps1')){throw 'This test may run only in the prepared Windows Sandbox'}
$out='C:\Audit';$caseRoot='C:\PythonBaseRecoveryTest'
New-Item -ItemType Directory -Force -Path $caseRoot | Out-Null
Start-Transcript -LiteralPath (Join-Path $out 'transcript.log') -Force | Out-Null
function Save-Json($Name,$Value){[IO.File]::WriteAllText((Join-Path $out $Name),($Value|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))}
$global:installerCalls=[Collections.Generic.List[object]]::new()
$rows=[Collections.Generic.List[object]]::new()
$timer=[Diagnostics.Stopwatch]::StartNew()
$started=Get-Date
try{
 $networkCount=@(Get-CimInstance Win32_NetworkAdapterConfiguration -Filter 'IPEnabled=True').Count
 $initialPython=Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\python.exe'
 if($networkCount -ne 0 -or (Test-Path -LiteralPath $initialPython) -or (Get-Command node,chrome -ErrorAction SilentlyContinue)){throw 'Sandbox is not a fresh offline runtime fixture'}
 Save-Json 'initial.json' @{user=$env:USERNAME;networkInterfaces=$networkCount;pythonPresent=$false;nodeChromeCommands=$false;isAdministrator=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)}
 $source=[IO.File]::ReadAllText('C:\InputPack\runtime\provision-runtime.ps1')
 $tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
 if($errors.Count){throw 'Product provisioner parse failure'}
 $native=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-NativeCapture'},$true).Extent.Text
 $wrapper=@'
function Invoke-NativeCapture([string]$Executable,[string[]]$Arguments,[int]$TimeoutSeconds=60){
 $isInstaller=[IO.Path]::GetFileName($Executable) -eq 'python-3.12.10-amd64.exe'
 $actualArguments=$Arguments
 if($isInstaller){$actualArguments+=@('/log',('C:\Audit\installer-'+$global:caseName+'.log'))}
 $r=Invoke-AuditNativeCapture -Executable $Executable -Arguments $actualArguments -TimeoutSeconds $TimeoutSeconds
 if($isInstaller){$global:installerCalls.Add([pscustomobject]@{case=$global:caseName;arguments=$Arguments;exit=$r.ExitCode;timedOut=$r.TimedOut;output=$r.Output})}
 Add-Content -LiteralPath 'C:\Audit\native.log' -Encoding UTF8 -Value (($Arguments -join ' ') + "`nexit="+$r.ExitCode+"`n"+$r.Output)
 return $r
}
'@
 $instrumented=$source.Replace($native,($native.Replace('function Invoke-NativeCapture(','function Invoke-AuditNativeCapture(')+"`n"+$wrapper))
 $global:caseName='fresh'
 [IO.File]::WriteAllText((Join-Path $out 'case.txt'),$global:caseName)
 $raw=& ([scriptblock]::Create($instrumented)) -PackRoot 'C:\InputPack' -TargetUserProfile $env:USERPROFILE -TargetCodexHome (Join-Path $caseRoot 'codex-home') -ProgressFile (Join-Path $out 'stage.txt') | Out-String
 $state=$raw|ConvertFrom-Json
 Save-Json 'fresh-runtime.json' $state
 if($state.status -ne 'ready' -or $state.python.health -ne 'passed' -or $state.manim.health -ne 'passed' -or $state.hyperframes.health -ne 'passed'){throw 'Fresh offline runtime did not become ready'}
 if($installerCalls.Count -ne 1 -or $installerCalls[0].arguments -notcontains '/repair' -or $installerCalls[0].exit -notin @(0,3010)){throw 'Fresh native installer contract failed'}
 $rows.Add([pscustomobject]@{case='fresh';installerCalls=$installerCalls.Count;state=$state.status})
 Save-Json 'installer-calls-fresh.json' $installerCalls.ToArray()
 $baseExe=[string]$state.python.baseInterpreter;$baseRoot=Split-Path -Parent $baseExe
 $ownedRoot=Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312'
 if(-not $baseRoot.Equals($ownedRoot,[StringComparison]::OrdinalIgnoreCase)){throw 'Refusing mutation outside the newly installed Sandbox Python'}
 foreach($name in @('ConvertTo-WindowsArgument','Stop-NativeProcessTree','Test-ExactPython','Test-BasePython')){
  . ([scriptblock]::Create($ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true).Extent.Text))
 }
 . ([scriptblock]::Create($native.Replace('function Invoke-NativeCapture(','function Invoke-AuditNativeCapture(')))
 . ([scriptblock]::Create($wrapper))
 $start=$source.IndexOf("Set-InstallProgress 'runtime.python'");$end=$source.IndexOf('$venvRoot =',$start)
 $baseBlock=[scriptblock]::Create($source.Substring($start,$end-$start))
 function Set-InstallProgress {param($Stage) [IO.File]::WriteAllText('C:\Audit\stage.txt',$Stage)}
 $profileFull=$env:USERPROFILE;$runtimeRoot=[string]$state.paths.runtimeRoot
 $pythonInstaller='C:\InputPack\runtime\bundled\python-3.12.10-amd64.exe'
 # Full provisioning sets the user's CODEX_PYTHON to its venv. This case
 # intentionally tests discovery of the base target without either hint.
 [Environment]::SetEnvironmentVariable('CODEX_PYTHON',$null,'User')
 $env:CODEX_PYTHON=$null;$env:PATH=$env:SystemRoot+'\System32;'+$env:SystemRoot
 $sentinel=Join-Path $baseRoot 'healthy-sentinel.txt';[IO.File]::WriteAllText($sentinel,'keep')
 $originalExeTime=(Get-Item -LiteralPath $baseExe).LastWriteTimeUtc
 $global:caseName='healthy';$installerCalls.Clear();. $baseBlock
 if($installerCalls.Count -ne 0 -or $basePython -ne $baseExe -or -not(Test-Path -LiteralPath $sentinel) -or (Get-Item -LiteralPath $baseExe).LastWriteTimeUtc -ne $originalExeTime){throw 'Healthy Python was not reused'}
 $rows.Add([pscustomobject]@{case='healthy-not-on-path';installerCalls=0;sentinelPreserved=$true})
 foreach($missing in @('venv','ensurepip','pip-wheel')){
  $global:caseName=$missing;[IO.File]::WriteAllText((Join-Path $out 'case.txt'),$missing)
  $target=if($missing -eq 'pip-wheel'){
   $wheels=@(Get-ChildItem -LiteralPath (Join-Path $baseRoot 'Lib\ensurepip\_bundled') -Filter 'pip-*.whl' -File)
   if($wheels.Count -ne 1){throw 'Expected one bootstrap wheel'};$wheels[0].FullName
  }else{Join-Path $baseRoot ('Lib\'+$missing)}
  $resolved=(Resolve-Path -LiteralPath $target).Path
  if(-not $resolved.StartsWith($baseRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Fault target escapes isolated Python'}
  $renamedName=[IO.Path]::GetFileName($resolved)+'.interrupted'
  $renamed=Join-Path (Split-Path -Parent $resolved) $renamedName
  if(-not $renamed.StartsWith($baseRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $renamed)){throw 'Fault backup path invalid'}
  Rename-Item -LiteralPath $resolved -NewName $renamedName
  if(Test-BasePython $baseExe){throw "Incomplete base was accepted: $missing"}
  $installerCalls.Clear();. $baseBlock
  if($installerCalls.Count -ne 1 -or -not(Test-BasePython $baseExe)){throw "Repair did not restore $missing"}
  Save-Json ('installer-calls-'+$missing+'.json') $installerCalls.ToArray()
  $probeVenv=Join-Path $caseRoot ('venv-'+$missing)
  $created=Invoke-NativeCapture -Executable $baseExe -Arguments @('-m','venv','--clear',$probeVenv) -TimeoutSeconds 600
  if($created.ExitCode -ne 0){throw $created.Output}
  $pip=Invoke-NativeCapture -Executable (Join-Path $probeVenv 'Scripts\python.exe') -Arguments @('-m','pip','--version')
  if($pip.ExitCode -ne 0){throw $pip.Output}
  $rows.Add([pscustomobject]@{case=$missing;installerCalls=1;moduleRestored=$true;actualVenvAndPip='passed'})
 }
 $global:caseName='healthy-after-repair';$installerCalls.Clear();. $baseBlock
 if($installerCalls.Count -ne 0){throw 'Healthy repaired Python was reinstalled'}
 $rows.Add([pscustomobject]@{case=$global:caseName;installerCalls=0})
 $chartScript=Join-Path $caseRoot 'chart.py'
 [IO.File]::WriteAllText($chartScript,@'
import numpy as np,pandas as pd,matplotlib
matplotlib.use('Agg')
from matplotlib import pyplot as plt
from PIL import Image
frame=pd.DataFrame({'x':np.arange(4),'y':[1,4,2,5]})
plt.figure(figsize=(4,3));plt.plot(frame.x,frame.y);plt.savefig('C:/Audit/chart.png');plt.close()
with Image.open('C:/Audit/chart.png') as im:assert im.size==(400,300)
print('chart 400x300 passed')
'@,[Text.UTF8Encoding]::new($false))
 $chart=Invoke-NativeCapture -Executable ([string]$state.python.executable) -Arguments @($chartScript)
 if($chart.ExitCode -ne 0){throw $chart.Output}
 $scene=Join-Path $caseRoot 'scene.py'
 [IO.File]::WriteAllText($scene,"from manim import Scene,Square`nclass LocalSmoke(Scene):`n    def construct(self):`n        self.add(Square())`n        self.wait(0.5)`n",[Text.UTF8Encoding]::new($false))
 $render=Invoke-NativeCapture -Executable ([string]$state.manim.executable) -Arguments @('-m','manim','render','--silent','--disable_caching','--media_dir',(Join-Path $caseRoot 'media'),'--format','mp4','-r','320,180','--fps','4',$scene,'LocalSmoke') -TimeoutSeconds 120
 if($render.ExitCode -ne 0){throw $render.Output}
 $videos=@(Get-ChildItem -LiteralPath (Join-Path $caseRoot 'media') -Recurse -Filter 'LocalSmoke.mp4' -File)
 if($videos.Count -ne 1){throw 'Expected rendered Manim video'}
 Copy-Item -LiteralPath $videos[0].FullName -Destination (Join-Path $out 'manim.mp4')
 $probeScript=Join-Path $caseRoot 'video-probe.py'
 [IO.File]::WriteAllText($probeScript,"import av,json`nwith av.open('C:/Audit/manim.mp4') as video:`n    frames=list(video.decode(video=0))`n    assert len(frames)==2 and frames[0].width==320 and frames[0].height==180`n    print(json.dumps(dict(frames=len(frames),width=frames[0].width,height=frames[0].height)))`n",[Text.UTF8Encoding]::new($false))
 $videoProbe=Invoke-NativeCapture -Executable ([string]$state.manim.executable) -Arguments @($probeScript)
 if($videoProbe.ExitCode -ne 0){throw $videoProbe.Output}
 Save-Json 'result.json' @{status='passed';seconds=$timer.Elapsed.TotalSeconds;networkInterfaces=$networkCount;results=$rows;chartBytes=(Get-Item 'C:\Audit\chart.png').Length;videoBytes=(Get-Item 'C:\Audit\manim.mp4').Length;videoProbe=($videoProbe.StdOut|ConvertFrom-Json);scope='fresh Windows Sandbox with Networking Disable; actual bundled installer and full local runtime provisioning; paid calls 0'}
}catch{
 Save-Json 'failure.json' @{status='failed';case=$global:caseName;error=$_.Exception.Message;position=$_.InvocationInfo.PositionMessage;results=$rows;installerCalls=$installerCalls.ToArray()}
 $ci=@(Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-CodeIntegrity/Operational';StartTime=$started} -ErrorAction SilentlyContinue | ForEach-Object {
  $eventXml=[xml]$_.ToXml();$fields=[ordered]@{}
  foreach($data in $eventXml.Event.EventData.Data){if($data.Name -in @('FileName','ProcessName','PolicyName','Status','RequestedSigningLevel','ValidatedSigningLevel')){$fields[$data.Name]=$data.InnerText}}
  [pscustomobject]@{id=$_.Id;time=$_.TimeCreated.ToUniversalTime().ToString('o');fields=$fields}
 })
 Save-Json 'code-integrity.json' $ci
 $policies=& "$env:SystemRoot\System32\citool.exe" --list-policies -json | ConvertFrom-Json
 Save-Json 'policy-names.json' @($policies.Policies|Select-Object FriendlyName,IsEnforced,IsAuthorized,IsOnDisk)
}finally{
 [IO.File]::WriteAllText('C:\Audit\finished.txt',[DateTime]::UtcNow.ToString('o'))
 Stop-Transcript | Out-Null
 # Only the disposable Sandbox is shut down; identity was checked above.
 & "$env:SystemRoot\System32\shutdown.exe" /s /t 0
}
