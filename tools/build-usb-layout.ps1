[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [string]$BundlePath = (Join-Path $PSScriptRoot "..\release\usb"),
    [string]$OutputPath = (Join-Path $PSScriptRoot "..\release\usb-final"),
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$installerItem = Get-Item -LiteralPath $installer
if ($installerItem.PSIsContainer -or $installerItem.Extension -ine ".exe") {
    throw "InstallerPath 必须是可直接运行的 Tauri 应用 EXE，而不是目录或其他文件"
}
if ($installerItem.Name -match "-setup\.exe$") {
    throw "不能把 NSIS -setup.exe 当作 U 盘入口；请传入 build-installer.ps1 生成的 CodexDesktopSetup.exe"
}
& (Join-Path $PSScriptRoot "verify-payload.ps1") -BundlePath $bundle -SkipSignature:$SkipSignature

$output = (New-Item -ItemType Directory -Force -Path $OutputPath).FullName
New-Item -ItemType Directory -Force -Path (Join-Path $output "payload") | Out-Null
Copy-Item -LiteralPath $installer -Destination (Join-Path $output "CodexDesktopSetup.exe") -Force
$resourceSource = Join-Path $installerItem.DirectoryName "resources"
if (Test-Path -LiteralPath $resourceSource -PathType Container) {
    Copy-Item -LiteralPath $resourceSource -Destination $output -Recurse -Force
    # Keep a user-visible root-level copy for the validation/USB layout.  The
    # installer command can also resolve resources/ccswitch when this folder
    # is embedded by Tauri, but the root copy makes the fallback manual launch
    # and post-install diagnostics obvious to a non-technical user.
    $ccswitchSource = Join-Path $resourceSource "ccswitch"
    if (Test-Path -LiteralPath $ccswitchSource -PathType Container) {
        $ccswitchOutput = Join-Path $output "ccswitch"
        # The output directory is intentionally a generated release folder.
        # Remove only its previous root-level CCSwitch directory before
        # copying, otherwise repeated builds can create ccswitch\ccswitch.
        if (Test-Path -LiteralPath $ccswitchOutput -PathType Container) {
            Remove-Item -LiteralPath $ccswitchOutput -Recurse -Force
        }
        Copy-Item -LiteralPath $ccswitchSource -Destination $ccswitchOutput -Recurse -Force
    }
}
Copy-Item -LiteralPath (Join-Path $bundle "manifest.json") -Destination (Join-Path $output "manifest.json") -Force
Get-ChildItem -LiteralPath (Join-Path $bundle "payload") -File | Copy-Item -Destination (Join-Path $output "payload") -Force

$lines = Get-ChildItem -LiteralPath $output -File -Recurse |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    ForEach-Object {
        $relative = [System.IO.Path]::GetRelativePath($output, $_.FullName).Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)
Write-Output "已生成 U 盘目录：$output"
