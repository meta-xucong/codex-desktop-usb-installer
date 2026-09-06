[CmdletBinding()]
param(
    [string]$BundlePath = "",
    [string]$ApiKey,
    [switch]$PromptForApiKey,
    [switch]$SkipNetwork,
    [switch]$SkipSignature
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = $PSScriptRoot }
$root = (Resolve-Path -LiteralPath $BundlePath).Path

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "验证失败：$Message" }
}

function Read-JsonFile([string]$Path) {
    Assert-Condition (Test-Path -LiteralPath $Path -PathType Leaf) "缺少 $Path"
    # Windows PowerShell 5.1 defaults to the active ANSI code page for
    # BOM-less UTF-8. Release metadata is UTF-8, so decode it explicitly.
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    return [System.IO.File]::ReadAllText($Path, $utf8) | ConvertFrom-Json
}

$manifestPath = Join-Path $root "manifest.json"
$usbManifestPath = Join-Path $root "usb-manifest.json"
$sumsPath = Join-Path $root "SHA256SUMS.txt"
$payloadVerifierPath = Join-Path $root "verify-payload.ps1"
$launcherVerifierPath = Join-Path $root "verify-usb-launcher.ps1"
$manifest = Read-JsonFile $manifestPath
$usbManifest = Read-JsonFile $usbManifestPath

Assert-Condition ($usbManifest.product -eq "codex-usb-oneclick") "不是 Codex 一键安装包"
Assert-Condition ($usbManifest.status -eq "oneclick") "联合清单状态不是 oneclick"
Assert-Condition ($usbManifest.secretsIncluded -eq $false) "U 盘包不应包含 API Key"
Assert-Condition ($usbManifest.automaticPostInstallLaunch -eq $true) "未声明安装后自动启动 CCSwitch"
Assert-Condition ($usbManifest.programLayout.mode -eq "verified-msix-extraction") "Codex 程序必须使用验签 MSIX 解包模式"
Assert-Condition ($usbManifest.programLayout.codexRoot -eq "apps/Codex") "Codex U 盘程序目录声明不正确"
Assert-Condition ($usbManifest.programLayout.usesAppxVolume -eq $false) "当前架构不得使用外部 AppXVolume"
Assert-Condition ($usbManifest.programLayout.createsWindowsApps -eq $false) "当前架构不得创建 WindowsApps"
Assert-Condition ($usbManifest.programLayout.createsWpSystem -eq $false) "当前架构不得创建 WpSystem"
Assert-Condition ($usbManifest.programLayout.directShellLaunch -eq $false) "不得创建绕过 U 盘管理器的直接 Codex 入口"
Assert-Condition ($usbManifest.dataLayout.codexHome -eq "%USERPROFILE%/.codex") "CODEX_HOME 本机路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.roaming -eq "%APPDATA%/Codex") "Roaming 数据路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.local -eq "%LOCALAPPDATA%/Codex") "Local 数据路径声明不正确"
Assert-Condition ($usbManifest.dataLayout.onUsb -eq $false) "可变 Codex 数据不得声明位于 U 盘"
Assert-Condition ($usbManifest.dataLayout.observedBeforeInstall -eq $true) "安装前必须核验数据实际卷"
Assert-Condition ($usbManifest.managedContent.skillsDirectory -eq "content/skills") "Skill U 盘目录声明不正确"
Assert-Condition ($usbManifest.managedContent.mcpDirectory -eq "content/mcp") "MCP U 盘目录声明不正确"
Assert-Condition ($usbManifest.managedContent.configLocation -eq "%USERPROFILE%/.codex/config.toml") "MCP/Codex 配置必须保存在本机"
Assert-Condition ($usbManifest.managedContent.secretsIncluded -eq $false) "托管内容目录不得包含密钥"
Assert-Condition ($usbManifest.managedContent.status -eq "layout-ready-import-later") "不得误报 Skill/MCP 已自动导入"
Assert-Condition ($usbManifest.startupElevation.mode -eq "usb-launcher-requireAdministrator") "USB 入口必须声明 requireAdministrator 启动提权模式"
Assert-Condition ($usbManifest.startupElevation.mechanism -eq "embedded requireAdministrator manifest + standard Windows UAC consent") "USB 入口提权机制不是标准 Windows UAC"
Assert-Condition ($usbManifest.startupElevation.marker -eq "--codex-usb-elevated") "USB 入口提权标记不正确"
Assert-Condition ($usbManifest.startupElevation.onCancel -eq "exit-before-install") "UAC 取消时必须在安装前退出"
Assert-Condition ($usbManifest.startupElevation.silentBypass -eq $false) "USB 入口不允许静默绕过 UAC"
Assert-Condition ($usbManifest.startupElevation.executionPolicy -eq "process-local-bypass") "PowerShell 执行策略必须声明为进程级兼容参数"
Assert-Condition ($usbManifest.startupElevation.executionPolicyScope -eq "temporary PowerShell child only") "PowerShell 执行策略作用域不正确"
Assert-Condition ($usbManifest.ccswitch.launchMode -eq "usb-direct") "CCSwitch 必须声明 USB 直接启动"
Assert-Condition ($usbManifest.ccswitch.graphicsMode -eq "child-scoped-webview2-software-rendering") "CCSwitch 必须声明子进程级 WebView2 软件渲染"
Assert-Condition ($usbManifest.ccswitch.successPostcondition -eq "stable-sizeable-window") "CCSwitch 启动成功必须以稳定可操作窗口为后置条件"
Assert-Condition ($usbManifest.ccswitch.foregroundPolicy -eq "restore-clamp-to-work-area-and-present") "CCSwitch 必须声明窗口恢复、屏内钳制和呈现策略"
Assert-Condition ($usbManifest.codexLaunch.initialMode -eq "software-rendering") "USB Codex 首次启动必须声明软件渲染"
Assert-Condition (($usbManifest.codexLaunch.initialArguments -join " ") -eq "--disable-gpu") "USB Codex 初始兼容参数不正确"
Assert-Condition (($usbManifest.codexLaunch.fallbackArguments -join " ") -eq "--disable-gpu --no-sandbox") "USB Codex 末级兼容参数不正确"
Assert-Condition ($usbManifest.codexLaunch.fallbackOnlyAfterNoUsableWindow -eq $true) "no-sandbox 只能在可操作窗口验收失败后使用"
Assert-Condition ($usbManifest.codexLaunch.successPostcondition -eq "stable-sizeable-window") "USB Codex 不得只以进程存活作为启动成功"
Assert-Condition ($usbManifest.codexLaunch.compatibilityMemory -eq "local-exact-codex-version") "USB Codex 兼容状态必须按精确版本保存在本机"
Assert-Condition ($usbManifest.codexLaunch.selfUpdate -eq "disabled-for-usb-child") "USB Codex 必须对子进程关闭原地自更新"
Assert-Condition ($usbManifest.codexLaunch.globalEnvironmentModified -eq $false) "启动兼容参数不得修改全局环境"
Assert-Condition ($usbManifest.usbGuard.localHelperRefresh -eq "length-and-sha256-before-new-lease") "本机看门狗必须在新租约前按哈希刷新"
Assert-Condition ([int]$usbManifest.sourceMediaCleanup.planSchema -eq 3) "安装介质二阶段计划版本不正确"
Assert-Condition ($usbManifest.sourceMediaCleanup.runtimeLicenseMode -eq "typed-manager-mutable-metadata") "运行期许可文件不得再被当作未知发行文件"
Assert-Condition ($usbManifest.sourceMediaCleanup.authorization -eq "active-lease-fingerprint+volume+immutable-manifest+license-snapshot") "安装介质清理授权证据不完整"
Assert-Condition ($usbManifest.sourceMediaCleanup.deleteOrder -eq "payload-first-license-and-retry-entry-last") "安装介质清理顺序不得提前删除重试身份"
Assert-Condition ($usbManifest.sourceMediaCleanup.unknownPolicy -eq "enumerate-and-preserve") "未知安装介质内容必须列名并保留"
Assert-Condition ($usbManifest.sourceMediaCleanup.usesRecycleBin -eq $false) "可移动介质清理不得依赖回收站"
Assert-Condition ($manifest.packageName -eq "OpenAI.Codex") "根 manifest 不是 Codex 离线安装清单"
Assert-Condition ($manifest.payload.path -eq "payload/OpenAI.Codex_x64.msix") "根 manifest 的 MSIX 路径不正确"

$payloadArgs = @{ BundlePath = $root }
if ($SkipSignature) { $payloadArgs.SkipSignature = $true }
$payloadReport = & $payloadVerifierPath @payloadArgs | ConvertFrom-Json
Assert-Condition ($payloadReport.ready -eq $true) "MSIX 身份、架构、大小或内部清单校验失败"

$installer = Join-Path $root "CodexDesktopSetup.exe"
$manager = Join-Path $root "CodexAppManager.exe"
$ccswitch = Join-Path $root "ccswitch\CodexSwitch.exe"
$embeddedCcswitch = Join-Path $root "resources\ccswitch\CodexSwitch.exe"
$vcRedist = Join-Path $root "resources\prerequisites\vc_redist.x64.exe"
$webView2 = Join-Path $root "resources\prerequisites\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
$appsReadme = Join-Path $root "apps\README.txt"
$contentReadme = Join-Path $root "content\README.txt"
$skillsReadme = Join-Path $root "content\skills\README.txt"
$mcpReadme = Join-Path $root "content\mcp\README.txt"
$expectedWebView2Hash = "1F4638309F3D82C31A3028C3CF7D75998F58E4D1407380F5CB8A8E9172CAF17D"
$expectedVcRedistHash = "843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C"
Assert-Condition (Test-Path -LiteralPath $installer -PathType Leaf) "缺少 CodexDesktopSetup.exe"
Assert-Condition (Test-Path -LiteralPath $manager -PathType Leaf) "缺少 CodexAppManager.exe（USB 入口启动的管理器本体）"
Assert-Condition (Test-Path -LiteralPath $launcherVerifierPath -PathType Leaf) "缺少 verify-usb-launcher.ps1"
Assert-Condition (Test-Path -LiteralPath $ccswitch -PathType Leaf) "缺少根目录 ccswitch\CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $embeddedCcswitch -PathType Leaf) "安装器资源中缺少 resources\ccswitch\CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $vcRedist -PathType Leaf) "缺少 resources\prerequisites\vc_redist.x64.exe"
Assert-Condition (Test-Path -LiteralPath $webView2 -PathType Leaf) "缺少 WebView2 x64 完整离线安装包；空白机将无法显示安装界面"
Assert-Condition (Test-Path -LiteralPath $appsReadme -PathType Leaf) "缺少 apps 程序目录契约"
Assert-Condition (Test-Path -LiteralPath $contentReadme -PathType Leaf) "缺少 content 托管内容目录契约"
Assert-Condition (Test-Path -LiteralPath $skillsReadme -PathType Leaf) "缺少 content/skills 目录契约"
Assert-Condition (Test-Path -LiteralPath $mcpReadme -PathType Leaf) "缺少 content/mcp 目录契约"
Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $root "WindowsApps"))) "发行包不得预建 WindowsApps"
Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $root "WpSystem"))) "发行包不得预建 WpSystem"
$actualWebView2Hash = (Get-FileHash -LiteralPath $webView2 -Algorithm SHA256).Hash
Assert-Condition ($actualWebView2Hash -ieq $expectedWebView2Hash) "WebView2 安装包哈希不匹配"
Assert-Condition ($usbManifest.webView2Runtime.file -eq "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe") "联合清单中的 WebView2 路径不正确"
Assert-Condition ($usbManifest.webView2Runtime.sha256 -ieq $expectedWebView2Hash) "联合清单中的 WebView2 哈希不正确"
Assert-Condition ([int64]$usbManifest.webView2Runtime.size -eq (Get-Item -LiteralPath $webView2).Length) "联合清单中的 WebView2 大小不正确"
Assert-Condition ($usbManifest.webView2Runtime.beforeUi -eq $true) "WebView2 必须在创建安装界面前预检"
$launcherVerification = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifierPath -Path $installer 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    throw "USB 入口清单验证失败：$launcherVerification"
}
$managerVerification = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $launcherVerifierPath -Path $manager -ExpectedLevel requireAdministrator 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    throw "USB 管理器清单验证失败：$managerVerification"
}
$actualVcRedistHash = (Get-FileHash -LiteralPath $vcRedist -Algorithm SHA256).Hash
Assert-Condition ($actualVcRedistHash -ieq $expectedVcRedistHash) "VC++ 运行库哈希不匹配：$actualVcRedistHash != $expectedVcRedistHash"
Assert-Condition ($usbManifest.vcRuntime.file -eq "resources/prerequisites/vc_redist.x64.exe") "联合清单的 VC++ 运行库路径不正确"
Assert-Condition ($usbManifest.vcRuntime.sha256 -ieq $expectedVcRedistHash) "联合清单中的 VC++ 运行库哈希不正确"
Assert-Condition ([int64]$usbManifest.vcRuntime.size -eq (Get-Item -LiteralPath $vcRedist).Length) "联合清单中的 VC++ 运行库大小不正确"

# Cross-check the generated metadata against the files that were just
# inspected. This catches a stale usb-manifest/SHA256SUMS pair after a partial
# copy or an interrupted release rebuild; checking only the root manifest can
# otherwise let a structurally valid but self-inconsistent bundle through.
$installerItem = Get-Item -LiteralPath $installer
$managerItem = Get-Item -LiteralPath $manager
$installerHash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$managerHash = (Get-FileHash -LiteralPath $manager -Algorithm SHA256).Hash.ToLowerInvariant()
$ccswitchItem = Get-Item -LiteralPath $ccswitch
$ccswitchHash = (Get-FileHash -LiteralPath $ccswitch -Algorithm SHA256).Hash.ToLowerInvariant()
$embeddedCcswitchHash = (Get-FileHash -LiteralPath $embeddedCcswitch -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-Condition ($usbManifest.codexInstaller.file -eq "CodexDesktopSetup.exe") "联合清单的入口文件名不正确"
Assert-Condition ($usbManifest.codexInstaller.sha256 -ieq $installerHash) "联合清单中的入口哈希与实际文件不一致"
Assert-Condition ([int64]$usbManifest.codexInstaller.size -eq $installerItem.Length) "联合清单中的入口大小与实际文件不一致"
Assert-Condition ($usbManifest.codexInstaller.payload -eq "payload/OpenAI.Codex_x64.msix") "联合清单的 MSIX 路径不正确"
Assert-Condition ($usbManifest.codexInstaller.payloadSha256 -ieq $payloadReport.sha256) "联合清单中的 MSIX 哈希与实际文件不一致"
Assert-Condition ([int64]$usbManifest.codexInstaller.payloadSize -eq [int64]$payloadReport.size) "联合清单中的 MSIX 大小与实际文件不一致"
Assert-Condition ($usbManifest.manager.file -eq "CodexAppManager.exe") "联合清单中的管理器本体路径不正确"
Assert-Condition ($usbManifest.manager.role -eq "requireAdministrator USB Tauri manager started by the launcher or directly") "联合清单未声明 USB 管理器 requireAdministrator 边界"
Assert-Condition ($usbManifest.manager.sha256 -ieq $managerHash) "联合清单中的管理器本体哈希与实际文件不一致"
Assert-Condition ([int64]$usbManifest.manager.size -eq $managerItem.Length) "联合清单中的管理器本体大小与实际文件不一致"
Assert-Condition ($usbManifest.ccswitch.executable -eq "ccswitch/CodexSwitch.exe") "联合清单的 CCSwitch 路径不正确"
Assert-Condition ($usbManifest.ccswitch.sha256 -ieq $ccswitchHash) "联合清单中的 CCSwitch 哈希与实际文件不一致"
Assert-Condition ([int64]$usbManifest.ccswitch.size -eq $ccswitchItem.Length) "联合清单中的 CCSwitch 大小与实际文件不一致"
Assert-Condition ($usbManifest.ccswitch.embeddedExecutable -eq "resources/ccswitch/CodexSwitch.exe") "联合清单的嵌入 CCSwitch 路径不正确"
Assert-Condition ($embeddedCcswitchHash -ieq $ccswitchHash) "根目录与资源中的 CCSwitch 载荷不一致"
if (-not $SkipSignature) {
    $webView2Signature = Get-AuthenticodeSignature -LiteralPath $webView2
    Assert-Condition ($webView2Signature.Status -eq "Valid") "WebView2 安装包 Authenticode 签名无效"
    Assert-Condition ($webView2Signature.SignerCertificate.Subject -match "Microsoft Corporation") "WebView2 安装包签名者不是 Microsoft Corporation"
    $signature = Get-AuthenticodeSignature -LiteralPath $vcRedist
    Assert-Condition ($signature.Status -eq "Valid") "VC++ 运行库 Authenticode 签名无效：$($signature.Status)"
    Assert-Condition ($signature.SignerCertificate.Subject -match "Microsoft Corporation") "VC++ 运行库签名者不是 Microsoft Corporation"
}

$failed = @()
$listed = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
Assert-Condition (Test-Path -LiteralPath $sumsPath -PathType Leaf) "缺少 SHA256SUMS.txt"
foreach ($line in Get-Content -LiteralPath $sumsPath) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line -split '\s+', 2
    Assert-Condition ($parts.Count -eq 2) "SHA256SUMS.txt 格式错误：$line"
    $relative = $parts[1].Trim()
    $relative = $relative.Replace('\', '/')
    if (-not $listed.Add($relative)) {
        $failed += "$relative（SHA256SUMS.txt 重复清单项）"
        continue
    }
    $file = Join-Path $root ($relative.Replace('/', '\'))
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        $failed += "$relative（文件不存在）"
        continue
    }
    $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $parts[0].ToLowerInvariant()) {
        $failed += "$relative（哈希不匹配）"
    }
}
# A stale file left by an interrupted/repeated build must not be silently
# accepted: it could contain an older payload that the manifest never covers.
$rootFull = [System.IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
foreach ($fileItem in Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object { $_.Name -ne "SHA256SUMS.txt" }) {
    $fileFull = [System.IO.Path]::GetFullPath($fileItem.FullName)
    if (-not $fileFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        $failed += "$($fileItem.FullName)（文件不在包根目录内）"
        continue
    }
    $relative = $fileFull.Substring($rootFull.Length).Replace('\', '/')
    if (-not $listed.Contains($relative)) {
        $failed += "$relative（未列入 SHA256SUMS.txt）"
    }
}
Assert-Condition ($failed.Count -eq 0) ($failed -join '; ')

$ccswitchVerifier = Join-Path $root "ccswitch\verify-product.ps1"
Assert-Condition (Test-Path -LiteralPath $ccswitchVerifier -PathType Leaf) "缺少 CCSwitch 验证脚本"
$args = @{ ArtifactDir = (Join-Path $root "ccswitch"); SkipNetwork = $SkipNetwork; PromptForApiKey = $PromptForApiKey }
if ($ApiKey) { $args.ApiKey = $ApiKey }
& $ccswitchVerifier @args

[pscustomobject]@{
    ready = $true
    mode = "oneclick"
    installer = $installer
    ccswitch = $ccswitch
    embeddedCcswitch = $embeddedCcswitch
    vcRuntime = $vcRedist
    webView2Runtime = $webView2
    codexPayload = $payloadReport
} | ConvertTo-Json -Depth 8
