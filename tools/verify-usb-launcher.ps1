[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [ValidateSet("requireAdministrator", "highestAvailable", "asInvoker")]
    [string]$ExpectedLevel = "requireAdministrator"
)

$ErrorActionPreference = "Stop"
$resolved = (Resolve-Path -LiteralPath $Path).Path
if (-not [System.IO.Path]::GetExtension($resolved).Equals(".exe", [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "USB 入口必须是 EXE：$resolved"
}

# Read RT_MANIFEST (resource type 24, id 1) without relying on the Windows SDK
# mt.exe tool. This keeps release verification usable on a stock Windows
# installation and catches accidental packaging of the normal asInvoker
# manager as the USB entry.
$typeName = "CodexUsbLauncherManifestVerifier"
if (-not ($typeName -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class CodexUsbLauncherManifestVerifier {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr LockResource(IntPtr data);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint SizeofResource(IntPtr module, IntPtr resource);
    public static string Read(string path) {
        const uint LOAD_LIBRARY_AS_DATAFILE = 0x00000002;
        var module = LoadLibraryEx(path, IntPtr.Zero, LOAD_LIBRARY_AS_DATAFILE);
        if (module == IntPtr.Zero) throw new InvalidOperationException("LoadLibraryEx failed: " + Marshal.GetLastWin32Error());
        var resource = FindResource(module, new IntPtr(1), new IntPtr(24));
        if (resource == IntPtr.Zero) throw new InvalidOperationException("RT_MANIFEST resource not found: " + Marshal.GetLastWin32Error());
        var data = LockResource(LoadResource(module, resource));
        var size = SizeofResource(module, resource);
        if (data == IntPtr.Zero || size == 0) throw new InvalidOperationException("RT_MANIFEST resource is empty");
        var bytes = new byte[size];
        Marshal.Copy(data, bytes, 0, (int)size);
        return System.Text.Encoding.UTF8.GetString(bytes);
    }
}
"@ -ErrorAction Stop
}

$manifestText = [CodexUsbLauncherManifestVerifier]::Read($resolved)
try {
    [xml]$manifest = $manifestText
} catch {
    throw "入口嵌入清单不是有效 XML：$($_.Exception.Message)"
}
$executionLevels = @($manifest.SelectNodes("//*[local-name()='requestedExecutionLevel']"))
$matchingLevel = $executionLevels |
    Where-Object { $_.GetAttribute("level") -eq $ExpectedLevel }
if (-not $matchingLevel) {
    throw "PE 嵌入清单缺少 requestedExecutionLevel=$ExpectedLevel"
}
Write-Output "PE 嵌入清单验证通过：requestedExecutionLevel=$ExpectedLevel path=$resolved"
