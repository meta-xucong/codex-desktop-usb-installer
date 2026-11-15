$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$source=[IO.File]::ReadAllText((Join-Path $repo 'installer-pack/runtime/provision-runtime.ps1'))
$tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Provisioner parse failed'}
foreach($name in @('Test-ExactPython','Test-BasePython')){
 . ([scriptblock]::Create($ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true).Extent.Text))
}
$start=$source.IndexOf("Set-InstallProgress 'runtime.python'");$end=$source.IndexOf('$venvRoot =',$start)
$block=[scriptblock]::Create($source.Substring($start,$end-$start))
$fixture=Join-Path $repo ('tmp/python-base-branch-'+[guid]::NewGuid().ToString('N'))
function Set-InstallProgress {param($Stage)}
function Get-Command {param($Name,$ErrorAction) throw 'No Python on test PATH'}
function Invoke-NativeCapture {
 param($Executable,[string[]]$Arguments,$TimeoutSeconds=60)
 if($Executable -eq $pythonInstaller){
  $script:installerCalls++;$script:installerArguments=$Arguments
  if($case -ne 'repair-failed'){
   New-Item -ItemType Directory -Force -Path (Split-Path -Parent $expectedBase)|Out-Null
   [IO.File]::WriteAllText($expectedBase,'fixture interpreter');$script:healthy[$expectedBase]=$true
  }
  return [pscustomobject]@{ExitCode=0;StdOut='';Output='';TimedOut=$false}
 }
 if($Arguments[0] -ne '-c'){throw 'Unexpected native command'}
 if($Arguments[1] -match '^import platform,sys,sysconfig'){
  return [pscustomobject]@{ExitCode=0;StdOut='3.12.10|cpython-312|win-amd64|AMD64';Output=''}
 }
 return [pscustomobject]@{ExitCode=$(if($script:healthy[$Executable]){0}else{1});StdOut='';Output='base dependencies'}
}
$savedPython=$env:CODEX_PYTHON
try{
 foreach($case in @('first','missing-venv','missing-ensurepip','missing-wheel','healthy-target','healthy-explicit','repair-explicit','repair-failed')){
  $profileFull=Join-Path $fixture ($case+' user');$runtimeRoot=Join-Path $profileFull 'runtime'
  $targetBase=Join-Path $profileFull 'AppData/Local/Programs/Python/Python312/python.exe'
  $explicitBase=Join-Path $profileFull 'registered-custom/python.exe'
  $expectedBase=if($case -in @('healthy-explicit','repair-explicit')){$explicitBase}else{$targetBase}
  $env:CODEX_PYTHON=if($case -in @('healthy-explicit','repair-explicit')){$explicitBase}else{$null}
  $pythonInstaller=Join-Path $fixture 'bundled-python-installer.exe'
  $script:healthy=@{};$script:installerCalls=0;$script:installerArguments=@()
  if($case -ne 'first'){
   New-Item -ItemType Directory -Force -Path (Split-Path -Parent $expectedBase)|Out-Null
   [IO.File]::WriteAllText($expectedBase,'fixture interpreter')
   $script:healthy[$expectedBase]=$case -like 'healthy-*'
  }
  $failure='';try{. $block}catch{$failure=$_.Exception.Message}
  $expectedCalls=if($case -like 'healthy-*'){0}else{1}
  if($installerCalls -ne $expectedCalls){throw "Unexpected installer calls: $case/$installerCalls"}
  if($installerCalls -and ($installerArguments -notcontains '/repair' -or $installerArguments -notcontains '/quiet' -or $installerArguments -notcontains 'PrependPath=0')){throw 'Repair changed the existing installation contract'}
  if($installerCalls -and $installerArguments[-1] -cne ('TargetDir='+ (Split-Path -Parent $targetBase))){throw 'TargetDir must be an unquoted argument value, including paths with spaces'}
  if($case -eq 'repair-failed'){
   if($failure -notmatch 'no healthy CPython'){throw 'Unhealthy repair was accepted'}
  } elseif($failure -or $basePython -ne $expectedBase){throw "Base selection failed: $case / $failure"}
  Write-Output "Base source branch passed: $case; installerCalls=$installerCalls"
 }
}finally{$env:CODEX_PYTHON=$savedPython}
