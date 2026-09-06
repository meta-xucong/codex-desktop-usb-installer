# Progress

## 2026-09-06 当前基线：v0.5.24

- 根据 v0.5.21 现场报告确认：主卸载成功后，二阶段把 manager 自动生成的 `usb-license.json` 当作清单外用户文件，因此自锁并保留整套 U 盘介质；28 个发行哈希均正常，回收站不是根因。
- 许可证继续随物理 U 盘保存，但改为严格的可变产品元数据：卸载前从当前进程租约捕获卷序列号、发行锚点和相对路径指纹，完整验证主许可与 `.bak`，二阶段计划 schema 3 再核对卷、发行清单和许可字节，许可只在最终提交时删除。
- 真正未知/篡改文件、目录和重解析点仍然阻止介质删除，并一次性列出所有具体路径；删除使用固定句柄，不依赖可移动盘回收站。只读许可可清理，被占用或系统安全策略拒绝时保留重试入口并明确失败。
- 正式候选 `release/usb-oneclick-v0.5.24-final`：29 文件、28 条校验、1,158,086,569 字节；入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`，manager `b97e40f0e5526267a6bf8790dc4756b2da277044f3047b2833a4f02b28f3abcf`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。完整验证见 `docs/releases/v0.5.24-usb-license-finalizer-audit.md`。

## 2026-09-06 当前基线：v0.5.23

- 根据目标机 v0.5.21 报告确认安装事务实际成功，失败位于启动交付：Codex GPU 进程退出，CCSwitch 主窗口未前置，旧实现又把进程创建当作成功。
- USB Codex 现先用 `--disable-gpu`，必须找到尺寸正常且三秒后仍存在的窗口；失败后精确关闭本次程序根，再以现场验证过的 `--disable-gpu --no-sandbox` 自动重试。末级兼容只在成功后按精确 Codex 版本记忆，不改全局环境。
- CCSwitch 现用子进程级 WebView2 软件渲染，忽略 14×14 内部 HWND，恢复真正主窗口、钳制到显示器工作区，并在两秒稳定复核后前置才报告成功；无窗口或瞬时窗口会清掉隐藏单实例进程并给出明确错误。
- 本地 `CodexUsbGuard.exe` 由“仅缺失时复制”升级为新租约前长度 + SHA-256 刷新，旧 helper 先正常退出并回收锁，防止新 U 盘继续运行旧路径/租约代码。
- 正式候选 `release/usb-oneclick-v0.5.23-final`：29 文件、28 条校验、1,158,037,323 字节；入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`，manager `8295f8c54d5ee8a599702428b01b97589f7777502a042790ab475893c44cac57`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。较早的 v0.5.22 候选不覆盖、不作为交付对象。详见 `docs/releases/v0.5.23-launch-compatibility-audit.md`。

## 2026-09-06 当前基线：v0.5.19

- 复核 v0.5.17 现场报告：staged 孤儿清理已由 v0.5.18 的已验签 MSIX 精确重水合链覆盖；确认新增 G5 为 Windows 10 的 `DeploymentResult.IsRegistered` 滞后导致安装成功却被管理器误报失败。
- 注册原生结果现仅记录为提示；最终成功改为最多 60 秒的当前用户精确 FullName、Status、部分暂存、USB 真实路径和独占所有权收敛。原生完成与后置条件分开持久化，超时续接不会重复 Stage/Register；外层兼容路径同步收口。
- 最终候选 `release/usb-oneclick-v0.5.19-final`：25 文件、24 校验条目、1,157,977,451 字节；入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`，manager `d3da411362a8b148f4d5161d7f785bfebb54a8a56ac454dc303b70d4e3991665`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 940 项自动测试通过，Windows x64/custom-protocol 编译、生产构建、离线 bundle、两套整包验证、真实 MSIX 签名和两个 `requireAdministrator` PE 清单均通过。开始菜单 ChatGPT 空白问题未混入本轮，等待独立现场报告。详见 `docs/releases/v0.5.19-registration-convergence-audit.md`。

## 2026-09-06 当前基线：v0.5.18

- 针对 v0.5.17 现场的精确机器级 Staged 孤儿（无 Owner/预配、包文件丢失、两条正式删除 API 分别失败）增加可信 MSIX 重水合：本地已验签副本按 journal SHA-256 再核验并持有防替换句柄，通过公开 `StagePackageAsync` 恢复同一 FullName，再迁移到绑定 U 盘或用正式 API 清理后进入正常 USB Stage；禁止私有 AppX 注册表、ACL 和 TrustedInstaller 手术。
- 修正 AppX Owner SID 的字符串/`Sid`/`Value` 投影兼容；机器修复 journal 新增 `failed`、步骤、次数上限及错误原文，支持中断后重新观察 Windows 状态并安全续接；安装和联合卸载共用同一精确授权边界。
- 最终候选 `release/usb-oneclick-v0.5.18-final`：25 文件、24 校验条目、1,157,966,897 字节；入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`，manager `743b2cab6a7c1a182121a71e0dbe588c55b237ddb45b227fd686f9bb33e0c0df`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。
- 937 项自动测试通过，Windows x64/custom-protocol 编译、生产构建、离线 bundle、两套整包验证、真实签名和两个 `requireAdministrator` PE 清单均通过。开发机未执行真实 AppX/HKLM 变更；目标 Windows 10 实机仍是最后验收层。详见 `docs/releases/v0.5.18-orphan-rehydration-audit.md`。

## 2026-09-04 当前基线：v0.5.8

- 按残留报告实现两阶段卸载：先系统/程序/事务恢复，再管理器退出后清理日志、WebView缓存、状态、锁和暂存；清理组件最后自删除并写可见最终报告。
- 独立的个人数据永久清除、U盘源安装包擦除选项，均有确认；精确归属/句柄/哈希/卷身份保护，失败保留恢复资料和重试入口，不显示完全成功。
- 退出期间重建数据、helper工作目录自占用、局部恢复未进入最终清理、查询错误被当作不存在等缺口已修复并回归。
- 711项常规测试通过，真实包只读预检及完整新发行包签名/哈希/PE清单验证通过。交付 `release/usb-oneclick-v0.5.8`；旧v0.5.7目录保留。详细范围和真机验收见 `docs/releases/v0.5.8-uninstall-audit.md` 与 `v0.5.8-validation.md`。
- 已修复旧portable测试隔离问题；测试产生的两项HKCU/快捷方式条目和一处微小模拟目录清理被工具策略拒绝，未绕过；详见审计第六节。真实AppX/HKLM卸载未执行，目标机仍需实测。

## 2026-09-04 当前基线：v0.5.7

- 完成 WorkBuddy 变更复审。保留原生注册表方向，纠正无证据的子进程令牌丢失结论；修复最小权限/视图/读回/journal，严格 USB 卷与本地数据策略前置条件。
- 删除系统卷 bootstrap；迁移失败保留原包，错误放盘只回收本次新增注册。保留数据卸载先创建可核验备份，备份或注册查询失败不得假报成功。
- 增加界面前完整离线 WebView2 自检安装、运行库有界等待、USB 入口单实例，以及展开空间检查和前端失败证据保留。
- 最终完整目录 `release/usb-oneclick`：v0.5.7，25 文件、1,157,201,764 字节。根入口 SHA-256 `62ea212dac934fa0020f01069b156bdb3a0f5685c8fd0672726e9b75786fec64`；manager `007a08920a5a01e995768b3a4f0748fd8cd3b9067a18af805bf44756222ae400`。
- 自动测试与静态产物验证完成；未对开发机执行真实 AppX 部署/卸载、HKLM 修改或关闭现用 Codex。目标机验收仍待执行。当前审计/验证见 `docs/releases/v0.5.7-audit.md`、`docs/releases/v0.5.7-validation.md`。

以下为历史记录，其中 C 盘 bootstrap、策略失败后继续安装等路线已废止。

- 2026-09-02：根据最新实机截图复盘出状态机缺陷：组合“策略+卷注册”UAC 失败后，旧逻辑只记录了“已尝试”而没有记录“成功”，导致后续 `Add-AppxVolume Access is denied` 的针对性 UAC 被跳过。已改为仅在组合子操作确认成功（含策略变更完成）时抑制重试；组合被拒绝、超时或失败时仍允许一次当前卷的标准 UAC 注册重试，并增加回归测试与用户可读诊断。
- 2026-09-02：同步更新 USB README、AppXVolume 开发/审计文档，重新构建 `release/usb-oneclick`；最终目录 22 个文件/21 个校验条目，入口 20,001,792 bytes，SHA-256 `f1b71106c5911fbb771fe7ed3ad7b3f91fe55f2285bef5dd7b862086ad637904`；MSIX SHA-256 保持 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`，CCSwitch SHA-256 保持 `75b847ca54d207cbe92866154da8f504c6874f6d253423899c7cddfa5fee0045`。
- 2026-09-02：修复版通过 `src-tauri` Rust 184/184、Windows x64/custom-protocol 编译检查、默认 Authenticode 发布校验、SHA256 清单/残留文件校验以及入口 `--codex-usb-guard` smoke（exit 0）。剩余唯一关键验收仍是目标 Windows 上批准 UAC 后真实 `Add-AppxVolume` 和 MSIX 部署。
- 2026-09-02：按用户要求增加 USB 入口启动前自动提权：仅当当前 EXE 旁存在 `manifest.json` + `payload` 时，通过 Windows 原生 `ShellExecuteW(runas)` 重启自身并追加 `--codex-usb-elevated`；管理员令牌通过 `OpenProcessToken`/`GetTokenInformation(TokenElevation)` 验证，取消/拒绝或未获得高完整性令牌时在安装前退出并显示明确提示。普通本机管理器、`--codex-usb-guard` 看门狗入口不受影响；不修改 UAC/组策略、不静默绕过权限。
- 2026-09-02：自动提权改造通过 Rust 186/186、Windows x64/custom-protocol 目标检查、重新编译入口、`verify-usb.ps1 -SkipNetwork`、完整 `tools/verify-oneclick-usb.ps1 -SkipNetwork` 和 `--codex-usb-guard` smoke；二次审计后 AppX helper 已改为复用入口已批准的高完整性令牌，避免重复 UAC。当前发布入口 `release/usb-oneclick/CodexDesktopSetup.exe` 为 20,053,504 bytes，SHA-256 `84646112d24b4c0177424be0ec5e4cc9ece194378e77ba3916ef3bd8d6375259`；MSIX/CCSwitch/VC redist 哈希保持不变。剩余实机验收是管理员账户 UAC 确认后的真实 AppXVolume 部署；拒绝 UAC、标准账户凭据切换和企业策略拒绝无法由合法程序强制通过。

- 2026-09-02：完成发布前最后一轮可控问题加固：USB `Add-AppxPackage` 使用独立 10 分钟有界时限；Windows PowerShell 5.1 脚本统一加入 UTF-8 BOM；CCSwitch 校验器显式按 UTF-8 读取 JSON；构建/校验脚本不再在参数绑定阶段依赖空的 `$PSScriptRoot`；`verify-usb.ps1` 支持指定包目录并拒绝 SHA256 清单之外的残留文件。
- 2026-09-02：重新构建入口和 `release/usb-oneclick`，默认（不跳过 Authenticode）静态校验通过；最终目录 22 个文件、21 个校验条目，入口 `CodexDesktopSetup.exe` 20,019,712 bytes，SHA-256 `e60314ac8a1facc49fd988d6184023ba12897e5521bf5d7a82adcabb98c156ac`；真实 MSIX 802,145,236 bytes，SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；CCSwitch SHA-256 `75b847ca54d207cbe92866154da8f504c6874f6d253423899c7cddfa5fee0045`。
- 2026-09-02：最终回归通过：codex-win-engine 114/114、src-tauri 183/183、前端 275/275、发布 56/56、离线 bundle 18 文件/17 校验条目、Windows x64/custom-protocol 编译检查、PowerShell 5.1 语法检查、真实 MSIX Authenticode 校验；临时“未列入 SHA256SUMS”文件也被验证器正确拒绝并清理。现在只剩真实空白 Windows + 本地 NTFS U 盘的 AppX/UAC/VC++ 运行验收。
- 2026-09-02：最终入口 `--codex-usb-guard` smoke 退出码 0；最终 MSIX 与内置 Microsoft VC++ x64 安装包的 Authenticode 均为 `Valid`。最终候选包已固定，可交给用户复制到 U 盘。

- 2026-09-01：根据最新实机截图定位第三个独立阻断点：普通权限运行的 `Add-AppxVolume` 在部分 Windows 版本返回 `Access is denied`，因为它写入机器级 AppX 卷注册元数据；这不是 MSIX 签名、VC++ 或 U 盘 NTFS 写入失败。新增窄范围 UAC 处理：先普通权限尝试，只有明确命中 `Add-AppxVolume` 权限错误时，才由管理员 PowerShell 复核当前盘符根/NTFS、注册该盘的 `WindowsApps` 并验证在线状态，然后仅重试同一份已校验 MSIX 一次；不提升整个安装器、不静默绕过 UAC。
- 2026-09-01：新增 `appx_volume_registration_needs_elevation` 分类测试和 elevated 脚本根路径/操作范围测试；`cargo test --lib` 179/179、Windows x64/custom-protocol `cargo check` 通过。更新离线 README、USB 发布 README 和 AppXVolume 开发文档，明确 UAC 行为与用户拒绝时的正规边界。
- 2026-09-01：重建 `release/setup` 与 `release/usb-oneclick`；`verify-usb.ps1 -SkipNetwork`、入口 `--codex-usb-guard` smoke、离线 bundle 和发布测试通过。当前发布入口 SHA-256 `0df0a904c26ce8a0a1e87fe32e1625217cf95fed4954ee731c8f60ed1c29a74d`，真实 MSIX SHA-256 保持 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。真实目标机仍需确认 UAC 接受后 Add-AppxVolume→MSIX 重试链。

- 2026-09-01：Authenticode 超时修复：`RunLimits` 新增独立的 `AUTHENTICODE_TIMEOUT=5 分钟`，`verify_openai_authenticode` 不再使用 45 秒普通探测预算；离线快速预检改为只做 manifest、大小、SHA-256、身份和入口检查。
- 2026-09-01：安装阶段改为“先本地暂存、再签名”：MSIX 复制到 `%TEMP%\\CodexDesktopInstaller\\<version>` 后二次计算 SHA-256，随后只对本地副本执行强制 OpenAI Marketplace Authenticode 校验；签名无效或超过 5 分钟仍拒绝进入 AppX 部署。
- 2026-09-01：本轮验证：src-tauri Rust 177/177、Windows x64 target 测试 177/177、Windows/custom-protocol `cargo check`、TypeScript、ESLint、前端 275/275、发布 56/56、离线 bundle、真实 MSIX `verify-usb.ps1 -SkipNetwork`、入口 guard smoke 均通过。最终入口 SHA-256 `9ae60a07c0325cdad19ef5e2e7f7eb2c8f2ecf45ff5c7d452be9028086ee0242`，MSIX SHA-256 保持 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。

- 2026-08-31：已阅读开发文档和当前源码；确认 Codex 是本机 MSIX，不能依赖 U 盘文件本身做运行期看门狗。
- 2026-08-31：开始 P1，优先实现可见、可卸载的本地看门狗；不修改 WindowsApps、不做 DLL 注入、不全局杀进程。
- 2026-08-31：完成 `usb_guard.rs`：可移动卷序列号、锚点哈希、包内相对路径、可选 Ed25519 许可证、原子租约/状态文件、5 秒单调宽限和本地无窗口 helper。
- 2026-08-31：接入 Tauri 主入口、Codex/CCSwitch PID 登记、授权门禁、卸载清理和 OfflineHome 状态卡；CCSwitch 增加源文件哈希校验及所有权标记。
- 2026-08-31：重新构建 `release/setup/CodexDesktopSetup.exe` 并组装 `release/usb-oneclick`，完成 Windows MSVC 条件编译和全量回归。
- 2026-08-31：收紧辅助进程锁为“锁文件记录 helper PID + 存活校验”，并让 CCSwitch 卸载复用 PID/镜像名二次校验；重新构建 Release 入口并复核 U 盘目录。
- 2026-08-31：新增 `docs/APPX-PROCESS-GUARD-DEVELOPMENT.md`，记录 P2 开始菜单保护的 AppX 身份、路径、PID、创建时间方案、任务生命周期、测试矩阵和误报边界。
- 2026-09-01：完成卸载残留加固：Codex 用户数据枚举遇到权限错误不再静默跳过；清理后重新验证所有已知路径；CCSwitch 只在有效所有权标记存在时关闭/删除本地副本，并对 USB 标记、数据目录和进程根做最终检查。
- 2026-09-01：重新编译并组装 `release/usb-oneclick`；Rust 173/173、前端源代码 275/275、发布 56/56、离线 bundle、USB 静态校验和无界面 guard smoke 全部通过。
- 2026-09-01：根据实机 AppXDeploymentServer 事件 404/756 定位 `0x80073CF9`：卸载保留的 `C:\Users\...\AppData\Local\Packages\OpenAI.Codex_*` 数据与目标 U 盘卷冲突。USB 安装脚本新增预扫描与错误驱动双重恢复：先在系统卷注册同一 MSIX，再调用 `Move-AppxPackage` 迁移到当前 U 盘；同时强制 UTF-8 输出，避免中文诊断乱码。
- 2026-09-01：新增保留数据重装 PowerShell 模拟回归（系统卷 bootstrap + USB Move 路径）；Rust `sys` 23/23、src-tauri 173/173、前端 275/275、发布 56/56、Windows 目标检查、重新打包和 `verify-usb.ps1 -SkipNetwork` 全部通过。最新入口已生成，真实空白机安装仍待用户实测。
- 2026-09-01：根据实机截图定位第二个独立问题：MSIX 健康检查/默认启动把 `shell:AppsFolder\\...` 直接传给 `Start-Process`，部分 Windows PowerShell/.NET 将其当 URI 解析并报“无效的 URI：未能分析主机名”。已统一改为 `explorer.exe -ArgumentList shell:AppsFolder\\...`，并同步覆盖正常启动路径；目标卷安装逻辑未改变。
- 2026-09-01：完成 URI 激活修复的 Rust/frontend/release 回归；生成新版 `release/usb-oneclick`，入口 SHA-256 为 `9e52863b0f8764fbc83bf9f8cd2eb57abd99ef4a39915ae6afdf37e6b478fe0c`。当前仍需用户在实机用新版目录复测安装。
- 2026-09-01：复核用户最新截图与源码分支后确认其文案来自“MSIX 离线安装失败”分支，而不是健康检查分支；此前的 Explorer URI 修复属于独立加固，并非本次安装失败的充分解释。USB 安装脚本新增本地 NTFS AppX 卷预检、失败操作/目标卷/AppXVolume 诊断，并优先使用 `Add-AppxVolume` 返回对象，减少慢速 U 盘上的注册竞态。
- 2026-09-01：已重新编译 `release/usb-oneclick`。新入口 SHA-256 为 `484c7b737db904818cf7c52c02c1d7720b0dd5151c9e29d029e43e7755c75bf3`，真实 MSIX SHA-256 保持 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；离线 bundle、PowerShell/Rust/前端/发布校验全部通过。当前主机的只读检查显示 E: 是 WebDAV 映射而非本地卷、F: 是 NTFS 可移动卷；若截图中的 E: 为同一台电脑，已确定根因是从映射盘运行 USB AppX 安装。
- 2026-09-01：用户补充完整错误信息，确认当前实机已通过本地 NTFS 预检，失败发生在 `Add-AppxPackage (USB AppX volume)`；日志中的 AppXVolume 是 `\\?\Volume{GUID}` + `E:\WindowsApps`，因此排除本次 WebDAV/文件系统假设。针对 Windows 版本差异，安装脚本改为按路径优先取得在线原生 AppxVolume、挂载并刷新离线旧注册，固定使用文档定义的 `-Path` 参数并将 `$args` 重命名为 `$installArgs`。
- 2026-09-01：新增 URI 兼容恢复：若 USB `-Volume` 直装仍抛出 `Invalid URI/hostname`，自动用同一 MSIX 在系统卷 bootstrap，再调用 `Move-AppxPackage` 迁移到 U 盘，保持固定 MSIX 语义且不触发 portable 回退；错误报告新增 AppxVolume 类型/离线状态、挂载/注册错误和实际包路径。
- 2026-09-01：根据空白电脑点击入口即提示缺少 `vcruntime_140.dll` 的实机反馈，确认旧入口与 CCSwitch 都导入 `VCRUNTIME140.dll/VCRUNTIME140_1.dll`。新增根目录 `.cargo/config.toml` 的静态 MSVC CRT 链接，使入口本身无需动态 VC runtime 即可启动；新增 `src-tauri/src/app/runtime.rs`，在启动和 CCSwitch 启动前检查系统 DLL，缺失时只运行 U 盘内哈希固定、Microsoft Authenticode 有效的 `resources/prerequisites/vc_redist.x64.exe`（`/install /quiet /norestart`），不复制单 DLL、不联网。
- 2026-09-02：复盘用户提供的空白机崩溃链，确认失败根因是旧版本 UAC 临时 `.ps1` 由 `Restricted/Undefined` 执行策略拦截，随后 `Add-AppxVolume D:\` 权限失败并落入不完整回滚路径；Authenticode 并非根因。AppX UAC 外层 PowerShell、`runas` 子进程和已提权直执行现在均显式传入进程级 `-ExecutionPolicy Bypass`，并将失败结果 JSON 改为普通哈希表以兼容更严格的 PowerShell ConstrainedLanguage 环境；不写入执行策略、不关闭 Defender。
- 2026-09-02：新增 Restricted-policy wrapper 回归测试（实际以 `-ExecutionPolicy Restricted` 启动外层并验证子进程 Bypass 可执行临时脚本），更新发布清单/README/审计说明并重建最终 `release/usb-oneclick`。干净正式目录入口 SHA-256 为 `19e84a298c1feac6bff597f9d712f5db6264fe74bfc2212b8988f989d8b57eca`；22 文件/21 校验项、MSIX Authenticode `Valid`、USB guard smoke 和全部自动测试通过。上一次生成目录的临时嵌套副本仅保留在工作区备查，正式目录已用逐文件校验后的 staging 内容覆盖。
- 2026-09-01：安装器构建脚本新增 VC redist 存在性、SHA-256 和签名门禁；Tauri resources、USB 联合清单、README、逐文件 SHA256SUMS 和 `verify-usb.ps1` 均纳入运行库。新版入口已重建，PE 导入仅含系统 DLL，无 `VCRUNTIME*`/`MSVCP*`/`api-ms-win-crt-*`；真实空白机仍需确认 UAC/Windows Installer 和 CCSwitch 首次启动。
- 2026-09-01：用户实机反馈“组件已安装但安装失败”。截图详情为 `Authenticode verification error: Get-AuthenticodeSignature: process exceeded total deadline`，定位到离线 802 MB MSIX 的 PowerShell 签名探针使用通用 45 秒 `RunLimits::probe()`，在可移动 U 盘读速、Defender 扫描或证书链检查较慢时超时。开发机同一 MSIX 实测最终状态为 `Valid`、约 6.08 秒，说明当前错误不是 VC++ 组件缺失、MSIX 哈希不匹配或签名无效，也尚未进入 Add-AppxPackage；下一步应给离线 Authenticode 使用更长专用预算，并优先对本地暂存副本校验。
- 2026-09-01：新增在线卷刷新、离线旧卷挂载、URI 直装失败迁移三组 PowerShell 模拟回归；聚焦 Windows 引擎测试通过 28/28，`sys.rs` rustfmt check 与 `git diff --check` 通过。真实空白机仍需用新版发布目录复测。
- 2026-09-01：补齐 `README.offline.md` 的 USB AppXVolume 故障处理说明，并以最终产物复核：入口 `release/usb-oneclick/CodexDesktopSetup.exe` SHA-256 为 `85c498d75fe056e2bb43b1f2e066123ab44b101b27569752b983ed46a409c0ff`，真实 MSIX SHA-256 为 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。本阶段源码、文档、静态发布校验已完成，剩余仅为真实 Windows 空白机验收。
- 2026-09-01：根据最新实机日志确认“系统卷 bootstrap after USB volume URI error”本身也返回无效 URI，旧回退链未能解决问题。安装脚本新增原生 WinRT `PackageManager.AddPackageAsync(file:// URI, PackageVolume)` USB 卷回退，显式解析系统 AppX 卷后才执行旧 bootstrap→Move，且记录原生 `IsRegistered`、ActivityId、扩展 HRESULT、卷名和包 URI；新增原生成功分支模拟测试，重建后的 `release/usb-oneclick/CodexDesktopSetup.exe` SHA-256 为 `258437ab946585b81ba1b487a0b57b433a076bd5e38c74d8fadb402d321e48d1`，真实 MSIX SHA-256 保持 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。Rust 引擎 29/29、src-tauri 173/173、发布 56/56、离线 bundle、USB 静态校验和 guard smoke 全部通过；仍需用户在真实本地 NTFS U 盘复测。
- 2026-09-02：根据目标机最新日志定位到真正的提权缺陷：登录账号虽属于 Administrators 组，但安装器以 `asInvoker` 过滤令牌运行，迟到的 PowerShell UAC 子脚本没有得到管理员令牌，最终在写 `HKLM\\Software\\Policies\\Microsoft\\Windows\\Appx` 时抛出 `UnauthorizedAccessException`；这不是 Authenticode 或执行策略问题。
- 2026-09-02：新增独立 `tools/codex-usb-launcher` Rust 启动器，嵌入 Windows `requestedExecutionLevel=requireAdministrator` 清单；它只检查同目录 `manifest.json`、`payload` 和 `CodexAppManager.exe`，随后把 `--codex-usb-elevated` 传给 asInvoker 管理器。普通本机管理器不受启动器清单影响，直接打开管理器仍保留原生 `runas` 回退；取消/拒绝 UAC 时在安装前退出，不能静默绕过 Windows 权限。
- 2026-09-02：构建脚本现同时生成 `CodexDesktopSetup.exe`（USB requireAdministrator 入口）、`CodexUsbLauncher.exe`（别名）和 `CodexAppManager.exe`（管理器本体）；`verify-usb-launcher.ps1` 直接读取 PE `RT_MANIFEST`，并被构建与最终 `verify-usb.ps1` 调用，防止误把普通管理器当作 U 盘入口。Windows PowerShell 5.1 的收集/模拟脚本补齐 UTF-8 BOM 和 Compression 程序集加载。
- 2026-09-02：重新构建并审计最终 `release/usb-oneclick`：24 个文件/23 个 SHA256 条目，入口 `343b19d759b85552ec3b3775cf49ea3e707efd738eec9117135838db5562121d`，管理器 `a8a36c61e043b8931c6f714ec280b4c40872ffa0bdba1c2cc6529c0acab0e213`，MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。完整 Authenticode、清单/哈希、Rust 186/186、前端 340/340、release 56/56、离线合成包和 PowerShell 5.1 解析均通过；剩余仅为目标机真实 UAC/AppXVolume 部署验收。

- 2026-09-02：深挖 v0.5.2 实机“权限不足”证据：`CodexDesktopSetup.exe` 虽然是 `requireAdministrator`，但同目录实际执行安装的 `CodexAppManager.exe` 仍是 `asInvoker`；旧中完整性管理器还可能先占用 Tauri 全局单实例锁，导致高完整性入口启动后由旧进程接管 UI，再次写 HKLM 失败。USB 管理器现由构建阶段 `mt.exe` 对最终 PE 写入并验证 `requireAdministrator`，高完整性 USB 会话跳过普通单实例锁，且启动令牌再做运行时核验。
- 2026-09-02：同一报告中的第二条独立失败链为 `packagePath=\\?\\C:\\...` 被 PowerShell AppX 绑定器/WinRT `System.Uri` 当作非法主机名；安装脚本现在在 Rust 边界去除 `\\?\\`/`\\??\\` 前缀，并在 WinRT 回退显式构造 `file:///C:/...` URI。新增扩展路径模拟回归，保留原生 USB 卷→系统卷 bootstrap/Move 回退。
- 2026-09-02：正式 `release/usb-oneclick` 已用修复候选包覆盖（跳过内容相同的 MSIX）；`usb-manifest.json`、`README.txt`、`SHA256SUMS.txt` 与实际文件同步。入口 SHA-256 `343b19d759b85552ec3b3775cf49ea3e707efd738eec9117135838db5562121d`，USB 管理器 SHA-256 `8440c0b35fe3db66547bead1c57ce5e67b9015a15fb0b47c98e07e72473a3085`。
- 2026-09-02：最终目录 `tools/verify-oneclick-usb.ps1 -SkipNetwork`、根目录 `verify-usb.ps1 -SkipNetwork`、两个 PE `requireAdministrator` 清单验证及 `git diff --check` 均通过；代码自动化测试与真实 MSIX 身份/签名仍保持通过。剩余只能在目标 Windows 10/11 + 本地 NTFS U 盘上确认真实 UAC、AppXVolume 注册和部署服务行为。
- 2026-09-02：追加发布门禁：`verify-oneclick-usb.ps1` 现在强制校验 `usb-manifest.manager.role=requireAdministrator USB Tauri manager started by the launcher or directly`，避免“实际 PE 已提权但联合清单仍写 asInvoker”的静默回归；正式包验证器和 `SHA256SUMS.txt` 已同步，根目录 24 个文件/23 个条目全部通过。
- 2026-09-02：复盘用户 v0.5.3 日志确认“MSIX sideload completed”只是 PowerShell cmdlet 未抛异常，不代表目标卷；`Get-AppxPackage` 实际落在 C:\Program Files\WindowsApps。该错盘结果与 HKLM 提权告警相互独立，旧版没有在脚本内核验 `InstallLocation`，因此会留下 C: 假成功。
- 2026-09-02：完成 v0.5.4 修复：USB 生成脚本在 Add-AppxPackage、WinRT PackageManager 和系统卷 bootstrap/Move 后强制校验目标驱动器；本次新建的错盘注册只做保留数据清理，已有外部注册不删除；清理后最多进行一次原生 USB 放置重试，仍失败则返回 expected/actual/cleanup 诊断并禁止 portable/C: 回退。
- 2026-09-02：增加 USB 安装提交边界的高完整性令牌检查，以及 PowerShell 管理员子进程的 `whoami /groups` 高完整性校验；若 UAC/企业策略未真正提升，在关闭 Codex 或 AppX 变更前停止，不再生成半成品。
- 2026-09-02：长 AppX PowerShell 代码改用临时 UTF-8 BOM `.ps1` 传输，避免 Windows 命令行长度 206；Rust 引擎错盘 mock、src-tauri 及 Windows target 检查全部通过。
- 2026-09-02：构建并覆盖正式 `release/usb-oneclick` v0.5.4；入口 SHA-256 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`，管理器 `c3a82c8b05c95e0ec821d7e74beb04855d7b75eaf4fdd29ef1800dae4cc1501d`，真实 MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；两个 PE 均验证为 `requireAdministrator`，正式目录静态验证和 guard smoke 通过。真实目标机 AppX/UAC 仍待用户验收。
- 2026-09-03：修复 v0.5.4 卸载路径对旧版/普通 sideload 包传入 `PreserveApplicationData` 导致 `0x80073CFA` 的问题。PowerShell 仅在明确 HRESULT 或“PreserveApplicationData + deployment mode/部署模式”组合错误时去掉标记，重试同一当前用户包；完成后继续复核 `Get-AppxPackage` 注册已为空，并把包内数据不保证保留写入 notes。新增 .NET/PowerShell 正则行为回归测试，Rust 引擎通过 117/117。
- 2026-09-03：重建 v0.5.5 正式 U 盘目录并同步 23 个非 MSIX 文件；`verify-usb.ps1 -SkipNetwork`、`verify-oneclick-usb.ps1 -SkipNetwork -SkipSignature`、两个 PE `requireAdministrator` 清单校验均通过。入口 SHA-256 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`，管理器 SHA-256 `7b1554329feb82822821bd8f11b43907df3d63d3987b727494cca8e9108e66f3`，MSIX SHA-256 `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`。真实 Windows AppX 卸载仍需目标机验收。
- 2026-09-03：根据 v0.5.5 日志继续收口安装/卸载：USB 管理器及 PowerShell 子进程现在同时验证 `TokenElevation`、完整/默认 `TokenElevationType` 和有效 Administrators SID；安装错盘回滚、数据卷冲突清理与显式卸载共用窄兼容逻辑，遇 `0x80073CFA` 或旧模块缺少 `PreserveApplicationData` 参数时只对同一当前用户包去掉该标记重试，普通权限/策略错误仍失败，并对包内数据不保证保留给出告警。
- 2026-09-03：新增缺少 Preserve 参数的安装回滚与卸载模拟回归；引擎 `cargo test` 122/122、src-tauri `cargo test` 187/187、Windows MSVC target `cargo check`、TypeScript/lint、前端 29 文件/275 测试、发布 2 文件/56 测试、离线 bundle 均通过。重新构建 v0.5.6 并从 next18 同步正式 `release/usb-oneclick`（24 文件/23 校验项）；入口 SHA-256 `d834c4c6708a17c7d06838ff0f969b26b0202d94b44b16498d80b0404a41ffe4`，管理器 `b89c4ad5e4e6595fa5e38ef6e49c337673f2f15e7827bfb514eca5521760a830`，真实 MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`；正式校验和两个 PE `requireAdministrator` 检查通过。真实 Windows AppXVolume/UAC 仍需目标机验收。

