$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$source=[IO.File]::ReadAllText((Join-Path $repo 'installer-pack/runtime/provision-runtime.ps1'))
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Provisioner parse failed'}
. ([scriptblock]::Create($ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Test-VenvPython'},$true).Extent.Text))
$start=$source.IndexOf('$venvRoot =');$split=$source.IndexOf("Set-InstallProgress 'runtime.manim'",$start);$end=$source.IndexOf("Set-InstallProgress 'runtime.nodeMedia'",$split)
$blocks=@{python=[scriptblock]::Create($source.Substring($start,$split-$start));manim=[scriptblock]::Create($source.Substring($split,$end-$split))}
$fixture=Join-Path $repo ('tmp/python-branch-'+[guid]::NewGuid().ToString('N'))
$basePython='fixture-base-python';$wheelDir='fixture-local-wheels';$lockPath='fixture-python.lock';$manimLockPath='fixture-manim.lock'
function Set-InstallProgress {param($Stage)}
function Test-ExactPython {param($Executable) return Test-Path -LiteralPath $Executable -PathType Leaf}
function Invoke-NativeCapture {
 param($Executable,[string[]]$Arguments,$TimeoutSeconds=60)
 $script:calls.Add(($Arguments -join ' '))
 if($Arguments[0] -eq '-m' -and $Arguments[1] -eq 'venv'){
  $script:clears++;$script:pipOk=$true;$script:importsOk=$false
  $exe=Join-Path $Arguments[-1] 'Scripts/python.exe';New-Item -ItemType Directory -Force -Path (Split-Path -Parent $exe)|Out-Null
  [IO.File]::WriteAllText($exe,'fixture interpreter')
 } elseif($Arguments[0] -eq '-m' -and $Arguments[1] -eq 'pip' -and $Arguments[2] -eq '--version'){
  return [pscustomobject]@{ExitCode=$(if($script:pipOk){0}else{1});StdOut=$(if($script:pipOk){'pip 25.0.1'}else{''});Output='pip probe'}
 } elseif($Arguments[0] -eq '-m' -and $Arguments[1] -eq 'pip' -and $Arguments[2] -eq 'install'){
  $script:installs++;$script:importsOk=$true
 } elseif($Arguments[0] -eq '-c'){
  return [pscustomobject]@{ExitCode=$(if($script:importsOk){0}else{1});StdOut=$(if($component -eq 'manim'){'0.21.0'}else{''});Output='import probe'}
 } else {throw 'Unexpected native invocation'}
 return [pscustomobject]@{ExitCode=0;StdOut='';Output=''}
}
foreach($component in @('python','manim')){
 foreach($case in @('cold','pip-incomplete','metadata-only','healthy')){
  $runtimeRoot=Join-Path $fixture ($component+'-'+$case)
  $exe=Join-Path $runtimeRoot ($component+'-env/Scripts/python.exe')
  if($case -ne 'cold'){New-Item -ItemType Directory -Force -Path (Split-Path -Parent $exe)|Out-Null;[IO.File]::WriteAllText($exe,'existing fixture')}
  $script:calls=[Collections.Generic.List[string]]::new();$script:clears=0;$script:installs=0
  $script:pipOk=$case -ne 'pip-incomplete';$script:importsOk=$case -eq 'healthy'
  . $blocks[$component]
  $expectedClears=if($case -eq 'healthy'){0}else{1}
  if($clears -ne $expectedClears -or $installs -ne 1){throw "Incorrect recovery flow: $component/$case"}
  $pipCall=@($calls|Where-Object {$_ -like '-m pip install*'})[0]
  if($pipCall -notlike '*--no-index*--find-links*--require-hashes*'){throw 'Original offline/integrity options changed'}
  if(@($calls|Where-Object {$_ -like '-c *'}).Count -lt 1){throw 'Post-install import check missing'}
  Write-Output "Source branch passed: $component/$case; rebuild=$clears; install=$installs"
 }
}
