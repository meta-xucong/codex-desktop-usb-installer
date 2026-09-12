# Skill Hub、模型同步与卸载二阶段修复开发文档

## 1. 目标与范围

本次变更同时处理三个相互独立、需要统一验收的回归面：

1. Skill Hub 热更新失败时，用户必须能看到可操作、可复盘的原因；超时不能吞掉后端错误，也不能泄露 URL、路径、凭据或签名材料。
2. 安装包要带有经过筛选的上游模型目录和默认模型策略，使新装环境默认使用当前支持的模型（当前基线为 `gpt-6-astra`），并能识别、淘汰已停用模型；不能把用户的原始 `models_cache.json`、隐藏提示词、账号信息或 API Key 放进公开安装包。
3. 卸载二阶段的报告、审计和收尾动作不能写入即将被删除的 Codex App Manager 数据目录。失败时必须给出具体文件/原因；卸载后打开日志不能重新创建已删除的产品目录。U 盘发行目录始终只读，卸载只清理目标电脑本地内容。

## 2. 冻结的行为契约

### 2.1 Skill Hub

- 后端仍使用现有 relay 优先、GitHub 回退链路；每个来源尝试有连接/请求/总截止时间。
- 所有来源尝试写入 App Manager 日志，至少包含阶段、来源类别、结果类别、耗时和重试序号；不记录完整 URL、Authorization、Cookie、令牌或本地用户路径。
- 失败结果保留 `code`、用户可读 `message` 和经过脱敏、截断的 `detail`。前端默认只显示简短错误；展开详情才显示脱敏细节，并提供打开日志/复制诊断的入口。
- 外层 UI 超时必须与后端截止时间协调；超时后的错误仍然可追溯，不能显示“重试并展开详情”而没有详情入口。
- 没有凭据或未勾选可选 MCP 时，安装/更新能力包应当成功完成基础包安装，并将可选服务标记为未配置，不得把可选项缺失误报成核心安装失败。

### 2.2 模型同步

- 发行包只包含公开、脱敏的 `model-catalog.json`：模型 ID、显示名、可见性、API 可用性、推理档位、优先级、快照版本和停用 ID；不包含 `model_messages`、用户配置、凭据或服务端隐藏指令。
- 安装器把目录写入包自有的 `.codex/codex-agent-kit/model-catalog.json`，并将默认模型策略写入受管理配置块。缺少模型配置或现有默认值是已停用 ID 时，默认设置为 `gpt-6-astra`。
- 保留用户选择的仍在目录中的活动模型；不静默覆盖有效的用户选择。停用模型只在有明确替代项时回退，并在 readiness 中记录迁移结果。
- 目录是离线快照，不冒充“永远与上游实时一致”。联网时 Codex 仍可按自身协议刷新官方模型缓存；下一次发行通过更新快照追随上游。
- 官方模型目录的权威来源是当前 Codex 二进制内置快照和账户范围的官方在线目录；CCSwitch 的 `providers.json`/第三方 `/models` 只代表第三方供应商，不能证明官方模型可用。安装器不伪造或删除 Codex 的服务缓存。
- 发布门禁必须从最终 MSIX 取证 `codex debug models --bundled`（或等价官方取证入口），记录内置目录版本、SHA-256、默认模型和停用模型；如果官方签名载荷仍带有可见的已停用模型，只能报告“默认已迁移、上游目录待刷新”，不能声称已从官方目录移除。
- 任何模型策略更新都必须可重复执行、幂等，并且在安装失败时不破坏原有 `config.toml`。

本切片的模型同步器还固定以下可执行契约：

- `model-catalog.json` 的模型条目只能包含 `id`、`displayName`、`visibility`、`supportedInApi`、`priority`、`defaultReasoningLevel` 和 `reasoningLevels`；字符串、`list|hide`、布尔值、整数、非空推理档位数组及默认档位归属均严格校验。`preferredModel` 必须指向 `visibility=list` 且 `supportedInApi=true` 的条目；`retiredModelIds` 必须存在、为唯一的非空字符串列表，允许保留不再出现在当前活动模型列表中的历史 ID。
- 配置中的官方已知模型若为 `hide` 或 `supportedInApi=false`，同步时迁移到 `preferredModel`；不在目录中的第三方模型保持原样。重复执行不得重复写入或重复创建迁移结果。
- `sync-model-catalog.ps1` 使用 `CmdletBinding(SupportsShouldProcess)`。`-WhatIf` 只读取并生成 `status=planned` 与 `planned` 动作清单，不写目录、配置、备份或临时 TOML 文件。

### 2.3 卸载二阶段

- 最终报告、收尾日志、失败清单统一写入独立临时目录（`%TEMP%\CodexAppManager\uninstall-reports`），该目录不得是 `io.github.wangnov.codexappmanager` 或其它被清理根目录的子目录。
- 删除管理器数据目录前先捕获日志尾部；删除后 finalizer 只能读写独立临时目录，不得尝试回写已删除产品根目录。
- 二阶段失败报告必须列出阶段、退出码、失败路径和可重试建议；不再只显示“未完成”。
- “打开日志”在产品日志目录不存在时转到独立临时报告目录，不能为了打开日志重新创建已卸载的产品数据目录。
- 不删除 U 盘发行目录、签名文件、构建源文件或用户未授权的其它目录。

## 3. 实施拆分

### A. Skill Hub 可诊断性

- `src-tauri/src/app/skill_repo_update.rs`：为来源尝试、HTTP 状态、relay 回退和摘要增加结构化日志；统一外层截止时间和错误码。
- `src-tauri/src/errors.rs` / `src/shared/types.ts`：补充安全 detail 的传输类型。
- `src/app/errorCopy.ts` / `src/app/views/SkillRepoUpdateCard.tsx`：保留并展示脱敏 detail，添加详情折叠和日志/诊断入口；补齐 stale、冲突、账本和回滚错误文案。
- 测试覆盖 relay 成功、relay 失败后 GitHub 回退、全源不可达、超时、脱敏和 UI 详情渲染。

### B. 模型目录与默认策略

- 新增 `installer-pack/model-catalog.json` 和幂等同步脚本。
- 将同步步骤接入 `install-to-codex.ps1` / `install-all.ps1`，输出 readiness 的模型同步状态。
- 对 JSON schema、停用模型迁移、用户活动模型保留、重复执行和路径安全增加测试。
- 构建时由模型目录生成清单和校验值；禁止把本机原始模型缓存复制进仓库或安装包。

### C. 卸载安全收尾

- 将 report root 固定到独立临时目录，并让日志打开命令使用存在性检查和 fallback。
- 增加报告路径不在删除根目录下的单元测试、finalizer 失败信息测试和日志 fallback 测试。
- 更新卸载说明，明确“本地数据清理”和“U 盘发行目录保留”的边界。

## 4. 审计与测试门槛

提交前必须通过：

- 前端 Skill Hub 定向单测、错误文案单测、TypeScript/ESLint。
- Rust `cargo test`（skill repo、uninstall cleanup、命令层）和 `cargo fmt --check`。
- 安装包兼容性审计、manifest/checksum 审计、模型目录脱敏审计。
- PowerShell 安装脚本的 `-WhatIf`/隔离目录测试：空配置、旧模型、活动模型、重复安装、无网络。
- 模型同步定向验证命令：`& "$env:USERPROFILE\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe" installer-pack/test-adapter-contracts.py ModelCatalogSyncContractTests -v`；至少覆盖严格字段失败、WhatIf 无写入、官方隐藏/API 不支持模型迁移、第三方模型保留和幂等重复执行。
- G22 卸载回归：删除管理器数据目录后仍能写报告并展示具体失败项；报告路径不回到 `io.github.wangnov.codexappmanager`；产品目录不被“打开日志”重新创建。
- 发布前在干净隔离目录构建 EXE、生成 manifest/checksum，并用同一发行目录生成本地 USB 签名产物；签名前后验证 bundle hash 一致。

## 5. 已知边界

模型目录和 Skill Hub 快照不能代替 Codex 官方服务端授权、账号登录或实时模型接口。网络被阻断时，安装器可以保证默认配置和可审计的离线快照，但不能保证未缓存的模型调用成功。所有不可用情况必须在 readiness、日志和 UI 中明确区分“安装成功但服务未配置/网络不可用”和“核心安装失败”。

## 6. G22 补充现场与验收结论

补充报告 [`codex-v0524-uninstall-stage2-selfref-g22.md`](reports/codex-v0524-uninstall-stage2-selfref-g22.md)
记录了旧发行版的具体故障：主进程清理了
`%LOCALAPPDATA%\io.github.wangnov.codexappmanager` 后，二阶段收尾仍试图在该目录写最终报告，Windows 因目录不存在弹出“位置不可用”，并返回 `E_FAIL`。本机 Codex、CCSwitch 和能力包清理实际上已经完成，失败发生在报告回写阶段。

修复实现把报告根目录固定到 `%TEMP%\CodexAppManager\uninstall-reports`，在提交二阶段清理前捕获日志尾部，并在报告中明确写入 `phase`、`exitCode`、`retrySuggestion` 和失败条目。`open_logs_dir` 在产品日志目录不存在时只打开该独立目录，不会重新创建已卸载的管理器数据目录。报告根目录还会在启动二阶段前与全部本地清理目标做规范化路径交叉检查；U 盘目录不参与这个清理集合。

G22 的验收必须同时检查：报告最终仍可读取、收尾失败包含具体文件和建议、旧版桌面报告可清理、打开日志不会复活产品目录，以及静态发行包不含 `usb-license.json`。后一文件只能由签名工具在用户复制到目标 U 盘后按当前发行包重新生成。
