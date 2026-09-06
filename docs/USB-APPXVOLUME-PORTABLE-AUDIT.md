# USB AppXVolume + 本地数据方案审计

> 历史审计快照：现行基线为 [v0.5.7-audit.md](releases/v0.5.7-audit.md)。本文旧版通过记录不代表当前版本或真机验证；系统卷 bootstrap 和策略失败继续的行为已废止。

审计对象：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`

审计日期：2026-09-03
审计基线：[USB-APPXVOLUME-PORTABLE-DEVELOPMENT.md](./USB-APPXVOLUME-PORTABLE-DEVELOPMENT.md)

## 1. 当前实现核对

| 项目 | 结果 | 证据 |
| --- | --- | --- |
| USB 模式入口 | 已接入 | `OfflineHome` 调用 `winInstallHistoricalRelease(..., installToUsb=true)` |
| AppX 卷安装 | 已接入代码 | `codex-win-engine::install_msix_to_volume_with_observer` 生成 `Get/Add-AppxVolume` + `-Volume` 脚本 |
| 卷根来源 | 后端派生 | `usb_guard::current_bundle_volume_root()` 从 `current_exe()` 读取盘符；不信任渲染层路径 |
| portable 回退 | USB 模式禁用 | `USB_APPX_INSTALL_MODE` 不允许 `allows_historical_portable_recovery` |
| 用户数据 | 有明确策略实现 | `appx_volume.rs` 在需要时设置并记录 `RestrictAppDataToSystemVolume=1` |
| 策略恢复 | 已接入 | Codex MSIX 卸载成功或无安装记录时尝试恢复 journal 前值 |
| UAC 边界 | 已加固 | USB 专用 `CodexDesktopSetup.exe` 嵌入 `requireAdministrator` 清单；确认后仅启动 `CodexAppManager.exe`，普通安装管理器仍为 asInvoker，机器级动作保持卷范围复核与有界重试 |
| 提权超时与诊断 | 已加固 | 临时 UTF-8 BOM PowerShell 子脚本 + 结果 JSON；UAC/卷注册等待 120 秒，超时结束包装进程并保留子操作异常 |
| 提权前后换盘保护 | 已加固 | `verify_authorized_volume_root` 在关闭进程、UAC 前后重新匹配租约的卷序列号/锚点/相对路径和当前盘符 |
| USB 部署超时 | 已加固 | USB `Add-AppxPackage` 使用独立 10 分钟有界时限；普通部署仍为 3 分钟 |
| 目标卷后置条件 | v0.5.6 已加固 | 生成的 PowerShell 在 Add/WinRT/bootstrap/Move 全部路径后复核 `InstallLocation` 根；错盘注册不会被报告为成功，且本次新建的错盘注册会保留数据清理 |
| 完整管理员令牌 | v0.5.6 已加固 | 管理器在提交边界同时核对 `TokenElevation`、`TokenElevationType` 和有效 Administrators SID；管理员 PowerShell 子进程用 `whoami /groups` 拒绝 deny-only 过滤令牌 |
| PowerShell 传输 | v0.5.4 已加固 | 超过内联命令行阈值的生成脚本通过临时 UTF-8 BOM `.ps1` 传输，避免 Windows `os error 206` |
| MSIX 安装/卸载兼容性 | v0.5.6 已加固 | 错盘回滚、数据卷冲突和显式卸载统一处理明确的 `0x80073CFA`/deployment-mode 不兼容，以及旧模块缺少 `PreserveApplicationData` 参数的情况；只对这些兼容性路径去掉标记重试并做注册后置校验，其他权限/策略错误仍失败 |
| CCSwitch 程序位置 | USB 模式直接运行 | `ccswitch.rs` 在 `guard_required()` 时不调用本地复制函数 |
| CCSwitch 用户数据 | 依赖产品默认用户目录 | 通过正常 Windows 用户环境启动；未使用不受支持的 `--data-dir` 参数 |
| API Key | 未进入 bundle | 清单 `secretsIncluded=false`；由 CCSwitch 首次配置对话框输入 |
| 旧开始菜单长期看门狗方案 | 已废弃 | 旧开发文档删除并由新文档替代；历史审计保留为追溯材料 |

## 2. 自动验证结果

在本次改造阶段执行：

| 命令 | 结果 |
| --- | --- |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 通过 |
| `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --features custom-protocol` | 通过 |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib -- --test-threads=1` | 114/114 通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol --lib`（USB 外部 MSIX 迁移测试后） | 186/186 通过 |
| `cargo check --tests --target x86_64-pc-windows-msvc --features custom-protocol` | 通过 |
| `npm run check`、`npm run lint`、`npm test -- --run src/app/views/OfflineHome.test.tsx` | 通过（9/9 离线主页测试） |
| `npm test -- --run src` | 29 文件 / 275 测试通过 |
| `npm test -- --run` | 32 文件 / 340 测试通过 |
| `npm run test:release` | 2 文件 / 56 测试通过（Windows 使用 PowerShell 祖先提交校验脚本） |
| `npm run test:offline-bundle` | 通过（本轮等价 `tools/test-offline-bundle.ps1`：20 文件 / 19 哈希条目） |
| Windows PowerShell 5.1 语法/默认路径回归 | 9 个构建、收集和验证脚本通过 |
| `verify-usb.ps1` 残留文件防线 | 未列入 `SHA256SUMS.txt` 的临时文件按预期拒绝 |
| `tools/build-oneclick-usb-layout.ps1` | 通过（真实 MSIX 签名校验） |
| `tools/verify-usb-launcher.ps1` | 通过（PE `RT_MANIFEST` 含 `requestedExecutionLevel=requireAdministrator`） |
| `release/usb-oneclick/verify-usb.ps1 -SkipNetwork` | 通过 |

### A-10：可移动介质上的 Authenticode 探测超时（已修复）

目标机实测同一份约 765 MB 的 MSIX 签名有效（`Status=Valid`），但从 U 盘执行
`Get-AuthenticodeSignature` 单次耗时 42.52 秒，第二次约 58 秒；旧实现把它放在普通
PowerShell 探测的 45 秒总时限内，因此在签名真正返回前被杀掉，错误表现为
`process exceeded total deadline`，不是签名无效。当前实现把快速 `offline_payload_probe`
改为只做清单、哈希、身份和入口检查；安装阶段先将文件复制到受保护的本地暂存目录，二次
SHA-256 通过后再在本地副本上执行 Authenticode，并使用独立的 5 分钟有界时限。签名仍是
进入 `Add-AppxPackage` 前的强制信任门槛，没有关闭校验或接受未签名载荷。

前端检查、离线 bundle 检查和最终构建已在本阶段再次执行，并已追加到
`.codex-longrun/test-log.md`。自动检查不会模拟真实 Windows `Add-AppxPackage`、UAC、
Authenticode、AppXVolume 挂载或物理拔盘。

本轮还修正了一个会真实阻断安装的 TOCTOU 分支：无 provenance 的 C: 外部 MSIX 现在在
USB 模式允许迁移到当前 U 盘；前端和后端都只放行 `source=msix`，外部 portable 仍要求
显式接管。若已托管的同版本 MSIX 仍在 C:，离线主页也不会误显示“已安装”，而会要求迁移。

当前开发机仅做了非破坏性预检：Windows 11 Pro Build 26200 x64，F: 为
NTFS/Removable/Healthy，C: 约 54.8 GB 可用、F: 约 30.6 GB 可用；C: 和 F: AppXVolume
均 Online，`Add-AppxVolume -WhatIf`、`Move-AppxPackage -WhatIf`、
`Add-AppxPackage -Volume ... -WhatIf` 均接受。由于该机仍有官方 C: Codex 进程，未执行真实
迁移/安装/卸载；这避免把验证变成对用户环境的破坏。

### A-11：UAC 无限等待、重复授权和换盘竞态（已修复）

USB 入口现在由独立的 `CodexDesktopSetup.exe` 启动器承担。该文件带有嵌入式
`requireAdministrator` 清单，管理员账户双击后只需确认一次 UAC，随后它启动同目录的
`CodexAppManager.exe`，管理器直接继承完整管理员令牌；普通本机版管理器仍为 asInvoker，不受
影响。直接打开管理器时保留 USB 布局限定的原生 `runas` 回退。启动器不修改 UAC/组策略，也
不会把取消或拒绝转换成成功；标准账户若提供的是其他管理员凭据，Windows 可能把按用户注册
的 AppX 包写入凭据对应账户，因此产品支持矩阵以当前登录账户本身为管理员为保证条件。

旧的管理员卷注册辅助逻辑依赖 PowerShell `Start-Process -Wait`，没有明确的超时和子进程
结果，用户不处理 UAC 对话框时可能让前端一直处于安装中；策略与卷注册又可能连续弹出两次
授权。当前实现把具体操作写入带 UTF-8 BOM 的临时 `.ps1`，由管理员子进程写一次性结果 JSON，
父进程最多等待 120 秒并在超时后结束包装进程、清理临时文件；当策略尚未启用时，策略写入和
精确 USB 卷注册在同一 UAC 子操作内完成，卷注册失败会在子进程内回滚策略。只有组合 UAC
明确成功并完成卷注册时才抑制后续提示；如果组合操作被拒绝、超时或失败，普通调用随后明确
返回 `Add-AppxVolume`/`Access is denied` 时仍保留“针对该卷单独提升”的路径。
临时脚本的外层和管理员子进程都使用进程级 `-ExecutionPolicy Bypass`，兼容默认
`Restricted/Undefined` 环境；不修改执行策略、不关闭安全软件。企业 MachinePolicy/UserPolicy、
WDAC 或 AppLocker 若仍禁止 PowerShell，必须保留明确失败，不能静默绕过。

管理员对话框前后都重新验证当前进程持有的 USB lease，并通过卷序列号、清单锚点、bundle 相对
路径和盘符复核实际可移动卷；换盘或盘符复用会在 `Add-AppxVolume`/重试 MSIX 前停止，并尝试
恢复本次新写入的机器策略。USB `Add-AppxPackage` 另外使用 10 分钟硬上限，避免慢速闪存、
Defender 扫描或 AppX 服务排队被普通 3 分钟限制误判，同时仍会在真正卡死时返回可重试错误。

上述处理降低了可由安装器控制的假失败和半成品风险，但不能把 UAC 拒绝、企业策略锁定或
Windows 本身禁止可移动 AppX 卷变成静默成功；这些情况仍必须明确停止并保留诊断。

### A-12：发布脚本的系统 PowerShell 兼容性与残留文件（已修复）

Windows 10/11 通常仍提供 Windows PowerShell 5.1；无 BOM 的 UTF-8 脚本可能把中文字符串
解析成乱码，参数默认值中的 `$PSScriptRoot` 也可能在参数绑定阶段为空。发布链路现在为构建、
收集和验证脚本统一使用 UTF-8 BOM，并在脚本主体中解析默认目录；CCSwitch 校验器对 JSON
使用显式 UTF-8 解码。`verify-usb.ps1` 还会枚举包内所有文件，拒绝未列入 `SHA256SUMS.txt`
的残留文件以及重复清单项。本轮已在 Windows PowerShell 5.1 解析器和真实发布目录上
回归通过；入口 PE 的 `RT_MANIFEST` 还由 `verify-usb-launcher.ps1` 独立读取并检查，避免
链接器或打包脚本把 USB 启动器错误替换成普通 asInvoker 管理器。

### A-08：保留数据后的跨卷重装冲突（已修复）

在一次真实部署事件中，Windows AppXDeployment-Server 记录了 `0x80073CF9`：卸载保留的
`OpenAI.Codex_*` 用户数据仍位于 C:，而新的 Add-AppxPackage 目标是 U 盘 F:。旧实现
只处理“仍有包注册”的 `Move-AppxPackage`，因此在“包已卸载、数据仍保留”的重装路径上会
失败。当前安装脚本会检查系统卷上的 Codex 包数据；若数据存在且机器级数据策略未生效，
先在系统卷 bootstrap 同一已校验 MSIX，再调用 `Move-AppxPackage` 到 USB，并保留 HRESULT/
ActivityId 诊断。该路径由 PowerShell mock 回归测试覆盖；仍需在空白 Windows 10/11 机器上
验证真实数据迁移和异常回滚。

### A-09：PowerShell USB 卷 URI 兼容性（已增加原生回退）

目标机的日志显示 USB `-Volume` 安装和旧的系统卷 bootstrap 都可能从 PowerShell AppX
包装层返回“Invalid URI/未能分析主机名”，即使卷已经 Online 且文件系统为 NTFS。当前脚本
在识别该错误后，先把 MSIX 转成 `file://` `System.Uri`，通过 WinRT
`PackageManager.AddPackageAsync` 直接传入原生 `PackageVolume`；结果会记录
`IsRegistered`、`ActivityId`、扩展 HRESULT 和包 URI。只有该调用失败，才继续显式解析系统
卷 bootstrap + `Move-AppxPackage`，不会降级为 C: portable。单元测试覆盖原生成功分支、
原生失败后的系统卷迁移分支，以及最终诊断字段；仍需在用户目标 Windows 版本上做真实部署
验收，因为 AppX Deployment Server、企业策略和可移动介质驱动可能继续拒绝该卷。

### A-10：v0.5.3 错盘“假成功”（v0.5.4 已修复）

目标机曾出现 `MSIX sideload completed`，但随后 `Get-AppxPackage` 显示
`InstallLocation=C:\Program Files\WindowsApps\...`。此前脚本把 PowerShell cmdlet
“未抛异常”当作成功，Rust 层只在报告带有完整 `packageVolume` 时才能补救，因此错误注册
可能留在 C:。v0.5.4 把目标卷校验放进生成脚本本身：成功返回前必须观测到目标盘根；
若本次原先没有注册，则对错盘新注册执行保留数据的 Remove-AppxPackage，并在等待当前用户
注册消失后返回失败。Rust 层仍保留一次针对目标卷的注册/重试，兼容旧脚本或缺少后置字段
的结果。该逻辑由“静默 C: 成功”PowerShell mock 回归测试覆盖；真实 AppX 服务仍需目标机
验收。

## 3. 主要风险

### A-01：AppXVolume 和文件系统格式需要真实 Windows 验证

不同 Windows 版本、U 盘文件系统、BitLocker/策略和可移动介质驱动可能拒绝
`Add-AppxVolume` 或把卷注册为不可用。代码会报告错误并停止，不会静默安装 C: portable；
但只有 Windows 10/11 空白机实测才能确认最终兼容范围。

### A-02：本地数据策略是机器级设置

`RestrictAppDataToSystemVolume` 位于 HKLM，可能影响其他 AppX 应用。代码只在 USB 模式、
值不是 1 时通过合并的 UAC 操作请求授权，并在卸载时按 journal 恢复旧值；如果用户在安装后由组策略管理员修改
该值，卸载恢复可能失败或被策略重新覆盖。UI 和文档必须保留明确告知。

### A-03：运行中进程不具备固定拔盘时限

MSIX 代码页可能已经被 Windows 缓存。USB AppXVolume 能可靠阻止新的文件访问/启动，但不能
保证已运行 Codex 在 5 秒内退出。现有看门狗只处理本产品登记的 manager/CCSwitch 会话，
不拦截独立的开始菜单进程。

### A-04：AppX 注册身份可能与旧官方安装共存

`Get-AppxPackage -Name OpenAI.Codex` 可能返回同一用户的多个版本/卷。当前代码沿用版本优先
选择和 provenance 记录；USB 模式对 `source=msix` 的外部包会先迁移到当前卷，再执行固定版本
安装；外部 portable 仍要求显式接管。从旧官方安装迁移到 USB 版前仍应在空白机、已有系统版
和 USB 版并存三种场景分别验收。普通“卸载 Codex”仍拒绝未托管外部包；只有“卸载 Codex 与 CCSwitch”
这条明确二次确认的联合路径，才允许清理无 provenance 的当前用户 OpenAI.Codex MSIX，并在
Remove-AppxPackage 后复核注册已消失，降低误删与假报成功风险。

### A-05：CCSwitch 数据目录需按实际构建验证

当前二进制直接从 USB 启动，工作目录在 USB，但没有向其注入未确认的 CLI 选项。若某个
CCSwitch 构建把数据库错误地写入工作目录，需在 CCSwitch 源码/实际运行日志中确认并改为
官方支持的用户目录参数；在没有该证据前，不能宣称任意版本都完全满足数据分离。

### A-06：发布包仍可能触发 SmartScreen/杀毒告警

未签名 EXE、UAC、AppX 卷和 USB 看门狗都是可解释的行为特征。免费版本只能公开来源、哈希、
安装/卸载范围和日志，不能保证免报毒，也不应通过关闭 Defender 或排除目录作为验收。

### A-07：卸载的“干净”边界与后置校验

本轮对卸载链路做了幂等清理加固：当前用户的所有 `OpenAI.Codex` AppX 注册逐个移除，
每个 Remove-AppxPackage 操作和便携程序目录/快捷方式/卸载注册项都有限重试，并在操作后
重新读取文件系统或 `Get-AppxPackage` 验证；CCSwitch 只扫描本产品记录的精确安装根和 USB
来源，不再使用全局进程名杀进程。数据清除开关关闭时，用户数据按设计保留；打开并二次
确认时，只删除文档列出的已知路径，权限/锁定/枚举错误会变成部分失败并保留重试入口。

对于旧版或普通 sideload 注册的 MSIX，Windows 可能暴露
`-PreserveApplicationData` 参数却返回 `0x80073CFA`，称该标记只能用于 deployment-mode
包；更旧的 AppX 模块也可能完全缺少该参数。v0.5.6 只在这两种明确的兼容性情况下移除
该参数并重试同一个当前用户包；安装错盘回滚也使用同一兼容重试，结果会明确说明包内
数据保留不能保证，其他错误不会被该降级吞掉。

因此“卸载完成”在本产品可控制的当前用户范围内有可验证后置条件，但不等同于删除其他用户、
系统预置包、企业策略或其他安装器的 Codex/CCSwitch。用户必须在目标机复核 `Get-AppxPackage`
和列出的数据路径；本开发机没有执行真实卸载以避免破坏现有官方 Codex。

## 4. 发布前验收清单

1. 在 Windows 10 22H2 和 Windows 11 x64 空白用户上，确认 `Get-AppxPackage` 的
   `InstallLocation` 位于 U 盘 `WindowsApps`，不是 C: 系统卷；
2. 确认 `C:\Users\<user>\AppData\Local\Packages\...`、`.codex` 和 `.cc-switch`
   能正常写入，U 盘目录没有 API Key；
3. 分别测试已有策略为缺失、0、1 和被组策略锁定的机器；
4. 拒绝 UAC、拔盘、换盘符、换另一 U 盘、U 盘只读和 AppXVolume 不支持时，确认不会生成
   C: portable Codex；
5. 关闭管理器后从开始菜单启动 Codex，记录 USB 在场/不在场的真实结果，不把推断写成保证；
6. CCSwitch 配置 404TOKEN 后重启，确认 provider、API Key 和 WebView 缓存位于本机用户目录；
7. 运行“卸载 Codex 与 CCSwitch”，分别验证托管包和无 provenance 的当前用户 OpenAI.Codex
   MSIX 的确认提示、保留数据/清除数据、策略恢复和外部 CCSwitch 不受影响；同时确认
   `Get-AppxPackage -Name OpenAI.Codex` 返回空、托管便携根/快捷方式/卸载注册项不存在，
   数据保留开关的结果与界面文案一致；
8. 对已运行的 CCSwitch、WebView 锁定、旧便携目录、安装记录缺失和重复点击做第二次卸载，
   验证重试后置条件和幂等性；
9. 确认双击 `CodexDesktopSetup.exe` 后由嵌入式清单先出现一次启动 UAC，管理员账户确认后
   `CodexAppManager.exe` 进入完整管理员令牌；策略缺失的首次安装在组合操作成功时不再重复请求同一权限。UAC 对话框保持打开超过 120 秒会安全超时，
   清理临时脚本且不会留下策略半成品；如果组合操作失败，随后明确的卷注册拒绝允许再出现一次
   针对当前卷的 UAC，不能因“已尝试”状态而误跳过；策略已为 1 时，只有卷注册权限拒绝才出现卷注册 UAC。
10. 用 `verify-usb.ps1 -SkipNetwork`、SHA256 清单和 Authenticode 记录正式包证据；v0.5.6
   的最终入口/管理器哈希以 `release/usb-oneclick/SHA256SUMS.txt` 和 `usb-manifest.json`
   为准，真实 MSIX SHA-256 保持
   `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。

## 5. 审计结论

代码层已经完成从“C 盘 MSIX + 本地复制 CCSwitch”到“USB AppXVolume + USB CCSwitch 直接启动 +
本地用户数据”的第一版切换，自动测试和实际发布目录静态验证均通过。当前可称为**可验证候选版本**，不能在真实
Windows AppXVolume/UAC/拔盘矩阵完成前宣称“所有电脑一键成功”或“拔盘必定杀进程”。
