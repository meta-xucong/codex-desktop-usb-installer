# G31：启动就绪反馈与后台控制台静默开发文档

状态：代码与静态审计完成，目标机发布包实测待验证

## 1. 用户原始要求

目标测试机为纯空白电脑，当前发现：

1. 安装器在“安装包已准备好”到“一键安装”允许点击之间有等待时间，但前端没有“加载中”的转圈提示，容易让用户误判。
2. “一键安装”变为可用后，用户没有点击任何按钮，电脑却不断弹出并立即关闭 CMD 窗口。
3. 希望所有类似的 CMD/PowerShell 弹窗都隐藏，任何时候不要影响客户体验。

## 2. 当前基线与证据

- 起始提交：`6e38cfbd157b47e0281eba25f72b141cba9e30ca`
- 当前分支：`checkpoint/pre-package-2026-09-09`
- 工作区在任务开始时已有大量未提交改动；这些改动属于既有基线，不得回滚、覆盖或冒充本任务差异。
- 未发现项目级 `AGENTS.md`。
- `src/app/views/OfflineHome.tsx` 在廉价状态探测完成后先设置 `phase="ready"`，再异步执行 `offlinePayloadProbe()`；当 `usbGuard.status === "starting"` 时只显示普通信息条，主区域不保持加载动画。
- `src-tauri/src/lib.rs` 将完整 USB SHA/AuthentiCode 校验放到启动后台 worker；`usb_guard::STARTUP_INTEGRITY_PENDING` 清除前不会开放 Codex 操作。
- `src-tauri/src/app/usb_guard.rs` 的 `current_status()` 每次都会通过 `guard_helper_is_live()` → `process_matches()` 调用 `tasklist.exe`；`tasklist.exe`、两处 `taskkill.exe` 没有设置 `CREATE_NO_WINDOW`。
- `src-tauri/src/app/offline_payload.rs` 的 Windows Build 查询调用 PowerShell，未设置 `CREATE_NO_WINDOW`。
- `src-tauri/src/app/capability_pack.rs` 的能力包安装调用 PowerShell，未设置 `CREATE_NO_WINDOW`。
- `crates/codex-theme-engine/src/transaction.rs` 的 Windows 进程存活检查调用 `tasklist.exe`，未设置 `CREATE_NO_WINDOW`。
- 已有的看门狗主进程、运行库安装器、AppX 提权操作、卸载清理器及 curl 路径已经设置隐藏相关标志，不能重复造实现。
- 发布 GUI 入口已有 `windows_subsystem = "windows"`；该设置不能替代子进程的 `CREATE_NO_WINDOW`。

## 3. 范围

### 3.1 本次必须完成

- 启动完整性/授权仍在进行时，页面明确呈现持续加载状态（至少有可见旋转指示和简短文案），并保持受保护操作不可用。
- 复用现有 `starting` 状态与就绪门禁，不改变签名、U 盘授权、看门狗和安装安全语义。
- 对本仓库生产运行路径中由安装器启动的后台 `tasklist`、`taskkill`、PowerShell 等控制台进程设置 Windows 隐藏窗口标志，避免空闲轮询或后台检查闪出窗口。
- 保留必要的 UAC 同意界面、安装器自己的错误提示、用户主动打开的日志/终端；不得用“隐藏所有窗口”掩盖安全确认或错误。
- 增加或更新定向测试，覆盖启动等待状态和关键 Windows 隐藏进程配置/源码门禁。

### 3.2 明确不做

- 不改变 USB 签名校验、许可证绑定、授权失败策略、轮询频率、看门狗生命周期或安装/更新业务流程。
- 不全局关闭 Windows 的 CMD/PowerShell，也不影响其他软件启动的终端。
- 不隐藏 UAC 提示，不删除错误日志，不静默吞掉后台命令失败。
- 不在本任务中打包 EXE、制作签名文件、推送 GitHub 或修改 Skill Hub/模型同步。
- 不升级依赖，不做无关重构，不改变公开 API、JSON schema 或持久化格式。

## 4. 冻结执行契约

### C1：就绪反馈

当 `payloadLoading`、`phase === "loading"` 或 USB 看门狗状态为 `starting` 时，界面必须显示可观察的加载指示；在授权真正可用前，安装和更新按钮必须继续禁用。已有授权门禁保持最终权威。

### C2：后台进程静默

Windows 生产路径中由 GUI 安装器启动的控制台子进程必须使用 `CREATE_NO_WINDOW`（需要脱离父进程时按现有语义组合 `DETACHED_PROCESS`），并继续保留 stdout/stderr 的现有捕获或日志行为。

### C3：安全和可诊断性

隐藏窗口不等于忽略失败。现有退出码、错误文本、日志和用户可见错误路径必须保留；UAC 同意提示不得隐藏。

### C4：兼容性

非 Windows 编译路径不引入 Windows 专属导入或行为；已有隐藏路径不被破坏。

### C5：范围纪律

允许改动：

- 本开发文档；
- `src/app/views/OfflineHome.tsx` 及其定向测试/必要 i18n 文案；
- `src-tauri/src/app/usb_guard.rs`；
- `src-tauri/src/app/offline_payload.rs`；
- `src-tauri/src/app/capability_pack.rs`；
- `crates/codex-theme-engine/src/transaction.rs`；
- 与上述改动直接对应的现有测试文件。

不得修改其他业务模块、依赖、安装产物或既有未提交改动。

## 5. 分工与交接

- 主控：当前会话，负责基线、契约、文档、集成和最终复验。
- 执行负责人：唯一代码写入者；只修改第 4 节允许的代码/测试文件。
- 独立审计负责人：执行版本冻结后只读审计，不能修改被审代码、测试或本文件的验收标准。
- `COMPLEXITY_GATE=ESCALATE_REQUIRED`：任务跨前端、Tauri Rust、两个内部 crate，并涉及 Windows 进程创建和安全可见性。
- `CONTRACT_REV=v1.3.1-route-guard`。
- `CONTEXT_MODE=ARMED`：任务包含跨模块交接和较多代码读取；仅在上下文退化或需要压缩时进入 ACTIVE。
- `MONITOR_MODE=DELIVERY_SILENT`：只在阶段交接、失败、审计结论或范围变化时报告。

## 6. 验收矩阵

| ID | 场景 | 预期证据 |
| --- | --- | --- |
| R1 | 前端启动完整性处于 `starting` | 测试证明可见加载指示；安装/更新继续禁用 |
| R2 | payload 探测未完成 | 既有加载测试继续通过，不提前开放操作 |
| R3 | USB 看门狗就绪后 | 既有状态轮询和按钮解锁测试继续通过 |
| R4 | `usb_guard` 进程查询/结束 | 生产调用均带 `CREATE_NO_WINDOW`，退出码/输出处理不变 |
| R5 | Windows Build 与能力包 PowerShell | 生产调用均带 `CREATE_NO_WINDOW`，错误仍可回传 |
| R6 | 主题事务进程检查 | Windows `tasklist` 调用不产生可见控制台 |
| R7 | 非 Windows 编译/测试 | cfg 结构有效，不引入平台编译错误 |
| R8 | 回归 | 前端定向测试、Rust 相关 crate 测试、格式/静态审计通过 |
| R9 | 反例 | 不把 UAC、安装器错误对话框、用户主动终端误判为应隐藏的后台窗口 |

目标机实测仍需验证：发布 EXE 双击后等待阶段、按钮解锁后静置至少 2 分钟、安装/修复/更新/卸载及 U 盘拔插路径均无意外可见 CMD/PowerShell。隐藏进程本身可以存在，验收对象是“无可见控制台窗口”，不是“系统中绝不出现 powershell.exe 进程”。

## 7. 版本冻结与审计要求

代码完成后先停止写入并记录最终差异/哈希。独立审计逐项对照 C1–C5、R1–R9，检查：

- 是否遗漏任何生产 Windows 控制台子进程；
- 是否误改授权、轮询、错误处理或 UAC 行为；
- 是否有超出范围的文件、依赖、抽象或业务行为；
- 测试是否绑定最终版本，是否把未运行或环境受限伪称通过。

审计结论只能为“符合”“不符合”或“证据不足”。若不符合，按“范围内修正 → 新版本冻结 → 受影响测试 → 独立复审”回流。

## 8. 当前状态

- 文档：已冻结。
- 首位执行负责人：已通过平台停止请求收敛；停止证据显示 `completed`，且报告“新增差异：无、无运行中的测试”。
- 当前唯一写入者：主控；接手前已确认首位执行负责人终止，避免并行写入。
- 代码：已按本契约落最小实现：启动就绪阶段复用同一门禁并显示旋转指示；Windows 后台 `tasklist`、`taskkill`、Build 查询 PowerShell、能力包 PowerShell 及主题事务 `tasklist` 均设置 `CREATE_NO_WINDOW`；未改变 UAC、错误回传、日志或授权语义。
- 定向测试：`npm run test:offline` 通过（28/28）；`npm run check`、`npm run lint`、`npm run build` 通过；`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`、主题引擎和 Codex Windows 引擎格式检查通过；`src-tauri` 单元测试通过（417，忽略 2）；`codex-theme-engine` 单元测试通过（50）；`codex-win-engine` 单元测试通过（180）。
- 版本冻结：主控已停止代码写入；上述测试和静态差异检查均针对当前工作树执行。独立审计负责人不得修改代码、测试或本文件验收标准。
- 审计：独立审计代理在两个有界窗口内未返回报告，已按协作规程终止且未产生任何差异；主控随后完成只读源码审计，结论逐项覆盖 C1–C5、R1–R9，并明确保留目标机实测缺口。
- 打包/签名：后续已按 `g31-final-20260916` 完成安装器、完整 U 盘 Bundle 和 portable 签名工具打包；本次未向任何 U 盘写入或生成 `usb-license.json`。

## 9. 实现记录（2026-09-16）

### 9.1 前端就绪反馈

- 新增 `startupReadinessPending`，统一覆盖 payload 探测、启动阶段和 USB 看门狗 `starting` 状态。
- 只要启动授权仍未完成，首页英雄区持续显示旋转加载指示，安装、Codex 更新及其他受保护动作保持禁用。
- 增加回归测试，覆盖 `starting` 状态下的加载指示与按钮门禁，并保留 payload 慢探测回归。

### 9.2 Windows 控制台静默

- `usb_guard` 的后台 `tasklist`/`taskkill` 使用 `CREATE_NO_WINDOW`；仍保留原有输出捕获、退出码和错误处理。
- Windows Build 探测与能力包安装的 PowerShell 使用 `CREATE_NO_WINDOW`；失败仍通过既有 `AppError` 路径返回。
- 主题事务的 Windows `tasklist` 存活检查使用 `CREATE_NO_WINDOW`。
- 本记录只覆盖生产路径；测试夹具和用户主动打开的终端不属于隐藏范围，UAC 仍按系统安全语义显示。

### 9.3 验收边界

单元测试和源码静态审计可以证明状态门禁、平台条件编译和进程创建标志；不能替代目标机对发布 EXE 的双击启动、静置观察和 UAC/错误路径实测。发布包实测仍按 R9 执行，未在本任务中冒充完成。

## 10. 审计结论（2026-09-16）

### 10.1 主控只读审计证据

- 对 `src-tauri/src`、`crates/codex-theme-engine/src`、`crates/codex-win-engine/src` 的 shell 调用做了全量关键词扫描。生产路径中的直接 `tasklist`、`taskkill`、PowerShell 均已带 `CREATE_NO_WINDOW` 或走 `codex-win-engine::process::hidden_command`；测试夹具/示例中的 shell 调用不属于发布 GUI 生产路径，且现有测试夹具也使用隐藏标志。
- 定向静态门禁 8/8 通过：`usb_guard` 两处 `tasklist`、两处 `taskkill`，Windows Build PowerShell，能力包 PowerShell，主题事务 `tasklist`，Codex Windows 引擎终止树 `taskkill` 均检测到隐藏标志。
- `git diff --check` 通过；未增加依赖、公开接口、持久化格式或打包产物。工作区中其他未提交改动均为任务开始前既有基线，未回滚或覆盖。

### 10.2 矩阵结论

| 项目 | 结论 | 证据/边界 |
| --- | --- | --- |
| C1 | 符合 | `OfflineHome` 回归测试 28/28；`starting` 与 payload/phase 等待均显示 spinner，受保护按钮禁用 |
| C2 | 符合 | 生产 shell 调用静态扫描与 8/8 定向门禁通过 |
| C3 | 符合 | 仅增加进程创建标志和等待态展示，未移除错误、日志、退出码或 UAC |
| C4 | 符合 | 三个 Rust crate 格式检查通过；Windows 专属导入均受 cfg 约束 |
| C5 | 符合 | 变更保持在开发文档、OfflineHome 定向测试、四个既定生产模块内；既有脏工作区未被处理 |
| R1–R8 | 符合 | 前端、TypeScript、lint、构建、Rust 单测、格式检查和静态审计均通过 |
| R9 | 证据不足 | 尚未运行本次代码对应的发布 EXE 目标机实测；必须观察双击启动等待、静置 2 分钟及安装/更新/卸载/拔插路径 |

### 10.3 最终工程判断

代码层问题已修复，可以进入下一步打包前验证。当前不能承诺“目标机绝不再闪窗”，因为发布 EXE 尚未在用户的空白测试机上实测；但所有已定位的安装器后台控制台创建点均已完成隐藏处理，且原先的空闲 `tasklist` 轮询路径已覆盖。

## 11. 后续打包记录（2026-09-16）

- 版本 profile：`release-profiles/usb-v0.5.24-g31-final-20260916-key216.json`。
- 完整 U 盘 Bundle：`release/usb-oneclick-v0.5.24-g31-final-20260916/`。
- U 盘入口：`CodexDesktopSetup.exe`；同目录的 `CodexAppManager.exe` 为管理器本体。Bundle 仍是待签名静态目录，必须整体复制到 U 盘后再用签名器写入运行期许可证。
- portable 签名工具目录：`release/usb-license-signer-gui-v0.5.24-g31-final-20260916-portable/`；必须连同 `codex-usb-signing-key.vault`、公共描述、profile、manifest 和校验文件整体携带，运行电脑不参与密钥选择。
- 三方绑定：`releaseId=g31-final-20260916`、`keyId=prod-2026-01`、`publicKeySha256=216ae3583c6895d0cd00338b8e9624c1a91941a1d62637705420d3fdad545c80`。
- 构建门禁：前端 32 个测试文件/353 项测试、安装器 Rust 417 项通过且 2 项按设计忽略、requireAdministrator PE 清单、Bundle 独立验证、pair preflight、签名工具 28+4 项测试和 signer 发布目录 SHA256 校验均通过。
