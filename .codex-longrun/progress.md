# Progress

## 2026-09-13 G22/G23 release candidate

- G22 归档报告已加入 `docs/reports/`；卸载二阶段报告根目录、phase/exitCode/retrySuggestion、日志 fallback 和路径交叉检查已纳入代码与 Rust 回归。
- Skill Hub 失败详情仅在该远程功能的 UI 详情中脱敏，卸载/本地清理详情保留原始失败路径，修复了全量前端测试发现的诊断兼容回归。
- 能力包新增 `scripts/build-pack.py` 与 `scripts/verify-package.ps1`，跨 Python/PowerShell 的 253 文件清单、校验和、来源树摘要均通过。
- 模型目录以 `gpt-6-astra` 为首选并记录 `gpt-5.2` 历史停用；官方 26.908.4834.0 载荷取证记录已加入文档，明确载荷内旧条目仍待上游刷新。
- 最新候选目录：`release/usb-oneclick-v0.5.24-g23-model-sync-g22-final-20260913-r4`；签名工具：`release/usb-license-signer-gui-v0.5.24-g23-model-sync-g22-20260913-r4`。两者均未写入物理 U 盘。

## 2026-09-13 G22/G23 follow-up

- 已读取 G22 卸载报告；确认核心回归是 finalizer 自引用：管理器数据根删除后仍向该根写收尾日志，导致 Windows Location unavailable/E_FAIL 弹窗。
- 已确认当前 Skill Hub 请求链路是 Tauri/Rust command，不是普通前端 fetch；现有 UI 丢弃了后端 detail，来源尝试也未进入主日志。
- 已确认安装包没有可公开分发的模型目录；当前用户 `models_cache.json` 含服务端模型消息，不可直接打包到公开 GitHub。
- 已落开发契约文档，下一步只改相应源码与安装脚本，保留其它用户改动。

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

## 2026-09-21 G39.4 complete delivery rebuild

- Signer audit confirmed the single `注册账号并签名` action, removable-volume serial display, generated `USB-xxxx` asset number display, automatic administrator credential loading, fixed `unified-api-internal` group, one group-scoped API key, and server-conflict retry allocation.
- Recovered the trusted production administrator credential only in the build process from the existing embedded signer; the new signer embeds it, while no raw key or `sub2api-config.json` is published beside the EXE.
- Generated `release-profiles/usb-v0.5.24-g39.4-20260921-r6-key216.json`; rebuilt the manager/launcher with `SignedRequired`; bundle and signer bind to `releaseId=g39.4-20260921-r6`, `keyId=prod-2026-01`, and public-key digest `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`.
- Complete USB directory: `release/usb-oneclick-g39.4-20260921-r6-final`; signer directory: `release/usb-license-signer-gui-v0.5.24-g39.4-r6-20260921-embedded`.
- Archives: `release/CodexUsb-OneClick-G39.4-v0.5.24-20260921-r6.zip` and `release/CodexUsb-LicenseSigner-G39.4-v0.5.24-20260921-r6-embedded.zip`.
- Physical NTFS U-disk sign/install/uninstall acceptance remains device-bound. CCSwitch isolated launch was not repeated because an unrelated external single-instance process already owns port 15721; final root/embedded payloads match the previously runtime-tested CCSwitch hash.

## 2026-09-21 G39.5/r7 final delivery rebuild

- Corrected the CCSwitch production build path to use the Tauri bundled release build, eliminating the stale development/localhost page that caused `localhost` connection refusal after installation. New CCSwitch SHA-256: `D9656D97BF0EB663A5EA090D965573C86B7774633D1E2D00971B4428C7BB5A55`.
- Rebuilt the final manager/launcher and copied the updated product into the complete package. Final manager SHA-256: `33A21F29B8F5289A5FEC17D3FC71ABB1A0DDF959EAC899F0A746883DCC0F54B1`.
- Final complete USB directory: `release/usb-oneclick-g39.5-20260921-r7-final-rebuilt` (289 files, 1,317,242,814 bytes). The older sibling `...-final` is an intermediate artifact and must not be used.
- Final signer directory: `release/usb-license-signer-gui-v0.5.24-g39.5-r7-20260921-embedded`; admin credential is compile-time embedded, no `sub2api-config.json` sidecar is shipped, and the single `注册账号并签名` workflow remains in place.
- The USB verifier, full one-click verifier, signing-pair verifier, manifest/hash cross-check and forbidden-secret scan all passed. Signer tests passed 48 library + 4 GUI tests; signer clippy and rustfmt checks passed. Manager tests passed 436/436; CCSwitch frontend passed 223/223 serially.
- The nested CCSwitch Rust suite passed 1,092 tests; one test was blocked by the unrelated pre-existing CCSwitch process occupying localhost port 15721, and the one Windows temp-path flaky test passed when rerun alone. This remains an external workstation condition, not a package verification failure.
- Physical NTFS U-disk registration/sign/install/update/uninstall and isolated CCSwitch GUI startup remain target-device acceptance items; no admin key or credential value was printed into the log.

## 2026-09-22 G39.5/r8 model-catalog and field-repair implementation

- Implemented the native Codex model catalog bridge through the confirmed official `model_catalog_json` configuration field. The manager fetches `https://404token.xyz/v1/models` with the signed device key, preserves all vendors, merges only stable/latest/valid-date aliases, keeps the raw upstream slug, and never writes the API key to catalog/config/logs.
- Added background refresh and a manual `刷新模型列表` action to the offline USB UI, plus safe stale-cache behavior, provider binding checks, managed-config conflict detection, malformed-block protection, and uninstall cleanup limited to this bridge's identity-marked files.
- Added two progress surfaces: real byte download progress and an indeterminate install-stage bar with localized verifying/applying/committing labels. Added bounded curl retry, portable extraction retry, console-window hiding for the managed Codex process tree, and uninstall transaction reconciliation.
- Re-ran targeted checks: model catalog 7/7, install transaction 13/13, command layer 27/27, Windows process 4/4, download 7/7, portable extraction 28 passed + 1 intentionally ignored real-MSIX test, TypeScript, formatter, focused frontend 82/82, and `git diff --check` all passed. Windows x64/custom-protocol target check passed with one pre-existing dead-code warning.
- Rebuilt `release/usb-oneclick-g39.5-20260922-r8-model-catalog`; package verifier, payload verifier, full one-click verifier, signing-pair verifier, manifest/hash cross-check, CCSwitch product verifier, and administrator PE manifest checks passed. The existing r7 embedded-key signer was reused because signer source was unchanged and the current shell intentionally did not expose the administrator Key.
- Remaining acceptance is physical: copy the r8 directory and the validated r7 embedded-key signer directory to an NTFS removable drive, then run registration/sign/install/launch/model-refresh/update/uninstall with 360 state recorded. This workspace cannot prove those device-bound Windows and 404token production steps.

## 2026-09-22 G39.5/r8 final regression and delivery

- Full manager regression passed: `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1` = 445 passed, 3 ignored; frontend = 32 files / 361 tests; `npm run check`, `npm run lint`, manager/engine rustfmt and Windows x64 target check passed.
- The engine full suite initially exposed 17 fixture failures because the host has no `E:` drive and `Join-Path` resolves a not-yet-mounted drive before AppX registration. Replaced only the generated lexical path combinations with `[System.IO.Path]::Combine`; after the fix engine = 179 passed, 1 intentionally ignored real-MSIX test.
- Rebuilt `release/setup` and final complete directory `release/usb-oneclick-g39.5-20260922-r8-final`; final directory has 289 files / 1,317,441,470 bytes. Final hashes: launcher `a9ce2475d6f2912aeb00a604da68cfcd8d2ba0a1291a0c3bc8a0bf4f2903c464`, manager `d49089db8406546fe7b3b60bec954a7abf576efbff650a2772a17a1ab17cf5ccf`, CCSwitch `d9656d97bf0eb663a5ea090d965573c86b7774633d1e2d00971b4428c7bb5a55`, MSIX `ab0feacc74e982742736da4a7f1d2b59e9fabaa3641606545d3064927da02b1e`.
- Rebuilt the signer in the same PowerShell process as Cargo so `CODEX_404TOKEN_ADMIN_API_KEY` reaches `option_env!`; verified the EXE contains the same embedded credential hash as the prior validated signer. Final signer directory: `release/usb-license-signer-gui-v0.5.24-g39.5-r7-20260922-final-embedded`, 8 files / 5,035,151 bytes, EXE SHA-256 `6dac6e4fbf4edf826bb166624eea64b63bfc4ca507459e94d36c7073ff7dd792`.
- Final package, payload, one-click, signing-pair, CCSwitch, PE-manifest and startup smoke gates passed. ZIP archives were generated and their central directories contain all required entries: USB SHA-256 `30b41e18851343e0e90a10e62414c8739060a8e1c1576088dfcc68e4a92170c0`, signer SHA-256 `e090a9551de844e82e62a6cc238df7b6535cd634968692c6f07a3602445695a4`.
- Remaining acceptance is physical: copy the final directory pair to an NTFS removable drive and run registration/sign/install/launch/model-refresh/update/uninstall with 360 state recorded. The external CCSwitch process owning port 15721 must be closed before an isolated GUI smoke; it was not touched automatically.

## 2026-09-22 G39.5/r9 unified-api-internal catalog and package sync

- Synchronized the non-secret CCSwitch provider catalog name to `404TOKEN（unified-api-internal）` and removed the fixed `fallbackModels` list; runtime discovery remains `/v1/models` with one injected API key.
- Tightened the CCSwitch product verifier and patch smoke checks to require the unified group label, runtime model source, no model whitelist/fallback list, and an API key requirement; strict-mode PowerShell property access is handled safely.
- Rebuilt `release/ccswitch-product-g39.5-20260922-r9-unified-api` from the existing patched binary (SHA-256 `D9656D97BF0EB663A5EA090D965573C86B7774633D1E2D00971B4428C7BB5A55`), then rebuilt the manager and complete U-disk directory `release/usb-oneclick-g39.5-20260922-r9-unified-api-final2` (289 files).
- r9 CCSwitch static/network verification passed: `/health` returned 200 and unauthenticated `/v1/models` returned 401. Manager/USB `SignedRequired` binding, payload, one-click, CCSwitch, PE-manifest, and signing-pair preflight all passed with the existing validated embedded-key signer.
- Generated ZIP `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r9-unified-api.zip` (SHA-256 `97E8D19425F7DB650BA22D2D7B2D60BCED7BFFC62B0D0A071F7379180EC31BB6`) with the complete r9 directory as its top-level entry; archive listing contains the launcher, manager, CCSwitch catalog, payload, and verification files.
- Target-device registration/sign/install/launch/model-refresh/update/uninstall and isolated CCSwitch GUI smoke remain external acceptance items; no real administrator/API credential was used or printed.

## 2026-09-22 G39.5/r10 USB installer UX and startup-state repair

- Removed the duplicate indeterminate install-stage bar from both the shared install progress screen and the USB payload-update surface; each operation now presents one progress bar with the phase text carrying the install/finishing state.
- Made native model refresh bounded and observable: the gateway request uses an 8-second connect / 20-second total native timeout, the renderer has a 20-second watchdog, installation completion triggers a forced refresh, and ready/stale/skipped/failed outcomes are shown in a dedicated status banner.
- Fixed the restart race that labeled the current USB Codex as installed elsewhere: path normalization accepts Windows device prefixes, the renderer treats a matching current USB portable tree as owned even when the first status snapshot is stale, and it re-reads `win_status` after USB authorization becomes live.
- Removed the unplanned 404token account password card from the USB home screen; the account backend remains isolated and unused by the one-click flow.
- Regression evidence: offline UI 32/32; root frontend suite `npx vitest run src` 364/364; `npm run check`, production renderer build, focused ESLint, model catalog Rust tests 7/7; the full root `npm test` invocation also scanned the independent CCSwitch tree and reported 37 pre-existing alias/dependency-import failures, while its root tests still passed.
- Rebuilt 289-file `release/usb-oneclick-g39.5-20260922-r10-ui-model-status-final`; manager SHA-256 is `07B6D46BDF6292EFD7F7D435077DACF6549A3A5A3AE49AACE7981879F486C4D0`. ZIP `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r10-ui-model-status.zip` has SHA-256 `98DCACC8FF39E1F9C5F9A0CBE07F94599FCF1F6DE3BC04EE7AC080B5B378BEA0`. USB, one-click, CCSwitch static/network, PE-manifest and signing-pair gates passed.
- Physical NTFS U-disk registration/sign/install/model-refresh/update/uninstall and isolated CCSwitch GUI smoke remain target-device acceptance items.

## 2026-09-22 G39.5/r11 fresh EXE and signer delivery

- Generated a new formal USB signing profile `release/usb-signing-release-profile-g39.5-20260922-r11.json` bound to `releaseId=g39.5-20260922-r11`, `keyId=prod-2026-01`, and public-key digest `216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`.
- Rebuilt the requireAdministrator USB installer/manager from the current renderer and Rust sources. New manager SHA-256: `4F894F455364C4E0EF82A977550F8F8F2D7F56E29624A57B8CE4518B65B2755A`; launcher/setup SHA-256: `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`.
- Rebuilt the GUI signer in a fresh Cargo target and output directory: `release/usb-license-signer-gui-v0.5.24-g39.5-r11-20260922-fresh`; its manifest confirms compile-time embedded administrator credential, matching portable vault, and r11 release binding. Signer EXE SHA-256: `F922D125C0334187D8B31F667596D34944F8BEC2C718355DAB75505F534C74F2`.
- Generated the complete fresh USB directory `release/usb-oneclick-g39.5-20260922-r11-fresh` (289 files, 1,317,438,744 bytes) and archive `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r11-fresh.zip` (1,259,250,753 bytes; SHA-256 `CF754FDB4953E7AB7CC8D72087C71D06DCA0BF65EEFF582F852C1286957ED818`). Payload SHA-256 remains `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`.
- Packaged the signer separately as `release/CodexUsb-LicenseSigner-G39.5-v0.5.24-20260922-r11-fresh.zip` (2,200,833 bytes; SHA-256 `16BFB98F22592237874F51D8E7A8789EB70FBB7B36A5DD38A3F9AF844EC6B359`; 9 entries).
- Fresh r11 static and network gates passed: package/payload/one-click verification, CCSwitch product verification, `/health` 200, unauthenticated `/v1/models` 401, PE `requireAdministrator` checks, signing-pair preflight, profile/portable-vault binding, and archive generation.
- Physical NTFS U-disk registration/sign/install/model-refresh/update/uninstall and isolated CCSwitch GUI smoke remain target-device acceptance items.

## 2026-09-22 G39.5/r12 install-stall and second-stage-uninstall repair

- 修复 Windows 便携回退安装在“正在替换程序文件”阶段长时间假死的问题：MSIX 解压/复制尚未发生首次安装根目录替换前，状态保持为可中断的 Applying；只有真正进入 MSIX 注册或原子替换边界后才进入 Committing，因此慢速 U 盘期间可见正确阶段且不会被错误阻止关闭。
- 修复二阶段卸载交接竞态：已授权的卸载 handoff 在 Tauri/WebView2 延迟退出期间保留代次 ID，重复完成调用保持幂等；显式完成后增加有界退出看门狗，避免管理器窗口无法退出而只能强关。
- 修复 Windows finalizer 启动命令的编码/引号脆弱性：Detached PowerShell 现在使用 UTF-16LE `-EncodedCommand`，启动确认等待从 10 秒扩大为 30 秒；复杂路径、中文路径和特殊字符不再使 finalizer 在写入 ready 标记前退出。
- 修复“主卸载已移除安装根但旧 NewInstalled 事务仍阻塞二阶段”的恢复策略：当新路径和备份均不存在时，仅清理已落地的 bookkeeping journal，不重放任何文件移动；不完整或仍有恢复路径的事务继续保留并阻断。
- 全量 Rust 回归：447 passed / 3 ignored；Windows x64/custom-protocol target check、TypeScript、ESLint、diff check 通过。
- 生成新的正式绑定 profile `release/usb-signing-release-profile-g39.5-20260922-r12-install-recovery.json`；重建 manager/launcher 和完整 U 盘目录 `release/usb-oneclick-g39.5-20260922-r12-install-recovery`（289 文件，1,317,353,472 字节）。Manager SHA-256 `B3DBAC10DFC27BD2A2858A28882E8E969716E77AF394BD53D247041B35C18A8D`，入口 SHA-256 `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`，MSIX SHA-256 `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。
- 刷新签名器发布目录 `release/usb-license-signer-gui-v0.5.24-g39.5-r12-20260922-install-recovery`，保留已验证的嵌入式签名器二进制（本轮签名器源码未改动），更新 profile/manifest/vault 绑定；签名器 SHA-256 `F922D125C0334187D8B31F667596D34944F8BEC2C718355DAB75505F534C74F2`。
- 完整 U 盘 ZIP `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r12-install-recovery.zip` SHA-256 `08AE0767D39524C532F76ACA3FF04A62FB5D93F44968464BFC25AB35731EEE91`；签名器 ZIP `release/CodexUsb-LicenseSigner-G39.5-v0.5.24-20260922-r12-install-recovery.zip` SHA-256 `888F555EEA3282593CC149ED2BE07E66844D2312589AD4E11131FBF0B89F656C`。包、载荷、完整一键验证、签名配对、PE requireAdministrator 清单、ZIP 中心目录均通过。
- 物理 NTFS U 盘注册/签名/安装/更新/卸载仍需在测试机执行；本工作区不能替代真实 U 盘和 Windows AppX/文件锁环境验收。

## 2026-09-22 G39.5/r13 portable extraction root-cause repair

- 根因已从日志和实际 MSIX 结构确认：旧便携回退先把整个 802 MB 压缩包解压到 U 盘 `staging/extracted`（约 1.96 GB），再把 `app` 目录完整复制到 `staging/payload`，同一批大文件在慢速 U 盘上被写两遍；这段路径没有解压进度事件，所以界面长期停在“正在替换程序文件”，日志也停在 `committing` 前，容易被误判为事务死锁。
- `prepare_portable_payload` 现在读取 `AppxManifest.xml` 的权威 `Application@Executable`，只把其所在目录树直接流式解压到一次性事务 payload；真实包的 `app/ChatGPT.exe` 因此只写一遍约 1.96 GB。保留无 Application 声明时的完整解压兼容回退，并对每 128 MiB/512 文件记录进度、校验每个文件长度和最终入口存在性。
- 新增 `portable_payload_extraction_copies_only_declared_app_tree` 回归，证明包外 `assets` 不会被写入且不再创建完整 `extracted` 中间树；引擎全量测试 180 passed / 1 ignored，管理器回归 447 passed / 3 ignored。
- 重新生成 profile `release/usb-signing-release-profile-g39.5-20260922-r13-portable-extract.json`，重建 manager/launcher 和完整目录 `release/usb-oneclick-g39.5-20260922-r13-portable-extract`（289 文件，1,317,364,224 bytes）。Manager SHA-256 `C1464191A9EB3A84283CF77B311173DC7E6FCDEDE373BFEAF37001C2AD3DF677`，launcher `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`，MSIX `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。
- 签名器源码本轮未变更，复用已验证的嵌入式二进制并刷新 r13 profile/manifest/vault 绑定；签名器 EXE `F922D125C0334187D8B31F667596D34944F8BEC2C718355DAB75505F534C74F2`。USB ZIP `release/CodexUsb-OneClick-G39.5-v0.5.24-20260922-r13-portable-extract.zip`（1,260,595,938 bytes，SHA-256 `799393F47F0207CD21458617546C3767B64CF103BF366A0EFDBBF2B2261B0A83`）；签名器 ZIP `release/CodexUsb-LicenseSigner-G39.5-v0.5.24-20260922-r13-portable-extract.zip`（2,277,755 bytes，SHA-256 `7BF8ACCE06E29ABA890FF7FE20883303C307D853AD76345B774AE9541217CFAD`）。
- 包、payload、完整 one-click、签名配对、CCSwitch、PE `requireAdministrator` 清单、ZIP 中心目录、TypeScript/ESLint、前端 364 测试、管理器 447 测试和引擎 180 测试均通过；另外用真实 r13 MSIX 运行了原先忽略的便携事务测试（本地存储 49.29 秒，通过且不启动 Codex/注册 AppX）。物理 NTFS U 盘安装仍需测试机验收。

## 2026-09-22 G39.5/r14 extraction progress and one-click recharge

- 接受“当前确实是慢速 U 盘，不重复按卡死审查”的补充信息；把优化范围收敛到可观测进度和写入效率。
- 便携解压新增按 ZIP 应用文件总字节数的实时回调，安装页沿用同一条进度事件显示真实百分比/已写入字节；Applying 阶段的进度事件不会再把操作回退成 Downloading。
- 解压改为可复用 1 MiB 缓冲区、按 4 MiB 或 200 ms 节流上报，并保留逐文件长度校验、入口存在校验和事务清理；这避免在慢速介质上产生过密 UI 事件，同时减少小块写入开销。
- 安装管理器挂载紧凑的 404token 账户卡片：默认只显示充值码输入和“一键充值”，未登录时才按需展开密码；充值使用现有用户 JWT `/api/v1/redeem`，管理员密钥不进入管理器，400/409 会显示可操作的失败提示。
- Rust 引擎 180/1 ignored、管理器 447/3 ignored、前端 32 文件/365 测试、TypeScript、ESLint、Windows x64 target check、格式和 diff check 均通过；新增解压进度单测通过。
- 新正式包目录：`release/usb-oneclick-g39.5-20260922-r14-progress-recharge`（289 文件，1,317,433,148 字节），管理器 SHA-256 `DC75E35C0C1A050235E5211E3879E3573C29CAD81D1FE6399ED11F885DF85262`，入口沿用 `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`，载荷沿用 `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。
- 新签名器目录：`release/usb-license-signer-gui-v0.5.24-g39.5-r14-20260922-progress-recharge`（8 文件，5,032,684 字节），复用已验证的嵌入式签名器二进制，SHA-256 `F922D125C0334187D8B31F667596D34944F8BEC2C718355DAB75505F534C74F2`；包/载荷/一键/签名配对/PE 清单校验通过。
- 仍需测试机做真实 NTFS U 盘慢速安装、充值码成功/失败和管理员网络条件验收；本工作区没有冒充完成这些设备绑定步骤。

## 2026-09-22 G39.5/r15 CCSwitch auto-provisioning and upstream model-refresh resilience

- 根因确认：旧包虽然在管理器进程层面拿到了 U 盘绑定，但 CCSwitch 仍是旧二进制，未消费 `CCSWITCH_404TOKEN_DEVICE_KEY`，所以安装后不会自动写入/激活 CCSwitch 的 Codex provider；模型刷新也只有单次请求，404token 间歇性不可达时界面只显示笼统失败。
- CCSwitch 源码已补齐 USB bootstrap：启动阶段读取管理器的短生命周期环境变量，在 SQLite 初始化后幂等写入并切换 `404token-sub2api` provider，保存/切换事务最多重试 3 次；USB 授权成功后由管理器隐藏主窗口并保留托盘进程，用户不再被弹到首页手动填 Key。新 CCSwitch 产物 SHA-256：`927AF2AA40E2F5E1B08ECEF90C9BA8063DFB9667007E4F7AF12395463E867378`。
- 模型目录刷新改为 3 次有界重试（连接 8 秒、单次请求 20 秒、指数退避），401/403 明确提示“404token 上游拒绝授权”，超时/网络/无效响应明确提示“404token 上游不可达/返回无效，不是本地程序问题”，并保留旧目录；前端自动刷新等待窗口同步到 60 秒并显示上游故障提示。
- 重新编译正式管理器并生成非 ZIP U 盘目录：`release/usb-oneclick-g39.5-20260922-r15-upstream-retry-tray`，289 文件、约 1,317,450,042 字节；管理器 SHA-256 `2E6E02CE3DBD43412EFC7DBCE16C8C68A5B11890F6742272C0B0ACCCCD829C38`，入口 `A9CE2475D6F2912AEB00A604DA68CFCD8D2BA0A1291A0C3BC8A0BF4F2903C464`，CCSwitch 根/嵌入副本均为 `927AF2AA40E2F5E1B08ECEF90C9BA8063DFB9667007E4F7AF12395463E867378`，MSIX 仍为 `AB0FEACC74E982742736DA4A7F1D2B59E9FABAA3641606545D3064927DA02B1E`。
- 当前包继续使用已验证的 r14 SignedRequired 公钥/profile，与配套签名器 `release/usb-license-signer-gui-v0.5.24-g39.5-r14-20260922-progress-recharge` 做正式 pair 校验通过；本轮签名器源码未改变，因此没有冒险生成未带管理员 Key 的新签名器。
- 回归证据：OfflineHome/managerApi 60/60；模型目录 7/7；CCSwitch manager 9/9；Windows window engine 5+1 helper；TypeScript check、release tests 56/56、offline bundle test、CCSwitch 静态验证、包内/仓库 one-click、payload、PE requireAdministrator、签名 pair 均通过。
- 仍需测试机做真实 NTFS U 盘注册/签名/安装后 CCSwitch 托盘与真实 404token 模型刷新验收；本工作区不能替代设备绑定和上游生产网络条件。

## 2026-09-22 G39.5/r16 404token account retry closure

- 签名器 `Sub2ApiClient` 为安全建号补上稳定 `usb-provision-USB-*` 幂等键；`create-user` 遇到传输/超时/5xx/无效响应时，最多 3 次有界恢复，重发前先按同一 USB 编号/邮箱查询，若服务器已经落库则直接续作登录和用户 API key，避免重复建号；管理员分组和 USB 用户查询也采用 3 次安全 GET 重试。
- 管理器 404token 登录、余额、用量、充值请求统一采用 3 次有界重试（8 秒连接、30 秒单次请求、500ms/1s 退避）；充值请求携带不含充值码明文的稳定 `Idempotency-Key`，并把最终网络错误明确标为 404token 上游不可达，不再让用户误判为本地安装失败；余额成功但用量失败时明确显示上游用量暂不可用。
- 新增 signer 本地 HTTP recovery fixture（服务器已接受建号但返回 503 时不重复 POST）、幂等键断言，以及管理器 transient HTTP/网络耗尽夹具、重试分类和充值幂等键测试。
- 当前源代码 release 编译通过；管理器重新构建并生成 `release/usb-oneclick-g39.5-20260922-r16-account-retry-manager`（289 文件，1,317,521,722 bytes），Manager SHA-256 `95711EE5FEA0BB375789075BF3FFDFF40C2C92E58F4F700C62A72ADA3888BDC0`。包内 verify-usb、仓库 one-click（InternalSigned）、payload、CCSwitch、PE manifest 和 signing-pair 校验均通过。
- 生产签名器 EXE 本轮没有用未配置管理员 Key 的构建覆盖现有版本；源代码和无密钥 release 编译已验证，正式 signer 替换必须在保有原管理员 Key 的安全构建环境中执行。现有已验证 signer 保持不变，避免破坏已实测可用的真实建号链路。

## 2026-09-22 G39.5/r16 final watchdog/package rebuild

- 审计发现模型刷新最坏耗时是 3 次 20 秒请求加 500ms/1s 退避，共约 61.5 秒；前端 watchdog 从 60 秒调为 75 秒，避免把 404token 上游最终诊断误报为本地超时。
- watchdog 改动后 `npm run check`、`npm run lint` 和 OfflineHome/managerApi 60/60 回归通过。
- 重新构建 `release/setup/CodexAppManager.exe`，并生成最终非 ZIP 目录 `release/usb-oneclick-g39.5-20260922-r16-account-retry-manager-final`（289 文件，1,317,521,722 bytes）；Manager SHA-256 `9C216EAC5AF7EB2761699B4908003CE7DE8D4E592A82D4DD5CE7D10759A52BD8`。
- 包内/仓库 one-click、payload、CCSwitch、PE 清单和签名配对校验全部通过；最终目录继续使用已验证的 r14 signer/profile pair。
