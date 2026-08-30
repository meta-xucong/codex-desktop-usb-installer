[CmdletBinding()]
param(
    [string]$InstallerPath = (Join-Path $PSScriptRoot "..\release\setup\CodexDesktopSetup.exe"),
    [string]$BundlePath = (Join-Path $PSScriptRoot "..\release\usb-final"),
    [string]$OutputPath = (Join-Path $PSScriptRoot "..\release\usb-oneclick"),
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$bundle = (Resolve-Path -LiteralPath $BundlePath).Path

& (Join-Path $PSScriptRoot "build-usb-layout.ps1") `
    -InstallerPath $installer `
    -BundlePath $bundle `
    -OutputPath $OutputPath `
    -SkipSignature:$SkipSignature
# The nested PowerShell script communicates failures through terminating
# errors.  `$LASTEXITCODE` is intentionally not inspected here because it can
# retain the exit code of a previous native command in the parent session.

$output = (Resolve-Path -LiteralPath $OutputPath).Path
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-payload.ps1") -Destination (Join-Path $output "verify-payload.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "verify-oneclick-usb.ps1") -Destination (Join-Path $output "verify-usb.ps1") -Force
# Tauri deliberately does not ship PowerShell files through the Windows
# resource bundle.  Keep the product verifier in the user-visible root
# ccswitch directory so offline diagnostics still work on a blank machine.
$ccswitchVerifier = Join-Path (Split-Path -Parent $repo) "release\ccswitch-product\verify-product.ps1"
if (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf) {
    Copy-Item -LiteralPath $ccswitchVerifier -Destination (Join-Path $output "ccswitch\verify-product.ps1") -Force
}

$installerItem = Get-Item -LiteralPath (Join-Path $output "CodexDesktopSetup.exe")
$payloadItem = Get-Item -LiteralPath (Join-Path $output "payload\OpenAI.Codex_x64.msix")
$ccswitchItem = Get-Item -LiteralPath (Join-Path $output "ccswitch\CodexSwitch.exe")
$installerHash = (Get-FileHash -LiteralPath $installerItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$payloadHash = (Get-FileHash -LiteralPath $payloadItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$ccswitchHash = (Get-FileHash -LiteralPath $ccswitchItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()

$usbManifest = [ordered]@{
    schemaVersion = 1
    product = "codex-usb-oneclick"
    status = "oneclick"
    description = "Offline Codex desktop installation followed by automatic per-user Codex Switch installation and launch."
    codexInstaller = [ordered]@{
        file = "CodexDesktopSetup.exe"
        size = [int64]$installerItem.Length
        sha256 = $installerHash
        payload = "payload/OpenAI.Codex_x64.msix"
        payloadSize = [int64]$payloadItem.Length
        payloadSha256 = $payloadHash
    }
    ccswitch = [ordered]@{
        directory = "ccswitch"
        executable = "ccswitch/CodexSwitch.exe"
        size = [int64]$ccswitchItem.Length
        sha256 = $ccswitchHash
        embeddedExecutable = "resources/ccswitch/CodexSwitch.exe"
        localInstallDirectory = "%LOCALAPPDATA%/Programs/CodexSwitch"
    }
    workflow = @(
        "Double-click CodexDesktopSetup.exe with manifest.json and payload/ beside it.",
        "The installer validates and installs the offline Codex MSIX.",
        "After Codex is confirmed, it copies ccswitch/* to %LOCALAPPDATA%/Programs/CodexSwitch and launches CodexSwitch.exe.",
        "On first launch Codex Switch opens the 404TOKEN setup dialog; enter the user's API Key and click Save and enable. No key is included in this bundle.",
        "The installed screen and Settings provide Uninstall Codex + CCSwitch; the default keeps %USERPROFILE%/.codex and %USERPROFILE%/.cc-switch, while the explicit purge option removes both."
    )
    secretsIncluded = $false
    automaticPostInstallLaunch = $true
    notes = @(
        "The root manifest.json is the Codex installer manifest and must not be replaced by usb-manifest.json.",
        "The CCSwitch copy/launch is best-effort; a missing payload does not roll back a successful Codex installation.",
        "The first release is an offline MSIX installer plus a per-user CCSwitch copy, not an MSI or self-updating package."
    )
}
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText((Join-Path $output "usb-manifest.json"), ($usbManifest | ConvertTo-Json -Depth 8), $utf8NoBom)

$readme = @'
Codex U 盘一键安装版（V1）
===========================

用途
----
这是一个离线的一键安装包。用户只需要双击根目录的
`CodexDesktopSetup.exe`：程序会先校验并安装 Codex Desktop，确认安装成功后，
自动把改造版 Codex Switch 复制到本机并启动。首次打开时会弹出 404TOKEN 配置框，
用户只需输入自己的 API Key 并点击“保存并启用”。

必须保持的目录结构
--------------------
CodexDesktopSetup.exe       唯一入口
manifest.json               Codex 安装器专用清单（不要覆盖）
usb-manifest.json           本一键包总清单
payload\OpenAI.Codex_x64.msix  离线 Codex 载荷
ccswitch\CodexSwitch.exe    CCSwitch 根目录回退载荷
resources\ccswitch\        安装器内置/旁路 CCSwitch 载荷
verify-usb.ps1              一键包静态验证脚本
verify-payload.ps1          MSIX 身份、架构、大小和内部清单验证脚本
SHA256SUMS.txt              全部文件哈希

使用步骤
--------
1. 将本目录完整复制到 U 盘；不要只复制 EXE，不要改变 payload、ccswitch、
   resources 的相对位置。
2. 双击 `CodexDesktopSetup.exe`。程序会自动完成：
   - 检查 Windows 版本、MSIX 哈希和 Codex 包身份；
   - 安装离线 Codex Desktop；
   - 检查 Codex 已安装且版本正确；
   - 将 CCSwitch 复制到 `%LOCALAPPDATA%\Programs\CodexSwitch`；
   - 自动启动 `CodexSwitch.exe`。
3. Codex Switch 首次启动会自动弹出 404TOKEN（Sub2API）配置框。输入 API Key，
   点击“保存并启用”即可；程序会自动把它设为当前 Codex 服务商。内置 Base URL 为
   `https://404token.xyz/v1`，不需要也不要改成
   `/v1/chat/completions`、`/v1/responses` 或 `127.0.0.1:8080`。
4. 看到 CCSwitch 窗口后即可拔出 U 盘；Codex 和 CCSwitch 已经复制/安装到本机。
5. 如需清理，在安装器主页（或“设置 → 更多”）点击“卸载 Codex 与 CCSwitch”。
   默认仅移除程序并保留 `%USERPROFILE%\.codex` 和 `%USERPROFILE%\.cc-switch`；
   关闭“保留我的数据”并完成第二次确认，才会一并清除配置和 CCSwitch WebView 缓存。

API Key 和安全边界
-------------------
- U 盘包不包含 API Key；必须由用户在 CCSwitch 界面输入。
- 安装器不会从 GitHub、Microsoft Store 或远程脚本下载组件。
- CCSwitch 是基于 GitHub 开源项目 CCSwitch 的第三方改造版，不是 OpenAI 官方发行版。
- 404TOKEN 是用户提供的第三方 Sub2API 服务，请自行确认账户、计费和数据处理政策。

验证
----
在 PowerShell 中进入本目录，执行：

  Set-ExecutionPolicy -Scope Process Bypass
  .\verify-usb.ps1 -SkipNetwork

需要同时检查 404TOKEN 网络连通性时执行：

  .\verify-usb.ps1

预期会看到 oneclick 模式、MSIX ready=true，以及 CCSwitch 静态检查通过。
拿到真实 Key 后，可使用 `-PromptForApiKey` 临时验证模型目录；Key 不会写入
清单、脚本或 SHA256SUMS.txt。

排障
----
- “未找到离线载荷”：确认根目录的 `manifest.json`、`payload` 和入口 EXE 同级；
  不要用 `usb-manifest.json` 覆盖 `manifest.json`。
- “CCSwitch 未自动启动”：确认 `ccswitch\CodexSwitch.exe` 存在；Codex 安装成功
  后可在安装器窗口点击“打开 Codex Switch”重试。
- 更新引擎/签名错误：以管理员身份重试，并保留 `SHA256SUMS.txt` 与验证输出。
- 模型获取失败：检查 API Key，以及 Base URL 是否保持 `https://404token.xyz/v1`。
'@
[System.IO.File]::WriteAllText((Join-Path $output "README.txt"), $readme, $utf8NoBom)

$lines = Get-ChildItem -LiteralPath $output -File -Recurse |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    ForEach-Object {
        $relative = [System.IO.Path]::GetRelativePath($output, $_.FullName).Replace('\', '/')
        "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $relative"
    }
[System.IO.File]::WriteAllLines((Join-Path $output "SHA256SUMS.txt"), $lines, $utf8NoBom)

Write-Output "已生成 Codex 一键 U 盘目录：$output"
Write-Output "入口：$(Join-Path $output 'CodexDesktopSetup.exe')"
