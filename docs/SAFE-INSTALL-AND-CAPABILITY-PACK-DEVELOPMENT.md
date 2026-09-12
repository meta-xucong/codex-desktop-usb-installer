# 安全安装策略与能力包安装整改方案

状态：实现、测试、本地只读审计与限定范围独立只读审计已完成；未签名/未在另一台物理 Windows 上运行，故不宣告正式发布验收。本文件作为本轮唯一任务记录与冻结契约。

日期：2026-09-09

## 0. 实施任务记录

- `TASK_ID`：`CAP-USB-SAFE-20260909`
- `CONTRACT_REV`：`safe-install-capability-v1`
- 复杂度门：`ESCALATE_REQUIRED`（涉及安装安全边界、用户目录、凭据持久化、子进程和跨模块状态）
- 主控/唯一写入者：当前 Codex 会话；不回滚工作区中本轮之前已有的用户改动
- 允许修改范围：离线安装界面与后端、能力包 PowerShell/Rust/TypeScript 契约、相关测试与本文档
- 非目标：不修改普通 Manager 的显式接管语义；不自动卸载外部 Codex；不把可选服务变成必选服务；不提交、推送或发布
- 版本门禁：代码修改完成后停止写入，固定差异并进行独立只读审计；审计未通过不得宣布验收

本轮复核新增发现：能力包安装在安全跳过外部同名 Skill/Agent 冲突后，旧逻辑仍无条件写入
`core.status=ready`；原生后端对显式 `partial` 还会错误使用旧数量兜底接受。该问题会把“核心能力缺项”
误报为已完成，已在本轮解冻修复并通过回归：readiness 现在按本次请求的期望 Skill/Agent 数量计算，缺项为
`partial`、总状态为 `failed`，Rust 端仅在显式 `ready` 时接受当前 schema；安全跳过的用户文件仍然保留。

## 1. 本轮目标

本方案合并两个问题：

1. 电脑已经存在其他来源的 Codex 时，U 盘安装器不得自动卸载、接管或覆盖；
2. Skills、Agents、MCP 资源和可选运行时必须与 Codex 的安装来源解耦，未勾选的外部服务不得因为缺少 API Key 导致能力包安装报错。

## 2. 诊断结论

### 2.1 未勾选飞书等服务不是截图错误的直接原因

当前前端只有在用户勾选服务后才做必填字段校验：

- 飞书未勾选时，不校验 App ID、App Secret 和 User Access Token；
- DashScope、Image-2、Seedance 未勾选时，不校验对应 API Key、端点、模型和路由字段；
- 未勾选服务不会进入 `enabledApiServices`；
- 一键安装使用 `persistConnections=false`，不会写入或清理外部连接凭据。

后端 `validate_options` 也只对 `enable_feishu=true` 或明确选中的服务执行必填字段校验。因此，截图中的错误不是“没有填写 API Key”直接触发的参数校验错误。

人工组件面板的 `persistConnections` 默认值仍为 `true`，用于在用户明确启用并填写服务时保存配置；整改后未勾选服务不会进入写入列表，也不会进入删除列表，因此不会清理用户已经存在的连接配置。自动一键安装仍使用 `persistConnections=false`。

### 2.2 截图对应的是安装后的 readiness 误判/失败

Rust 代码先判断 PowerShell 子进程是否成功退出；只有退出码为 0 后，才读取本机 readiness 文件。如果出现：

```text
能力包安装脚本已结束，但 readiness 校验未通过
```

说明脚本退出码为 0，但以下至少一项不满足：

- readiness 文件不存在、不可读或 JSON 无效；
- `pack.id` 不是 `codex-starter`；
- 运行时状态不是 `ready`；
- 安装脚本写入的用户目录与 Rust 校验读取的用户目录不一致。

当前错误信息已经包含实际 Skills/Agents 数量、readiness 路径、运行时状态和配置状态；详细阶段输出仍写入脱敏诊断文件，
用户可以据此区分核心文件缺项、Python/Node/飞书 MCP、用户目录或配置合并问题。

### 2.3 当前实现仍会无条件执行运行时准备

虽然用户没有勾选飞书等外部服务，旧版 `install-to-codex.ps1` 仍会调用完整的 `runtime/provision-runtime.ps1`，并准备：

- Python 3.12；
- 离线 Python wheelhouse；
- Node.js 20；
- 飞书 MCP 的离线 npm 包；
- 四个运行时入口和健康检查。

随后 Rust 又把 `runtime.status == ready` 作为整个能力包成功的必要条件。因此，未勾选飞书不会触发字段校验，但仍可能因为飞书 MCP、Python、Node 或当前用户目录问题导致整个能力包被显示为失败。整改后未启用飞书时会跳过 Node/飞书 MCP 的健康检查；Python、配置和可选运行时分别报告，运行时部分失败不再抹掉已安装的核心 Skills/Agents。

这属于能力包安装流程的设计缺陷，而不是官方 Codex 与 U 盘版 Codex 的兼容性问题。

## 3. Codex 安全安装策略

### 3.1 状态规则

| 当前状态 | U 盘一键安装 | 允许的动作 |
|---|---:|---|
| 未检测到 Codex | 允许 | 首次安装 |
| 当前 U 盘已托管 Codex | 允许 | 启动、修复、更新 |
| 外部 AppX Codex | 禁止 | 重新校验、查看日志 |
| 外部便携版 Codex | 禁止 | 重新校验、查看日志 |
| 其他位置的托管 Codex | 禁止 | 重新校验、查看日志 |
| 本机版本高于 U 盘版本 | 禁止 | 更新 U 盘包后重试 |
| 状态检测失败/恢复未完成 | 禁止 | 重新校验或完成恢复 |

### 3.2 安全边界

- U 盘安装流程不调用外部 AppX 卸载；
- 不调用外部 Codex 接管或迁移；
- 不因安装 U 盘版而删除本机用户数据；
- UI 门禁和 Rust 后端都必须重新检查状态，防止状态竞争绕过前端；
- 普通 Manager 的显式“接管”能力不在本轮离线 U 盘安装流程中复用。

## 4. 能力包优化方案

### 4.1 拆分为核心能力和可选运行时

#### 核心能力必须独立成功

包括：

- Skills；
- Agents；
- 配置模板；
- 本地 MCP wrapper；
- 能力包所有权记录；
- 核心 readiness 记录。

核心能力安装不检查 Codex 是官方版、U 盘版还是尚未安装。

#### 运行时单独报告

包括：

- Python 3.12 和 wheelhouse；
- Node.js 20；
- 飞书 MCP npm closure；
- 依赖运行时的健康检查。

运行时失败时应显示“核心能力已安装，部分运行时未就绪”，不能把所有 Skills 和 Agents 伪装成未安装。

### 4.2 未勾选外部服务的行为

- 未勾选：不要求 API Key；
- 未勾选：不写入该服务环境变量；
- 未勾选：不启用该 MCP/API 配置；
- 未勾选：只提示“未启用”，不报错；
- 未勾选：不删除已有环境变量；清理必须由用户单独、明确授权，并且只允许清理安装器自己记录为拥有的变量；
- 已勾选但缺字段：阻止该次提交，并明确提示缺哪些字段；
- 已勾选且字段完整：安装并执行对应连接健康检查。

对于飞书 MCP，建议把“离线包文件已复制”和“飞书连接已启用”分开。没有凭据时可以保留本地包，但必须保持禁用状态；如果飞书包本身健康检查失败，也不能影响普通 Skills 和 Agents。

### 4.3 统一目标用户目录

安装脚本与 Rust 校验必须共享同一个绝对路径对象：

```text
targetUserProfile
targetKitHome
readinessPath
runtimeStatePath
```

不得一边使用显式 `TargetUserProfile`，另一边重新读取当前进程的 `USERPROFILE` 来猜测路径。

readiness 还应记录：

- 目标用户目录；
- 安装器进程用户；
- 能力包版本；
- 能力包源哈希；
- 本次安装运行 ID；
- 核心能力状态；
- 每个运行时的状态和错误摘要。

### 4.4 改进错误诊断

Rust 当前已经捕获 PowerShell 输出，但没有展示。整改后应：

- 保存脱敏后的 stdout/stderr；
- 记录失败阶段和退出码；
- 报告实际 readiness 路径；
- 报告目标用户目录；
- 明确指出是核心能力、Python、Node、飞书 MCP 还是配置合并失败；
- 不写入 API Key、App Secret、Token 或 Cookie；
- UI 默认显示简短提示，详情放入可展开区域和日志。

## 5. 建议的状态模型

能力包报告建议至少区分：

```text
coreStatus: ready | partial | failed
pythonStatus: ready | skipped | failed
nodeStatus: ready | skipped | failed
feishuStatus: disabled | ready | failed
configStatus: ready | partial | failed
overallStatus: ready | partial | failed
```

`overallStatus=partial` 不应阻断用户继续使用已安装的 Skills 和 Agents，也不应触发 Codex 卸载或回滚。

## 6. 验收测试

### 安全安装

1. 无 Codex 时允许一键安装；
2. 外部 AppX 存在时一键安装不可用；
3. 外部便携版存在时一键安装不可用；
4. 外部 Codex 不会被调用卸载或接管；
5. 当前 U 盘已托管 Codex 时修复和更新仍可用；
6. 本机版本较新时不允许降级；
7. 页面状态变化后，后端仍拒绝不安全安装。

### 能力包

1. 无 Codex 时可安装核心能力；
2. 官方 AppX Codex 存在时可安装核心能力；
3. U 盘版 Codex 存在时可安装核心能力；
4. 外部 Codex 存在时组件修复仍可单独运行；
5. 所有可选项未勾选且没有 API Key 时，核心安装成功且连接保持禁用；
6. 只勾选飞书但缺字段时，只显示字段提示，不启动安装器；
7. Python 失败时 Skills/Agents 仍报告已安装；
8. Node 或飞书 MCP 失败时不伪装为整个能力包未安装；
9. 管理员/UAC 环境下 readiness 写入和读取同一用户目录；
10. 旧版本 readiness 不会冒充本次安装成功；
11. 外部同名 Skill/Agent 被安全跳过时，用户文件不被覆盖且 readiness 不误报完整；
12. 失败详情不泄露凭据。

## 7. 实施顺序

1. 先落地外部 Codex 禁止自动接管/卸载策略；
2. 修复能力包目标用户目录和 readiness 传递；
3. 保留核心能力安装与运行时安装的独立状态；
4. 补充子进程日志和阶段化错误报告；
5. 增加上述前端、Rust、PowerShell 契约测试；
6. 重新构建完整 U 盘包，执行隔离安装和真实另一台 Windows 机器测试；
7. 通过后再将版本提升到下一个修订版本。

## 8. 当前结论

截图错误不是因为用户没有填写飞书或其他 API Key。未勾选服务的参数校验逻辑符合预期；真正需要整改的是：

1. 可选运行时被错误地纳入整体成功条件；
2. readiness 校验缺少精确失败原因；
3. 显式目标用户目录与 Rust 读取目录可能不一致；
4. 外部 Codex 自动迁移策略需要改为安全阻止策略；
5. 同名外部 Skill/Agent 被安全跳过后，核心 readiness 仍会被误报为完整。

## 9. 本轮实施结果（已落地）

已落地的代码方向：

1. U 盘 UI 与 Rust 安装尾部都拒绝外部 Codex；同时拒绝已托管但程序不在当前 U 盘 `apps\codex` 目录的状态。旧的 AppX 迁移/接管入口不再出现在离线安装页面。
2. `CapabilityPackReport` 增加 `coreReady`、`overallStatus`、运行时/配置状态和脱敏诊断路径；核心就绪但运行时部分失败时返回可继续使用的 `partial`，不再把整个能力包误报为未安装。
3. 目标用户目录从 Rust 传递到 readiness、运行时 Python 目标路径和配置渲染器；不再依赖管理员进程的 `USERPROFILE`/`LocalApplicationData` 推断目标用户。
4. 未选服务不再删除已有环境变量；未启用飞书时运行时跳过 Node/飞书 MCP 的必需健康检查；子进程输出写入能力包目录下的脱敏诊断文件。
5. 修复了三个实际安装缺陷：Windows PowerShell 生成 `.cmd` wrapper 的编码/首行问题、飞书禁用时空参数导致配置渲染器参数错位、隔离目标目录中 Feishu wrapper 无法继承父进程运行时环境的问题。wrapper 现在使用 ASCII 写入，配置阶段始终传入稳定的非空占位路径，Feishu 启用时优先使用能力包自带 Node。
6. 连接向导只记录安装器拥有的环境变量名，不记录值；“清理未启用连接”不会删除外部已有凭据。
7. readiness 核心状态按请求数量核对：正常全量安装要求 50 个 Skill、7 个 Agent；缺项记录为 `partial`，
   整体记录为 `failed`，Rust 不再接受显式 `partial` 的旧数量兜底。

本轮已完成的验证证据：

- `npm test`：34 个测试文件、385 个测试通过；`npm run check`、`npm run lint`、`npm run build` 全部通过。生产构建的 renderer entry isolation 检查通过；仅保留既有 chunk size 提示。
- `cargo fmt -- --check`、`cargo check` 通过；`cargo test`：352 passed、0 failed、1 ignored（含 3 项 readiness 回归测试）。
- `installer-pack/audit-codex-compatibility.py --root installer-pack` 通过：50 Skills、7 Agents、19 个连接字段、25 个 Feishu 工具；安装包逐文件 hash/provenance 校验通过。
- PowerShell 入口解析检查通过，`install-wizard.ps1 -ValidateOnly` 通过；`test-adapter-contracts.py` 的 34 项契约测试通过。
- 隔离安装已验证：所有可选服务关闭时核心 Skills/Agents、Python、配置合并均 ready，Node/Feishu 为 skipped/disabled；启用 Feishu 的离线包安装、Node 健康检查与 wrapper dry-run 通过，输出未泄露测试 Secret；`install-all.ps1 -DisableFeishu` 的一键入口也通过。
- 新增 readiness 回归：干净临时用户目录得到 `50/7`、`core=ready`、运行时 `ready`、配置 `merged`；预置外部同名文件时得到 `49/6`、`core=partial`、整体 `failed`，用户文件保持不变。
- 本轮隔离测试期间发现的问题已在代码中修复并重新生成 `file-manifest.json` 与 `checksums.sha256`；测试目录已清理，避免被项目 lint 或安装包审计误收。
- `npm run tauri:build` 通过，生成常规生产 EXE 和 NSIS 安装包；`tools/build-installer.ps1 -SkipNpmInstall -SkipTests` 通过，生成 USB 专用 Manager、Launcher 和 `CodexDesktopSetup.exe`，三者均通过 `requireAdministrator` PE 清单校验。
- `tools/test-offline-bundle.ps1 -InstallerPath release/setup/CodexUsbLauncher.exe` 通过：合成 MSIX 收集、布局、逐文件 SHA-256 与无 `WindowsApps/WpSystem` 检查，输出 277 个文件、276 个 checksum 条目。
- 用已有的上一版 MSIX bundle 作为输入生成了最终验收目录 `release/usb-oneclick-v0.5.24-readiness-fixed-20260909-v2`；`verify-oneclick-usb.ps1 -SkipNetwork -SkipSignature` 通过，嵌入的当前能力包再次通过兼容性审计（50 Skills、7 Agents）。该目录用于验收，不等同于已签名发布版本。

尚未宣称的边界：未使用真实飞书租户凭据进行第三方 RPC/权限验证，也未在另一台物理 Windows 机器上运行最终签名 EXE；这些属于发布前验收，不应由本地隔离测试替代。

独立审计门禁记录：本轮新增的限定范围 Explorer 只读审计返回三项 PASS：Rust 执行 `install-all.ps1` 且包装脚本请求全量 profile；显式
`partial/failed` readiness 不会被 Rust 的旧数量 fallback 升级；未勾选外部服务不触发凭据校验或秘密写入。该审计没有重新读取完整包清单计数，
因此 50/7 的实际数量仍以本地包兼容性审计与隔离安装证据为准。此前两次超时和一次无证据 Agent 记录为失败尝试，不作为 PASS 依据。

版本门禁：源码与安装包构建输入已冻结；本轮不提交、不推送。未签名、未做真实第三方 RPC、未在另一台物理 Windows 上跑最终 EXE，仍属于正式发布前的外部验收项。
