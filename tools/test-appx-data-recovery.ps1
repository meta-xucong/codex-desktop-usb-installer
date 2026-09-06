# Executes real journal/manifest/hash/restore code against disposable fixtures.
# All deployment/context callbacks are replaced before the recovery entry runs.
param([string]$ScriptPath = (Join-Path $PSScriptRoot '..\crates\codex-win-engine\src\appx_data_recovery.ps1'))
$ErrorActionPreference = 'Stop'
. (Join-Path (Split-Path -Parent $ScriptPath) 'appx_package_path.ps1')
. $ScriptPath
Add-Type -AssemblyName System.IO.Compression -ErrorAction Stop
$script:CaseCount = 0
$script:RealJournalWriter = (Get-Command Write-CodexRecoveryJournal).ScriptBlock
$script:RealStage = (Get-Command Invoke-CodexRecoveryStage).ScriptBlock
$script:RealStagedQuery = (Get-Command Get-CodexRecoveryStagedPackage).ScriptBlock
$script:RealRegister = (Get-Command Invoke-CodexRecoveryRegister).ScriptBlock
$script:RealTaskAdapter = (Get-Command ConvertTo-CodexRecoveryDeploymentTask).ScriptBlock
$script:RealRegistrationOptions = (Get-Command New-CodexRecoveryRegistrationOptions).ScriptBlock
$script:RealUsbFreeBytes = (Get-Command Get-CodexRecoveryUsbFreeBytes).ScriptBlock

function Assert-Test($Condition, [string]$Message) { if (-not $Condition) { throw ('TEST FAILED: ' + $Message) } }
function Write-FixtureFile([string]$Relative, [string]$Text) {
  $path = Join-Path $script:Fixture.Context.dataRoot $Relative
  [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
  [IO.File]::WriteAllText($path, $Text, [Text.Encoding]::UTF8)
}
function Remove-FixtureTree([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path)
  if (-not $full.StartsWith($script:Fixture.Root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture deletion escaped its explicit unique root' }
  if (Test-Path -LiteralPath $full) {
    foreach ($entry in @(Get-ChildItem -LiteralPath $full -Force -Recurse -ErrorAction Stop)) {
      if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Fixture deletion refuses redirected descendants' }
    }
    [IO.Directory]::Delete($full, $true)
  }
}
function Get-CodexRecoveryContext([string]$UsbRoot, $UsbVolume) {
  if ($script:Fixture.FailContext) { throw 'fixture recovery context unavailable' }
  return $script:Fixture.Context
}
function Get-CodexRecoveryCurrentUsbVolume($Context) { return [pscustomobject]@{Name=$Context.usbName} }
function Get-CodexSystemPackageStore { return Join-Path $script:Fixture.Root 'system\WindowsApps' }
function Set-FixtureSystemPackageAlias($Package) {
  $target=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $script:Fixture.FullName
  $store=Get-CodexSystemPackageStore
  [IO.Directory]::CreateDirectory($target)|Out-Null; [IO.Directory]::CreateDirectory($store)|Out-Null
  $link=Join-Path $store $script:Fixture.FullName
  New-Item -ItemType Junction -Path $link -Target $target -ErrorAction Stop|Out-Null
  if ($Package) { $Package.InstallLocation=$link }
  return @{link=$link;target=$target;store=(Split-Path -Parent $target)}
}
function Get-CodexRecoveryUsbFreeBytes($Context) { return [uint64]$script:Fixture.FreeBytes }
function Invoke-CodexRecoveryStage([string]$PackagePath, $Context) {
  if ($script:Fixture.NativeBoundary) { return & $script:RealStage $PackagePath $Context }
  $Context.stageInvokedThisRun=$true
  $script:Fixture.StageCalls++
  $script:Fixture.Staged = [pscustomobject]@{ PackageFullName=$script:Fixture.FullName; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0'; InstallLocation=Join-Path (Join-Path $Context.usbRoot 'WindowsApps') $script:Fixture.FullName }
  if ($script:Fixture.WrongStagedVolume) { $script:Fixture.Staged.InstallLocation=Join-Path $script:Fixture.Root 'system-WindowsApps\wrong-volume' }
  if ($script:Fixture.SystemPackageAlias) { Set-FixtureSystemPackageAlias $script:Fixture.Staged | Out-Null }
  if ($script:Fixture.FailStage) { throw 'simulated Stage completion interruption' }
  return @{ activityId='fixture-stage'; operation='mock-stage' }
}
function Get-CodexRecoveryStagedPackage([string]$FullName, $Context) {
  if ($script:Fixture.NativeBoundary) { return & $script:RealStagedQuery $FullName $Context }
  return $script:Fixture.Staged
}
function Invoke-CodexRecoveryRegister([string]$FullName, $Context) {
  if ($script:Fixture.NativeBoundary) { return & $script:RealRegister $FullName $Context }
  $Context.registerInvokedThisRun=$true
  $script:Fixture.RegisterCalls++
  $script:Fixture.Current = $script:Fixture.Staged
  Write-FixtureFile 'Settings\settings.dat' 'windows-default'
  if ($script:Fixture.CustomParentAcl) {
    $settings=Join-Path $Context.dataRoot 'Settings'
    $acl=[IO.Directory]::GetAccessControl($settings)
    $sid=New-Object Security.Principal.SecurityIdentifier('S-1-5-32-555')
    $rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,[Security.AccessControl.FileSystemRights]::ReadAttributes,
      ([Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit),
      [Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Allow)
    $acl.AddAccessRule($rule); [IO.Directory]::SetAccessControl($settings,$acl)
  }
  if ($script:Fixture.UpdateNeedsRepair -and $script:Fixture.RegisterCalls -eq 1) { Write-FixtureFile 'Settings\settings.dat' 'upgraded-user-content' }
  if ($script:Fixture.PreBaselineUserWrite) { Write-FixtureFile 'Settings\settings.dat' 'new-user-before-baseline' }
  if ($script:Fixture.FailRegister) { throw 'simulated Register completion interruption' }
  return @{ activityId='fixture-register'; operation='mock-register-system-data' }
}

# Exercise the production Stage/query/Register/Wait wrappers. Only native OS
# endpoints and the task adapter are replaced here; tasks really delay/fault/
# cancel, and no fixture can reach a real deployment entry point.
Add-Type -TypeDefinition @'
using System; using System.Threading; using System.Threading.Tasks; using System.Runtime.InteropServices;
namespace CodexRecoveryAsyncFixture {
 public static class Tasks {
  public static async Task<object> Start(string mode, int delay, object result) {
   await Task.Delay(delay);
   if(mode=="fault") throw new COMException("fixture deployment error ActivityId=9afe3240-101d-0002-37b9-8aa11d10dd01", unchecked((int)0x80073CF0));
   if(mode=="cancel") throw new OperationCanceledException("fixture deployment cancelled");
   return result;
  }
 }
}
'@ -ErrorAction Stop | Out-Null

function Complete-FixtureNativeOperations {
  if (-not $script:Fixture.NativeBoundary) { return }
  if ($script:Fixture.StageTask -and $script:Fixture.StageTask.Status -eq 'RanToCompletion' -and $script:Fixture.StageTask.Result -and -not $script:Fixture.StageTask.Result.ExtendedErrorCode -and -not $script:Fixture.StageProjected) {
    $script:Fixture.StageProjected=$true
    if (-not $script:Fixture.EmptyAfterStage) {
      $script:Fixture.Staged=[pscustomobject]@{PackageFullName=$script:Fixture.FullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $script:Fixture.FullName}
      if ($script:Fixture.WrongStagedVolume) { $script:Fixture.Staged.InstallLocation=Join-Path $script:Fixture.Root 'system-WindowsApps\wrong-volume' }
      if ($script:Fixture.SystemPackageAlias) { Set-FixtureSystemPackageAlias $script:Fixture.Staged | Out-Null }
    }
  }
  if ($script:Fixture.RegisterTask -and $script:Fixture.RegisterTask.Status -eq 'RanToCompletion' -and $script:Fixture.RegisterTask.Result -and -not $script:Fixture.RegisterTask.Result.ExtendedErrorCode -and -not $script:Fixture.RegisterProjected) {
    $script:Fixture.RegisterProjectionQueries++
    if ($script:Fixture.RegisterProjectionQueries -gt $script:Fixture.RegisterProjectionDelayQueries) {
      $script:Fixture.RegisterProjected=$true; $script:Fixture.Current=$script:Fixture.Staged
      Write-FixtureFile 'Settings\settings.dat' 'windows-default'
    }
  }
}
function Convert-FixtureNativePackage($Package) {
  if ($null -eq $Package) { return $null }
  $status=New-Object PSObject
  $status|Add-Member -MemberType ScriptMethod -Name VerifyIsOK -Value { return -not $script:Fixture.UnhealthyStaged }
  return [pscustomobject]@{Id=[pscustomobject]@{FullName=$Package.PackageFullName;FamilyName=$Package.PackageFamilyName};Status=$status;InstalledLocation=[pscustomobject]@{Path=$Package.InstallLocation}}
}
function Get-CodexRecoveryNativeVolumes($Context) {
  if (-not $script:Fixture.NativeBoundary) { throw 'Unexpected native boundary call; real deployment is forbidden in fixtures' }
  $manager=New-Object PSObject; $usb=New-Object PSObject; $system=[pscustomobject]@{Name='fixture-native-system'}
  $usb|Add-Member -MemberType ScriptMethod -Name FindPackages -Value {
    param($family)
    Assert-Test ($family -eq 'OpenAI.Codex_2p2nqsd0c76g0') 'USB query must be scoped to exact PFN'
    $script:Fixture.UsbQueries++
    if ($script:Fixture.FailUsbQuery) { throw 'fixture target-volume query denied' }
    Complete-FixtureNativeOperations
    return @(Convert-FixtureNativePackage $script:Fixture.Staged)
  }
  $manager|Add-Member -MemberType ScriptMethod -Name FindPackages -Value {
    param($family)
    Assert-Test ($family -eq 'OpenAI.Codex_2p2nqsd0c76g0') 'diagnostic global query must be scoped to exact PFN'
    $script:Fixture.GlobalQueries++
    return @(Convert-FixtureNativePackage $script:Fixture.GlobalStaged)
  }
  $manager|Add-Member -MemberType ScriptMethod -Name StagePackageAsync -Value {
    param($uri,$dependencies,$options,$volume)
    Assert-Test ($uri.IsFile -and $uri.LocalPath -eq $script:Fixture.Payload -and $null -ne $volume) 'Stage receives exact local payload and bound USB'
    $script:Fixture.StageCalls++
    return [pscustomobject]@{kind='stage';mode=$script:Fixture.StageMode;Status='Started';ErrorCode=$null}
  }
  $manager|Add-Member -MemberType ScriptMethod -Name RegisterPackagesByFullNameAsync -Value {
    param($fullNames,$options)
    Assert-Test (@($fullNames).Count -eq 1 -and [string]$fullNames[0] -eq $script:Fixture.FullName -and $options.AppDataVolume.Name -eq 'fixture-native-system') 'Register receives exact full name and system data volume'
    $script:Fixture.RegisterCalls++
    return [pscustomobject]@{kind='register';mode=$script:Fixture.RegisterMode;Status='Started';ErrorCode=$null}
  }
  return @{manager=$manager;usb=$usb;system=$system}
}
function New-CodexRecoveryRegistrationOptions($Native) {
  if ($script:Fixture -and $script:Fixture.NativeBoundary) { return [pscustomobject]@{AppDataVolume=$Native.system} }
  return & $script:RealRegistrationOptions $Native
}
function ConvertTo-CodexRecoveryDeploymentTask($Operation) {
  if (-not ($script:Fixture -and $script:Fixture.NativeBoundary)) { return & $script:RealTaskAdapter $Operation }
  $errorCode = if ($Operation.mode -eq 'result-error') { New-Object Runtime.InteropServices.COMException('fixture extended error', -2147009296) } else { $null }
  $result=[pscustomobject]@{ActivityId=[guid]'9afe3240-101d-0002-37b9-8aa11d10dd01';ExtendedErrorCode=$errorCode;ErrorText='fixture extended text';IsRegistered=($Operation.mode -ne 'false-hint')}
  if ($Operation.mode -eq 'null-result') { $result=$null }
  $task=[CodexRecoveryAsyncFixture.Tasks]::Start([string]$Operation.mode,200,$result)
  if ($Operation.kind -eq 'stage') { $script:Fixture.StageTask=$task; $script:Fixture.StageProjected=$false }
  else { $script:Fixture.RegisterTask=$task; $script:Fixture.RegisterProjected=$false }
  return $task
}
function Write-CodexRecoveryJournal($Journal, $Context) {
  if ($script:Fixture.FailCompletedStageJournal -and $Journal.lastDeployment -and $Journal.lastDeployment.operation -eq 'StagePackageAsync USB' -and $Journal.lastDeployment.state -eq 'completed') {
    $script:Fixture.FailCompletedStageJournal=$false
    throw 'fixture completed Stage journal write denied'
  }
  & $script:RealJournalWriter $Journal $Context
  if ($Journal.lastDeployment) { $script:Fixture.DeploymentRecords.Add(($Journal.lastDeployment|ConvertTo-Json -Depth 8|ConvertFrom-Json)) }
  if ($script:Fixture.FailDuringGap -and $Journal.pendingFile -and $Journal.phase -eq 'restoring' -and
      -not [CodexAppxRecovery.NativeFiles]::Exists((Join-Path $Context.dataRoot $Journal.pendingFile.path), $false) -and
      [CodexAppxRecovery.NativeFiles]::Exists((Join-Path (Join-Path $Context.backupRoot ('recovery-files-' + $Journal.id)) ($Journal.pendingFile.artifact + '.replaced')), $false)) {
    $script:Fixture.FailDuringGap=$false; throw 'simulated interruption between no-replace renames'
  }
  if ($script:Fixture.ChangeVolumeBeforeRemove -and $Journal.phase -eq 'prepared') {
    $script:Fixture.Context.usbId='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'; $script:Fixture.ChangeVolumeBeforeRemove=$false
  }
  if ($script:Fixture.FailAfterRestoredFile -and $Journal.phase -eq 'restoring' -and
      $null -eq $Journal.pendingFile -and [IO.File]::Exists((Join-Path $Context.dataRoot 'LocalState\conversation.json')) -and
      [IO.File]::ReadAllText((Join-Path $Context.dataRoot 'LocalState\conversation.json')) -eq 'original-conversation') {
    $script:Fixture.FailAfterRestoredFile = $false
    throw 'simulated interrupted restore after one file'
  }
}
function New-FixtureBackup($Package, [string]$Root, [string]$Name) {
  $script:Fixture.BackupCalls++
  $source = $script:Fixture.Context.dataRoot
  if (-not [IO.Directory]::Exists($source)) { return $null }
  $backup = Join-Path $script:Fixture.Context.backupRoot ([guid]::NewGuid().ToString('N'))
  [IO.Directory]::CreateDirectory($backup) | Out-Null
  $files = @(); $directories = @{}; $volumes = @{}
  foreach ($entry in @(Get-ChildItem -LiteralPath $source -Force -Recurse)) {
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Fixture backup does not traverse a link' }
    $relative = $entry.FullName.Substring($source.Length + 1)
    $destination = Join-Path $backup $relative
    if ($entry.PSIsContainer) {
      [IO.Directory]::CreateDirectory($destination) | Out-Null
      $directories[$relative]=''; $volumes[$relative]=''
    } else {
      [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
      [IO.File]::Copy($entry.FullName, $destination, $false)
      $files += @{ path=$relative; sha256=Get-CodexRecoveryHash $destination }
    }
  }
  if ($script:Fixture.TraversalManifest) { $files += @{path='..\escape.txt';sha256=('a' * 64)} }
  $manifest = @{source=$source;packageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';files=@($files);directories=$directories;appxVolumes=$volumes}
  [IO.File]::WriteAllText(($backup + '.manifest.json'), ($manifest | ConvertTo-Json -Depth 8), [Text.Encoding]::UTF8)
  $script:Fixture.Backups.Add($backup)
  if ($script:Fixture.ChangeAfterBaseline -and $script:Fixture.BackupCalls -eq 2) { Write-FixtureFile 'Settings\settings.dat' 'new-user-after-baseline' }
  return $backup
}
function Invoke-FixtureRecovery {
  $callbacks = @{
    GetPackage={ Complete-FixtureNativeOperations; return $script:Fixture.Current }
    AssertOwnership={ param($package) if ($script:Fixture.Shared) { throw 'shared package was not authorized' } }
    StopPackage={ param($package) $script:Fixture.StopCalls++ }
    BackupPackage={ param($package,$root,$name) New-FixtureBackup $package $root $name }
    RemovePackage={ param($package,$backup)
      $script:Fixture.RemoveCalls++
      if ($script:Fixture.FailRemoveBefore) { throw 'simulated pre-removal failure' }
      Assert-Test ($package.PackageFullName -eq $script:Fixture.FullName) 'remove must use exact package'
      if ($backup) { Assert-Test (Test-Path -LiteralPath ($backup + '.manifest.json')) 'backup must exist before remove' }
      $script:Fixture.Current=$null; $script:Fixture.Staged=$null
      Remove-FixtureTree $script:Fixture.Context.dataRoot
      if ($script:Fixture.FailRemoveAfter) { throw 'simulated interruption after remove' }
    }
    AssertDataPlacement={param($root)
      if ($script:Fixture.UpdateNeedsRepair -and $script:Fixture.RegisterCalls -eq 1) { return @{state='redirected-unverified'} }
      return @{state=$(if([IO.Directory]::Exists($script:Fixture.Context.dataRoot)){'system-volume-observed'}else{'not-created'})} }
  }
  return Invoke-CodexDataRecovery -PackagePath $script:Fixture.Payload -ExpectedFullName $script:Fixture.FullName -UsbRoot $script:Fixture.Context.usbRoot -UsbVolume ([pscustomobject]@{Name='unused mock volume'}) -Callbacks $callbacks -NonDestructiveUpdate:$script:Fixture.Update
}
function New-Fixture([bool]$Existing) {
  $root = Join-Path ([IO.Path]::GetTempPath()) ('codex-data-recovery-test-' + [guid]::NewGuid().ToString('N'))
  [IO.Directory]::CreateDirectory($root) | Out-Null
  $local = Join-Path $root "profile Chinese 空格 'quote\Local"
  $usb = Join-Path $root 'usb'
  $full = 'OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0'
  $context = @{sid='S-1-5-21-111-222-333-1001';usbId='11111111-1111-4111-8111-111111111111';usbName='\\?\Volume{11111111-1111-4111-8111-111111111111}\';usbRoot=$usb;systemId='22222222-2222-4222-8222-222222222222';systemName='fixture-system';systemRoot=[IO.Path]::GetPathRoot($root);dataRoot=Join-Path $local 'Packages\OpenAI.Codex_2p2nqsd0c76g0';backupRoot=Join-Path $local 'CodexAppxDataBackups\OpenAI.Codex_2p2nqsd0c76g0'}
  [IO.Directory]::CreateDirectory($local) | Out-Null; [IO.Directory]::CreateDirectory($usb) | Out-Null
  $payload = Join-Path $root 'verified # fixture %.msix'
  $stream = [IO.File]::Open($payload, [IO.FileMode]::CreateNew)
  $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false)
  try {
    $entry = $archive.CreateEntry('app.txt').Open()
    try { $bytes = [Text.Encoding]::UTF8.GetBytes('fixture signed-payload stand-in'); $entry.Write($bytes, 0, $bytes.Length) }
    finally { $entry.Dispose() }
  } finally { $archive.Dispose(); $stream.Dispose() }
  $script:Fixture = @{Root=$root;Context=$context;Payload=$payload;FullName=$full;Current=$null;Staged=$null;StageCalls=0;RegisterCalls=0;RegisterProjectionQueries=0;RegisterProjectionDelayQueries=0;RemoveCalls=0;BackupCalls=0;StopCalls=0;UsbQueries=0;GlobalQueries=0;Backups=(New-Object 'System.Collections.Generic.List[string]');DeploymentRecords=(New-Object 'System.Collections.Generic.List[object]');Shared=$false;FailContext=$false;FailStage=$false;FailRegister=$false;FailRemoveBefore=$false;FailRemoveAfter=$false;FailCompletedStageJournal=$false;TraversalManifest=$false;PreBaselineUserWrite=$false;ChangeAfterBaseline=$false;FailAfterRestoredFile=$false;LastFailure=''}
  $script:CodexRecoveryRegistrationPollAttempts=240
  $script:CodexRecoveryRegistrationPollDelayMs=1
  $script:Fixture.FreeBytes = 100GB
  if ($Existing) {
    $script:Fixture.Current=[pscustomobject]@{PackageFullName=$full;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path $root 'old-program'}
    Write-FixtureFile 'Settings\settings.dat' 'original-settings'
    Write-FixtureFile 'LocalState\conversation.json' 'original-conversation'
  }
}
function Expect-FixtureFailure([scriptblock]$Action, [string]$Pattern) {
  $failed=$false
  try { & $Action | Out-Null } catch { $failed=$true; $script:Fixture.LastFailure=$_.Exception.Message; Assert-Test ($_.Exception.Message -match $Pattern) ('unexpected error: ' + $_.Exception.Message) }
  Assert-Test $failed ('expected failure matching ' + $Pattern)
}
function Run-Case([string]$Name,[scriptblock]$Body) {
  $script:Fixture=$null
  try { & $Body; $script:CaseCount++; Write-Output ('PASS ' + $Name) }
  finally {
    if ($script:Fixture -and (Test-Path -LiteralPath $script:Fixture.Root)) {
      # The unique fixture root is created above, never a user/workspace root.
      $resolved=[IO.Path]::GetFullPath($script:Fixture.Root)
      $tempPrefix=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\'
      if (-not $resolved.StartsWith($tempPrefix,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notmatch '^codex-data-recovery-test-[a-f0-9]{32}$') { throw 'Unsafe fixture cleanup root' }
      Initialize-CodexRecoveryFileHandles
      $cleanupQueue=New-Object 'System.Collections.Generic.Queue[string]'; $cleanupQueue.Enqueue($resolved)
      while($cleanupQueue.Count) {
        $cleanupDir=$cleanupQueue.Dequeue()
        foreach($entry in @([CodexAppxRecovery.NativeFiles]::Entries($cleanupDir))) {
          $child=Join-Path $cleanupDir $entry.Name
          if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { [IO.Directory]::Delete(('\\?\' + $child),$false) }
          elseif($entry.PSIsContainer) {$cleanupQueue.Enqueue($child)}
        }
      }
      [IO.Directory]::Delete(('\\?\' + $resolved),$true)
    }
  }
}

function Read-FixtureJournal {
  return [IO.File]::ReadAllText((Join-Path $script:Fixture.Context.backupRoot 'recovery.json'))|ConvertFrom-Json
}
Run-Case 'fresh Stage USB then exact Register system data, no Remove' {
  New-Fixture $false; $result=Invoke-FixtureRecovery
  Assert-Test ($result -isnot [array]) 'recovery emits exactly one public result object'
  Assert-Test $result.success 'fresh must succeed'
  Assert-Test ($script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'fresh operation counts'
  Assert-Test (-not(Test-Path -LiteralPath (Join-Path $script:Fixture.Context.backupRoot 'recovery.json'))) 'active journal archived'
}
Run-Case 'real junction at system package path resolves to exact USB bytes' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.SystemPackageAlias=$true
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'native Stage/Register adapters accept verified alias without wrong-volume rollback'
  $link=$script:Fixture.Current.InstallLocation
  $info=Get-CodexPackagePathInfo $link $script:Fixture.FullName (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps')
  Assert-Test ($info.state -eq 'verified-system-junction' -and $info.effectivePath -eq $script:Fixture.Staged.InstallLocation.Replace((Get-CodexSystemPackageStore),(Join-Path $script:Fixture.Context.usbRoot 'WindowsApps'))) 'effective location is USB, original alias retained'
  Assert-Test ((Get-CodexSystemPackageEntryAudit $script:Fixture.FullName).status -eq 'pending') 'existing system entry is not reported removed'
}
Run-Case 'dangling real junction is diagnosed and Stage failure retains evidence' {
  New-Fixture $false; $alias=Set-FixtureSystemPackageAlias $null
  Remove-FixtureTree $alias.target
  $info=Get-CodexPackagePathInfo $alias.link $script:Fixture.FullName $alias.store
  Assert-Test ($info.state -eq 'dangling-system-junction' -and $null -eq $info.effectivePath) 'dangling target is not installed USB bytes'
  $script:Fixture.FailStage=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'simulated Stage completion interruption'
  $journal=Read-FixtureJournal
  Assert-Test ($journal.systemEntryBeforeStage.state -eq 'dangling-system-junction' -and $script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 0) 'pre-Stage exact alias evidence saved; no blind deletion/register'
  Assert-Test ((Get-CodexSystemPackageEntryAudit $script:Fixture.FullName).status -eq 'pending') 'dangling link is present despite absent target'
}
Run-Case 'wrong package target and chained junction are rejected' {
  New-Fixture $false; $alias=Set-FixtureSystemPackageAlias $null
  $wrong=Join-Path $alias.store 'unrelated'; [IO.Directory]::CreateDirectory($wrong)|Out-Null
  [IO.Directory]::Delete($alias.link,$false)
  New-Item -ItemType Junction -Path $alias.link -Target $wrong | Out-Null
  $info=Get-CodexPackagePathInfo $alias.link $script:Fixture.FullName $alias.store
  Assert-Test ($null -eq $info.effectivePath -and $info.state -eq 'rejected-system-link') 'sibling target not accepted'
  [IO.Directory]::Delete($alias.link,$false); [IO.Directory]::Delete($alias.target,$false)
  New-Item -ItemType Junction -Path $alias.target -Target $wrong | Out-Null
  New-Item -ItemType Junction -Path $alias.link -Target $alias.target | Out-Null
  $info=Get-CodexPackagePathInfo $alias.link $script:Fixture.FullName $alias.store
  Assert-Test ($null -eq $info.effectivePath -and $info.state -eq 'rejected-system-link') 'second-hop target not accepted'
}
Run-Case 'verified alias and physical process paths match but siblings never do' {
  New-Fixture $false; $alias=Set-FixtureSystemPackageAlias $null
  $roots=@(Get-CodexPackageProcessRoots $alias.link $script:Fixture.FullName $alias.store)
  Assert-Test ($roots.Count -eq 2) 'exact alias and target roots returned'
  foreach ($root in $roots) {
    Assert-Test (Test-CodexProcessPathUnderRoots (Join-Path $root 'app\ChatGPT.exe') $roots) 'registered alias or physical process is live'
    Assert-Test (Test-CodexProcessPathUnderRoots ('\\?\'+(Join-Path $root 'app\Codex.exe')) $roots) 'extended executable path recognized'
    Assert-Test (-not(Test-CodexProcessPathUnderRoots ($root+'-unrelated\Codex.exe') $roots)) 'sibling process excluded'
  }
  Remove-FixtureTree $alias.target
  Expect-FixtureFailure {Get-CodexPackageProcessRoots $alias.link $script:Fixture.FullName $alias.store} 'Cannot resolve exact package process roots'
}
Run-Case 'ordinary system package directory remains a real wrong-volume error' {
  New-Fixture $false; $path=Join-Path (Get-CodexSystemPackageStore) $script:Fixture.FullName
  [IO.Directory]::CreateDirectory($path)|Out-Null
  $package=[pscustomobject]@{PackageFullName=$script:Fixture.FullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=$path}
  Expect-FixtureFailure {Assert-CodexRecoveryPackage $package $script:Fixture.FullName $script:Fixture.Context} 'not staged/registered on the bound USB'
  Assert-Test ((Get-CodexSystemPackageEntryAudit $script:Fixture.FullName).status -eq 'pending') 'ordinary remaining directory not called clean'
}
Run-Case 'package entry query denied is unknown, never absent or a valid USB alias' {
  New-Fixture $false
  function Get-Item { [CmdletBinding()]param($LiteralPath,[switch]$Force) throw [UnauthorizedAccessException]::new('fixture query denied') }
  $path=Join-Path (Get-CodexSystemPackageStore) $script:Fixture.FullName
  $info=Get-CodexPackagePathInfo $path $script:Fixture.FullName (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps')
  Assert-Test ($info.state -eq 'path-query-failed' -and $null -eq $info.effectivePath) 'query denied cannot authorize placement'
  Assert-Test ((Get-CodexSystemPackageEntryAudit $script:Fixture.FullName).status -eq 'pending') 'query denied retains cleanup evidence'
}
Run-Case 'exact system entry absence and invalid paths are handled without mutation' {
  New-Fixture $false
  Assert-Test ((Get-CodexSystemPackageEntryAudit $script:Fixture.FullName).status -eq 'absent') 'missing exact entry is absent'
  foreach ($bad in @('\\server\share\package','C:\WindowsApps\..\other','C:\WindowsApps\pkg:stream')) {
    $info=Get-CodexPackagePathInfo $bad $script:Fixture.FullName
    Assert-Test ($null -eq $info.effectivePath) 'ambiguous or remote paths rejected'
  }
  Expect-FixtureFailure {Get-CodexSystemPackageEntryAudit '..\OtherApp'} 'Invalid exact Codex'
}
Run-Case 'deployment space uses uncompressed bytes and actual DriveInfo adapter' {
  New-Fixture $false
  $required = Get-CodexRecoveryDeploymentSpaceBudget $script:Fixture.Payload
  Assert-Test ($required -eq (512MB + 4096 + [Text.Encoding]::UTF8.GetByteCount('fixture signed-payload stand-in'))) 'budget is expanded bytes plus metadata and working margin'
  $free = & $script:RealUsbFreeBytes @{usbRoot=[IO.Path]::GetPathRoot($script:Fixture.Root)}
  Assert-Test ($free -gt 0) 'production free-space adapter works on a real local volume without writes'
}
Run-Case 'fresh low-space failure cannot start deployment' {
  New-Fixture $false; $script:Fixture.FreeBytes=0
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Insufficient USB deployment space'
  Assert-Test ($script:Fixture.StageCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'no deployment on insufficient space'
  $script:Fixture.FreeBytes=100GB; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $result.resumed -and $script:Fixture.StageCalls -eq 1) 'freeing space resumes the same preserved journal'
}
Run-Case 'repair low-space check precedes existing-package removal' {
  New-Fixture $true; $script:Fixture.FreeBytes=0
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Insufficient USB deployment space'
  Assert-Test ($script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.StageCalls -eq 0 -and $null -ne $script:Fixture.Current) 'existing package retained before repair removal'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'original-settings') 'existing data unchanged'
}
Run-Case 'completed USB Stage resumes with no room for a duplicate package' {
  New-Fixture $false; $script:Fixture.FailStage=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'simulated Stage completion interruption'
  $script:Fixture.FailStage=$false; $script:Fixture.FreeBytes=0
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'healthy exact staged bytes reused without another full-space budget'
}
Run-Case 'pending journal alone does not waive space when Stage bytes are missing' {
  New-Fixture $false; $script:Fixture.FailStage=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'simulated Stage completion interruption'
  $script:Fixture.FailStage=$false; $script:Fixture.Staged=$null; $script:Fixture.FreeBytes=0
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Insufficient USB deployment space'
  Assert-Test ($script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'journal cannot authorize an under-budget new Stage'
}
# Loading enum metadata is read-only; the actual native manager is never created
# by these fixtures. They run under Windows PowerShell 5.1 like the installer.
$null = [Windows.Management.Deployment.DeploymentOptions,Windows,ContentType=WindowsRuntime]
Run-Case 'real volume-query wrapper ignores global same-name candidate and stages the USB' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true
  $script:Fixture.GlobalStaged=[pscustomobject]@{PackageFullName=$script:Fixture.FullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=('C:\Program Files\WindowsApps\'+$script:Fixture.FullName)}
  $timer=[Diagnostics.Stopwatch]::StartNew(); $result=Invoke-FixtureRecovery; $timer.Stop()
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'global candidate must not skip Stage'
  Assert-Test ($script:Fixture.GlobalQueries -gt 0 -and $script:Fixture.UsbQueries -ge 3) 'production target query and diagnostic global query both ran'
  Assert-Test ($timer.ElapsedMilliseconds -ge 350) 'production GetAwaiter waited for both delayed tasks'
  Assert-Test (@($script:Fixture.DeploymentRecords|Where-Object {$_.operation -eq 'StagePackageAsync USB' -and $_.state -eq 'completed' -and $_.nativeInvoked}).Count -gt 0) 'Stage native result persisted before Register'
}
Run-Case 'fresh staging failure before landing retries same journal without Remove' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.StageMode='fault'
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'fixture deployment error'
  $first=Read-FixtureJournal
  Assert-Test ($first.phase -eq 'staging' -and $first.mode -eq 'fresh' -and $null -eq $first.original -and $first.lastDeployment.state -eq 'failed') 'failed fresh stage recorded, not deleted'
  Assert-Test ($first.lastDeployment.hresult -eq '0x80073CF0' -and $first.lastDeployment.activityId -eq '9afe3240-101d-0002-37b9-8aa11d10dd01') 'original native HRESULT and ActivityId retained'
  Assert-Test ($null -eq $script:Fixture.Staged -and $script:Fixture.RegisterCalls -eq 0) 'failure landed no package'
  # Emulate the old v0.5.11 record (lastDeployment=null), not a new schema.
  $first.lastDeployment=$null
  [IO.File]::WriteAllText((Join-Path $script:Fixture.Context.backupRoot 'recovery.json'),($first|ConvertTo-Json -Depth 16),[Text.Encoding]::UTF8)
  $script:Fixture.StageMode=''; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $result.resumed -and $result.journalPath.EndsWith(('recovery-'+$first.id+'.completed.json'))) 'same journal id safely resumed and archived'
  Assert-Test ($script:Fixture.StageCalls -eq 2 -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'exact retry counts with no Remove'
}
Run-Case 'completed Stage outcome is durable even if USB placement is absent' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.EmptyAfterStage=$true
  $script:Fixture.GlobalStaged=[pscustomobject]@{PackageFullName=$script:Fixture.FullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=('C:\Program Files\WindowsApps\'+$script:Fixture.FullName)}
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'identity changed or is missing.*expectedPath=.*actualPath=<none>'
  $first=Read-FixtureJournal
  Assert-Test ($first.phase -eq 'staging' -and $first.lastDeployment.state -eq 'completed' -and $first.lastDeployment.nativeInvoked -and $first.lastDeployment.activityId) 'native completion survives postcondition failure'
  Assert-Test ($script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.LastFailure -match 'globalMatchingCandidates.*Program Files') 'global C candidate remains diagnostic-only after USB Stage did not land'
  $script:Fixture.EmptyAfterStage=$false; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $result.journalPath.EndsWith(('recovery-'+$first.id+'.completed.json')) -and $script:Fixture.StageCalls -eq 2 -and $script:Fixture.RemoveCalls -eq 0) 'missing payload safely retried on same evidence'
}
Run-Case 'completed Stage with journal-write failure resumes without another Stage' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.FailCompletedStageJournal=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Deployment completed but result journal write failed.*ActivityId=9afe3240-101d-0002-37b9-8aa11d10dd01'
  $first=Read-FixtureJournal
  Assert-Test ($first.phase -eq 'staging' -and $first.lastDeployment.state -eq 'started') 'write-ahead started state retained after completed Stage result write failed'
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'retry queries and reuses exact staged USB package'
}
Run-Case 'real target-query wrapper reuses an existing USB stage without calling Stage' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true
  $script:Fixture.Staged=[pscustomobject]@{PackageFullName=$script:Fixture.FullName;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $script:Fixture.FullName}
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 1) 'target stage reused only after original path/identity checks'
}
Run-Case 'native Stage wrong-path result reports actual and expected without Register' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.WrongStagedVolume=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'not staged/registered.*expectedPath=.*actualPath=.*recoveryDiagnostic=.*stageInvokedThisRun":true'
  $j=Read-FixtureJournal
  Assert-Test ($j.lastDeployment.state -eq 'completed' -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'successful native call cannot substitute for correct placement'
}
foreach($mode in @('cancel','result-error','null-result')) {
  Run-Case ('production await rejects '+$mode+' before any registration') {
    New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.StageMode=$mode
    $pattern=switch($mode){'cancel' {'fixture deployment cancelled|canceled|cancelled'} 'result-error' {'fixture extended text'} 'null-result' {'without a DeploymentResult'}}
    Expect-FixtureFailure {Invoke-FixtureRecovery} $pattern
    $j=Read-FixtureJournal
    Assert-Test ($j.lastDeployment.state -eq 'failed' -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'failed or invalid async result is durable and non-destructive'
  }
}
Run-Case 'Register failure is recorded before retry and does not restage the USB' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.RegisterMode='fault'
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'fixture deployment error'
  $first=Read-FixtureJournal
  Assert-Test ($first.phase -eq 'registering' -and $first.lastDeployment.operation -match '^Register' -and $first.lastDeployment.state -eq 'failed' -and $first.lastDeployment.nativeInvoked) 'registration failure preserved'
  $script:Fixture.RegisterMode=''; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 2 -and $script:Fixture.RemoveCalls -eq 0) 'Register retry never restages or removes'
}
Run-Case 'false IsRegistered hint waits for the authoritative current-user postcondition' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true
  $script:Fixture.RegisterMode='false-hint'; $script:Fixture.RegisterProjectionDelayQueries=4
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 1) 'false native hint must not override an exact successful registration'
  Assert-Test ($script:Fixture.RegisterProjectionQueries -ge 5) 'registration postcondition waited for PackageUser projection convergence'
  $observed=@($script:Fixture.DeploymentRecords | Where-Object {
    $_.operation -match '^RegisterPackagesByFullNameAsync' -and $_.state -eq 'completed' -and
    $_.isRegisteredHint -eq $false -and $_.postconditionState -eq 'observed-current-user-package'
  })
  Assert-Test ($observed.Count -gt 0) 'journal distinguishes native false hint from observed registration success'
}
Run-Case 'missing registration postcondition times out durably and resumes without duplicate Register' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true
  $script:Fixture.RegisterMode='false-hint'; $script:Fixture.RegisterProjectionDelayQueries=999
  $script:CodexRecoveryRegistrationPollAttempts=3
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'postcondition was not observed.*IsRegisteredHint=False'
  $first=Read-FixtureJournal
  Assert-Test ($first.phase -eq 'registering' -and $first.lastDeployment.state -eq 'completed' -and $first.lastDeployment.postconditionState -eq 'timed-out' -and $first.lastDeployment.isRegisteredHint -eq $false) 'native completion and postcondition timeout remain distinct and durable'
  $script:Fixture.RegisterProjectionDelayQueries=0
  $script:CodexRecoveryRegistrationPollAttempts=240
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $result.resumed -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.StageCalls -eq 1) 'resume observes the committed package without replaying Register or Stage'
}
Run-Case 'tampered registration postcondition cannot authorize journal resume' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true
  $script:Fixture.RegisterMode='false-hint'; $script:Fixture.RegisterProjectionDelayQueries=999
  $script:CodexRecoveryRegistrationPollAttempts=1
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'postcondition was not observed'
  $active = Join-Path $script:Fixture.Context.backupRoot 'recovery.json'
  $journal = Get-Content -LiteralPath $active -Raw | ConvertFrom-Json
  $journal | Add-Member -NotePropertyName registrationPostcondition -NotePropertyValue ([pscustomobject]@{
    state='observed'; attempts=1; expectedFullName=$script:Fixture.FullName
    observedFullName='OpenAI.Codex_0.0.0.0_x64__2p2nqsd0c76g0'; nativeIsRegisteredHint=$false
  }) -Force
  [IO.File]::WriteAllText($active, ($journal | ConvertTo-Json -Depth 20), [Text.Encoding]::UTF8)
  $script:CodexRecoveryRegistrationPollAttempts=240
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'registration postcondition does not match'
}
Run-Case 'target-volume query failure is not an empty candidate list' {
  New-Fixture $false; $script:Fixture.NativeBoundary=$true; $script:Fixture.FailUsbQuery=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'fixture target-volume query denied'
  Assert-Test ($script:Fixture.StageCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'query errors authorize no deployment'
  Assert-Test ($script:Fixture.LastFailure -match 'targetQueryError.*fixture target-volume query denied') 'target query failure is retained in the bounded package probe'
}
Run-Case 'context initialization failure keeps original error and safe diagnostic defaults' {
  New-Fixture $false; $script:Fixture.FailContext=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} '^fixture recovery context unavailable$'
  Assert-Test ($script:Fixture.StageCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'context failure invokes no deployment or removal'
}
# Real WinRT IAsyncOperationWithProgress -> the unmodified production AsTask
# reflection and GetResult. AsyncInfo wraps managed tasks; it does NOT call
# PackageManager or deploy anything. DeploymentResult has no public constructor,
# so completed-null is deliberately rejected, not counted as deployment success.
Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
Add-Type -TypeDefinition @'
using System; using System.Threading; using System.Reflection; using System.Runtime.InteropServices;
namespace CodexRecoveryRealAsyncFixture {
 public static class Completion {
  public static void Later(object source,string mode,int delay) {
   ThreadPool.QueueUserWorkItem(delegate {
    Thread.Sleep(delay); Type type=source.GetType();
    if(mode=="fault") type.GetMethod("SetException",new Type[]{typeof(Exception)}).Invoke(source,new object[]{new COMException("real adapter fixture failure",unchecked((int)0x80073CF0))});
    else if(mode=="cancel") type.GetMethod("SetCanceled",Type.EmptyTypes).Invoke(source,null);
    else type.GetMethod("SetResult").Invoke(source,new object[]{null});
   });
  }
 }
}
'@ -ErrorAction Stop | Out-Null
function New-CodexRecoveryRealAsyncOperation([string]$Mode,[int]$Delay) {
  $resultType=[Windows.Management.Deployment.DeploymentResult,Windows,ContentType=WindowsRuntime]
  $progressType=[Windows.Management.Deployment.DeploymentProgress,Windows,ContentType=WindowsRuntime]
  $tcsOpen=[System.Threading.Tasks.TaskCompletionSource[int]].GetGenericTypeDefinition()
  $tcsType=$tcsOpen.MakeGenericType($resultType); $tcs=[Activator]::CreateInstance($tcsType)
  $ct=[Threading.CancellationToken]; $progressOpen=[IProgress[int]].GetGenericTypeDefinition(); $progress=$progressOpen.MakeGenericType($progressType)
  $funcOpen=[Func[int,int,int]].GetGenericTypeDefinition(); $func=$funcOpen.MakeGenericType($ct,$progress,$tcs.Task.GetType())
  $p1=[Linq.Expressions.Expression]::Parameter($ct,'token'); $p2=[Linq.Expressions.Expression]::Parameter($progress,'progress')
  $body=[Linq.Expressions.Expression]::Property([Linq.Expressions.Expression]::Constant($tcs,$tcsType),'Task')
  $parameters=[Linq.Expressions.ParameterExpression[]]@($p1,$p2)
  $lambdaMethod=[Linq.Expressions.Expression].GetMethods()|Where-Object {$_.Name -eq 'Lambda' -and -not $_.IsGenericMethodDefinition -and $_.GetParameters().Count -eq 3 -and $_.GetParameters()[0].ParameterType -eq [type]}|Select-Object -First 1
  $invokeArgs=New-Object object[] 3; $invokeArgs[0]=$func; $invokeArgs[1]=$body; $invokeArgs[2]=$parameters
  $delegate=$lambdaMethod.Invoke($null,$invokeArgs).Compile()
  $run=[System.Runtime.InteropServices.WindowsRuntime.AsyncInfo].GetMethods()|Where-Object {$_.Name -eq 'Run' -and $_.IsGenericMethodDefinition -and $_.GetGenericArguments().Count -eq 2}|Select-Object -First 1
  $operation=$run.MakeGenericMethod($resultType,$progressType).Invoke($null,@($delegate))
  [CodexRecoveryRealAsyncFixture.Completion]::Later($tcs,$Mode,$Delay)
  return $operation
}
foreach($mode in @('delayed-null','fault','cancel')) {
  Run-Case ('actual WinRT AsTask reflection handles '+$mode+' without deployment') {
    New-Fixture $false
    $operation=New-CodexRecoveryRealAsyncOperation $mode 250
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $pattern=switch($mode){'delayed-null' {'without a DeploymentResult'} 'fault' {'real adapter fixture failure'} 'cancel' {'cancel|canceled|取消'}}
    Expect-FixtureFailure {Wait-CodexRecoveryDeployment $operation} $pattern
    $timer.Stop()
    Assert-Test ($timer.ElapsedMilliseconds -ge 150) 'real AsTask must wait, not assume completion'
    Assert-Test ($script:Fixture.StageCalls -eq 0 -and $script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'native async adapter validation made zero deployments'
    # AsyncInfo.Run may complete its managed Task a few milliseconds before
    # the projected IAsyncInfo.Status catches up. Calling Close in that gap is
    # itself an illegal state transition on Windows PowerShell 5.1 and made
    # this fixture flaky even though the production await assertion passed.
    # Production never calls Close on these deployment operations. Prove the
    # synthetic adapter reaches a terminal state, then let this short-lived
    # test process release the projection normally.
    $statusDeadline=[DateTime]::UtcNow.AddSeconds(2)
    while ([string]$operation.Status -eq 'Started' -and [DateTime]::UtcNow -lt $statusDeadline) {
      Start-Sleep -Milliseconds 10
    }
    Assert-Test ([string]$operation.Status -in @('Completed','Error','Canceled')) 'real async fixture did not reach a terminal WinRT status'
  }
}
Run-Case 'repair original data and preserve content written before baseline' {
  New-Fixture $true; $script:Fixture.PreBaselineUserWrite=$true; $result=Invoke-FixtureRecovery
  Assert-Test $result.success 'repair must succeed'
  Assert-Test ($script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.BackupCalls -eq 2) 'exactly one removal, two full backups'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'original-settings') 'original settings restored'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $result.baselineBackup 'Settings\settings.dat')) -eq 'new-user-before-baseline') 'new pre-baseline content retained'
}
Run-Case 'resume after Remove completed without repeating removal or original backup' {
  New-Fixture $true; $script:Fixture.FailRemoveAfter=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'interruption after remove'
  $script:Fixture.FailRemoveAfter=$false; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $result.resumed -and $script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.BackupCalls -eq 2) 'resume avoids destructive replay'
}
Run-Case 'resume after Stage completion does not restage existing payload' {
  New-Fixture $true; $script:Fixture.FailStage=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Stage completion interruption'
  $script:Fixture.FailStage=$false; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.StageCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 1) 'staged journal recovery'
}
Run-Case 'resume after Register completion does not register or remove again' {
  New-Fixture $true; $script:Fixture.FailRegister=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Register completion interruption'
  $script:Fixture.FailRegister=$false; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 1) 'registered journal recovery'
}
Run-Case 'post-baseline user edit is not overwritten' {
  New-Fixture $true; $script:Fixture.ChangeAfterBaseline=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'changed after baseline'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'new-user-after-baseline') 'new content preserved'
  Assert-Test ($script:Fixture.Backups.Count -eq 2) 'both backups retained'
}
Run-Case 'interrupted file restore resumes from original/desired hashes' {
  New-Fixture $true; $script:Fixture.FailAfterRestoredFile=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'interrupted restore after one file'
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.BackupCalls -eq 2) 'partial restore must not repeat removal or backup'
}
Run-Case 'tampered backup file prevents further recovery mutations' {
  New-Fixture $true; $script:Fixture.FailRemoveAfter=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'interruption after remove'
  [IO.File]::WriteAllText((Join-Path $script:Fixture.Backups[0] 'Settings\settings.dat'),'tampered')
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'does not match its manifest'
  Assert-Test ($script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.StageCalls -eq 0) 'tampered backup blocks deployment'
}
Run-Case 'tampered SID journal does not delete or deploy' {
  New-Fixture $true; $script:Fixture.FailRemoveBefore=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'pre-removal failure'
  $path=Join-Path $script:Fixture.Context.backupRoot 'recovery.json'; $j=[IO.File]::ReadAllText($path)|ConvertFrom-Json; $j.sid='S-1-5-21-other'
  [IO.File]::WriteAllText($path,($j|ConvertTo-Json -Depth 16))
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'journal context'
  Assert-Test ($script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.StageCalls -eq 0) 'tampered SID is non-mutating'
}
Run-Case 'manifest traversal rejected before Remove' {
  New-Fixture $true; $script:Fixture.TraversalManifest=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Unsafe relative recovery'
  Assert-Test ($script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.StageCalls -eq 0) 'traversal never authorizes removal'
}
Run-Case 'ownership conflict never starts backup/removal/deployment' {
  New-Fixture $true; $script:Fixture.Shared=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'shared package'
  Assert-Test ($script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.BackupCalls -eq 0 -and $script:Fixture.StageCalls -eq 0) 'shared refusal counts'
}
Run-Case 'ambiguous interrupted removal never repeats Remove' {
  New-Fixture $true; $script:Fixture.FailRemoveBefore=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'pre-removal failure'
  $script:Fixture.FailRemoveBefore=$false
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'no second removal'
  Assert-Test ($script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.BackupCalls -eq 1) 'ambiguous removal not replayed'
}
Run-Case 'physical volume identity change rejects existing journal' {
  New-Fixture $true; $script:Fixture.FailRemoveAfter=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'interruption after remove'
  $script:Fixture.Context.usbId='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'journal context'
  Assert-Test ($script:Fixture.StageCalls -eq 0) 'replacement USB not used'
}
Run-Case 'directory pin prevents parent rename and reparse redirection' {
  New-Fixture $false; Initialize-CodexRecoveryFileHandles
  $directory=Join-Path $script:Fixture.Root 'pinned'; [IO.Directory]::CreateDirectory($directory)|Out-Null
  $pins=Lock-CodexRecoveryDirectory $directory
  try { Expect-FixtureFailure {[IO.Directory]::Move($directory,($directory+'-changed'))} 'being used|access|used by another|另一个|访问|使用' }
  finally { Close-CodexRecoveryHandles $pins }
}
Run-Case 'interruption between displaced-file and new-file renames resumes safely' {
  New-Fixture $true; $script:Fixture.FailDuringGap=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'between no-replace renames'
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RemoveCalls -eq 1) 'rename gap resumed without another Remove'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'original-settings') 'gap restored original settings'
}
Run-Case 'concurrent file in interrupted rename gap is never overwritten' {
  New-Fixture $true; $script:Fixture.FailDuringGap=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'between no-replace renames'
  Write-FixtureFile 'Settings\settings.dat' 'user-created-in-gap'
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'changed after baseline'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'user-created-in-gap') 'concurrent gap content preserved'
}
Run-Case 'locked existing journal is not treated as absent or overwritten' {
  New-Fixture $true; $script:Fixture.FailRemoveBefore=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'pre-removal failure'
  $path=Join-Path $script:Fixture.Context.backupRoot 'recovery.json'
  $before=Get-CodexRecoveryHash $path
  $exclusive=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
  try { Expect-FixtureFailure {Invoke-FixtureRecovery} 'Cannot open relative|being used|sharing' } finally { $exclusive.Dispose() }
  Assert-Test ((Get-CodexRecoveryHash $path) -eq $before -and $script:Fixture.RemoveCalls -eq 1) 'locked journal unchanged'
}
Run-Case 'physical USB replacement inside operation blocks before Remove' {
  New-Fixture $true; $script:Fixture.ChangeVolumeBeforeRemove=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'context changed'
  Assert-Test ($script:Fixture.RemoveCalls -eq 0) 'phase-boundary physical identity rechecked'
}
Run-Case 'nondestructive upgrade preserves upgraded data and removes no package' {
  New-Fixture $true; $script:Fixture.Update=$true
  $old='OpenAI.Codex_26.824.5000.0_x64__2p2nqsd0c76g0'
  $script:Fixture.Current=[pscustomobject]@{PackageFullName=$old;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $old}
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RemoveCalls -eq 0 -and $script:Fixture.BackupCalls -eq 1) 'upgrade never removed or restored over new data'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'windows-default') 'new-version state not replaced by old backup'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $result.originalBackup 'Settings\settings.dat')) -eq 'original-settings') 'pre-upgrade backup preserved'
}
Run-Case 'nondestructive upgrade resumes completed Register without another deployment' {
  New-Fixture $true; $script:Fixture.Update=$true; $script:Fixture.FailRegister=$true
  $old='OpenAI.Codex_26.824.5000.0_x64__2p2nqsd0c76g0'
  $script:Fixture.Current=[pscustomobject]@{PackageFullName=$old;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $old}
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'Register completion interruption'
  $script:Fixture.FailRegister=$false; $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RegisterCalls -eq 1 -and $script:Fixture.RemoveCalls -eq 0) 'update resumes without destructive replay'
}

function Initialize-FixtureReparseProbe {
  if ('CodexRecoveryFixture.Probe' -as [type]) { return }
  Add-Type -TypeDefinition @'
using System; using System.IO; using System.Text; using System.ComponentModel;
using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles;
namespace CodexRecoveryFixture {
public static class Probe {
  public static string RaceDirectory, RaceTarget;
  public static void BeforeRename() {
    if(RaceDirectory!=null) { string dir=RaceDirectory,target=RaceTarget; RaceDirectory=null; RaceTarget=null; SetJunction(dir,target); }
  }
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateFileW(string p,uint a,uint s,IntPtr z,uint d,uint f,IntPtr t);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool DeviceIoControl(SafeFileHandle h,uint c,byte[] b,int n,IntPtr o,int m,out int r,IntPtr v);
  public static void SetJunction(string directory,string target) {
    // Test attack uses only FILE_WRITE_ATTRIBUTES, which bypasses normal data
    // share locks. Both paths are unique temporary fixture directories.
    using(var h=CreateFileW("\\\\?\\"+directory,0x100,7,IntPtr.Zero,3,0x02200000,IntPtr.Zero)) {
      if(h.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error());
      byte[] sub=Encoding.Unicode.GetBytes("\\??\\"+target), print=Encoding.Unicode.GetBytes(target);
      byte[] buffer=new byte[16+sub.Length+2+print.Length+2];
      Buffer.BlockCopy(BitConverter.GetBytes(0xa0000003u),0,buffer,0,4);
      Buffer.BlockCopy(BitConverter.GetBytes((ushort)(buffer.Length-8)),0,buffer,4,2);
      Buffer.BlockCopy(BitConverter.GetBytes((ushort)sub.Length),0,buffer,10,2);
      Buffer.BlockCopy(BitConverter.GetBytes((ushort)(sub.Length+2)),0,buffer,12,2);
      Buffer.BlockCopy(BitConverter.GetBytes((ushort)print.Length),0,buffer,14,2);
      Buffer.BlockCopy(sub,0,buffer,16,sub.Length); Buffer.BlockCopy(print,0,buffer,18+sub.Length,print.Length);
      int returned; if(!DeviceIoControl(h,0x900a4,buffer,buffer.Length,IntPtr.Zero,0,out returned,IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
  }
}}
'@ -ErrorAction Stop | Out-Null
}
Run-Case 'real metadata-only reparse mutation cannot redirect relative create/read/rename' {
  New-Fixture $false; Initialize-CodexRecoveryFileHandles; Initialize-FixtureReparseProbe
  $parent=Join-Path $script:Fixture.Root 'pinned-empty'; $outside=Join-Path $script:Fixture.Root 'outside-recovery-target'
  [IO.Directory]::CreateDirectory($parent)|Out-Null; [IO.Directory]::CreateDirectory($outside)|Out-Null
  [IO.File]::WriteAllText((Join-Path $outside 'outside-secret.txt'),'untouched')
  $pins=Lock-CodexRecoveryDirectory $parent; $rootHandle=$pins[$pins.Count-1]
  $source=Join-Path $script:Fixture.Root 'rename-source.txt'; [IO.File]::WriteAllText($source,'source-retained')
  $sourcePins=Lock-CodexRecoveryDirectory $script:Fixture.Root
  $handle=[CodexAppxRecovery.NativeFiles]::Relative($sourcePins[$sourcePins.Count-1],'rename-source.txt',$false,$false,$true)
  try {
    [CodexRecoveryFixture.Probe]::SetJunction($parent,$outside)
    Expect-FixtureFailure {[CodexAppxRecovery.NativeFiles]::CreateNewChild($rootHandle,'unexpected.txt')} 'redirected|reparse'
    Expect-FixtureFailure {[CodexAppxRecovery.NativeFiles]::OpenRead((Join-Path $parent 'outside-secret.txt'))} 'redirected|reparse|NTSTATUS'
    Expect-FixtureFailure {[CodexAppxRecovery.NativeFiles]::Rename($handle,$rootHandle,'renamed.txt',$false)} 'redirected|reparse'
    Assert-Test (-not [IO.File]::Exists((Join-Path $outside 'unexpected.txt')) -and -not [IO.File]::Exists((Join-Path $outside 'renamed.txt'))) 'no external file was created or replaced'
    Assert-Test ([IO.File]::ReadAllText((Join-Path $outside 'outside-secret.txt')) -eq 'untouched') 'external content untouched'
  } finally {
    $handle.Dispose(); Close-CodexRecoveryHandles $sourcePins; Close-CodexRecoveryHandles $pins
    # Unlink only this known fixture junction, never its external target tree.
    [IO.Directory]::Delete($parent,$false)
  }
}
Run-Case 'actual mid-Rename reparse race after final check cannot redirect native operation' {
  New-Fixture $false; Initialize-CodexRecoveryFileHandles; Initialize-FixtureReparseProbe
  # Compile only an in-memory test copy with one hook inserted AFTER the native
  # function's final ordinary-directory check. Production source stays intact.
  $initializer=(Get-Command Initialize-CodexRecoveryFileHandles).ScriptBlock.ToString()
  $match=[regex]::Match($initializer,"(?s)@'\r?\n(?<native>.*?)\r?\n'@")
  Assert-Test $match.Success 'extract actual current NativeFiles source for race injection'
  $native=$match.Groups['native'].Value.Replace('namespace CodexAppxRecovery {','namespace CodexRecoveryRaceProbe {')
  $native=$native.Replace('public static class NativeFiles {','public static class NativeFiles { public static Action BeforeRename;')
  $needle='IoStatus io; Check(NtSetInformationFile('
  Assert-Test ($native.Contains($needle)) 'inject immediately before actual NtSetInformationFile'
  $native=$native.Replace($needle,'if(BeforeRename!=null) BeforeRename(); IoStatus io; Check(NtSetInformationFile(')
  Add-Type -TypeDefinition $native -ErrorAction Stop | Out-Null
  $parent=Join-Path $script:Fixture.Root 'race-parent'; $outside=Join-Path $script:Fixture.Root 'race-outside'
  [IO.Directory]::CreateDirectory($parent)|Out-Null; [IO.Directory]::CreateDirectory($outside)|Out-Null
  $pins=[CodexRecoveryRaceProbe.NativeFiles]::LockDirectories($parent,$false)
  $sourcePins=[CodexRecoveryRaceProbe.NativeFiles]::LockDirectories($script:Fixture.Root,$false)
  [IO.File]::WriteAllText((Join-Path $script:Fixture.Root 'native-race-source.txt'),'source-intact')
  $source=[CodexRecoveryRaceProbe.NativeFiles]::Relative($sourcePins[$sourcePins.Count-1],'native-race-source.txt',$false,$false,$true)
  try {
    [CodexRecoveryFixture.Probe]::RaceDirectory=$parent; [CodexRecoveryFixture.Probe]::RaceTarget=$outside
    [CodexRecoveryRaceProbe.NativeFiles]::BeforeRename=[Action]{[CodexRecoveryFixture.Probe]::BeforeRename()}
    Expect-FixtureFailure {[CodexRecoveryRaceProbe.NativeFiles]::Rename($source,$pins[$pins.Count-1],'escaped.txt',$false)} 'NTSTATUS=0xC0000280'
    Assert-Test (-not [IO.File]::Exists((Join-Path $outside 'escaped.txt'))) 'NtSetInformationFile refused changed root'
    Assert-Test ([IO.File]::Exists((Join-Path $script:Fixture.Root 'native-race-source.txt'))) 'failed race kept source file'
  } finally {
    $source.Dispose(); Close-CodexRecoveryHandles $pins; Close-CodexRecoveryHandles $sourcePins
    [CodexRecoveryFixture.Probe]::RaceDirectory=$null; [CodexRecoveryFixture.Probe]::RaceTarget=$null
    [CodexRecoveryRaceProbe.NativeFiles]::BeforeRename=$null
    [IO.Directory]::Delete($parent,$false)
  }
}
Run-Case 'update retained cross-volume data promotes exact-new-version controlled repair' {
  New-Fixture $true; $script:Fixture.Update=$true; $script:Fixture.UpdateNeedsRepair=$true
  $old='OpenAI.Codex_26.824.5000.0_x64__2p2nqsd0c76g0'
  $script:Fixture.Current=[pscustomobject]@{PackageFullName=$old;PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0';InstallLocation=Join-Path (Join-Path $script:Fixture.Context.usbRoot 'WindowsApps') $old}
  $result=Invoke-FixtureRecovery
  Assert-Test ($result.success -and $script:Fixture.RemoveCalls -eq 1 -and $script:Fixture.RegisterCalls -eq 2) 'only the new exact version repaired'
  Assert-Test ([IO.File]::ReadAllText((Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat')) -eq 'upgraded-user-content') 'updated state restored rather than old-version backup'
  Assert-Test ($script:Fixture.Backups.Count -eq 3) 'pre-update, updated and post-rebuild states all retained'
}
Run-Case 'wrong-volume staged payload never registers on system program volume' {
  New-Fixture $false; $script:Fixture.WrongStagedVolume=$true
  Expect-FixtureFailure {Invoke-FixtureRecovery} 'not staged/registered on the bound USB'
  Assert-Test ($script:Fixture.RegisterCalls -eq 0 -and $script:Fixture.RemoveCalls -eq 0) 'wrong stage stops before Register without C bootstrap'
}
Run-Case 'long local recovery paths use relative handles without MAX_PATH failure' {
  New-Fixture $false; Initialize-CodexRecoveryFileHandles
  $long=$script:Fixture.Root
  foreach($part in 1..6) { $long=Join-Path $long ('long-component-'+('x'*30)+$part) }
  $pins=Lock-CodexRecoveryDirectory $long $true
  try {
    $path=Join-Path $long 'long.txt'; $stream=[CodexAppxRecovery.NativeFiles]::CreateNew($path)
    try { $bytes=[Text.Encoding]::UTF8.GetBytes('long-path-data');$stream.Write($bytes,0,$bytes.Length);$stream.Flush($true) } finally { $stream.Dispose() }
    Assert-Test ((Get-CodexRecoveryHash $path) -eq (Get-CodexRecoveryTextHash 'long-path-data')) 'long path read/write hash verified'
    [CodexAppxRecovery.NativeFiles]::MoveChecked($path,(Join-Path $long 'renamed.txt'),(Get-CodexRecoveryTextHash 'long-path-data'),$false)
  } finally { Close-CodexRecoveryHandles $pins }
}
Run-Case 'restored files inherit Windows destination parent ACL rather than backup ACL' {
  New-Fixture $true; $script:Fixture.CustomParentAcl=$true; $result=Invoke-FixtureRecovery
  $path=Join-Path $script:Fixture.Context.dataRoot 'Settings\settings.dat'
  $rules=[IO.File]::GetAccessControl($path).GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])
  $matched=@($rules|Where-Object {$_.IdentityReference.Value -eq 'S-1-5-32-555' -and $_.IsInherited})
  Assert-Test ($matched.Count -gt 0) 'restored file naturally inherits temporary destination parent-specific rule'
  $originalRules=[IO.File]::GetAccessControl((Join-Path $result.originalBackup 'Settings\settings.dat')).GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])
  Assert-Test (@($originalRules|Where-Object {$_.IdentityReference.Value -eq 'S-1-5-32-555'}).Count -eq 0) 'backup did not already contain destination-specific ACL'
}
Run-Case 'traverse-only ancestor does not require LIST_DIRECTORY for a readable child' {
  New-Fixture $false; Initialize-CodexRecoveryFileHandles
  $ancestor=Join-Path $script:Fixture.Root 'traverse-only'; $leaf=Join-Path $ancestor 'readable-child'
  [IO.Directory]::CreateDirectory($leaf)|Out-Null; [IO.File]::WriteAllText((Join-Path $leaf 'content.txt'),'readable-data')
  $testAcl=[IO.Directory]::GetAccessControl($ancestor)
  $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User
  $deny=New-Object Security.AccessControl.FileSystemAccessRule($sid,[Security.AccessControl.FileSystemRights]::ListDirectory,
    [Security.AccessControl.InheritanceFlags]::None,[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Deny)
  $testAcl.AddAccessRule($deny); [IO.Directory]::SetAccessControl($ancestor,$testAcl)
  try {
    Expect-FixtureFailure {[CodexAppxRecovery.NativeFiles]::Entries($ancestor)} 'NTSTATUS=0xC0000022'
    Assert-Test ((Get-CodexRecoveryHash (Join-Path $leaf 'content.txt')) -eq (Get-CodexRecoveryTextHash 'readable-data')) 'child opened through traverse-only ancestor'
    Assert-Test (@([CodexAppxRecovery.NativeFiles]::Entries($leaf)).Count -eq 1) 'leaf enumeration independently requests list access'
  } finally {
    $restoreAcl=[IO.Directory]::GetAccessControl($ancestor)
    $restoreAcl.RemoveAccessRuleSpecific($deny)
    [IO.Directory]::SetAccessControl($ancestor,$restoreAcl)
  }
}
Write-Output ('AppX data recovery fixtures passed: ' + $script:CaseCount)
