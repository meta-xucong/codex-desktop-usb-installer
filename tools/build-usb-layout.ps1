[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [string]$BundlePath = "",
    [string]$OutputPath = "",
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = Join-Path $repo "release\usb" }
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $repo "release\usb-final" }
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path

# Windows PowerShell 5.1/.NET Framework does not provide
# [System.IO.Path]::GetRelativePath (the release builder is intentionally
# usable on a stock Windows installation). Keep the fallback root-scoped so a
# future output-path change cannot make the checksum list point outside the
# generated bundle.
function Get-RelativePathCompat([string]$BasePath, [string]$FilePath) {
    $base = [System.IO.Path]::GetFullPath($BasePath).TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath($FilePath)
    if (-not $full.StartsWith($base, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "文件不在 U 盘输出目录内：$FilePath"
    }
    return $full.Substring($base.Length)
}
$installerItem = Get-Item -LiteralPath $installer
if ($installerItem.PSIsContainer -or $installerItem.Extension -ine ".exe") {
    throw "InstallerPath 必须是可直接运行的 Tauri 应用 EXE，而不是目录或其他文件"
}
if ($installerItem.Name -match "-setup\.exe$") {
    throw "不能把 NSIS -setup.exe 当作 U 盘入口；请传入 build-installer.ps1 生成的 CodexUsbLauncher.exe"
}
$isUsbLauncher = $false
if ($installerItem.Name -ieq "CodexUsbLauncher.exe" -or $installerItem.Name -ieq "CodexDesktopSetup.exe") {
    $launcherVerifier = Join-Path $PSScriptRoot "verify-usb-launcher.ps1"
    if (-not (Test-Path -LiteralPath $launcherVerifier -PathType Leaf)) {
        throw "缺少 USB 入口清单验证脚本：$launcherVerifier"
    }
    $launcherCheck = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifier -Path $installer 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) {
        $isUsbLauncher = $true
        Write-Output $launcherCheck.Trim()
    } elseif ($installerItem.Name -ieq "CodexUsbLauncher.exe") {
        throw "CodexUsbLauncher.exe 未通过 embedded requireAdministrator 清单验证：$launcherCheck"
    }
}
& (Join-Path $PSScriptRoot "verify-payload.ps1") -BundlePath $bundle -SkipSignature:$SkipSignature

$outputFull = [System.IO.Path]::GetFullPath($OutputPath).TrimEnd('\')
$volumeRoot = [System.IO.Path]::GetPathRoot($outputFull).TrimEnd('\')
foreach ($unsafeRoot in @($volumeRoot, $repo.TrimEnd('\'), (Split-Path -Parent $repo).TrimEnd('\'), $bundle.TrimEnd('\'), $installerItem.DirectoryName.TrimEnd('\'))) {
    if ($outputFull -ieq $unsafeRoot) { throw "输出目录不能覆盖磁盘根目录、工作区或输入目录：$outputFull" }
}
if ((Test-Path -LiteralPath $outputFull) -and
    @(Get-ChildItem -LiteralPath $outputFull -Force -ErrorAction Stop).Count -gt 0) {
    throw "发布输出目录必须为空；请使用新的版本目录，避免残留旧 EXE、资源或许可证：$outputFull"
}
$output = (New-Item -ItemType Directory -Force -Path $outputFull).FullName
if ((Get-Item -LiteralPath $output).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
    throw "U 盘输出目录不能是链接或联接点：$output"
}
New-Item -ItemType Directory -Force -Path (Join-Path $output "payload") | Out-Null
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
foreach ($directory in @("apps", "content", "content\skills", "content\mcp")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $output $directory) | Out-Null
}
[System.IO.File]::WriteAllText(
    (Join-Path $output "apps\README.txt"),
    "此目录由安装器管理。Codex 程序将解包到 apps\Codex；账号、会话、缓存和配置不写入这里。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\README.txt"),
    "U 盘托管内容目录。Skill 与 MCP 的程序/资源可放在对应子目录；API Key、登录凭证和用户配置不得放入 U 盘。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\skills\README.txt"),
    "预装 Skill 载荷目录（首期仅预留）。后续导入器通过受支持的本地发现入口引用此处内容，用户数据仍保存在本机。`r`n",
    $utf8NoBom
)
[System.IO.File]::WriteAllText(
    (Join-Path $output "content\mcp\README.txt"),
    "预装 MCP 可执行文件和静态资源目录（首期仅预留）。MCP 配置写入本机 %USERPROFILE%\.codex\config.toml，密钥不得写入 U 盘。`r`n",
    $utf8NoBom
)
Copy-Item -LiteralPath $installer -Destination (Join-Path $output "CodexDesktopSetup.exe") -Force
$managerSource = Join-Path $installerItem.DirectoryName "CodexAppManager.exe"
if ($isUsbLauncher) {
    if (-not (Test-Path -LiteralPath $managerSource -PathType Leaf)) {
        throw "CodexUsbLauncher.exe 必须与同目录 CodexAppManager.exe 一起构建"
    }
    Copy-Item -LiteralPath $managerSource -Destination (Join-Path $output "CodexAppManager.exe") -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-usb-launcher.ps1") -Destination (Join-Path $output "verify-usb-launcher.ps1") -Force
}
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
            $resolvedCcswitchOutput = (Resolve-Path -LiteralPath $ccswitchOutput).Path
            if ($resolvedCcswitchOutput -ine (Join-Path $output "ccswitch") -or
                ((Get-Item -LiteralPath $ccswitchOutput).Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
                throw "拒绝清理输出目录之外的 CCSwitch 或链接目录：$ccswitchOutput"
            }
            Remove-Item -LiteralPath $ccswitchOutput -Recurse -Force
        }
        Copy-Item -LiteralPath $ccswitchSource -Destination $ccswitchOutput -Recurse -Force
    }
}
$vcRedistOutput = Join-Path $output "resources\prerequisites\vc_redist.x64.exe"
if (-not (Test-Path -LiteralPath $vcRedistOutput -PathType Leaf)) {
    throw "安装器资源中缺少离线 VC++ x64 运行库：$vcRedistOutput"
}
$webView2Output = Join-Path $output "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
if (-not (Test-Path -LiteralPath $webView2Output -PathType Leaf)) {
    throw "安装器资源中缺少 WebView2 x64 完整离线安装包：$webView2Output"
}
Copy-Item -LiteralPath (Join-Path $bundle "manifest.json") -Destination (Join-Path $output "manifest.json") -Force
Get-ChildItem -LiteralPath (Join-Path $bundle "payload") -File | Copy-Item -Destination (Join-Path $output "payload") -Force

$lines = Get-ChildItem -LiteralPath $output -File -Recurse |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    ForEach-Object {
        $relative = Get-RelativePathCompat $output $_.FullName
        $relative = $relative.Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)
Write-Output "已生成 U 盘目录：$output"
