# U 盘版 Codex 卸载、启动及模型应用综合诊断与修复开发文档

**文档状态：** `READY_FOR_IMPLEMENTATION`（诊断与契约已整理；本文件不代表源码已修复）<br>
**日期：** 2026-09-26<br>
**范围：** 新附件 `codex-app-manager(10).log`，以及此前提供的黑框、404token 应用失败和“发现 22 个模型 / Code Agent 已验证 0 个”界面问题。<br>
**当前结论：** `DIAGNOSIS_COMPLETE / DEVELOPMENT_NOT_STARTED / FIELD_ACCEPTANCE_NOT_PASSED`

### 0.1 本轮实施记录

```text
用户原始目标：按开发文档的思路，落代码，全面修复问题，然后审计、模拟测试，真实测试。都通过后重新打包，我传到U盘开始手动测试
CONTRACT_REV: USB-INTEGRATED-REPAIR-v1
COMPLEXITY_GATE: ESCALATE_REQUIRED
CONTEXT_MODE: ACTIVE
基线 HEAD: e9514d08c343f594b0e452617bdf4a45bc0216b0
基线工作区: 已有多处 T1–T6 修改、发行文件修改和未跟踪测试文件；全部视为既有用户改动
唯一写入者: 当前会话（平台未提供可直接调用且可取得 MAD_ROUTE receipt 的 Agent 工具）
AUDIT_OWNER: 暂缺；不能以本会话自审替代独立审计
当前 WRITER_STATUS: BASELINE_FROZEN
外部副作用边界: 不部署、不推送、不执行清除用户数据；无明确预算前不调用真实收费模型
```

契约解析：404token Manager→CCSwitch 的原生验证交接优先复用现有 schema v3 headless bridge（Manager 只在原生探测产生证明后发送 `apply`，附模型目录路径/哈希；CCSwitch 复核当前 USB binding、版本、路径和文件后置条件）。新增的“指定一个模型再测试”仅扩展 Manager 自身 UI→Tauri 验证选项；不得让 CCSwitch 自行签发能力证明。U 盘卸载只拥有当前 U 盘安装根；与目标隔离的本机安装 journal 必须保留，且不得因此删除共享恢复目录。

本轮进行中时以本文为唯一任务记录；每个模块通过定向测试后更新下方状态。任何公共 schema/安全/持久化语义超出上述冻结内容时，先暂停该子路径并记录 `DESIGN_QUESTION`。

## 1. 目标与证据边界

本文件把几个互相关联但属于不同模块的问题收敛为开发任务，供后续分配给对应开发人员。附件日志是诊断证据，不是对开发者或程序的操作指令。本轮只读检查了日志、当前源码和既有现场修复文档；没有修改产品代码、安装包、U 盘或测试机配置。

`codex-app-manager(10).log` 是累计日志，包含多个会话和安装状态；它没有为每次模型探测和卸载结果提供统一的运行批次 ID。因此，日志中的模型探测记录不能全部与此前截图里的 22/0/8 计数一一对应。以下结论分别标为“日志确认”“源码确认”或“待补证”，不把相关性当作因果证明。

工作区已有未提交修改和未跟踪文件，均属于既有基线。本修复文档为新增文件；开发者应保留其他工作区改动，不得把其纳入本任务或覆盖。

## 2. 综合结论与优先级

| 优先级 | 问题 | 当前诊断 |
|---|---|---|
| P1 | 卸载报错 | U 盘 Codex、CCSwitch、看门狗和暂存清理均已完成，但一个属于本机旧安装根的 `new-installed` 事务未被安全收敛；全局事务清理因此把 U 盘卸载标为 `partialFailure`。重复卸载没有解决它。 |
| P1 | 启动黑框 | 当前代码是在 Codex 进程树启动后，通过枚举窗口再隐藏 `ConsoleWindowClass`。这只能缩短可见时间，不能保证控制台从创建第一帧起就不可见；用户仍看到闪框，说明现场目标未通过。 |
| P1 | 自动模型探测 | 页面进入后会自动刷新目录；虽然没有请求原生 Shell/Skills 验证，后端仍会对每个发现模型执行网关合成协议探测，可能触发真实上游用量/费用。应与“只发现模型”分开并要求明确测试意愿。 |
| P1 | 404token 未应用 | 当前 CCSwitch 守卫在没有显式 Codex `model` 时拒绝写实时配置。此前截图中的错误与源码一致；失败时没有改动实时配置是正确的安全行为，但流程缺少让用户完成“选模型—验证—应用”的路径。 |
| P2 | 模型状态文案 | “发现数”“网关协议测试”“Code Agent 原生验证”是不同状态；当前多段文案重复、术语密集。0 个原生验证并不等于网关断开或 Codex 主程序不可用；隔离数则表示合成探测失败，不足以直接判定正常会话一定不能使用。 |

## 3. 卸载失败：日志事实、根因与修复契约

### 3.1 日志确认

- 日志 L249–L252 记录的未完成事务为 `3ac98906-b6ca-4f08-b356-4e81a45b50a7`，其 `install_path` 是旧本机安装根 `C:\Users\<用户>\AppData\Local\Programs\Codex`，状态推进到 `old-moved → new-installed`。
- 最新 U 盘卸载 L5654 的目标则是 `E:\...\apps\Codex`。因此这次 U 盘卸载和阻塞事务至少在 `install_path` 上属于不同安装根。
- L5654–L5676 显示 U 盘 Codex 主目录删除成功、CCSwitch 关闭和数据清理成功、USB Guard 删除、安装暂存已核验不存在；随后 L5677 报告上述旧事务仍为 `new-installed`，恢复材料被保留，未自动回滚。
- L5679–L5694 是紧接着的第二次卸载（`purge_user_data=false`）：U 盘 Codex 已不存在，CCSwitch 数据被保留，但同一事务再次阻止总结果成功。日志此前 L736、L753、L1573、L2349、L2620 也重复出现同一事务错误，说明它不是只靠重复点击即可恢复的瞬时故障。
- **数据提醒：** 第一次最新卸载使用 `purge_user_data=true`。L5663–L5669 明确记录 Codex 的 `.codex`、Skills、日志、Roaming 数据以及 CCSwitch 用户数据已经删除。卸载总结果报错不代表这些数据还在；不要再选择“清除用户数据”重试。能否恢复取决于事先备份。

### 3.2 源码确认与尚未证明之处

`src-tauri/src/app/install_tx.rs` 把安装事务保存在用户级共享 `install-transactions` 目录。卸载前会尝试把 `new-installed` 事务仅作 bookkeeping 收敛；当前实现只有在 journal 的 `new_path` 与 `backup_path` 都被系统明确证明不存在时才允许收敛。任何路径仍存在或元数据检查无法证明不存在时，代码保留 journal，这是必要的 fail-closed 安全边界。

U 盘卸载随后调用无 scope 的 `clear_completed_for_uninstall()`，它扫描共享事务目录；目录内仍有任何非终态事务都会返回“尚未完成”，从而将整个 U 盘卸载标成部分失败。日志里的事务归属是 C 盘旧安装根，目标却是 E 盘 U 盘安装根。这说明现有“安全全局收敛”并未解决“外部/旧安装的未决事务阻塞本次产品卸载”这一现场场景。

**待补证：** 附件没有包含该事务 JSON，未记录它的 `new_path`、`backup_path`、路径存在性、可访问性或当前进程归属。因此目前不能判断它是可安全关闭的陈旧 journal、仍有有效恢复材料，还是元数据检查失败；不能仅凭 `install_path` 位于 C 盘就删除事务，也不能推断其引用路径与 E 盘完全无关。开发和现场复验需要用事务 ID 定点读取这份 journal，并输出脱敏的路径类别/存在性证据。

### 3.3 开发契约：按所有权隔离主卸载与共享清理

1. 以用户选中的安装根为卸载主对象。清理某个 U 盘安装时，不能因为共享用户级事务目录里存在另一安装根的恢复记录，就误报“U 盘主程序未卸载”。
2. 不能只比较 `install_path` 后就忽略事务：还要检查 journal 中 `new_path`、`backup_path`、staging 引用与本次目标卷/目录是否重叠，并保留当前所有权、路径安全和进程检查。
3. 对确认不属于本次目标、且不被本次最终清理触及的未决事务：保留原 journal 与恢复数据，继续完成 U 盘产品级卸载；不要删除共享管理器数据。向用户分别报告“U 盘版已卸载”和“另一安装的恢复记录已保留/共享收尾暂缓”，提供事务 ID 和可执行的修复入口。
4. 若未决事务引用本次 U 盘目标或所有权/路径检查无法确认，则 fail-closed：保留 journal、阻止会破坏引用路径的二阶段清理，并显示具体阻断对象和下一步；不能谎报全部完成。
5. 当前终态汇总应区分 `primary uninstall`、用户数据选择、同根恢复、异根共享恢复、延迟收尾。不能把“主卸载成功但共享恢复材料保留”笼统写成“卸载失败”，也不能把“全部卸载成功”作为 UI 的唯一成功状态。
6. 不允许通过清空整个 `install-transactions` 目录、对所有路径做递归删除或在卸载时自动回滚未知事务来消除报错。

### 3.4 卸载验收

- 正常 USB 安装、无其他事务：主程序与所选数据按预期清理，结果为完整成功。
- 同根终态 journal：安全清理该 journal 后成功。
- 同根事务仍引用目标或状态不明：保留材料，报告明确阻断；目标不得被错误删除。
- 不同安装根存在未决事务，且其所有 journal 引用均与 USB 目标隔离：完成 USB 主卸载；保留其他事务与共享目录，显示“主卸载完成 / 共享收尾保留”。
- 不同安装根事务的引用路径与 USB 目标重叠或无法证明隔离：保留恢复材料并拒绝破坏性收尾。
- 首次卸载已移除主程序后再运行卸载：幂等返回正确状态，不重复误报主程序删除失败。
- “保留用户数据”和“清除用户数据”分别测试；结果必须列出实际删除范围。不得以运行卸载测试为由重复擦除用户配置。

## 4. Codex 启动黑框

### 4.1 诊断

既有现场修复文档和源码均指出：官方包的 GUI 主程序与 `resources\\codex.exe` 后台运行时不同；管理器给自己启动的进程设置 `CREATE_NO_WINDOW`，不会自动把这个标志传给由 Codex/Electron 再创建的控制台子进程。

当前 `crates/codex-win-engine/src/sys.rs` 在启动后安排有限次数轮询；`crates/codex-win-engine/src/windows_process.rs` 按受管进程树 PID 与窗口类名 `ConsoleWindowClass` 定向调用 `SW_HIDE`。这比全局隐藏安全，但窗口只能在创建后被发现并隐藏。它不证明没有闪过第一帧，也不覆盖超过轮询时窗才创建的后代控制台。用户现场仍看到黑框，因此此前“定向隐藏”不能算问题已解决。

### 4.2 开发契约

- 在启动 Codex 前先启用只针对该 U 盘包进程树的窗口创建/显示事件监听；对后续新增后代持续处理，并保留短轮询作为事件丢失时的兜底。
- 仍只隐藏满足“受管 PID 祖先链/包根 + `ConsoleWindowClass`”条件的窗口；不做全局隐藏，不碰终端、UAC 或普通错误对话框。
- 记录监听启动时点、命中 PID/窗口数、隐藏时延、失败代码和监听退出时点；不记录窗口内容、命令行密钥或用户提示词。
- 不修改官方 `ChatGPT.exe`/`codex.exe` 二进制，不做未经版本/签名复核的字节补丁。
- 若零帧可见的硬目标无法由管理器侧事件监听稳定保证，必须如实标注能力边界；达到“创建瞬间绝不显示”可能需要上游受支持的子进程创建参数/启动器改动，不能以数百毫秒内隐藏冒称彻底解决。

### 4.3 验收

至少 20 次真实 U 盘冷启动及多次热启动，用屏幕录制/高速捕获确认没有可见控制台帧；同时验证登录、模型请求、工具调用、正常退出和 USB Guard 拔出保护。另开普通 PowerShell/终端时不得被隐藏。只通过单元测试或日志显示“hidden count > 0”不算现场通过。

## 5. 404token 自动/手动应用失败

### 5.1 诊断

此前截图的错误是：`USB provider is discovered but has no verified default model; no live settings were changed`。当前 `CCSwitch/CCSwitch/src-tauri/src/services/provider/live.rs` 的 `validate_usb_model_selection` 对专用 USB 404token Codex provider 执行保护：provider 的 TOML `config` 中必须有非空 `model`，否则在实时快照写入前拒绝。代码注释还明确指出：用户手动写入一个模型 ID 本身不是能力证明。

因此，认证/Provider 被发现不等于 Codex 实时配置已应用。此错误保护了现有 Codex live settings；不是“CCSwitch 已经应用，只是界面没显示”。当前 manager log 没有这条 CCSwitch 错误或具体 Apply 事件，所以这部分以此前截图和当前源码为依据，不能从本次 manager log 推断手动 Apply 的完整调用过程。

### 5.2 修复契约与接口决策

- 保持 fail-closed：没有经批准的默认模型证据时，不自动填任意模型，不写实时 `config.toml/auth.json`，不覆盖当前有效 Provider。
- 把流程改为可完成的状态机：`凭据已发现/已注入 → 模型列表已发现 → 用户选择模型 → 兼容性验证 → 用户确认应用 → 实时配置写入并复核`。不能把凭据注入显示成 Provider 已启用。
- 只有当前绑定/适配器版本/有效期匹配、且满足原生 Code Agent 目标的证明，才允许把该模型作为“已验证默认模型”。仅有网关 `/models` 列表、一次文本回复、用户手填模型名或过期缓存均不满足此条件。
- 无模型证明时，CCSwitch 的用户提示要告诉用户下一步“先选择并完成 Codex 兼容测试”，而不是内部开发错误式原文；并明确当前 live 设置未改变。
- **`DESIGN_QUESTION`：** 当前 CCSwitch 守卫只看 provider TOML 的 `model` 字段，不能独立验证 Manager 的原生能力证据。开发前需冻结 Manager 与 CCSwitch 间复用的既有验证产物/接口，或定义经审查的新证据传递契约（绑定、模型 ID、部署/适配器版本、验证时间/TTL）。不得把“非空 model”悄悄升级为“已验证”，也不得为让按钮通过而取消原守卫。
- 应用必须先保留旧 live 配置快照；写入后回读确认 Provider/model/auth 指向预期值，失败则按现有事务能力恢复。未验证、用户取消或写入失败均不得报告成功。

### 5.3 验收

- 无默认模型/无证明：自动与手动应用都不变更 live 配置，并显示可操作说明。
- 用户选择但未完成原生验证：仍不得声称“已验证”或自动应用。
- 当前绑定下模型通过全部必需原生检查且用户确认：准确应用该模型并复核实际文件；不得应用列表中的其他模型。
- 过期证据、适配器版本变化、U 盘换绑、部分写入/进程失败：拒绝或恢复旧快照，状态清楚可诊断。

## 6. 模型发现、真实探测与前端状态

### 6.1 诊断与风险

此前截图里的 `发现 22 个 / Code Agent 已验证 0 个 / 已隔离 8 个` 实际混合了至少三种含义：

- **发现 22 个：** 网关模型目录可读取，不代表 22 个模型都兼容 Codex。
- **Code Agent 已验证 0 个：** 原生运行时验证要求专门的用户确认和部署版本信息；自动刷新没有提交这些参数，所以 0 表示“当前没有原生验证证据”，不能单独推出 Codex 主程序故障或网关断开。
- **隔离 8 个：** 网关协议探测返回非重试错误后被安全隔离；这说明合成兼容测试失败，不等同于完整真实会话矩阵已证明这些模型必然不可用。

更重要的是，`src/app/views/OfflineHome.tsx` 在进入已安装页面约 250 ms 后自动调用刷新，不带原生验证参数；但 `src-tauri/src/app/model_catalog.rs` 对发现目录中的每个唯一模型仍调用 `ProbeClient::probe_model`。该方法可能执行文本、工具调用/结果、多工具、工具错误、流式以及多轮续接；全部走通时最多约 12 次模型请求/模型，且按模型顺序消耗总预算。22 个模型的最坏情形可能达到约 264 次模型请求，实际会因失败/预算提前停止而更少。它们是合成请求，不应被误认成用户主动的原生 Code Agent 测试；是否计费取决于上游策略，但会消耗真实上游配额/产生费用的风险必须按真实调用对待。

附件累计日志 L5073–L5171 有多模型 `text/tool-call/tool-result/stream` 探测记录和 `stream-invalid`、`upstream-response`、`probe-budget-exhausted` 错误。这证明该日志中的运行确实执行过模型探测；由于日志没有批次 ID，不证明这些事件与截图 22/0/8 属于同一刷新批次。该日志也没有记录 CCSwitch 的精确 Apply 错误。

### 6.2 修复契约

1. 页面自动刷新默认只做轻量健康检查与模型目录读取；不得对全部模型发生成式/工具/流式测试请求。
2. 把“网关可达/目录发现”“网关协议兼容测试”“Codex 原生 Code Agent/Shell/Skills 验证”分成独立状态和独立操作。原生验证仍按既有 T0/部署版本/用户确认门禁运行。
3. 深度测试只对用户明确选择的模型启动；测试前说明会向上游发送合成请求、可能消耗配额/费用。默认限制为一个模型、可取消、请求和总时限有硬上限；批量测试另设明确选择与预算上限，不在页面挂载时触发。
4. 不因模型发现或网关探测结果自动选择默认模型；只有第 5 节规定的当前原生验证证据才允许进入应用流程。
5. 将状态卡压缩为单一摘要和按需展开详情。建议主文案：`网关已连接 · 发现 22 个模型 · 尚未进行 Codex 原生测试`；存在协议测试失败时另显示 `有 8 个模型的接口测试未通过`。提供“选择模型并测试”动作；默认折叠每个模型和十项检查详情。
6. 删除重复 banner/card 对同一计数和状态的重复输出。技术码、请求 ID、每项 Shell/Skills 检查放在诊断详情，不占常驻空间。
7. UI 区分 `未测试`、`测试中`、`测试通过`、`测试失败/需重试`、`证据过期`、`原生验证未运行`。不要把“网关可用”写成“模型已兼容”，也不要把“原生测试尚未运行”呈现成红色故障。
8. 为发现刷新、一次兼容测试和一次原生测试生成独立 `run_id`；记录模型 ID、测试阶段、安全错误码、适配器/部署版本、耗时和取消状态，不记录 API Key、Authorization、提示词、模型响应正文或工具参数。

### 6.3 验收

- 页面首次显示时用网络 mock/请求计数证明只执行目录级请求，不发模型 Responses/Chat completions 生成请求。
- 未勾选/未确认深度测试时，即使发现 22 个模型，生成请求数为 0。
- 明确选择一个模型测试时，只探测该模型；取消后不接受迟到结果，不启动后续模型；上限/超时可验证。
- 文案正确表达“发现不等于兼容”“原生验证未运行不是主程序故障”；失败原因可展开，摘要简短且无重复计数。
- 所有非通过/过期/不同绑定证据均不能触发 404token 默认模型应用。

## 7. 推荐实现顺序与写入边界

本文件是契约和派工依据；不同任务不得并发修改共享模型契约。

| 顺序/任务 | 唯一责任范围 | 主要文件/模块 |
|---|---|---|
| U1 卸载事务隔离 | 事务归属、卸载分层结果、延迟共享清理及卸载测试 | `src-tauri/src/app/install_tx.rs`、`src-tauri/src/commands.rs`、`src-tauri/src/app/uninstall_cleanup.rs`、对应卸载视图/测试 |
| C1 控制台窗口 | 管理器侧进程树窗口事件与安全兜底 | `crates/codex-win-engine/src/sys.rs`、`windows_process.rs`、Windows 定向测试；不改官方 Codex 二进制 |
| M1 发现/探测/文案 | 轻量自动发现、显式合成测试、调用预算和模型状态卡 | `src-tauri/src/app/model_catalog.rs`、`model_probe.rs`、`src/app/views/OfflineHome.tsx`、`NativeModelStatusCard.tsx`、`nativeModelStatus.ts`、i18n 与测试 |
| M2 404token 应用闭环 | 在 M1 的证据契约冻结后，接通选择、原生验证证据和 CCSwitch 安全应用 | `CCSwitch/CCSwitch/src-tauri/src/services/provider/live.rs` 及 Manager/CCSwitch 已批准的接入边界与集成测试 |
| I1 诊断关联 | 统一运行 ID 和脱敏状态记录，帮助复现上述任务 | `src-tauri/src/app/model_diagnostics.rs`、卸载结果序列化/日志及测试 |

M1 与 M2 共用“已验证默认模型”契约，必须先冻结接口再开发；不得两个执行者同时改同一模型状态或 provider schema。每项需先记录基线和工作区已有修改，提交固定补丁/版本后再独立审计。附件没有授权实际安装、卸载、部署、真实上游付费探测、签名或发布；这些仍须测试人员显式安排。

## 8. 总体验收门

只有以下全部通过，才可把此次综合修复交给 U 盘现场实测：

1. 卸载事务隔离单测、Windows 路径/进程集成测试通过；确认不会删除非目标恢复材料，且 USB 主卸载状态清晰。
2. 黑框现场冷启动验收通过；现有 Codex 功能、USB Guard、普通终端窗口均无回归。
3. 自动目录刷新不产生合成模型请求；显式测试的费用提示、模型范围、取消和请求上限有可复现证据。
4. 404token 无证据不写 live config；验证通过并经用户确认后只应用被验证选择，写后复核/失败恢复通过。
5. 前端摘要不重复、不将“0 个原生验证”误报为系统故障；详细错误仍可查看。
6. 重建 Manager 与 CCSwitch 最新发行件，核对发行清单/哈希；所有测试必须对应该固定版本，不能用旧二进制的绿灯放行。
7. 在测试机对事务 ID `3ac98906-b6ca-4f08-b356-4e81a45b50a7` 做定点、安全只读检查，确认引用路径、访问状态和实际所有权；不得先手工删除 journal 来制造卸载通过。
8. 用户数据清理必须在可恢复的专用测试账号/快照上验证。当前日志已经表明一次 `purge_user_data=true` 清除了用户数据；不得在主账号重复做破坏性验证。

**不满足任一项时状态保持 `NOT_ACCEPTED`。** 自动刷新发现模型、主程序删除成功、单元测试通过或静态审查 conforming，都不能单独代表本综合修复已完成现场验收。

## 9. 实施、复核与 USB 候选包（2026-09-26）

本节记录本轮源码实施和本机验收。它不是独立审计签字，也不替代测试机的真实模型/物理 U 盘验收。

### 9.1 已落地的实现

- **U1 卸载隔离：** 卸载前只收敛当前安装根的终态事务；外部事务只读核对，若未知或引用目标 USB 卷则在动产品前 fail-closed。主程序卸载完成后，若共享目录有其他事务阻塞最终清理，则保留共享用户数据并向 UI 返回 `cleanupDeferred`，不再把已卸载的 U 盘产品伪报成完全失败，也不启动危险的后台清理。
- **C1 黑框：** Codex 子进程创建前 arm 进程树范围的 WinEvent 创建/显示监听，监听按受管进程树及 `ConsoleWindowClass` 过滤；进程树存活期间持续监听，启动失败时采用预启动轮询兜底。没有全局隐藏窗口，也没有修改官方二进制。
- **M1 模型：** 常规刷新改为目录/元数据读取，不对所有模型发合成生成请求；显式原生测试必须明确选中一个目录模型，只对该模型测试。首页摘要压缩、模型和诊断细节默认折叠；`run_id` 进入脱敏诊断链路。
- **M2 404token：** CCSwitch 普通 Provider Apply/同步不能绕过验证直接改 Codex 实时配置；保留 fail-closed，要求通过 Manager 的已验证模型桥接路径。原生验证请求明确绑定所选模型 ID。
- **测试入口：** 根 Vitest 排除嵌套 CCSwitch 工程，ESLint 排除 `.codex-audit` 历史快照。CCSwitch 自己的完整 UI 测试仍单独运行。修正了一条已过时的 TheRouter 测试断言，使其符合“不从 Provider 预设隐式指定 Codex 模型”的现行安全契约；未恢复默认模型。

### 9.2 本机模拟/构建验收结果

| 验收 | 结果 |
|---|---|
| 根 `npm run check`、生产 `npm run build` | 通过；生产构建包含 renderer-entry isolation 检查 |
| 根 `npm run lint` | 通过（将审计快照排除后） |
| 根 `npm test -- --run` | 38 个套件通过；457 项通过、1 项跳过 |
| CCSwitch `pnpm run test:unit` | 38 个套件通过；227 项通过 |
| Manager Rust `cargo test --offline --manifest-path src-tauri/Cargo.toml --lib` | 515 项通过、0 失败、3 项既有 ignored |
| CCSwitch Rust 全量库测试 | 1,110 项通过、0 失败 |
| Windows 进程/窗口定向测试 | 通过，包含监听先于子进程启动的回归用例 |
| CCSwitch Release 构建 + `verify-product.ps1 -SkipNetwork` | 通过 |
| USB 成品 `verify-usb.ps1 -InternalSigned -SkipNetwork` | 通过；MSIX、CCSwitch、离线前置资源及 bundle 结构校验通过 |
| USB Launcher/Manager PE 权限清单 | 两者均通过 `requireAdministrator` 校验 |
| USB 与签名器配对预检 | 通过；`keyId`、公钥摘要、版本、releaseId、portable-vault 绑定匹配 |
| USB/签名器 `SHA256SUMS.txt` | 分别 290/290、7/7 个文件匹配 |
| `git diff --check` | 退出码 0；仅有 Git 的既有换行格式提示 |

`cargo fmt --check` 曾报告工作区内多处 Rust 格式差异，包含既有 T1–T6/工作区文件；本轮没有对整棵 crate 执行全量格式化，以免改写无关工作。Cargo 编译和测试均通过。根目录直接无边界地运行 Vitest/ESLint 的初次检查会把独立子工程或审计快照算入根项目；上述项目边界修复后，各工程正式入口均通过。

### 9.3 交付目录和固定哈希

- USB 根目录完整包（最终采用）：`release\usb-oneclick-repair-20260926-r2\`，总计约 **1.35 GB**；入口为 `CodexDesktopSetup.exe`。必须复制整包，不要只复制 EXE。
- 配套签名器：`release\usb-license-signer-gui-v0.5.24-g39.5-r23-codex917-consolefix\`。签名器源码本轮未变，因此复用既有签名器；它与新 USB 包 profile 完全匹配，配对脚本和 7 项哈希已验证。签名器及加密 vault 留在签名电脑，不要复制到 U 盘。
- USB Manager SHA-256：`4c442db1d32d01b51c49acd9dea65d6669484d3059f744c7905399e7caa8fce9`
- USB Launcher SHA-256：`a9ce2475d6f2912aeb00a604da68cfcd8d2ba0a1291a0c3bc8a0bf4f2903c464`
- CCSwitch SHA-256：`3767b8af4e5b9000d09205955b749d046d29d169d2cf675905dfb11b58770693`
- Codex MSIX SHA-256：`ccd6002255042589b6ee1e711818a6703509d4a288851bea455da5a8125f25d0`
- 签名器 SHA-256：`37bb2fc057d7b52a408cfb584d440a3030b2f83d7b4f5b574760c91536dce225`
- 包和签名器的 `releaseId` 均为 `g39.5-20260924-r23-codex917-consolefix`，策略为 `SignedRequired`。

旧 `release\setup` 已保留为 `release\setup-before-repair-20260926-132337`；打包前 CCSwitch 资源快照保留在 `release\ccswitch-resource-before-repair-20260926-132337`。本轮未覆盖旧 USB 发布目录。

首次生成的 `release\usb-oneclick-repair-20260926\` 已因随包说明仍沿用“CCSwitch 自动应用模型”的旧文案而标记为过时；用户手册和 USB manifest 工作流说明修正后，重新生成并复核的 `-r2` 才是可供测试的目录。不要把 r1 复制到 U 盘。

**注意：** 新 USB 目录是带 `SignedRequired` 策略和正确公钥绑定的干净发行包，尚未包含 `usb-license.json`。配对检查只验证签名器与包匹配，不代表已经注册账号、签发许可证或完成 404token 绑定。实际签名器操作可能创建账号/授权并可触发预充值；尚未执行。先在签名器界面确认金额为 0 并由操作者明确授权后再签名。不得把签名工具的加密 vault 放进 USB 包。

### 9.4 未完成验收与当前状态

- 未调用任何真实 GLM/DeepSeek/Claude/404token 模型；没有产生真实模型费用或上游用量。
- 未执行真实安装、卸载、清除用户数据、许可证签发/账号创建/预充值，也未做物理 U 盘冷启动录屏。尤其是“无可见黑框帧”仍需按第 4.3 节进行至少 20 次冷启动实测；事件监听测试通过不能代替视觉验收。
- 针对日志中的事务 ID `3ac98906-b6ca-4f08-b356-4e81a45b50a7`，只读检查当前账户已知的 Roaming/Local 事务目录，未找到该 ID 的 journal；未修改任何事务。故不能据此认定原测试机恢复记录已解决。
- 本会话没有可用的独立审计 Agent 路由；本轮属于作者自检与可复现测试，不宣称独立审计 `conforming`。

```yaml
SOURCE_IMPLEMENTATION: COMPLETE
LOCAL_SIMULATION_AND_BUILDS: PASS
USB_BUNDLE_STATIC_VERIFICATION: PASS
SIGNER_BUNDLE_MATCH: PASS
USB_LICENSE_ISSUED: NO
LIVE_MODEL_TESTS: NOT_RUN
PHYSICAL_USB_COLD_STARTS: NOT_RUN
INDEPENDENT_AUDIT: PENDING
USB_ACCEPTANCE: NOT_ACCEPTED_PENDING_OPERATOR_AND_FIELD_TESTS
```
