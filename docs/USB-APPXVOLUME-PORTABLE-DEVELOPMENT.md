# Codex U 盘版（USB AppXVolume + 本地数据）开发需求文档

> 文档状态：V1.2 历史设计；当前实现约束见 [v0.5.7 审计与加固](releases/v0.5.7-audit.md)。旧文中的系统卷 bootstrap、策略失败继续和无数据保障卸载回退已废止。
>
> 适用范围：Windows x64 离线 U 盘版 Codex Desktop + CCSwitch
> 更新日期：2026-09-02

## 1. 目标与边界

### 1.1 产品目标

在不改写 Codex 官方 MSIX 内容、不制作独立 Codex 外壳、不引入隐藏系统服务的前提下，
提供一个面向小白用户的一键入口：

1. 从 U 盘读取经过清单、SHA-256、包身份和架构校验的 Codex MSIX，并在本地暂存副本上完成
   安装前的 Authenticode 校验；
2. 把 Codex 的**程序文件**注册到当前 U 盘的 Windows AppX 卷；
3. 默认把 Codex/CCSwitch 的**用户数据**留在当前 Windows 用户的本地目录；若目标系统
   拒绝数据策略，必须明确提示 Codex AppX 数据可能随包卷保存；
4. 自动从 U 盘启动改造版 CCSwitch，用户只需填写 API Key；
5. U 盘拔出后，下一次启动无法读取程序文件；已运行进程不承诺固定时间内退出；
6. 卸载时只清理本产品有证据证明属于自己的安装记录和数据，且恢复本产品设置过的系统策略。

USB 入口权限交互：根目录 `CodexDesktopSetup.exe` 是独立的轻量启动器，嵌入 Windows
`requireAdministrator` 清单。双击时由 Windows 在创建 Tauri/WebView 界面前显示一次标准
UAC；确认后启动同目录的 `CodexAppManager.exe`，整个 USB 管理器和它执行的 AppX 卷安装
继承已批准的完整管理员令牌。正常安装在本机的管理器仍保持 `asInvoker`，不会因此每次启动都弹
UAC。直接双击管理器时，管理器自身仍保留仅限 USB 布局的原生 `ShellExecute` `runas` 回退。
该设计是显式同意的提权，不修改 UAC/组策略，也不把拒绝提升变成“成功”。

### 1.2 明确不做的事情

- 不把 MSIX 解包成 C 盘 portable 版作为 USB 版的静默回退；
- 不通过全局镜像名杀进程、不拦截所有开始菜单 Codex、不创建隐藏常驻服务；
- 不承诺对专业逆向、管理员或可写本机注册表的用户提供 DRM 级防复制；
- 不把 API Key、登录令牌或用户配置写入 U 盘；
- 不把“拔盘后 5 秒结束进程”作为 AppXVolume 的保证。Windows 可能缓存已加载页面，
  运行中的进程是否结束取决于其后续访问和现有的会话看门狗。

## 2. 关键技术决策

### 2.1 文件与数据分离

```text
U 盘（例如 F:\）                         本机系统盘（例如 C:\）
┌────────────────────────────┐          ┌───────────────────────────────┐
│ CodexDesktopSetup.exe       │          │ %LOCALAPPDATA%\Packages\...   │
│ CodexAppManager.exe         │          │ %LOCALAPPDATA%\CodexUsbGuard\ │
│ manifest.json               │          │ Codex AppX LocalState/Cache    │
│ payload\OpenAI.Codex_*.msix│          │ %USERPROFILE%\.codex            │
│ ccswitch\CodexSwitch.exe   │          │ %USERPROFILE%\.cc-switch         │
│ usb-manifest.json           │          │ %LOCALAPPDATA%\CodexUsbAppx     │
│ SHA256SUMS.txt              │          │   data-policy.json               │
└────────────────────────────┘          └───────────────────────────────┘
       │                                          │
       └─ Add-AppxPackage -Volume F:\             └─ 用户数据和日志
```

Windows 的 `Add-AppxPackage` 支持 `-Volume`，并允许把包部署到注册的 `AppxVolume`；
Microsoft 的文档说明包卷也会影响默认的用户数据位置。因此 USB 版在部署前尝试设置
`RestrictAppDataToSystemVolume=1`，把用户数据强制放回系统卷；如果当前账户不能批准
UAC、系统版本/版本策略不支持该设置，且目标卷已经预先注册，安装仍可继续但会明确提示
数据可能随包卷保存；如果卷注册也需要授权，程序会在明确收到 `Add-AppxVolume` 的权限拒绝后
针对当前卷请求一次标准 UAC，仍失败才停止并给出重试诊断。
[Add-AppxPackage](https://learn.microsoft.com/en-us/powershell/module/appx/add-appxpackage?view=windowsserver2025-ps)、[ApplicationManagement Policy CSP](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-applicationmanagement)

### 2.2 为什么不是把 MSIX 复制到任意 U 盘目录

`Add-AppxPackage -Volume` 接受的是 Windows 注册的 AppX 卷，而不是普通目录。实现时使用
`F:\WindowsApps` 作为包存储路径，并通过 `Get-AppxVolume` / `Add-AppxVolume` 确认卷已经注册。
`Add-AppxVolume` 属于机器级 AppX 元数据写入；某些 Windows 版本允许普通用户读写 U 盘，
却会让未提升的注册调用返回 `Access is denied`。如果数据策略尚未启用，安装器会将
`RestrictAppDataToSystemVolume=1` 和当前 U 盘卷注册合并为一个有界的 `Start-Process -Verb RunAs`
操作；子操作失败会在管理员进程内回滚策略，父进程不会写入无主的恢复记录。组合操作若被拒绝、
超时或以其他原因失败，不能被标记为“卷已注册”；随后普通权限调用明确返回
`Add-AppxVolume`/`Access is denied` 时仍要另行弹出一次标准 UAC。只有组合操作确认成功时才抑制
这次针对性重试。USB 入口 `CodexDesktopSetup.exe` 已通过嵌入式 `requireAdministrator` 清单
在显示界面前完成一次标准 UAC，并以管理员令牌启动 `CodexAppManager.exe`，因此进入安装命令
时通常已经拥有管理员令牌；如果 Windows 仍要求卷级授权，现有的有界子进程仍会执行精确的
当前卷操作。正常本机版仍不强制提升。用户拒绝 UAC 或没有管理员凭据时必须停止并报告，
不存在合法的静默绕过方式。管理员子进程有 120 秒上限并返回具体 PowerShell 异常，避免 UAC
对话框无人处理时界面无限等待。
临时管理员脚本的外层与子进程均显式传入 `-ExecutionPolicy Bypass`，因此不会再被目标机默认
`Restricted/Undefined` 执行策略拦截；这是仅限本次 PowerShell 子进程的参数，不写入任何执行策略，
不关闭 Defender。若 MachinePolicy/UserPolicy、WDAC 或 AppLocker 强制禁用 PowerShell，仍应按
企业管理员策略处理并让安装停止。
脚本优先用 `Get-AppxVolume -Path <盘符根> -Online` 重新取得当前在线的原生
`AppxVolume` 对象，避免把换盘、重格式化后留下的离线旧对象传给 `-Volume`；必要时会先
调用 `Mount-AppxVolume`。调用 AppX 命令前还会使用 `Get-Volume` 预检盘符是否解析为本地
NTFS 卷；网络映射、WebDAV、exFAT/FAT32 直接给出可读错误，不再让 AppX 服务返回含义
不清的“Invalid URI/hostname”消息。安装参数固定使用文档定义的 `-Path`（不依赖旧的
`LiteralPath` 别名），并保留 `AppxVolume` 原生对象。每个失败分支还会记录 `operation=...`、
目标卷、对象类型/在线状态以及已解析的 AppXVolume 信息，便于在另一台 Windows 机器上
定位是卷注册、挂载、bootstrap、迁移还是最终安装失败。
如果 PowerShell 在 USB `-Volume` 参数集内部仍抛出“Invalid URI/hostname”，脚本会先调用
WinRT `PackageManager.AddPackageAsync(file:// URI, PackageVolume)`，绕过包装层直接部署
（参见微软的[PackageManager.AddPackageAsync](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager.addpackageasync?view=winrt-28000)）；
只有原生调用失败后才进入显式系统卷 bootstrap，再用 `Move-AppxPackage` 迁移。
卷的身份以 Windows 媒体 ID 为主，不能只把盘符当作永久身份。[Add-AppxVolume](https://learn.microsoft.com/en-us/powershell/module/appx/add-appxvolume?view=windowsserver2025-ps)、[PackageVolume](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.management.deployment.packagevolume?view=windows-app-sdk-2.0)

### 2.3 选择当前 U 盘而不是让前端传路径

后端通过 `current_exe()` 找到安装器所在 bundle，再从其盘符派生卷根。渲染层只能触发
“USB 安装”模式，不能传入任意 `C:\`、UNC 或网络路径。后端再次验证路径必须是形如
`F:\` 的本地盘根，并要求 USB guard 当前状态为 `authorized`。

## 3. 端到端工作流

### 3.1 预检阶段

1. `usb_guard::start_for_manager` 验证可移动属性、卷序列号、清单锚点哈希和 bundle 相对路径；
2. `offline_payload_probe` 验证 `manifest.json`、载荷大小、SHA-256、包身份、版本、架构和入口。
   为避免在 U 盘上触发大型 MSIX 的慢速签名扫描，首次预检不执行 Authenticode；界面明确显示
   签名会在安装阶段验证；
3. 检查 Windows Build、剩余空间和 `Add-AppxPackage` 能力；真正部署前由安装脚本
   再检查 `Get/Add-AppxVolume` 与 `Add-AppxPackage -Volume` 参数；
4. 任何包文件缺失或身份不匹配都在 UI 中阻断，不执行安装。

### 3.2 暂存阶段

现有安全链路仍把 MSIX 复制到 `%TEMP%\CodexDesktopInstaller\<version>` 并二次计算 SHA-256，
然后对**本地暂存副本**执行 OpenAI Marketplace Authenticode 校验。签名校验使用独立的 5 分钟
总时限，而不是普通 PowerShell 探测的 45 秒；这样不会把 U 盘读取、Defender 扫描和证书链
检查的正常耗时误判成签名无效。只有本地副本的哈希和签名都通过，才会进入 AppX 部署。
这只是安装期间的临时读缓存，不是最终程序安装位置；安装完成后删除。暂存目录必须位于
受保护的临时根下，不能接受渲染层任意删除路径。

### 3.3 AppXVolume 部署阶段

后端以 `msix-usb-volume` 模式进入现有 `install_windows_historical_release`：

1. 再次检查确认时的已安装状态，避免用户在校验后替换版本；
2. 再次复核 U 盘租约、卷序列号、清单锚点和盘符；如果策略值不是 `1`，在关闭进程前以一次
   有界 UAC 操作同时设置 `HKLM\Software\Policies\Microsoft\Windows\Appx\RestrictAppDataToSystemVolume=1`
   并确认 `<USB 根>\WindowsApps` 已注册为 AppXVolume。卷注册失败时管理员子进程会回滚策略，
   不写入恢复记录；成功后把旧值写入 `%LOCALAPPDATA%\CodexUsbAppx\data-policy.json`；
3. `CodexDesktopSetup.exe` 的嵌入式 `requireAdministrator` 清单已经在显示界面前获得用户
   确认，并以完整管理员令牌启动 `CodexAppManager.exe`；因此通常可直接确认卷。若 Windows
   仍让 `Add-AppxVolume` 返回 `Access is denied`，请求一次只针对该卷的 UAC 提升。
   若此前组合策略+卷注册 UAC 没有成功，也不能跳过这次重试。
   授权前后复核 U 盘身份，再执行同一份已校验的 MSIX；UAC
   被拒绝、超时或 U 盘发生替换时保留原始 `operation=Add-AppxVolume` 诊断，不尝试 C: portable；
4. 关闭已有的**本产品托管** Codex 进程；
5. 若同一用户已有 Codex 注册在其他卷，调用 `Move-AppxPackage` 先迁移到目标卷，
   再调用 `Add-AppxPackage` 更新固定 MSIX；这样避免 `0x80073CF9` 的应用数据卷冲突；
   如果卸载时选择保留数据导致注册已消失但 `C:\Users\<用户>\AppData\Local\Packages\OpenAI.Codex_*`
   仍存在，且 `RestrictAppDataToSystemVolume` 未生效，则先用同一 MSIX 在系统卷 bootstrap，
   再用 `Move-AppxPackage` 迁移到 USB，保留数据且不生成 portable 副本；
6. 调用 `Add-AppxPackage`，参数包含文档定义的 `-Path $packagePath`、`-Volume $volume`、
      `-ForceUpdateFromAnyVersion` 和 Windows 版本支持的关闭旧进程参数；如果某些 Windows
      版本在 USB `-Volume` 路径内部抛出“Invalid URI/hostname”，先调用 WinRT
      `PackageManager.AddPackageAsync` 搭配原生 `PackageVolume` 重试；仅当该调用也失败时，
      才改走“显式系统卷 bootstrap → `Move-AppxPackage` 到 USB”的固定 MSIX 恢复路径，
      不切换为 portable；
7. USB 路径给 `Add-AppxPackage` 使用独立的 10 分钟有界时限（普通路径仍为 3 分钟），覆盖
   慢速闪存、Defender 扫描和 AppX 服务排队；超时会结束 PowerShell 进程树并保留可重试的
   `process exceeded total deadline` 诊断，不会无限等待；
8. 读取注册包，报告 `InstallLocation`、版本、FamilyName 和 `packageVolume`；
9. 进行已有的 AppX 注册、AUMID、框架依赖和真实激活健康检查；
10. 健康检查失败、卷注册失败或迁移失败时，返回诊断并**不**改装 portable 版；如果卷已经
    可用而只有数据策略无法应用，才降级为明确警告，提示应用数据可能随包卷保存。

### 3.4 CCSwitch 阶段

1. 从 `ccswitch\CodexSwitch.exe`（或打包资源旁路）读取并校验 `manifest.json` 中的哈希；
2. USB 模式直接启动 U 盘上的 EXE，工作目录为其所在目录；不复制到
   `%LOCALAPPDATA%\Programs\CodexSwitch`；
3. 通过环境标记 `CODEX_USB_CCSWITCH_MODE=usb` 提供诊断信息，但不依赖未确认的第三方
   命令行参数；
4. CCSwitch 使用其正常的 Windows 用户目录保存 providers、API Key、登录状态和 WebView
   缓存。安装器不把用户数据路径改到 U 盘，也不把 API Key 写入清单；
5. 首次启动显示 404TOKEN（`https://404token.xyz/v1`）配置框，用户输入 API Key 后由
   CCSwitch 自己保存并启用。

如果 CCSwitch 进程已经在运行，卸载仍会按本产品 USB lease 中的精确 PID/可执行路径关闭，
并清理本产品可能遗留的本地副本；绝不按全局镜像名终止其他用户安装。

### 3.5 拔盘语义

| 场景 | 预期 |
| --- | --- |
| U 盘在，管理器启动 Codex | AppX 包可读，允许启动 |
| U 盘拔出后再次从管理器启动 | USB guard 拒绝，包路径也不可读 |
| U 盘拔出后再次从开始菜单启动 | Windows 通常无法启动新实例；若系统仍显示入口，错误由系统返回 |
| 已有 Codex 进程仍在运行 | 可能继续运行到下一次访问包文件，不能承诺固定 5 秒 |
| U 盘换盘符 | AppX 注册卷由 Windows 管理，guard 通过卷指纹重新发现；需实机验证 |
| 复制整个目录到 C 盘 | guard 锚点/可移动属性不匹配；且 AppX 注册仍指向原卷 |

现有 1 秒轮询、5 秒宽限的看门狗只作为管理器会话辅助清理。它不承担“拦截任意开始菜单
启动”的产品承诺，也不应成为隐藏常驻程序。

## 4. 代码改造清单

### 4.1 已实现的后端模块

| 文件 | 改造 |
| --- | --- |
| `crates/codex-win-engine/src/sys.rs` | 新增 `install_msix_to_volume_with_observer`；PowerShell 脚本检查并注册 AppXVolume、传入 `-Volume`；`InstalledWindowsCodex.packageVolume` 描述实际卷 |
| `crates/codex-win-engine/src/lib.rs` | 导出 USB 卷安装入口，更新安全边界说明 |
| `src-tauri/src/app/appx_volume.rs` | 校验盘根、读取/设置/恢复 AppX 数据策略；记录前值并支持幂等卸载 |
| `src-tauri/src/app/usb_guard.rs` | 暴露 `current_bundle_volume_root()`，后端从当前 EXE 派生卷根 |
| `src-tauri/src/app/win_update.rs` | 新增 `USB_APPX_INSTALL_MODE`；USB 路径禁用 portable fallback；成功卸载后恢复策略 |
| `src-tauri/src/app/ccswitch.rs` | USB 模式直接启动 U 盘 EXE；普通模式仍保留本地复制行为 |
| `src-tauri/src/commands.rs` | `win_install_historical_release` 增加可选 `installToUsb`，只由离线 UI 传 `true` |

### 4.2 前端契约

`managerApi.winInstallHistoricalRelease` 的最后一个可选参数为 `installToUsb`。离线主页
固定传入 `true`；在线主页和历史版本页继续使用原有行为。返回的
`InstalledWindowsCodex.packageVolume` 和 `CcswitchLaunchReport.launchMode` 可用于后续 UI
显示，旧 JSON 没有这两个字段时按 `null`/旧模式兼容。

### 4.3 不应再新增的旧路线

- `APPX-PROCESS-GUARD-DEVELOPMENT.md` 中的长期登录任务、开始菜单全局拦截、AppX + PID
  扫描不是本方案验收项；
- 不在 USB 版中调用 `install_portable_after_stage`；
- 不在 UI 文案中写“安装完成后即可拔出 U 盘继续使用”；
- 不复制 CCSwitch EXE 到 C 盘作为 USB 模式的默认路径。

## 5. AppX 数据策略设计

### 5.1 注册表事务

记录文件：`%LOCALAPPDATA%\CodexUsbAppx\data-policy.json`

```json
{
  "schemaVersion": 1,
  "product": "codex-usb-oneclick",
  "previousValue": null,
  "appliedValue": 1,
  "appliedAtMs": 0
}
```

规则：

1. 当前值已经是 `1` 时不弹 UAC，不写入恢复记录；
2. 当前值为 `0` 或不存在时，先尝试经 UAC 设置为 `1`，再写入恢复记录；
3. Add-AppxPackage 明确失败且没有目标包落地时，尝试立即恢复；
4. 卸载本产品管理的 Codex 成功后，按记录恢复 `0` 或删除原本不存在的值；
5. 恢复失败不掩盖主卸载结果，但必须在报告和日志中显示；
6. 策略提升失败时继续 USB 包安装并在结果中警告；只有“设置成功但恢复记录无法写入且
   回滚也失败”才中止，避免留下无主的机器级设置；
7. 该策略是机器级设置，产品 UI、README 和卸载页必须明确告知用户。

### 5.2 风险与取舍

该策略可能影响其他 AppX 应用的默认数据卷选择，因此不是“无副作用”的用户级开关。
当前版本选择可预测的“安装一次、卸载恢复”路径；如果策略不可用，产品仍能安装但数据
可能落在 USB AppX 卷。商业发布前应在 Windows 10/11、标准用户、
企业策略锁定和已有值为 `1` 的机器上完成实机验证。若产品未来需要严格避免机器级策略，
只能接受 Codex 数据跟随 AppXVolume 的事实，或改造 Codex 本身，二者都超出本版范围。

## 6. UI 与一键设置入口

### 6.1 离线主页

保留现有 Codex App Manager 视觉样式，只把状态卡文案改为：

- `USB AppXVolume`：显示目标盘符和“程序文件在 U 盘”；
- `本地数据`：显示“Codex/CCSwitch 配置在当前用户目录”；
- `管理员确认`：USB 入口 `CodexDesktopSetup.exe` 的嵌入式 `requireAdministrator` 清单会
  先显示一次标准 UAC；确认后启动 `CodexAppManager.exe` 并在管理员令牌下运行，仍在每个机器级动作前后复核 U 盘身份。普通本机版不强制显示 UAC；任何路径都不静默绕过
  系统权限；
- `便携回退`：固定显示“关闭（USB 模式不生成 C 盘程序副本）”；
- `CCSwitch`：显示“USB 直接启动 / 本地用户数据”。

安装按钮只做一件事：调用预检 → 暂存 → USB AppX 安装 → 健康检查 → CCSwitch 启动。
用户不需要再选择 C 盘安装目录。API Key 仍在 CCSwitch 首次启动对话框中填写，不在安装器
里收集或落盘。

### 6.2 设置与卸载

“设置 → 更多”保留现有“卸载 Codex 与 CCSwitch”。卸载页应新增一行：

> 如果本产品曾为 USB AppX 版设置系统数据策略，卸载成功后会尝试恢复原值；用户数据是否
> 删除仍由“保留我的数据”二次确认决定。

当本安装器的 provenance 记录存在且与当前版本匹配时，卸载只处理托管的 Codex。若记录
缺失但当前用户仍有 `OpenAI.Codex` MSIX（例如旧版本在部署完成前崩溃，未能保存记录），
USB 联合卸载页在明确警告和二次确认后允许移除这个当前用户注册。Windows 的包身份不能
证明它来自哪个安装器，因此界面必须提示“如果来自其他来源请取消”；普通“卸载 Codex”
入口仍拒绝未托管的外部安装。`Remove-AppxPackage` 返回后必须再次查询当前用户注册，
仍存在时报告失败而不是假报成功。

卸载顺序：

```text
关闭本产品租约进程
      ↓
按精确路径关闭 CCSwitch；Remove-AppxPackage 逐个重试（托管包；USB 联合卸载可按确认清理无记录的当前用户 OpenAI.Codex）
      ↓
删除托管便携目录、快捷方式、卸载注册项和 USB 所有权记录，并逐项复核不存在
      ↓
恢复并复核 data-policy.json 对应的系统策略
      ↓
按确认清理已知 Codex/CCSwitch 用户数据；保留数据时明确跳过，并在最后重新检测残留
```

每个文件系统删除操作都有有限重试和 `symlink_metadata` 后置检查；AppX 卸载在
`Get-AppxPackage -Name OpenAI.Codex` 为空前持续轮询。任一步骤仍有残留时，报告为部分失败
并保留对应恢复动作，不能显示为“已完全卸载”。“清除数据”只覆盖明确列出的当前用户目录，
不递归清空 `%LOCALAPPDATA%`/`%APPDATA%`，也不处理其他用户、系统预置包或外部安装器的便携目录。

部分旧版或普通 sideload 包会在运行时拒绝 `Remove-AppxPackage
-PreserveApplicationData`（HRESULT `0x80073CFA`），即使 PowerShell 参数存在；更旧的 AppX
模块可能完全不暴露该参数。v0.5.6 安装错盘回滚和显式卸载都会在这两种明确的兼容性情形
下改用不带该标记的普通卸载，并在结果中说明包内数据保留不确定；其他错误仍按失败处理。

U 盘已经拔出时，管理器可显示“包卷不在场”；不应借此删除未知 AppX 包。用户可重新插回
原 U 盘完成卸载，也可在 Windows“已安装的应用”中手动移除后再重试策略恢复。

## 7. 发布目录与构建要求

### 7.1 固定布局

```text
usb-oneclick/
├─ CodexDesktopSetup.exe              # requireAdministrator USB launcher
├─ CodexAppManager.exe                # USB 专用 requireAdministrator Tauri manager
├─ manifest.json
├─ usb-manifest.json
├─ SHA256SUMS.txt
├─ payload/OpenAI.Codex_x64.msix
├─ ccswitch/CodexSwitch.exe
├─ ccswitch/manifest.json
├─ ccswitch/providers.json
├─ resources/ccswitch/...                 # Tauri 旁路资源
├─ verify-usb.ps1
├─ verify-usb-launcher.ps1
└─ verify-payload.ps1
```

`build-oneclick-usb-layout.ps1` 必须在联合清单中声明：

- `appxVolume.packageStore = <USB-drive>\\WindowsApps`；
- `appxVolume.dataPolicy = RestrictAppDataToSystemVolume=1`；
- `appxVolume.portableFallback = false`；
- `ccswitch.launchMode = usb-direct`；
- `secretsIncluded = false`。
- `startupElevation.mode = usb-launcher-requireAdministrator`；
- `startupElevation.mechanism = embedded requireAdministrator manifest + standard Windows UAC consent`；
- `manager.file = CodexAppManager.exe`，且入口与管理器分别做 SHA-256 校验。

构建完成后必须运行 `verify-usb-launcher.ps1 -Path <CodexUsbLauncher.exe>`，它直接读取
PE 的 `RT_MANIFEST` 资源并检查 `requestedExecutionLevel=requireAdministrator`；最终包把该脚本
复制到根目录，`verify-usb.ps1` 会在其他大文件校验前自动调用它。

MSIX 仍以旁路文件保存，避免把约 800 MB 载荷嵌入 Tauri EXE。`manifest.json`（Codex 载荷
清单）和 `usb-manifest.json`（联合清单）不得互相覆盖。正式包应使用签名 USB license；
开发包的 unsigned auto-enroll 只用于内部验证。

### 7.2 Windows 兼容性

目标为 Windows 10 19041+ / Windows 11 x64。部署前必须检查 `Get-AppxVolume`、
`Add-AppxVolume` 和 `Add-AppxPackage -Volume` 参数是否存在。若目标系统不支持，显示明确
的“USB AppX 卷不可用”，不能暗中切到 portable。MSIX 默认路径通常是
`C:\Program Files\WindowsApps`，但 Windows 支持配置其他 PackageVolume。[Packaged desktop app behind the scenes](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)

## 8. 测试与验收

### 8.1 自动测试

```powershell
cargo test --manifest-path .\src-tauri\Cargo.toml
cargo check --manifest-path .\src-tauri\Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol
npm run check
npm run lint
npm test -- --run src
npm run build
npm run test:offline-bundle
```

必须覆盖：

- `validate_volume_root` 只接受盘符根，拒绝 UNC、相对路径和子目录；
- AppXVolume PowerShell 脚本包含 `Get-AppxVolume`、`Add-AppxVolume`、`-Volume` 和清晰错误；
- 旧 MSIX JSON 缺失 `packageVolume` 时仍可读取；
- USB 模式请求使用 `installToUsb=true`，普通在线调用不带该字段；
- CCSwitch USB 报告为 `launchMode=usb` 且不调用本地复制函数；
- 旧版 bundle layout 清单不再写“复制 CCSwitch 到本地后可拔盘”。

### 8.2 Windows 实机验收矩阵

| 场景 | 预期 |
| --- | --- |
| 空白 Windows、NTFS U 盘、批准合并 UAC | 一次授权完成策略和卷注册，包位于 U 盘 AppXVolume，数据位于 C: 用户目录 |
| USB 入口双击、登录账户为管理员、批准启动 UAC | `CodexDesktopSetup.exe` 的嵌入式清单触发 UAC，并以管理员令牌启动 `CodexAppManager.exe`；随后完成策略/卷注册和固定 MSIX 安装，不生成 C: portable |
| 策略已为 1、管理员令牌下 `Add-AppxVolume` 仍返回 Access is denied、批准卷注册 UAC | 只提升当前 USB 卷注册动作，身份复核后重试 MSIX |
| 启动 UAC 被拒绝或取消 | 在安装前退出，保留明确的 UAC/账户诊断，不伪报安装成功 |
| 目标卷注册 UAC 被拒绝或超时 | 安装停止，详情保留 `operation=Add-AppxVolume`、UAC 状态和权限提示；不生成 C: portable |
| 已存在 `RestrictAppDataToSystemVolume=1` | 不再弹第二次 UAC，安装成功 |
| USB AppX 部署遇到慢速介质或 Defender 扫描 | 使用独立 10 分钟上限；超时可重试并保留日志，不无限等待 |
| 用户拒绝 UAC、但卸载后 C: 仍有 Codex 数据 | 先在系统卷 bootstrap 再迁移到 USB，保留数据；不出现 C: portable 副本 |
| 启动 UAC 被拒绝、但卷已预先注册 | 仍在安装前退出；用户重新启动并批准 UAC 后再继续，避免管理员/普通令牌状态不一致 |
| U 盘为 exFAT/FAT32 | 安装前明确拒绝并提示必须改用本地 NTFS；不尝试把 MSIX 静默回退到 C: 或便携版 |
| 安装后查看 `Get-AppxPackage` | `InstallLocation` 在 `<USB>\WindowsApps`，`PackageFamilyName` 正确 |
| 关闭管理器后从开始菜单启动 | USB 在场可运行；USB 不在场不得承诺启动成功 |
| CCSwitch 首次配置 | 404TOKEN provider 默认启用，API Key 仅写本机用户目录 |
| 拔 U 盘后再点管理器启动 | guard 拒绝，且不会从 C: 复制启动 Codex/CCSwitch |
| 卸载并保留数据 | 包/本地 CCSwitch 程序清理，`.codex`/`.cc-switch` 保留，策略恢复；无 provenance 的当前用户 OpenAI.Codex 需二次确认后清理 |
| 卸载并清除数据 | 二次确认后清理明确列出的目录；其他 CCSwitch 不受影响 |
| 另一块 U 盘复制同目录 | 卷指纹/清单锚点不匹配；不应视为已授权安装器 |

自动化测试不能替代真实 AppX 部署、Authenticode、UAC 和拔盘测试；这些必须在干净的
Windows 10/11 虚拟机或备用机器记录日志和截图。

## 9. 安全、杀毒与合规

本方案不试图绕过安全软件。未签名 EXE、从可移动介质运行、创建 `WindowsApps` 卷、
请求 UAC 和轮询/关闭本产品子进程都可能触发 SmartScreen 或杀毒提示。免费发布时的
正规优化是：公开源代码和哈希、使用固定的 GitHub Release、保持清晰安装/卸载入口、
不混淆/加壳/注入、不创建隐藏任务，并在文档中解释机器级 AppX 数据策略。微软说明新文件
和未签名文件需要积累信誉，不能保证不告警。[SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)

## 10. 迁移与回滚

### 10.1 从旧版本迁移

旧版本可能已经把 Codex 装在系统 AppXVolume、把 CCSwitch 复制到
`%LOCALAPPDATA%\Programs\CodexSwitch`。USB 版首次安装前：

1. 读取当前托管记录；普通卸载不自动删除外部 Codex，USB 联合卸载只有在用户明确确认时才会清理当前用户 OpenAI.Codex MSIX；
2. 若发现旧的本地 CCSwitch 副本，先按现有所有权标记保留，安装成功后由卸载入口处理；
3. USB 版成功后把 provenance 记录更新为 MSIX + `packageVolume`；
4. 如果用户不批准 UAC，不修改旧安装，不做 portable 迁移。

### 10.2 失败回滚

- 策略设置成功但 AppX 部署尚未开始：恢复策略；
- AppX 部署明确失败且目标包未落地：恢复策略，保留临时 MSIX 供重试；
- 结果不确定或目标包已注册：不盲目恢复策略，报告需重试/卸载；
- 卸载失败：保留 journal 和日志，下一次卸载继续尝试；
- 不执行 `git reset --hard`、不删除用户未授权的其他 AppX/CCSwitch 数据。

## 11. 版本验收标准

V1.1 只有在以下条件满足后才算完成：

1. 离线主页只能触发 `msix-usb-volume`，不会产生 C: portable Codex 副本；
2. 空白 Windows 上真实 `Get-AppxPackage.InstallLocation` 落在 U 盘 AppXVolume；
3. Codex/CCSwitch 的用户配置在本机用户目录，U 盘不包含 API Key；
4. U 盘拔出后管理器拒绝新启动，开始菜单行为有实机记录且文案不夸大；
5. UAC 拒绝、AppXVolume 不支持、健康检查失败都有明确错误和可重试路径；
6. 卸载能按所有权清理程序，并在安全时恢复策略；
7. 自动测试全部通过，真实部署和拔盘矩阵完成；
8. 旧的长期 AppX 进程保护方案只保留为历史记录，不再作为本项目开发基线。
