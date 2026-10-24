# Codex Browser 与跨会话工具适配诊断和修复（2026-10-24）

## 1. 目标与边界

诊断 USB Codex 中 Browser / `node_repl` 和跨会话管理工具不可用，并只修复本仓库能通过受支持的 Codex 配置明确证明的回归。

硬约束：

- 优先恢复 Codex 原生默认行为；不仿造 Codex Desktop/app-server，不新增私有会话 RPC、durable/workspace 代理或 owner 映射层。
- 不强制开启未由模型/Provider 声明支持的 multi-agent 协议。
- 不改媒体 MCP、其他 MCP、`config.toml` 合并/注册流程、CCSwitch、Product Design、上游 Codex 二进制或认证凭据。
- 保留仓库现有未提交改动，不重置、覆盖或清理无关文件。
- 只允许改本合同、`src-tauri/src/app/model_catalog.rs` 中新增模型目录项和紧邻测试。

风险分类：D0（用户要求诊断并修复，且此前明确倾向恢复 Codex 原生支持、少做自定义代码；本合同将其约束到有上游证据的最小范围），I2（Browser 注册、模型能力字段、host 工具暴露和远端认证是不同层的交互故障），A1（会话工具与 Browser 受跨模块/上游运行时控制，必须区分配置代理与真实能力）。执行模型/effort 与本次子 Agent 的实际路由未获得独立运行时元数据，记录为 `ROUTE_UNVERIFIED`；子 Agent 只可提供辅助检查，不能作为正式独立审计凭据。

## 2. 两份诊断报告的证据分级

### 2.1 Browser

用户提供的两份报告给出的运行时证据（本机未取得原始日志文件作二次校验）：

1. USB Codex 安装资源含官方 `openai-bundled/plugins/browser`；用户 USB `CODEX_HOME` 的插件缓存没有对应 Browser 插件。
2. Codex 启动日志记录 `bundled_plugins_reconcile_skipped_features_unavailable reason=focus`，随后记录 Browser 侧栏生命周期已注册。
3. 当前回合工具清单中没有 Browser / `node_repl` 控制工具。
4. 本仓库 `conservative_tested_model_entry` 曾显式写入 `node_repl_disabled = true`。

第 4 项是本仓库可直接修复的额外限制。USB 包固定测试 Codex 版本为 0.157.1；对应上游 `ModelInfo.node_repl_disabled` 有 serde 默认值 `false`。Codex 将该值作为执行元数据传入每轮请求。因此 USB 可选模型模板不应无依据地显式关闭该原生工具。删除该字段后由 Codex 原生缺省处理。

`model_native_probe.rs` 中另一个隔离探测用临时模型仍显式关闭 `node_repl`。它只用于探测测试，不会写入用户活动模型目录；本次保留该隔离边界，不把它和已添加模型混为一谈。

第 1–3 项发生在 Codex Desktop 的 bundled plugin reconcile、插件目录及当前回合工具注入阶段。仓库包含 Codex MSIX 的提取/启动逻辑，但没有 Codex Desktop 的 reconcile 实现。报告中的 `reason=focus` 只能证明该次 reconcile 被跳过，不能证明是 Windows 窗口焦点丢失，也不能证明本地管理器应绕过这个 gate。不得仅据此手动复制缓存、写私有插件表或伪造 node_repl server 配置。辅助审阅复核了上游字段缺省值，但不能验证这些用户提供的运行日志或实际 USB profile。

故 Browser 当前判定：**本仓库关闭 `node_repl` 的回归可修复；USB profile 的官方插件注册/工具注入仍需由 Codex Desktop 或其受支持的插件安装流程确认。不能把字段修复宣传为完整端到端修复。**

### 2.2 跨会话管理与委派

报告区分了本地线程管理和远端路由：

- `thread/start`、`thread/list` 本地 app-server 调用成功，只证明本地线程存储/协议可用。
- 日志中的 durable thread 登录要求、workspace routing 不可用及 HTTP 432，证明当前 `404token` 自定义 Provider 会话没有取得对应 ChatGPT durable/workspace 服务认证。不能通过 Codex 本地提示词或打开普通 MCP 端口获得该认证。
- 当前模型上下文没有 `list_threads`、`send_message_to_thread`、`handoff_thread` 等 host 工具，说明 Codex/宿主没有把这些工具交给该模型回合。提示词只能约束已存在能力，不能新增 callable tools。
- 新模型目录未声明 `multi_agent_version`，所以不得假定这个模型获准使用 Codex 的 multi-agent 协议。Codex 原生 multi-agent 工具生成和线程路由由 Codex 核心负责；为所有网关模型强行写 `v1` 会把未验证的能力和协议要求伪装成已支持。
- `client-new-thread` 与 canonical ID 重绑、`unknown conversation` 是值得调查的关联线索，但报告未证明它们造成 Browser 缺工具或 durable/workspace 认证失败。本次不据此改 ID 路由。

跨会话当前判定：**本项目不能修复缺失的 Codex host 工具注入或 ChatGPT durable/workspace 认证；本次不得用自制代理替代。** 若用户期望跨会话操作，应先在受支持认证下验证原生工具暴露情况，再对照自定义 Provider 的具体差异；若仅 remote durable/workspace 失败，则必须由登录/Provider 产品能力解决。

## 3. 允许实施的最小修复

在新增的自定义模型目录项中删除显式的 `node_repl_disabled: true`。Codex 上游结构将其默认为 `false`，因此省略字段比复制一个人工开关更符合原生缺省。

更新紧邻的条目合同测试，断言新条目不序列化 `node_repl_disabled`，并通过当前 Codex `ModelInfo` 的默认语义确认省略该字段表示未禁用。若仓库没有可直接复用的上游模型结构，则测试 JSON 字段缺省，不新增仿真配置层。

此改动不保证第三方模型本身能正确处理 browser/node_repl 的工具调用；也不安装 Browser 插件，不变更其注册或启用状态。端到端成功必须由 USB 测试机在更新后的 Codex 与当前模型上确认。

## 4. 禁止实施

- 不改 `multi_agent_version`、认证、ChatGPT 登录、durable/workspace 路由或模型工具白名单。
- 不添加跨会话代理、线程 ID 映射/重绑代码、消息重放、delivery receipt 或自动重试循环。
- 不从本机普通 `.codex` 用户目录复制 Browser 插件到 USB profile；不手写 `plugins/cache`、marketplace 或 `mcp_servers.node_repl`。
- 不改 Windows 窗口焦点、启动顺序或 Codex 二进制来绕过 `features_unavailable` gate。
- 不改媒体 MCP 配置与启动，不修改 Product Design 插件/技能。
- 不声称本地静态测试证明 Browser 插件已注册或跨会话端到端可用。

## 5. 验收

1. 新模型条目没有显式 `node_repl_disabled=true`；其它 reasoning、search、plugin、app、MCP 字段保持原值。
2. 聚焦的 Rust 测试通过，证明字段缺省；既有模型目录相关测试通过。
3. `git diff --check` 通过；变更仅在本合同与允许的 Rust 实现/测试范围内。
4. 独立只读审计（若可用且路由有可核验来源）返回 PASS；否则明确记为未独立通过。
5. USB 真实测试单独记录：Browser 插件已进入 USB `CODEX_HOME`、`node_repl` server 已注册且当回合收到 Browser 工具，才可报 Browser 端到端通过；自定义 Provider 下跨会话工具须实测工具清单与一次 A→B 的接收回执。当前报告不足以满足这些运行时验收。

## 6. 权威技术材料

- USB 包固定版本 0.157.1 的 Codex 上游 `ModelInfo` 定义与默认值：<https://github.com/openai/codex/blob/rust-v0.157.1/codex-rs/protocol/src/openai_models.rs#L3043-L3049>
- Codex 上游 multi-agent 工具规格：<https://github.com/openai/codex/blob/rust-v0.157.1/codex-rs/core/src/tools/handlers/multi_agents_spec.rs>
- OpenAI 插件安装/本地 marketplace 文档：<https://developers.openai.com/plugins/build/plugins>

注：Agents API 的 multi-agent 文档针对 Agents API harness，不能单独证明 Codex Desktop 行为；因此不作为 Desktop 工具暴露结论的依据。

## 7. 审计、实施与验证记录

辅助只读审阅：`INSUFFICIENT_EVIDENCE`。审阅确认 0.157.1 上游字段默认值支持本次变更；同时指出报告原始日志未在当前仓库，且该字段变更不证明 Browser 注册或线程管理端到端可用。子 Agent 路由来源未能独立核验，所以不作为正式独立审计通过。

实现：仅从 `conservative_tested_model_entry` 删除 `node_repl_disabled: true`，其相邻测试断言新条目不序列化该字段。未改变 probe-only 模型、思考档位、其它模型能力、`config.toml`、MCP、插件注册、桌面焦点或线程映射。

本机测试：`cargo test --manifest-path src-tauri/Cargo.toml --lib app::model_catalog::tests`，在 `TEMP`/`TMP` 指向仓库隔离临时目录后 **73 passed / 0 failed**。首次默认临时目录运行有 25 个 Windows 文件写入失败；重定向后全套通过。`git diff --check` 通过。整文件 `rustfmt --check --edition 2024` 被当前已存在的多个区域格式差异挡住；这些差异包含本任务之前的工作区修改，未运行格式化以免扩大差异。没有运行 Codex Desktop USB profile 的真实 Browser 测试，也没有向跨会话目标发送 A→B 测试。

结论：模型条目对 Node REPL 的显式封锁已修；Browser bundled plugin 注册/工具注入与跨会话 host 工具仍未闭环。本变更不得作为这两个功能已端到端恢复的证据。正式独立代码审计及测试机验收仍待具备可核验路由/运行时资料后完成。
