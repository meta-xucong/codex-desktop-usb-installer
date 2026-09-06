# Loaded by the engine after its authenticated payload/USB/ownership checks.
# Defines functions only. No deployment, compilation or filesystem writes occur
# until Invoke-CodexDataRecovery is called. Windows PowerShell 5.1 compatible.

function Initialize-CodexRecoveryFileHandles {
  if ('CodexAppxRecovery.NativeFiles' -as [type]) { return }
  Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.ComponentModel;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace CodexAppxRecovery {
  public static class NativeFiles {
    [StructLayout(LayoutKind.Sequential)] struct Info {
      public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Created;
      public System.Runtime.InteropServices.ComTypes.FILETIME Accessed, Written;
      public uint VolumeSerial, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
      IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool GetFileInformationByHandle(SafeFileHandle handle, out Info info);
    [StructLayout(LayoutKind.Sequential)] struct UnicodeString { public ushort Length, MaximumLength; public IntPtr Buffer; }
    [StructLayout(LayoutKind.Sequential)] struct ObjectAttributes {
      public int Length; public IntPtr RootDirectory, ObjectName; public uint Attributes;
      public IntPtr SecurityDescriptor, SecurityQualityOfService;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoStatus { public IntPtr Status; public UIntPtr Information; }
    [DllImport("ntdll.dll")] static extern int NtCreateFile(out SafeFileHandle handle, uint access,
      ref ObjectAttributes attributes, out IoStatus status, IntPtr allocation, uint fileAttributes,
      uint share, uint disposition, uint options, IntPtr ea, uint eaLength);
    [DllImport("ntdll.dll")] static extern uint RtlNtStatusToDosError(int status);
    [DllImport("ntdll.dll")] static extern int NtSetInformationFile(SafeFileHandle handle,
      out IoStatus io, IntPtr info, uint length, int infoClass);
    [DllImport("ntdll.dll")] static extern int NtQueryDirectoryFile(SafeFileHandle handle,
      IntPtr evt, IntPtr apc, IntPtr context, out IoStatus io, IntPtr info, uint length,
      int infoClass, [MarshalAs(UnmanagedType.U1)] bool single, IntPtr name,
      [MarshalAs(UnmanagedType.U1)] bool restart);
    public sealed class Entry { public string Name; public bool PSIsContainer; public FileAttributes Attributes; }
    static void Check(int status, string operation) {
      if (status < 0) throw new Win32Exception((int)RtlNtStatusToDosError(status), operation + " NTSTATUS=0x" + unchecked((uint)status).ToString("X8"));
    }
    public static void CheckOrdinary(SafeFileHandle h, bool directory) {
      Info i;
      if (!GetFileInformationByHandle(h, out i)) throw new Win32Exception(Marshal.GetLastWin32Error());
      if ((i.Attributes & 0x400u) != 0 || ((i.Attributes & 0x10u) != 0) != directory) {
        throw new IOException("Recovery handle is redirected or has the wrong type");
      }
      if (!directory && i.Links != 1) throw new IOException("Recovery file has multiple hard links");
    }
    static void Leaf(string name) {
      if (String.IsNullOrEmpty(name) || name=="." || name==".." || name.IndexOfAny(new char[]{'\\','/',':','\0'})>=0 ||
          name.EndsWith(".") || name.EndsWith(" ")) throw new IOException("Unsafe recovery leaf");
    }
    public static SafeFileHandle Relative(SafeFileHandle parent, string name, bool directory, bool create, bool rename) {
      return RelativeCore(parent,name,directory,create,rename,false);
    }
    static SafeFileHandle RelativeCore(SafeFileHandle parent, string name, bool directory, bool create, bool rename, bool list) {
      if(!(list && directory && name=="")) Leaf(name);
      CheckOrdinary(parent, true);
      IntPtr chars=Marshal.StringToHGlobalUni(name), uni=IntPtr.Zero;
      try {
        var u=new UnicodeString { Length=checked((ushort)(name.Length*2)), MaximumLength=checked((ushort)(name.Length*2+2)), Buffer=chars };
        uni=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(UnicodeString))); Marshal.StructureToPtr(u,uni,false);
        var a=new ObjectAttributes { Length=Marshal.SizeOf(typeof(ObjectAttributes)), RootDirectory=parent.DangerousGetHandle(),
          ObjectName=uni, Attributes=0x1040 }; // OBJ_DONT_REPARSE | OBJ_CASE_INSENSITIVE
        SafeFileHandle h; IoStatus io;
        // Traversing an ancestor does not require listing that ancestor. In
        // WpSystem these rights legitimately differ; only enumeration asks LIST.
        uint access=directory ? (list?0x001000a1u:0x001000a0u) : (create ? 0xc0110000u : (rename ? 0x80110000u : 0x80100000u));
        int s=NtCreateFile(out h,access,ref a,out io,IntPtr.Zero,0x80,directory?3u:1u,
          create?(directory?3u:2u):1u,0x00200000u|0x20u|(directory?1u:0x40u),IntPtr.Zero,0);
        GC.KeepAlive(parent);
        if(s<0) { if(h!=null) h.Dispose(); Check(s,"Cannot open relative recovery leaf " + name); }
        try { CheckOrdinary(h,directory); return h; } catch { h.Dispose(); throw; }
      } finally { if(uni!=IntPtr.Zero) Marshal.FreeHGlobal(uni); Marshal.FreeHGlobal(chars); }
    }
    public static List<IDisposable> LockDirectories(string path, bool create) {
      if(path.Length<3 || path[1]!=':' || path[2]!='\\' || path.IndexOf('/',0)>=0) throw new IOException("Not a local DOS recovery path");
      var result=new List<IDisposable>();
      try {
        var root=CreateFileW("\\\\?\\"+path.Substring(0,3),0x001000a0,3,IntPtr.Zero,3,0x02200000,IntPtr.Zero);
        if(root.IsInvalid) { root.Dispose(); throw new Win32Exception(Marshal.GetLastWin32Error()); }
        result.Add(root); CheckOrdinary(root,true); var parent=root;
        foreach(string part in path.Substring(3).TrimEnd('\\').Split(new char[]{'\\'},StringSplitOptions.RemoveEmptyEntries)) {
          var child=Relative(parent,part,true,create,false); result.Add(child); parent=child;
        }
        return result;
      } catch { foreach(var h in result) h.Dispose(); throw; }
    }
    static SafeFileHandle Last(List<IDisposable> dirs) { return (SafeFileHandle)dirs[dirs.Count-1]; }
    static void Close(List<IDisposable> dirs) { foreach(var h in dirs) h.Dispose(); }
    public static SafeFileHandle Open(string path, bool directory) {
      if(directory) {
        var dirs=LockDirectories(path,false); var h=Last(dirs); dirs.RemoveAt(dirs.Count-1); Close(dirs); return h;
      }
      var parents=LockDirectories(Path.GetDirectoryName(path),false);
      try { return Relative(Last(parents),Path.GetFileName(path),false,false,false); } finally { Close(parents); }
    }
    public static bool Exists(string path, bool directory) {
      try { using(var h=Open(path,directory)) return true; }
      catch(Win32Exception e) { if(e.NativeErrorCode==2 || e.NativeErrorCode==3) return false; throw; }
    }
    public static FileStream CreateNew(string path) {
      var dirs=LockDirectories(Path.GetDirectoryName(path),false);
      try { return new FileStream(Relative(Last(dirs),Path.GetFileName(path),false,true,false),FileAccess.ReadWrite); }
      finally { Close(dirs); }
    }
    public static FileStream OpenReadChild(SafeFileHandle parent,string leaf) { return new FileStream(Relative(parent,leaf,false,false,false),FileAccess.Read); }
    public static FileStream CreateNewChild(SafeFileHandle parent,string leaf) { return new FileStream(Relative(parent,leaf,false,true,false),FileAccess.ReadWrite); }
    public static string Hash(FileStream stream) {
      stream.Position=0;
      using(var hash=SHA256.Create()) return BitConverter.ToString(hash.ComputeHash(stream)).Replace("-","").ToLowerInvariant();
    }
    public static void Rename(SafeFileHandle source, SafeFileHandle parent, string leaf, bool replace) {
      Leaf(leaf); CheckOrdinary(source,false); CheckOrdinary(parent,true);
      // Native FILE_RENAME_INFORMATION supports a stable RootDirectory handle.
      // Only a single basename is accepted; no absolute path is reparsed here.
      int rootOffset=IntPtr.Size==8?8:4, lengthOffset=rootOffset+IntPtr.Size, nameOffset=lengthOffset+4;
      byte[] name=Encoding.Unicode.GetBytes(leaf); int length=nameOffset+name.Length+2;
      IntPtr buffer=Marshal.AllocHGlobal(length);
      try {
        for(int i=0;i<length;i++) Marshal.WriteByte(buffer,i,0);
        Marshal.WriteByte(buffer,0,replace?(byte)1:(byte)0); Marshal.WriteIntPtr(buffer,rootOffset,parent.DangerousGetHandle());
        Marshal.WriteInt32(buffer,lengthOffset,name.Length); Marshal.Copy(name,0,IntPtr.Add(buffer,nameOffset),name.Length);
        IoStatus io; Check(NtSetInformationFile(source,out io,buffer,(uint)length,10),"Cannot rename recovery file relative to pinned directory");
        CheckOrdinary(parent,true); CheckOrdinary(source,false);
      } finally { Marshal.FreeHGlobal(buffer); }
    }
    public static void MoveChecked(string source, string target, string hash, bool replace) {
      var src=LockDirectories(Path.GetDirectoryName(source),false);
      List<IDisposable> dst=null;
      try {
        dst=LockDirectories(Path.GetDirectoryName(target),false);
        using(var stream=new FileStream(Relative(Last(src),Path.GetFileName(source),false,false,true),FileAccess.Read)) {
          if(hash!=null && !String.Equals(Hash(stream),hash,StringComparison.OrdinalIgnoreCase)) throw new IOException("Recovery rename source hash changed");
          Rename(stream.SafeFileHandle,Last(dst),Path.GetFileName(target),replace);
        }
      } finally { Close(src); if(dst!=null) Close(dst); }
    }
    public static Entry[] Entries(string path) {
      var dirs=LockDirectories(path,false);
      try { return EntriesHandle(Last(dirs)); } finally { Close(dirs); }
    }
    public static Entry[] EntriesHandle(SafeFileHandle directory) {
      // Empty relative name reopens precisely the captured directory object,
      // not a mutable absolute path, with LIST_DIRECTORY for enumeration only.
      using(var listing=RelativeCore(directory,"",true,false,false,true)) {
        Info before,after;
        if(!GetFileInformationByHandle(directory,out before) || !GetFileInformationByHandle(listing,out after)) throw new Win32Exception(Marshal.GetLastWin32Error());
        if(before.VolumeSerial!=after.VolumeSerial || before.IndexHigh!=after.IndexHigh || before.IndexLow!=after.IndexLow) throw new IOException("Enumeration directory identity changed");
        return EnumerateHandle(listing);
      }
    }
    static Entry[] EnumerateHandle(SafeFileHandle directory) {
      var list=new List<Entry>(); IntPtr buffer=Marshal.AllocHGlobal(65536);
      try {
        bool restart=true;
        while(true) {
          CheckOrdinary(directory,true); IoStatus io;
          int s=NtQueryDirectoryFile(directory,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,out io,buffer,65536,1,true,IntPtr.Zero,restart);
          restart=false; if(unchecked((uint)s)==0x80000006u) break; Check(s,"Cannot enumerate recovery directory");
          int bytes=Marshal.ReadInt32(buffer,60); if(bytes<0 || bytes>65536-64 || (bytes&1)!=0) throw new IOException("Invalid directory entry size");
          string name=Marshal.PtrToStringUni(IntPtr.Add(buffer,64),bytes/2); if(name=="." || name=="..") continue; Leaf(name);
          uint attributes=unchecked((uint)Marshal.ReadInt32(buffer,56));
          list.Add(new Entry {Name=name,PSIsContainer=(attributes&0x10)!=0,Attributes=(FileAttributes)attributes});
        }
        return list.ToArray();
      } finally { Marshal.FreeHGlobal(buffer); }
    }
    public static FileStream OpenRead(string path) { return new FileStream(Open(path, false), FileAccess.Read); }
  }
}
'@ -ErrorAction Stop | Out-Null
}

function ConvertTo-CodexRecoveryLocalPath([string]$Path) {
  if ([string]::IsNullOrWhiteSpace($Path) -or $Path -notmatch '^[A-Za-z]:\\' -or
      $Path.Substring(2).Contains(':') -or $Path.Contains('/')) { throw 'Recovery requires an absolute local DOS path' }
  foreach ($part in $Path.Substring(3).Split('\')) {
    if ($part -in @('.', '..') -or $part.EndsWith('.') -or $part.EndsWith(' ')) { throw 'Recovery path contains an ambiguous component' }
  }
  return [IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Assert-CodexRecoveryRelativePath([string]$Path) {
  if ([string]::IsNullOrWhiteSpace($Path) -or [IO.Path]::IsPathRooted($Path) -or
      $Path.IndexOfAny([char[]]'/:') -ge 0 -or $Path.IndexOf([char]0) -ge 0) { throw 'Unsafe relative recovery path' }
  foreach ($part in $Path.Split('\')) {
    if ($part -in @('', '.', '..') -or $part.EndsWith('.') -or $part.EndsWith(' ') -or
        $part -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)' -or
        $part.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0) { throw 'Unsafe relative recovery component' }
  }
}

function Lock-CodexRecoveryDirectory([string]$Path, [bool]$Create = $false) {
  $full = ConvertTo-CodexRecoveryLocalPath $Path
  if ($full.Length -eq 2) { $full += '\' }
  return ,([CodexAppxRecovery.NativeFiles]::LockDirectories($full, $Create))
}

function Close-CodexRecoveryHandles($Handles) {
  if ($null -ne $Handles) { foreach ($handle in $Handles) { $handle.Dispose() } }
}

function Get-CodexRecoveryHash([string]$Path) {
  $locks = Lock-CodexRecoveryDirectory ([IO.Path]::GetDirectoryName($Path))
  $stream = $null; $sha = $null
  try {
    $stream = [CodexAppxRecovery.NativeFiles]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
  } finally { if ($sha) { $sha.Dispose() }; if ($stream) { $stream.Dispose() }; Close-CodexRecoveryHandles $locks }
}

function Get-CodexRecoveryTextHash([string]$Text) {
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))).Replace('-', '').ToLowerInvariant() }
  finally { $sha.Dispose() }
}

function Get-CodexRecoveryContext([string]$UsbRoot, $UsbVolume) {
  $root = (ConvertTo-CodexRecoveryLocalPath $UsbRoot) + '\'
  if ($root -notmatch '^[A-Za-z]:\\$') { throw 'Recovery USB root must be a drive root' }
  $name = [string]$UsbVolume.Name
  if ($name -notmatch '(?i)^\\\\\?\\Volume\{(?<id>[0-9a-f-]{36})\}\\?$') { throw 'Recovery USB has no canonical volume identity' }
  $usbId = ([guid]$Matches.id).ToString('D')
  $physical = @(Get-Volume -DriveLetter $root.Substring(0, 1) -ErrorAction Stop)
  if ($physical.Count -ne 1 -or [string]$physical[0].FileSystem -ine 'NTFS') { throw 'Recovery USB is not an available NTFS volume' }
  foreach ($idPath in @([string]$physical[0].UniqueId, [string]$physical[0].Path)) {
    if ($idPath -notmatch '(?i)^\\\\\?\\Volume\{(?<id>[0-9a-f-]{36})\}\\?$' -or ([guid]$Matches.id).ToString('D') -ine $usbId) { throw 'Recovery USB physical identity changed' }
  }
  if ([bool]$UsbVolume.IsOffline -or [bool]$UsbVolume.IsSystemVolume -or
      ([string]$UsbVolume.PackageStorePath).TrimEnd('\') -ine (Join-Path $root 'WindowsApps')) { throw 'Recovery USB AppX store is not the verified online non-system store' }
  $systems = @(Get-AppxVolume -Online -ErrorAction Stop | Where-Object { $_.IsSystemVolume })
  if ($systems.Count -ne 1 -or [bool]$systems[0].IsOffline) { throw 'Recovery requires one online system AppX volume' }
  $systemRoot = ([string]$env:SystemDrive).TrimEnd('\') + '\'
  if ($systemRoot -notmatch '^[A-Za-z]:\\$' -or $systemRoot -ieq $root -or
      [IO.Path]::GetPathRoot([string]$systems[0].PackageStorePath) -ine $systemRoot) { throw 'System AppX volume does not match Windows system drive' }
  $systemName = [string]$systems[0].Name
  if ($systemName -notmatch '(?i)^\\\\\?\\Volume\{(?<id>[0-9a-f-]{36})\}\\?$') { throw 'System AppX volume identity unavailable' }
  $systemId = ([guid]$Matches.id).ToString('D')
  $local = ConvertTo-CodexRecoveryLocalPath ([string]$env:LOCALAPPDATA)
  if ([IO.Path]::GetPathRoot($local) -ine $systemRoot) { throw 'LocalAppData is not on the Windows system volume' }
  return @{
    sid = [string][Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    usbId = $usbId; usbName = $name; usbRoot = $root; systemId = $systemId; systemName = $systemName; systemRoot = $systemRoot
    dataRoot = Join-Path (Join-Path $local 'Packages') 'OpenAI.Codex_2p2nqsd0c76g0'
    backupRoot = Join-Path (Join-Path $local 'CodexAppxDataBackups') 'OpenAI.Codex_2p2nqsd0c76g0'
  }
}

function Get-CodexRecoveryNativeVolumes($Context) {
  Add-Type -AssemblyName System.Runtime.WindowsRuntime -ErrorAction Stop
  $pm = [Windows.Management.Deployment.PackageManager,Windows,ContentType=WindowsRuntime]::new()
  $usb = $pm.FindPackageVolume([string]$Context.usbName)
  $system = $pm.FindPackageVolume([string]$Context.systemName)
  if ($null -eq $usb -or $null -eq $system -or $usb.IsOffline -or $system.IsOffline -or
      $usb.IsSystemVolume -or -not $system.IsSystemVolume -or
      ([string]$usb.PackageStorePath).TrimEnd('\') -ine (Join-Path $Context.usbRoot 'WindowsApps')) { throw 'Native AppX volume binding changed' }
  return @{ manager = $pm; usb = $usb; system = $system }
}

function ConvertTo-CodexRecoveryDeploymentTask($Operation) {
  $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and $_.GetGenericArguments().Count -eq 2 -and $_.GetParameters().Count -eq 1
  } | Select-Object -First 1
  if ($null -eq $asTask) { throw 'WinRT deployment awaiter is unavailable' }
  $closed = $asTask.MakeGenericMethod([Windows.Management.Deployment.DeploymentResult], [Windows.Management.Deployment.DeploymentProgress])
  return $closed.Invoke($null, @($Operation))
}

function Get-CodexRecoveryFailureText($Failure) {
  $parts = @([string]$Failure, [string]$Failure.Exception.Message)
  if ($Failure.ErrorDetails -and $Failure.ErrorDetails.Message) { $parts += [string]$Failure.ErrorDetails.Message }
  $exception = $Failure.Exception
  for ($depth=0; $null -ne $exception -and $depth -lt 6; $depth++) {
    $parts += [string]$exception.Message
    if ($exception -is [Runtime.InteropServices.COMException]) { $parts += ('HRESULT=0x{0:X8}' -f [int]$exception.HResult) }
    $exception = $exception.InnerException
  }
  return (@($parts | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique) -join ' | ')
}

function Wait-CodexRecoveryDeployment($Operation) {
  try {
    $task = ConvertTo-CodexRecoveryDeploymentTask $Operation
    if ($null -eq $task) { throw 'WinRT deployment returned no awaitable task' }
    $result = $task.GetAwaiter().GetResult()
  } catch {
    $detail = Get-CodexRecoveryFailureText $_
    # Failed/cancelled WinRT operations may throw before DeploymentResult is
    # available. Preserve their error/status without inventing a result code.
    try {
      if ($Operation.ErrorCode) { $detail += ('; operationErrorCode=0x{0:X8}: {1}' -f [int]$Operation.ErrorCode.HResult, [string]$Operation.ErrorCode.Message) }
      if ($null -ne $Operation.Status) { $detail += '; asyncStatus=' + [string]$Operation.Status }
    } catch { }
    throw $detail
  }
  if ($null -eq $result) { throw 'WinRT deployment completed without a DeploymentResult' }
  if ($result.ExtendedErrorCode -and [int]$result.ExtendedErrorCode.HResult -ne 0) {
    throw ('AppX recovery deployment failed HRESULT=0x{0:X8} ActivityId={1}: {2}' -f [int]$result.ExtendedErrorCode.HResult, [string]$result.ActivityId, [string]$result.ErrorText)
  }
  return $result
}

function Get-CodexRecoveryDeploymentSpaceBudget([string]$PackagePath) {
  # Central directory only: never extract the signed payload to measure it.
  Add-Type -AssemblyName System.IO.Compression -ErrorAction Stop
  $stream = [CodexAppxRecovery.NativeFiles]::OpenRead($PackagePath)
  $archive = $null
  try {
    $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true)
    [decimal]$required = 512 * 1024 * 1024
    foreach ($entry in $archive.Entries) {
      $required += [decimal]$entry.Length + 4096
      if ($required -gt [decimal][uint64]::MaxValue) { throw 'MSIX expanded space budget overflow' }
    }
    return [uint64]$required
  } finally {
    if ($null -ne $archive) { $archive.Dispose() }
    $stream.Dispose()
  }
}

function Get-CodexRecoveryUsbFreeBytes($Context) {
  return [uint64]([IO.DriveInfo]::new([string]$Context.usbRoot).AvailableFreeSpace)
}

function Assert-CodexRecoveryDeploymentSpace([string]$PackagePath, $Context) {
  $required = Get-CodexRecoveryDeploymentSpaceBudget $PackagePath
  $free = Get-CodexRecoveryUsbFreeBytes $Context
  if ($free -lt $required) {
    throw "Insufficient USB deployment space: target=$($Context.usbRoot) requiredBytes=$required availableBytes=$free; no package removal or new Stage was started by this space gate"
  }
}

function Invoke-CodexRecoveryStage([string]$PackagePath, $Context) {
  $native = Get-CodexRecoveryNativeVolumes $Context
  $uri = [Uri]::new($PackagePath, [UriKind]::Absolute)
  if (-not $uri.IsFile) { throw 'Recovery MSIX URI must be a local file' }
  $Context.stageInvokedThisRun = $true
  $operation = $native.manager.StagePackageAsync($uri, $null, [Windows.Management.Deployment.DeploymentOptions]::None, $native.usb)
  $result = Wait-CodexRecoveryDeployment $operation
  return @{ activityId = [string]$result.ActivityId; operation = 'StagePackageAsync USB' }
}

function Get-CodexRecoveryStagedPackage([string]$FullName, $Context) {
  $native = Get-CodexRecoveryNativeVolumes $Context
  # A globally staged copy (for example on C:) is not evidence that this USB
  # has the package. Only the bound native PackageVolume may satisfy this probe.
  $family = 'OpenAI.Codex_2p2nqsd0c76g0'
  $Context.packageProbe = @{ source='bound PackageVolume.FindPackages(PFN)'; expectedFullName=$FullName; expectedPath=(Join-Path (Join-Path $Context.usbRoot 'WindowsApps') $FullName); targetVolume=$Context.usbId; targetMatches=$null; actualPaths=@() }
  try {
    $packages = @($native.usb.FindPackages($family) | Where-Object { [string]$_.Id.FullName -ieq $FullName })
  } catch {
    $Context.packageProbe.targetQueryError = Get-CodexRecoveryFailureText $_
    throw
  }
  $Context.packageProbe.targetMatches = $packages.Count
  if ($packages.Count -eq 0) {
    # Diagnostics only. Neither another volume nor an unavailable global query
    # authorizes moving/removing a package or counts as a successful USB stage.
    try {
      $Context.packageProbe.globalMatchingCandidates = @($native.manager.FindPackages($family) | Where-Object { [string]$_.Id.FullName -ieq $FullName } | ForEach-Object { [string]$_.InstalledLocation.Path })
    } catch { $Context.packageProbe.globalQueryError = Get-CodexRecoveryFailureText $_ }
    return $null
  }
  $Context.packageProbe.actualPaths = @($packages | ForEach-Object { [string]$_.InstalledLocation.Path })
  if ($packages.Count -ne 1) { throw 'Exact staged package is ambiguous' }
  $package = $packages[0]
  $Context.packageProbe.healthy = [bool]$package.Status.VerifyIsOK()
  if (-not $Context.packageProbe.healthy) { throw 'Exact staged package is not healthy' }
  return [pscustomobject]@{ PackageFullName = [string]$package.Id.FullName; PackageFamilyName = [string]$package.Id.FamilyName; InstallLocation = [string]$package.InstalledLocation.Path }
}

function New-CodexRecoveryRegistrationOptions($Native) {
  $options = [Windows.Management.Deployment.RegisterPackageOptions,Windows,ContentType=WindowsRuntime]::new()
  $options.AppDataVolume = $Native.system
  return $options
}

function Invoke-CodexRecoveryRegister([string]$FullName, $Context) {
  $native = Get-CodexRecoveryNativeVolumes $Context
  $options = New-CodexRecoveryRegistrationOptions $native
  # Exact full name prevents a newer globally staged family version being chosen.
  # Neither developer mode nor unsigned-package exceptions are enabled.
  $Context.registerInvokedThisRun = $true
  $operation = $native.manager.RegisterPackagesByFullNameAsync([string[]]@($FullName), $options)
  $result = Wait-CodexRecoveryDeployment $operation
  # On some Windows 10 builds DeploymentResult.IsRegistered can still be false
  # after the OnDemandRegister operation has successfully committed PackageUser.
  # It is therefore diagnostic-only. The caller performs an independent,
  # bounded current-user/identity/health/USB/ownership postcondition below.
  $isRegisteredHint = $null
  try {
    if ($result.PSObject.Properties.Name -contains 'IsRegistered') {
      $isRegisteredHint = [bool]$result.IsRegistered
    }
  } catch { }
  return @{
    activityId = [string]$result.ActivityId
    operation = 'RegisterPackagesByFullNameAsync system AppDataVolume'
    isRegisteredHint = $isRegisteredHint
  }
}

$script:CodexRecoveryRegistrationPollAttempts = 240
$script:CodexRecoveryRegistrationPollDelayMs = 250

function Wait-CodexRecoveryRegistrationPostcondition(
    [string]$FullName,
    [string]$AllowedPreviousFullName,
    $Context,
    $Callbacks,
    $NativeResult = $null) {
  $lastObservation = 'current-user package has not been queried'
  $observedFullName = $null
  for ($attempt = 1; $attempt -le $script:CodexRecoveryRegistrationPollAttempts; $attempt++) {
    Assert-CodexRecoveryContextUnchanged $Context
    $candidate = $null
    try {
      $candidate = & $Callbacks.GetPackage
    } catch {
      # A read failure is never treated as absence or success. Retrying only
      # this read-only observation tolerates AppX projection convergence while
      # preserving the last concrete error if the bounded wait expires.
      $lastObservation = 'current-user query failed: ' + (Get-CodexRecoveryFailureText $_)
    }
    if ($null -eq $candidate) {
      if ($lastObservation -notmatch '^current-user query failed:') {
        $lastObservation = 'current-user package is not visible yet'
      }
    } else {
      $observedFullName = [string]$candidate.PackageFullName
      if ($observedFullName -ieq $FullName) {
        try {
          Assert-CodexRecoveryPackage $candidate $FullName $Context
          if ($candidate.PSObject.Properties.Name -contains 'Status' -and
              -not [string]::IsNullOrWhiteSpace([string]$candidate.Status) -and
              [string]$candidate.Status -ine 'Ok') {
            throw ('current-user package status is ' + [string]$candidate.Status)
          }
          if ($candidate.PSObject.Properties.Name -contains 'IsPartiallyStaged' -and
              [bool]$candidate.IsPartiallyStaged) {
            throw 'current-user package remains partially staged'
          }
          & $Callbacks.AssertOwnership $candidate | Out-Null
          $Context.registrationProbe = @{
            state = 'observed'
            attempts = $attempt
            expectedFullName = $FullName
            observedFullName = $observedFullName
            status = [string]$candidate.Status
            isPartiallyStaged = $(if ($candidate.PSObject.Properties.Name -contains 'IsPartiallyStaged') { [bool]$candidate.IsPartiallyStaged } else { $null })
            nativeIsRegisteredHint = $(if ($NativeResult -and (Test-CodexRecoveryProperty $NativeResult 'isRegisteredHint')) { $NativeResult.isRegisteredHint } else { $null })
          }
          return $candidate
        } catch {
          $lastObservation = 'exact package is visible but its postcondition is not ready: ' + (Get-CodexRecoveryFailureText $_)
        }
      } elseif (-not [string]::IsNullOrWhiteSpace($AllowedPreviousFullName) -and
                $observedFullName -ieq $AllowedPreviousFullName) {
        $lastObservation = 'previous current-user package is still visible during registration convergence: ' + $observedFullName
      } else {
        throw "An unexpected current-user package appeared during registration convergence: expected=$FullName observed=$observedFullName"
      }
    }
    if ($attempt -lt $script:CodexRecoveryRegistrationPollAttempts) {
      Start-Sleep -Milliseconds $script:CodexRecoveryRegistrationPollDelayMs
    }
  }
  $hint = if ($NativeResult -and (Test-CodexRecoveryProperty $NativeResult 'isRegisteredHint')) {
    [string]$NativeResult.isRegisteredHint
  } else { '<unavailable>' }
  $activity = if ($NativeResult -and (Test-CodexRecoveryProperty $NativeResult 'activityId')) {
    [string]$NativeResult.activityId
  } else { '<resume/no-new-native-call>' }
  $Context.registrationProbe = @{
    state = 'timed-out'
    attempts = $script:CodexRecoveryRegistrationPollAttempts
    expectedFullName = $FullName
    observedFullName = $observedFullName
    nativeIsRegisteredHint = $hint
    lastObservation = $lastObservation
    activityId = $activity
  }
  throw "Native registration completed or was resumed, but the exact current-user AppX postcondition was not observed within the bounded convergence window; expectedFullName=$FullName observedFullName=$observedFullName IsRegisteredHint=$hint ActivityId=$activity lastObservation=$lastObservation"
}

function Assert-CodexRecoveryPackage($Package, [string]$FullName, $Context, [bool]$RequireUsb = $true) {
  $expectedPath = Join-Path (Join-Path $Context.usbRoot 'WindowsApps') $FullName
  $actualPath = if ($null -eq $Package) { '<none>' } else { [string]$Package.InstallLocation }
  if ($null -eq $Package -or [string]$Package.PackageFullName -ine $FullName -or
      [string]$Package.PackageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0') { throw "Recovery package identity changed or is missing; expectedFullName=$FullName expectedPath=$expectedPath actualPath=$actualPath" }
  if ($RequireUsb) {
    $path = [string]$Package.InstallLocation
    if ($path.StartsWith('\\?\')) { $path = $path.Substring(4) }
    try { $normalizedPath = ConvertTo-CodexRecoveryLocalPath $path }
    catch { throw "Recovery package path is invalid; expectedPath=$expectedPath actualPath=$actualPath; $(Get-CodexRecoveryFailureText $_)" }
    if ($normalizedPath -ine $expectedPath) {
      $locationInfo = Get-CodexPackagePathInfo $normalizedPath $FullName (Join-Path $Context.usbRoot 'WindowsApps')
      $Context.packagePathEvidence = $locationInfo
      if ([string]$locationInfo.effectivePath -ine $expectedPath) {
        throw "Recovery package is not staged/registered on the bound USB volume; expectedPath=$expectedPath actualPath=$actualPath packagePathEvidence=$($locationInfo | ConvertTo-Json -Compress)"
      }
    }
  }
}

function Invoke-CodexRecoveryRecordedDeployment([string]$Kind, [string]$Payload, [string]$FullName, $Journal, $Context) {
  $operation = if ($Kind -eq 'stage') { 'StagePackageAsync USB' } elseif ($Kind -eq 'register') { 'RegisterPackagesByFullNameAsync system AppDataVolume' } else { throw 'Unknown recovery deployment operation' }
  # Persist the native result separately from the later placement/registration
  # postcondition. Stage callers discard the return value; Register callers use
  # only its diagnostic hint while proving the current-user package independently.
  $Journal.lastDeployment = @{ operation=$operation; phase=[string]$Journal.phase; state='started'; nativeInvoked=$false; activityId=$null; hresult=$null; errorText=$null; isRegisteredHint=$null; postconditionState=$(if($Kind -eq 'register'){'not-started'}else{'not-applicable'}) }
  Write-CodexRecoveryJournal $Journal $Context
  try {
    $result = if ($Kind -eq 'stage') { Invoke-CodexRecoveryStage $Payload $Context } else { Invoke-CodexRecoveryRegister $FullName $Context }
  } catch {
    $detail = Get-CodexRecoveryFailureText $_
    $Journal.lastDeployment.state = 'failed'; $Journal.lastDeployment.errorText = $detail
    $Journal.lastDeployment.nativeInvoked = if ($Kind -eq 'stage') { [bool]$Context.stageInvokedThisRun } else { [bool]$Context.registerInvokedThisRun }
    if ($detail -match '(?i)(0x[0-9a-f]{8})') { $Journal.lastDeployment.hresult = $Matches[1] }
    if ($detail -match '(?i)Activity(?:Id| ID)[^0-9a-f]*([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})') { $Journal.lastDeployment.activityId = $Matches[1] }
    try { Write-CodexRecoveryJournal $Journal $Context }
    catch { $detail += '; failure journal write also failed: ' + (Get-CodexRecoveryFailureText $_) }
    throw $detail
  }
  $Journal.lastDeployment.state = 'completed'; $Journal.lastDeployment.activityId = [string]$result.activityId
  $Journal.lastDeployment.nativeInvoked = if ($Kind -eq 'stage') { [bool]$Context.stageInvokedThisRun } else { [bool]$Context.registerInvokedThisRun }
  if ($Kind -eq 'register') {
    $Journal.lastDeployment.postconditionState = 'pending-current-user-query'
    if (Test-CodexRecoveryProperty $result 'isRegisteredHint') {
      $Journal.lastDeployment.isRegisteredHint = $result.isRegisteredHint
    }
  }
  # Persist the native outcome BEFORE querying/validating package placement.
  # A later wrong-path error must not leave lastDeployment=null on disk.
  try { Write-CodexRecoveryJournal $Journal $Context }
  catch { throw ("Deployment completed but result journal write failed; operation=$operation ActivityId=$($result.activityId); " + (Get-CodexRecoveryFailureText $_)) }
  return $result
}

function Read-CodexRecoveryBackup([string]$BackupPath, $Context, $Binding = $null) {
  if ([string]::IsNullOrWhiteSpace($BackupPath)) { return $null }
  $path = ConvertTo-CodexRecoveryLocalPath $BackupPath
  if ([IO.Path]::GetDirectoryName($path) -ine $Context.backupRoot -or [IO.Path]::GetFileName($path) -notmatch '^[0-9a-fA-F]{32}$') { throw 'Recovery backup is outside the fixed package backup root' }
  $manifestPath = $path + '.manifest.json'
  $manifestHash = Get-CodexRecoveryHash $manifestPath
  $manifestLocks = Lock-CodexRecoveryDirectory $Context.backupRoot
  $manifestStream = $null; $reader = $null
  try {
    $manifestStream = [CodexAppxRecovery.NativeFiles]::OpenRead($manifestPath)
    if ($manifestStream.Length -gt 32MB) { throw 'Recovery backup manifest is too large' }
    $reader = New-Object IO.StreamReader($manifestStream, [Text.Encoding]::UTF8, $true)
    $manifest = $reader.ReadToEnd() | ConvertFrom-Json -ErrorAction Stop
  } finally { if ($reader) { $reader.Dispose() } elseif ($manifestStream) { $manifestStream.Dispose() }; Close-CodexRecoveryHandles $manifestLocks }
  if ([string]$manifest.packageFamilyName -ine 'OpenAI.Codex_2p2nqsd0c76g0' -or
      (ConvertTo-CodexRecoveryLocalPath ([string]$manifest.source)) -ine $Context.dataRoot) { throw 'Backup manifest belongs to a different user/package source' }
  if ($null -eq $manifest.PSObject.Properties['files'] -or $null -eq $manifest.PSObject.Properties['directories'] -or
      $null -eq $manifest.PSObject.Properties['appxVolumes']) { throw 'Recovery requires a complete current-format backup manifest' }
  $files = @{}; $directories = @{}; $treeLines = New-Object 'System.Collections.Generic.List[string]'
  foreach ($property in @($manifest.directories.PSObject.Properties)) {
    $relative = [string]$property.Name; Assert-CodexRecoveryRelativePath $relative
    if ($directories.ContainsKey($relative)) { throw 'Duplicate backup directory' }
    $directories[$relative] = $true; $treeLines.Add('D|' + $relative.ToLowerInvariant())
  }
  foreach ($record in @($manifest.files)) {
    $relative = [string]$record.path; Assert-CodexRecoveryRelativePath $relative
    $hash = [string]$record.sha256
    if ($hash -notmatch '^[0-9a-fA-F]{64}$' -or $files.ContainsKey($relative) -or $directories.ContainsKey($relative)) { throw 'Invalid or duplicate backup file record' }
    $files[$relative] = $hash.ToLowerInvariant(); $treeLines.Add('F|' + $relative.ToLowerInvariant() + '|' + $hash.ToLowerInvariant())
  }
  foreach ($relative in @($files.Keys) + @($directories.Keys)) {
    $parent = [IO.Path]::GetDirectoryName($relative)
    if ($parent -and -not $directories.ContainsKey($parent)) { throw 'Backup manifest omits a parent directory' }
  }
  $locks = Lock-CodexRecoveryDirectory $path
  $queue = New-Object 'System.Collections.Generic.Queue[string]'; $queue.Enqueue('')
  $actualFiles = 0; $actualDirectories = 0
  try {
    while ($queue.Count) {
      $relativeParent = $queue.Dequeue(); $directory = if ($relativeParent) { Join-Path $path $relativeParent } else { $path }
      $directoryLocks = Lock-CodexRecoveryDirectory $directory
      try {
        foreach ($entry in @([CodexAppxRecovery.NativeFiles]::Entries($directory))) {
          if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Recovery backup contains redirected content' }
          $relative = if ($relativeParent) { Join-Path $relativeParent $entry.Name } else { [string]$entry.Name }
          Assert-CodexRecoveryRelativePath $relative
          if ($entry.PSIsContainer) {
            if (-not $directories.ContainsKey($relative)) { throw 'Recovery backup contains an unlisted directory' }
            $actualDirectories++; $queue.Enqueue($relative)
          } else {
            if (-not $files.ContainsKey($relative) -or (Get-CodexRecoveryHash (Join-Path $directory $entry.Name)) -ine $files[$relative]) { throw 'Recovery backup content does not match its manifest' }
            $actualFiles++
          }
        }
      } finally { Close-CodexRecoveryHandles $directoryLocks }
    }
  } finally { Close-CodexRecoveryHandles $locks }
  if ($actualFiles -ne $files.Count -or $actualDirectories -ne $directories.Count -or (Get-CodexRecoveryHash $manifestPath) -ine $manifestHash) { throw 'Recovery backup inventory changed or is incomplete' }
  $treeHash = Get-CodexRecoveryTextHash ((@($treeLines | Sort-Object) -join "`n"))
  if ($Binding -and ([string]$Binding.name -cne [IO.Path]::GetFileName($path) -or [string]$Binding.manifestSha256 -ine $manifestHash -or [string]$Binding.treeSha256 -ine $treeHash)) { throw 'Recovery backup binding was modified' }
  return @{ path = $path; files = $files; directories = $directories; binding = @{ name = [IO.Path]::GetFileName($path); manifestSha256 = $manifestHash; treeSha256 = $treeHash } }
}

function Get-CodexRecoveryBoundBackup($Binding, $Context) {
  if ($null -eq $Binding) { return $null }
  if ([string]$Binding.name -notmatch '^[0-9a-fA-F]{32}$') { throw 'Invalid recovery backup identifier' }
  return Read-CodexRecoveryBackup (Join-Path $Context.backupRoot ([string]$Binding.name)) $Context $Binding
}

function Write-CodexRecoveryJournal($Journal, $Context) {
  $locks = Lock-CodexRecoveryDirectory $Context.backupRoot $true
  $path = Join-Path $Context.backupRoot 'recovery.json'
  $temp = Join-Path $Context.backupRoot ('recovery-write-' + [guid]::NewGuid().ToString('N') + '.tmp')
  $stream = $null
  try {
    $bytes = [Text.Encoding]::UTF8.GetBytes(($Journal | ConvertTo-Json -Depth 16))
    $stream = [CodexAppxRecovery.NativeFiles]::CreateNew($temp)
    $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true); $stream.Dispose(); $stream = $null
    if ([CodexAppxRecovery.NativeFiles]::Exists($path, $false)) {
      $read = [CodexAppxRecovery.NativeFiles]::OpenRead($path); $read.Dispose()
    }
    [CodexAppxRecovery.NativeFiles]::MoveChecked($temp, $path, (Get-CodexRecoveryTextHash ($Journal | ConvertTo-Json -Depth 16)), $true)
  } finally { if ($stream) { $stream.Dispose() }; Close-CodexRecoveryHandles $locks }
}

function Read-CodexRecoveryJournal($Context) {
  $path = Join-Path $Context.backupRoot 'recovery.json'
  if (-not [CodexAppxRecovery.NativeFiles]::Exists($path, $false)) {
    if ([CodexAppxRecovery.NativeFiles]::Exists(($path + '.bak'), $false)) { throw 'Recovery journal primary is missing; previous evidence retained, automatic mutation stopped' }
    return $null
  }
  $locks = Lock-CodexRecoveryDirectory $Context.backupRoot
  $stream = $null; $reader = $null
  try {
    $stream = [CodexAppxRecovery.NativeFiles]::OpenRead($path)
    if ($stream.Length -gt 32MB) { throw 'Recovery journal is too large' }
    $reader = New-Object IO.StreamReader($stream, [Text.Encoding]::UTF8, $true)
    return ($reader.ReadToEnd() | ConvertFrom-Json -ErrorAction Stop)
  } finally { if ($reader) { $reader.Dispose() } elseif ($stream) { $stream.Dispose() }; Close-CodexRecoveryHandles $locks }
}

function Test-CodexRecoveryProperty($Object, [string]$Name) {
  if ($null -eq $Object) { return $false }
  if ($Object -is [Collections.IDictionary]) { return $Object.Contains($Name) }
  return $Object.PSObject.Properties.Name -contains $Name
}

function Set-CodexRecoveryProperty($Object, [string]$Name, $Value) {
  if ($null -eq $Object) { throw 'Cannot update a null recovery record' }
  if ($Object -is [Collections.IDictionary]) { $Object[$Name] = $Value; return }
  if ($Object.PSObject.Properties.Name -contains $Name) { $Object.$Name = $Value; return }
  $Object | Add-Member -NotePropertyName $Name -NotePropertyValue $Value -Force
}

function Assert-CodexRecoveryJournal($Journal, $Context, [string]$FullName, [string]$PayloadHash) {
  if ([string]$Journal.schema -cne 'codex-appx-data-recovery' -or [int]$Journal.version -ne 1 -or
      [string]$Journal.id -notmatch '^[0-9a-f]{32}$' -or [string]$Journal.sid -cne $Context.sid -or
      [string]$Journal.fullName -cne $FullName -or [string]$Journal.usbId -ine $Context.usbId -or
      [string]$Journal.systemId -ine $Context.systemId -or [string]$Journal.payloadSha256 -ine $PayloadHash -or
      [string]$Journal.mode -notin @('fresh', 'repair', 'update') -or [string]$Journal.phase -notin @('prepared','removing','removed','staging','staged','registering','registered','baseline-captured','restoring','verifying','completed')) {
    throw 'Recovery journal context/phase does not match this verified user, payload and physical volumes'
  }
  if ($Journal.mode -eq 'update' -and ([string]$Journal.oldFullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$' -or
      [version](([string]$Journal.oldFullName).Split('_')[1]) -ge [version]($FullName.Split('_')[1]))) { throw 'Invalid previous version in nondestructive update journal' }
  if ($Journal.pendingFile -and $Journal.phase -ne 'restoring') { throw 'Pending file is inconsistent with recovery journal phase' }
  if ($Journal.PSObject.Properties['machineRepair'] -and $null -ne $Journal.machineRepair) {
    if ([string]$Journal.machineRepair.fullName -cne $FullName -or
        [string]$Journal.machineRepair.state -notin @('pending','running','failed','completed') -or
        ([string]$Journal.machineRepair.state -ne 'completed' -and [string]$Journal.phase -ne 'prepared')) {
      throw 'Machine-package repair record is inconsistent with the recovery journal'
    }
    if (Test-CodexRecoveryProperty $Journal.machineRepair 'attempts') {
      $attemptText = [string]$Journal.machineRepair.attempts
      if ($attemptText -notmatch '^\d+$' -or [int64]$attemptText -gt 8) {
        throw 'Machine-package repair attempt counter is invalid'
      }
    }
    if ((Test-CodexRecoveryProperty $Journal.machineRepair 'lastError') -and
        ([string]$Journal.machineRepair.lastError).Length -gt 4096) {
      throw 'Machine-package repair error record is oversized'
    }
  }
  if (Test-CodexRecoveryProperty $Journal 'lastDeployment') {
    $deployment = $Journal.lastDeployment
    if ($null -ne $deployment) {
      if ([string]$deployment.operation -notin @('StagePackageAsync USB', 'RegisterPackagesByFullNameAsync system AppDataVolume') -or
          [string]$deployment.state -notin @('started','failed','completed')) {
        throw 'Recovery deployment record contains an invalid operation/state'
      }
      if ((Test-CodexRecoveryProperty $deployment 'postconditionState') -and
          [string]$deployment.postconditionState -notin @('not-started','not-applicable','pending-current-user-query','observed-current-user-package','timed-out')) {
        throw 'Recovery deployment postcondition state is invalid'
      }
      if ((Test-CodexRecoveryProperty $deployment 'isRegisteredHint') -and
          $null -ne $deployment.isRegisteredHint -and $deployment.isRegisteredHint -isnot [bool]) {
        throw 'Recovery deployment registration hint is invalid'
      }
    }
  }
  if (Test-CodexRecoveryProperty $Journal 'registrationPostcondition') {
    $postcondition = $Journal.registrationPostcondition
    if ($null -eq $postcondition -or [string]$postcondition.state -cne 'observed' -or
        [string]$postcondition.expectedFullName -cne $FullName -or
        [string]$postcondition.observedFullName -cne $FullName) {
      throw 'Recovery registration postcondition does not match the exact package identity'
    }
    $attemptText = [string]$postcondition.attempts
    if ($attemptText -notmatch '^\d+$' -or [int64]$attemptText -lt 1 -or [int64]$attemptText -gt 240) {
      throw 'Recovery registration postcondition attempt counter is invalid'
    }
    if ((Test-CodexRecoveryProperty $postcondition 'nativeIsRegisteredHint') -and
        $null -ne $postcondition.nativeIsRegisteredHint -and $postcondition.nativeIsRegisteredHint -isnot [bool]) {
      throw 'Recovery registration postcondition hint is invalid'
    }
  }
  foreach ($item in @($Journal.replacements)) {
    if ($null -eq $item) { throw 'Invalid completed recovery artifact record' }
    Assert-CodexRecoveryRelativePath ([string]$item.path)
    if ([string]$item.artifact -notmatch '^[a-f0-9]{32}$' -or [string]$item.desiredSha256 -notmatch '^[a-f0-9]{64}$' -or
        ($item.beforeSha256 -and [string]$item.beforeSha256 -notmatch '^[a-f0-9]{64}$')) { throw 'Invalid completed recovery artifact binding' }
  }
}

function Get-CodexRecoveryCurrentUsbVolume($Context) {
  $current = @(Get-AppxVolume -Online -ErrorAction Stop | Where-Object {
    ([string]$_.PackageStorePath).TrimEnd('\') -ieq (Join-Path $Context.usbRoot 'WindowsApps')
  })
  if ($current.Count -ne 1) { throw 'Bound USB AppX volume disappeared or became ambiguous' }
  return $current[0]
}

function Assert-CodexRecoveryContextUnchanged($Context) {
  $current = Get-CodexRecoveryContext $Context.usbRoot (Get-CodexRecoveryCurrentUsbVolume $Context)
  foreach ($key in @('sid','usbId','usbRoot','systemId','systemName','dataRoot','backupRoot')) {
    if ([string]$current[$key] -ine [string]$Context[$key]) { throw ('Recovery physical volume/user context changed: ' + $key) }
  }
}

function Assert-CodexRecoveryData($Context, $Callbacks, [bool]$AllowContained = $false) {
  $placement = & $Callbacks.AssertDataPlacement $Context.usbRoot
  if ($AllowContained -and [string]$placement.state -eq 'contained-links-verified') {
    $pins = Lock-CodexRecoveryDirectory $Context.dataRoot
    Close-CodexRecoveryHandles $pins
    return
  }
  if ([string]$placement.state -notin @('not-created', 'system-volume-observed')) { throw 'Recovery registration has not placed ordinary package data on the system volume' }
  if ([CodexAppxRecovery.NativeFiles]::Exists($Context.dataRoot, $true)) {
    $locks = Lock-CodexRecoveryDirectory $Context.dataRoot
    try {
      $pending = New-Object 'System.Collections.Generic.Queue[string]'; $pending.Enqueue($Context.dataRoot)
      while ($pending.Count) {
        $dir = $pending.Dequeue(); $pins = Lock-CodexRecoveryDirectory $dir
        try {
          foreach ($entry in @([CodexAppxRecovery.NativeFiles]::Entries($dir))) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Recovery destination has a redirected child' }
            if ($entry.PSIsContainer) { $pending.Enqueue((Join-Path $dir $entry.Name)) }
          }
        } finally { Close-CodexRecoveryHandles $pins }
      }
    } finally { Close-CodexRecoveryHandles $locks }
  }
}

function Restore-CodexRecoveryData($Journal, $Original, $Baseline, $Context, $Callbacks) {
  if ($null -eq $Original) { return }
  $targetLocks = Lock-CodexRecoveryDirectory $Context.dataRoot $true
  $artifactRoot = Join-Path $Context.backupRoot ('recovery-files-' + [string]$Journal.id)
  $artifactLocks = Lock-CodexRecoveryDirectory $artifactRoot $true
  try {
    if ($Journal.pendingFile) {
      $pending = $Journal.pendingFile
      Assert-CodexRecoveryRelativePath ([string]$pending.path)
      if (-not $Original.files.ContainsKey([string]$pending.path) -or
          [string]$pending.desiredSha256 -ine [string]$Original.files[[string]$pending.path] -or
          [string]$pending.artifact -notmatch '^[a-f0-9]{32}$' -or
          ($pending.beforeSha256 -and [string]$pending.beforeSha256 -notmatch '^[a-f0-9]{64}$')) { throw 'Invalid pending recovery artifact binding' }
      $allowedBefore = if ($Baseline -and $Baseline.files.ContainsKey([string]$pending.path)) { [string]$Baseline.files[[string]$pending.path] } else { '' }
      if ([string]$pending.beforeSha256 -ine $allowedBefore) { throw 'Pending recovery baseline hash was modified' }
    }
    foreach ($relative in @($Original.directories.Keys | Sort-Object { $_.Length }, { $_ })) {
      Assert-CodexRecoveryRelativePath $relative
      $pins = Lock-CodexRecoveryDirectory (Join-Path $Context.dataRoot $relative) $true
      Close-CodexRecoveryHandles $pins
    }
    $batch = 0
    foreach ($relative in @($Original.files.Keys | Sort-Object)) {
      if (($batch++ % 32) -eq 0) {
        Assert-CodexRecoveryContextUnchanged $Context
        $package = & $Callbacks.GetPackage
        Assert-CodexRecoveryPackage $package $Journal.fullName $Context
        & $Callbacks.StopPackage $package | Out-Null
      }
      $target = Join-Path $Context.dataRoot $relative
      $pins = Lock-CodexRecoveryDirectory ([IO.Path]::GetDirectoryName($target)) $true
      $sourceStream = $null; $out = $null
      try {
        $desiredHash = [string]$Original.files[$relative]
        $existingHash = if ([CodexAppxRecovery.NativeFiles]::Exists($target, $false)) { Get-CodexRecoveryHash $target } else { $null }
        $baselineHash = if ($Baseline -and $Baseline.files.ContainsKey($relative)) { [string]$Baseline.files[$relative] } else { $null }
        $pending = $Journal.pendingFile
        if ($pending -and [string]$pending.path -ine $relative) {
          if ($existingHash -ieq $desiredHash) { continue }
          throw 'Unfinished recovery file must be resolved before another file changes'
        }
        if ($existingHash -ieq $desiredHash) {
          if ($pending) {
            $displacedPath = Join-Path $artifactRoot ([string]$pending.artifact + '.replaced')
            $hasDisplaced = [CodexAppxRecovery.NativeFiles]::Exists($displacedPath, $false)
            if (($pending.beforeSha256 -and -not $hasDisplaced) -or
                ($hasDisplaced -and (Get-CodexRecoveryHash $displacedPath) -ine [string]$pending.beforeSha256)) { throw 'Completed pending restore has no matching displaced evidence' }
            $Journal.replacements = @($Journal.replacements) + @($pending)
            $Journal.pendingFile = $null; Write-CodexRecoveryJournal $Journal $Context
          }
          continue
        }
        if ($existingHash -and ($null -eq $baselineHash -or $existingHash -ine $baselineHash)) { throw ('Recovery destination changed after baseline; original and new data retained: ' + $relative) }
        if (-not $existingHash -and $baselineHash -and -not $pending) { throw ('Recovery destination disappeared after baseline: ' + $relative) }
        $token = if ($pending) { [string]$pending.artifact } else { [guid]::NewGuid().ToString('N') }
        # Create in the Windows-created destination directory so that the new
        # file inherits its AppX ACL, not the ordinary backup directory ACL.
        # The single randomized leaf is transaction-bound by pending.artifact.
        $temp = Join-Path ([IO.Path]::GetDirectoryName($target)) ('.codex-recovery-' + $token + '.new')
        $replaced = Join-Path $artifactRoot ($token + '.replaced')
        if (-not $pending) {
          $Journal.pendingFile = @{ path = $relative; beforeSha256 = $existingHash; desiredSha256 = $desiredHash; artifact = $token }
          Write-CodexRecoveryJournal $Journal $Context
          $pending = $Journal.pendingFile
        }
        $hasDisplaced = [CodexAppxRecovery.NativeFiles]::Exists($replaced, $false)
        if ($hasDisplaced -and (-not $pending.beforeSha256 -or (Get-CodexRecoveryHash $replaced) -ine [string]$pending.beforeSha256)) { throw ('Unexpected displaced recovery content retained: ' + $replaced) }
        if (-not $existingHash -and $pending.beforeSha256 -and -not $hasDisplaced) { throw 'Missing recovery target has no verified displaced copy' }
        $source = Join-Path $Original.path $relative
        $sourcePins = Lock-CodexRecoveryDirectory ([IO.Path]::GetDirectoryName($source))
        try {
          if ([CodexAppxRecovery.NativeFiles]::Exists($temp, $false) -and (Get-CodexRecoveryHash $temp) -ine $desiredHash) {
            # A crash may interrupt only our new staging copy. Preserve its
            # bytes under a fresh name, never delete it or adopt it as data.
            $partial = Join-Path $artifactRoot ([guid]::NewGuid().ToString('N') + '.incomplete')
            [CodexAppxRecovery.NativeFiles]::MoveChecked($temp, $partial, (Get-CodexRecoveryHash $temp), $false)
          }
          if (-not [CodexAppxRecovery.NativeFiles]::Exists($temp, $false)) {
            $sourceStream = [CodexAppxRecovery.NativeFiles]::OpenRead($source)
            $out = [CodexAppxRecovery.NativeFiles]::CreateNew($temp)
            $sourceStream.CopyTo($out); $out.Flush($true); $out.Dispose(); $out = $null; $sourceStream.Dispose(); $sourceStream = $null
          }
        } finally { Close-CodexRecoveryHandles $sourcePins }
        if ((Get-CodexRecoveryHash $temp) -ine $desiredHash) { throw 'Restore staging copy failed its backup SHA256' }
        $latestHash = if ([CodexAppxRecovery.NativeFiles]::Exists($target, $false)) { Get-CodexRecoveryHash $target } else { $null }
        if ($latestHash -cne $existingHash) { throw ('Recovery target changed before atomic replacement: ' + $relative) }
        if ($existingHash) {
          if ($hasDisplaced) { throw 'Both original target and displaced artifact exist unexpectedly; content retained' }
          [CodexAppxRecovery.NativeFiles]::MoveChecked($target, $replaced, $existingHash, $false)
          # Persist the gap for diagnostics; recovery also infers it from the
          # exact journal-bound displaced hash if the crash precedes this write.
          Write-CodexRecoveryJournal $Journal $Context
        }
        # Two no-replace relative-handle renames intentionally leave a resumable
        # gap. A new concurrent destination is never overwritten. The old file
        # remains in the journal-bound artifact directory even on interruption.
        [CodexAppxRecovery.NativeFiles]::MoveChecked($temp, $target, $desiredHash, $false)
        if ((Get-CodexRecoveryHash $target) -ine $desiredHash) { throw 'Restored file does not match the original backup' }
        $Journal.replacements = @($Journal.replacements) + @($Journal.pendingFile)
        $Journal.pendingFile = $null; Write-CodexRecoveryJournal $Journal $Context
      } finally { if ($sourceStream) { $sourceStream.Dispose() }; if ($out) { $out.Dispose() }; Close-CodexRecoveryHandles $pins }
    }
  } finally { Close-CodexRecoveryHandles $artifactLocks; Close-CodexRecoveryHandles $targetLocks }
}

function Complete-CodexRecoveryJournal($Journal, $Context) {
  $Journal.phase = 'completed'; $Journal.pendingFile = $null; Write-CodexRecoveryJournal $Journal $Context
  $locks = Lock-CodexRecoveryDirectory $Context.backupRoot
  try {
    $active = Join-Path $Context.backupRoot 'recovery.json'
    $archive = Join-Path $Context.backupRoot ('recovery-' + [string]$Journal.id + '.completed.json')
    if ([CodexAppxRecovery.NativeFiles]::Exists($archive, $false)) { throw 'Recovery completion archive already exists; evidence retained' }
    # Move the previous journal first; a crash still leaves the authoritative
    # completed active journal for the next invocation to archive safely.
    if ([CodexAppxRecovery.NativeFiles]::Exists(($active + '.bak'), $false)) {
      [CodexAppxRecovery.NativeFiles]::MoveChecked(($active + '.bak'), (Join-Path $Context.backupRoot ('previous-' + [guid]::NewGuid().ToString('N') + '.json')), $null, $false)
    }
    [CodexAppxRecovery.NativeFiles]::MoveChecked($active, $archive, (Get-CodexRecoveryHash $active), $false)
    return $archive
  } finally { Close-CodexRecoveryHandles $locks }
}

function Invoke-CodexDataRecovery {
  [CmdletBinding()]
  param([Parameter(Mandatory=$true)][string]$PackagePath,
        [Parameter(Mandatory=$true)][string]$ExpectedFullName,
        [Parameter(Mandatory=$true)][string]$UsbRoot,
        [Parameter(Mandatory=$true)]$UsbVolume,
        [Parameter(Mandatory=$true)][hashtable]$Callbacks,
        [switch]$NonDestructiveUpdate,
        [switch]$MachineRepairRequired)
  $context = $null; $journal = $null; $payloadPins = $null; $payloadStream = $null; $rootPins = $null
  if ($ExpectedFullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$') { throw 'Recovery requires the exact verified Codex x64 payload identity' }
  foreach ($name in @('GetPackage','AssertOwnership','StopPackage','BackupPackage','RemovePackage','AssertDataPlacement')) {
    if ($Callbacks[$name] -isnot [scriptblock]) { throw ('Missing trusted recovery callback: ' + $name) }
  }
  if ($Callbacks.ContainsKey('RepairMachineState') -and $Callbacks.RepairMachineState -isnot [scriptblock]) {
    throw 'Invalid trusted recovery callback: RepairMachineState'
  }
  # Compile and lock harmless existing ancestors BEFORE any Remove. A failure
  # here propagates directly; it cannot be mistaken for a deployment result.
  Initialize-CodexRecoveryFileHandles
  $context = (Get-CodexRecoveryContext $UsbRoot $UsbVolume).Clone()
  $context.stageInvokedThisRun = $false; $context.registerInvokedThisRun = $false; $context.packageProbe = $null
  $payload = ConvertTo-CodexRecoveryLocalPath $PackagePath
  $payloadPins = Lock-CodexRecoveryDirectory ([IO.Path]::GetDirectoryName($payload))
  try {
    $payloadStream = [CodexAppxRecovery.NativeFiles]::OpenRead($payload)
    $payloadHash = Get-CodexRecoveryHash $payload
    $rootPins = Lock-CodexRecoveryDirectory $context.backupRoot $true
    $journal = Read-CodexRecoveryJournal $context
    $resuming = $null -ne $journal
    $package = & $Callbacks.GetPackage
    if ($journal) { Assert-CodexRecoveryJournal $journal $context $ExpectedFullName $payloadHash }
    $updating = $NonDestructiveUpdate -or ($journal -and $journal.mode -eq 'update')
    if ($package) {
      $initialFullName = [string]$package.PackageFullName
      if ($updating -and $initialFullName -ine $ExpectedFullName) {
        if ($initialFullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$' -or
            [version]($initialFullName.Split('_')[1]) -ge [version]($ExpectedFullName.Split('_')[1]) -or
            ($journal -and $initialFullName -ine [string]$journal.oldFullName)) { throw 'Unexpected package version during nondestructive update' }
        Assert-CodexRecoveryPackage $package $initialFullName $context
      } else { Assert-CodexRecoveryPackage $package $ExpectedFullName $context $false }
    }
    & $Callbacks.AssertOwnership $package | Out-Null
    if (-not $journal) {
      if ($package) { & $Callbacks.StopPackage $package | Out-Null }
      $backupPackage = if ($package) { $package } else { [pscustomobject]@{ PackageFullName=$ExpectedFullName; PackageFamilyName='OpenAI.Codex_2p2nqsd0c76g0' } }
      $backupPath = & $Callbacks.BackupPackage $backupPackage $context.usbRoot $context.usbName
      $original = Read-CodexRecoveryBackup ([string]$backupPath) $context
      $journal = [pscustomobject]@{
        schema='codex-appx-data-recovery'; version=1; id=[guid]::NewGuid().ToString('N'); sid=$context.sid; fullName=$ExpectedFullName
        usbId=$context.usbId; systemId=$context.systemId; payloadSha256=$payloadHash; mode=$(if($package -and $updating -and $initialFullName -ine $ExpectedFullName){'update'}elseif($package){'repair'}else{'fresh'})
        oldFullName=$(if($updating -and $package){$initialFullName}else{$null}); preUpdateOriginal=$null
        phase='prepared'; original=$(if($original){$original.binding}else{$null}); baseline=$null; pendingFile=$null; replacements=@(); lastDeployment=$null
        machineRepair=$(if($MachineRepairRequired){@{state='pending';fullName=$ExpectedFullName;attempts=0;lastError=$null;step='pending'}}else{$null})
      }
      Write-CodexRecoveryJournal $journal $context
    }
    Assert-CodexRecoveryJournal $journal $context $ExpectedFullName $payloadHash
    $original = Get-CodexRecoveryBoundBackup $journal.original $context
    $preUpdate = Get-CodexRecoveryBoundBackup $journal.preUpdateOriginal $context
    $machineRepairRecordMissing = (-not $journal.PSObject.Properties['machineRepair']) -or $null -eq $journal.machineRepair
    if ($MachineRepairRequired -and $machineRepairRecordMissing) {
      if ([string]$journal.phase -ne 'prepared') { throw 'Machine-package repair was discovered after deployment had already started; recovery retained without mutation' }
      $journal | Add-Member -NotePropertyName machineRepair -NotePropertyValue @{state='pending';fullName=$ExpectedFullName;attempts=0;lastError=$null;step='pending'} -Force
      Write-CodexRecoveryJournal $journal $context
    }
    if ($journal.PSObject.Properties['machineRepair'] -and $null -ne $journal.machineRepair) {
      if ([string]$journal.machineRepair.state -eq 'completed' -and $MachineRepairRequired) {
        throw 'Machine-package orphan state reappeared after its recorded cleanup; recovery retained without a second destructive repair'
      }
      if ([string]$journal.machineRepair.state -ne 'completed') {
        if ([string]$journal.phase -ne 'prepared' -or $Callbacks.RepairMachineState -isnot [scriptblock]) {
          throw 'Machine-package repair cannot resume safely; recovery evidence was retained'
        }
        Assert-CodexRecoveryContextUnchanged $context
        if (-not (Test-CodexRecoveryProperty $journal.machineRepair 'attempts')) {
          Set-CodexRecoveryProperty $journal.machineRepair 'attempts' 0
        }
        if ([int]$journal.machineRepair.attempts -ge 8) {
          throw 'Machine-package repair exceeded its bounded retry budget; recovery evidence was retained'
        }
        Set-CodexRecoveryProperty $journal.machineRepair 'attempts' ([int]$journal.machineRepair.attempts + 1)
        Set-CodexRecoveryProperty $journal.machineRepair 'lastError' $null
        Set-CodexRecoveryProperty $journal.machineRepair 'state' 'running'; Write-CodexRecoveryJournal $journal $context
        try {
          & $Callbacks.RepairMachineState $(if($original){$original.path}else{$null}) $payload $journal $context | Out-Null
        } catch {
          $repairFailure = Get-CodexRecoveryFailureText $_
          if ($repairFailure.Length -gt 4096) { $repairFailure = $repairFailure.Substring(0, 4096) }
          Set-CodexRecoveryProperty $journal.machineRepair 'state' 'failed'
          Set-CodexRecoveryProperty $journal.machineRepair 'lastError' $repairFailure
          try { Write-CodexRecoveryJournal $journal $context }
          catch { $repairFailure += '; machine-repair failure journal write failed: ' + (Get-CodexRecoveryFailureText $_) }
          throw $repairFailure
        }
        Set-CodexRecoveryProperty $journal.machineRepair 'state' 'completed'; Write-CodexRecoveryJournal $journal $context
        $package = & $Callbacks.GetPackage
        & $Callbacks.AssertOwnership $package | Out-Null
      }
    }
    if ($journal.phase -eq 'completed') {
      Assert-CodexRecoveryPackage $package $ExpectedFullName $context
      Assert-CodexRecoveryData $context $Callbacks ($journal.mode -eq 'update')
      $archive = Complete-CodexRecoveryJournal $journal $context
      return @{ success=$true; resumed=$true; journalPath=$archive; originalBackup=$(if($original){$original.path}else{$null}); preUpdateBackup=$(if($preUpdate){$preUpdate.path}else{$null}); baselineBackup=$null; message='Completed recovery verified and archived' }
    }
    if ($journal.phase -eq 'prepared') {
      Assert-CodexRecoveryContextUnchanged $context
      if ($journal.mode -eq 'repair') {
        Assert-CodexRecoveryPackage $package $ExpectedFullName $context $false
        & $Callbacks.AssertOwnership $package | Out-Null
        # Removal may release the only staged copy. Check before removing even
        # if this is a resume whose manager intentionally deferred its budget.
        Assert-CodexRecoveryDeploymentSpace $payload $context
        & $Callbacks.StopPackage $package | Out-Null
        $journal.phase = 'removing'; Write-CodexRecoveryJournal $journal $context
        & $Callbacks.RemovePackage $package $(if($original){$original.path}else{$null}) | Out-Null
        if ($null -ne (& $Callbacks.GetPackage)) { throw 'Current-user package remains after recovery removal; original backup retained' }
      } elseif ($journal.mode -eq 'update') {
        Assert-CodexRecoveryPackage $package $journal.oldFullName $context
        & $Callbacks.StopPackage $package | Out-Null
      } elseif ($null -ne $package) { throw 'A package appeared after the fresh recovery snapshot; it was not removed' }
      $journal.phase = 'removed'; Write-CodexRecoveryJournal $journal $context
    } elseif ($journal.phase -eq 'removing') {
      if ($null -ne $package) { throw 'Interrupted removal is ambiguous and registration remains; no second removal or backup was attempted' }
      $journal.phase = 'removed'; Write-CodexRecoveryJournal $journal $context
    }
    if ($journal.phase -in @('removed','staging')) {
      Assert-CodexRecoveryContextUnchanged $context
      $package = & $Callbacks.GetPackage
      if ($journal.mode -eq 'update') { Assert-CodexRecoveryPackage $package $journal.oldFullName $context }
      elseif ($null -ne $package) { throw 'Unexpected registration appeared before recovery staging' }
      & $Callbacks.AssertOwnership $package | Out-Null
      $staged = Get-CodexRecoveryStagedPackage $ExpectedFullName $context
      if ($null -eq $staged) {
        # Existing exact/healthy USB bytes can be registered without reserving
        # a second entire package. Missing bytes still require the full budget.
        Assert-CodexRecoveryDeploymentSpace $payload $context
        # Observe only the exact system-store entry. A pre-existing alias is
        # diagnostic evidence, never authorization to overwrite/delete it.
        $entryInfo = Get-CodexPackagePathInfo (Join-Path (Get-CodexSystemPackageStore) $ExpectedFullName) $ExpectedFullName (Join-Path $context.usbRoot 'WindowsApps')
        $journal | Add-Member -NotePropertyName systemEntryBeforeStage -NotePropertyValue $entryInfo -Force
        $journal.phase='staging'; Write-CodexRecoveryJournal $journal $context
        # The recorded helper now returns native metadata for Register callers;
        # Stage only needs the durable journal and placement postcondition, so
        # suppress its return value to keep this function's public output singular.
        $null = Invoke-CodexRecoveryRecordedDeployment 'stage' $payload $ExpectedFullName $journal $context
      }
      $staged = Get-CodexRecoveryStagedPackage $ExpectedFullName $context
      Assert-CodexRecoveryPackage $staged $ExpectedFullName $context
      $journal.phase='staged'; Write-CodexRecoveryJournal $journal $context
    }
    if ($journal.phase -in @('staged','registering')) {
      Assert-CodexRecoveryContextUnchanged $context
      $staged = Get-CodexRecoveryStagedPackage $ExpectedFullName $context
      Assert-CodexRecoveryPackage $staged $ExpectedFullName $context
      $package = & $Callbacks.GetPackage
      $registerResult = $null
      if ($null -eq $package -or ($journal.mode -eq 'update' -and [string]$package.PackageFullName -ieq [string]$journal.oldFullName)) {
        & $Callbacks.AssertOwnership $package | Out-Null
        if ($package) { & $Callbacks.StopPackage $package | Out-Null }
        $journal.phase='registering'; Write-CodexRecoveryJournal $journal $context
        $registerResult = Invoke-CodexRecoveryRecordedDeployment 'register' $payload $ExpectedFullName $journal $context
      }
      $allowedPrevious = if ($journal.mode -eq 'update') { [string]$journal.oldFullName } else { '' }
      $package = Wait-CodexRecoveryRegistrationPostcondition $ExpectedFullName $allowedPrevious $context $Callbacks $registerResult
      if ($journal.lastDeployment -and [string]$journal.lastDeployment.operation -match '^RegisterPackagesByFullNameAsync') {
        $journal.lastDeployment.postconditionState = 'observed-current-user-package'
      }
      $journal | Add-Member -NotePropertyName registrationPostcondition -NotePropertyValue $context.registrationProbe -Force
      Write-CodexRecoveryJournal $journal $context
      Assert-CodexRecoveryPackage $package $ExpectedFullName $context
      & $Callbacks.AssertOwnership $package | Out-Null
      & $Callbacks.StopPackage $package | Out-Null
      if ($journal.mode -ne 'update') { Assert-CodexRecoveryData $context $Callbacks }
      $journal.phase='registered'; Write-CodexRecoveryJournal $journal $context
    }
    if ($journal.phase -eq 'registered') {
      Assert-CodexRecoveryContextUnchanged $context
      $package = & $Callbacks.GetPackage; Assert-CodexRecoveryPackage $package $ExpectedFullName $context
      & $Callbacks.StopPackage $package | Out-Null
      if ($journal.mode -eq 'update') {
        $needsDataRepair = $false
        try { Assert-CodexRecoveryData $context $Callbacks $true } catch { $needsDataRepair = $true }
        if (-not $needsDataRepair) {
          $archive = Complete-CodexRecoveryJournal $journal $context
          return @{success=$true;resumed=$resuming;installed=$package;journalPath=$archive;originalBackup=$(if($original){$original.path}else{$null});preUpdateBackup=$(if($original){$original.path}else{$null});baselineBackup=$null;message='Nondestructive update verified on USB with system-volume data; pre-update backup retained'}
        }
        # Registration can retain an older cross-volume layout. First preserve
        # the UPDATED data through the engine's strict authorized resolver. Only
        # then promote this transaction to exact-new-version controlled repair.
        # The pre-update backup is never restored over upgraded application data.
        Assert-CodexRecoveryContextUnchanged $context
        & $Callbacks.AssertOwnership $package | Out-Null
        $updatedPath = & $Callbacks.BackupPackage $package $context.usbRoot $context.usbName
        $updated = Read-CodexRecoveryBackup ([string]$updatedPath) $context
        if (-not $updated) { throw 'Updated data placement is unknown and there is no verified backup; update retained without removal' }
        $journal.preUpdateOriginal = $journal.original
        $journal.original = $updated.binding; $journal.mode = 'repair'; $journal.phase = 'prepared'
        Write-CodexRecoveryJournal $journal $context
        return Invoke-CodexDataRecovery -PackagePath $payload -ExpectedFullName $ExpectedFullName -UsbRoot $UsbRoot -UsbVolume $UsbVolume -Callbacks $Callbacks
      }
      Assert-CodexRecoveryData $context $Callbacks
      # This is a full backup, not an assumption that Windows made every file.
      # It preserves user content written even before the first baseline scan.
      $baselinePath = & $Callbacks.BackupPackage $package $context.usbRoot $context.usbName
      $baseline = Read-CodexRecoveryBackup ([string]$baselinePath) $context
      $journal.baseline = if ($baseline) { $baseline.binding } else { $null }
      $journal.phase='baseline-captured'; Write-CodexRecoveryJournal $journal $context
    }
    $baseline = Get-CodexRecoveryBoundBackup $journal.baseline $context
    if ($journal.phase -in @('baseline-captured','restoring','verifying')) {
      Assert-CodexRecoveryContextUnchanged $context
      $package = & $Callbacks.GetPackage; Assert-CodexRecoveryPackage $package $ExpectedFullName $context
      & $Callbacks.AssertOwnership $package | Out-Null; & $Callbacks.StopPackage $package | Out-Null
      Assert-CodexRecoveryData $context $Callbacks
      $journal.phase='restoring'; Write-CodexRecoveryJournal $journal $context
      Restore-CodexRecoveryData $journal $original $baseline $context $Callbacks
      $journal.phase='verifying'; Write-CodexRecoveryJournal $journal $context
      Assert-CodexRecoveryContextUnchanged $context
      Assert-CodexRecoveryData $context $Callbacks
      $package = & $Callbacks.GetPackage; Assert-CodexRecoveryPackage $package $ExpectedFullName $context
      if ($original) {
        foreach ($relative in @($original.files.Keys)) {
          if ((Get-CodexRecoveryHash (Join-Path $context.dataRoot $relative)) -ine [string]$original.files[$relative]) { throw 'Final restored-data checksum verification failed' }
        }
      }
      $archive = Complete-CodexRecoveryJournal $journal $context
      return @{ success=$true; resumed=$resuming; installed=$package; journalPath=$archive; originalBackup=$(if($original){$original.path}else{$null}); preUpdateBackup=$(if($preUpdate){$preUpdate.path}else{$null}); baselineBackup=$(if($baseline){$baseline.path}else{$null}); message='USB program and system-volume data verified; original and post-registration backups retained' }
    }
    throw 'Recovery reached an unsupported journal transition'
  } catch {
    $failure = $_
    $phase='before-journal'; $stageInvoked=$false; $registerInvoked=$false; $packageProbe=$null; $lastDeployment=$null; $machineRepair=$null
    if ($journal -and $journal.PSObject.Properties['phase']) { $phase=[string]$journal.phase }
    if ($journal -and $journal.PSObject.Properties['lastDeployment']) { $lastDeployment=$journal.lastDeployment }
    if ($journal -and $journal.PSObject.Properties['machineRepair']) { $machineRepair=$journal.machineRepair }
    if ($journal -and $context -is [Collections.IDictionary] -and $context.Contains('registrationProbe')) {
      if ($journal.lastDeployment -and [string]$journal.lastDeployment.operation -match '^RegisterPackagesByFullNameAsync') {
        $probeState = [string]$context.registrationProbe.state
        $journal.lastDeployment.postconditionState = if ($probeState -eq 'observed') {
          'observed-current-user-package'
        } elseif ($probeState -eq 'timed-out') {
          'timed-out'
        } else {
          'pending-current-user-query'
        }
        try { Write-CodexRecoveryJournal $journal $context } catch { }
      }
    }
    if ($context -is [Collections.IDictionary]) {
      if ($context.Contains('stageInvokedThisRun')) { $stageInvoked=[bool]$context.stageInvokedThisRun }
      if ($context.Contains('registerInvokedThisRun')) { $registerInvoked=[bool]$context.registerInvokedThisRun }
      if ($context.Contains('packageProbe')) { $packageProbe=$context.packageProbe }
    }
    $registrationProbe = if ($context -is [Collections.IDictionary] -and $context.Contains('registrationProbe')) { $context.registrationProbe } else { $null }
    $diagnostic = @{ phase=$phase; stageInvokedThisRun=$stageInvoked; registerInvokedThisRun=$registerInvoked; packageProbe=$packageProbe; registrationProbe=$registrationProbe; lastDeployment=$lastDeployment; machineRepair=$machineRepair }
    if ($journal -and $journal.PSObject.Properties['systemEntryBeforeStage']) { $diagnostic.systemEntryBeforeStage=$journal.systemEntryBeforeStage }
    if ($context -and $context.Contains('packagePathEvidence')) { $diagnostic.packagePathEvidence=$context.packagePathEvidence }
    throw ('Controlled AppX data recovery stopped; existing backups/journal were retained. ' + (Get-CodexRecoveryFailureText $failure) + '; recoveryDiagnostic=' + ($diagnostic | ConvertTo-Json -Depth 6 -Compress) + '; at ' + [string]$failure.InvocationInfo.Line)
  } finally { if ($payloadStream) { $payloadStream.Dispose() }; Close-CodexRecoveryHandles $payloadPins; Close-CodexRecoveryHandles $rootPins }
}
