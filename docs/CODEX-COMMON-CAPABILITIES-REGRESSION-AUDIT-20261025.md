# U 盘版 Codex 基础能力回归审计与修复契约（2026-10-25）

## 目标与范围

目标是恢复 U 盘版 Codex 的基础能力一致性：工具是否由 Codex 注册并注入、模型目录是否无依据地收窄原生能力、U 盘运行环境是否能被 Codex 子进程正确解析。单个 skill、HyperFrames 和孤立测试用例不属于修复目标。

保留 Codex 官方 Desktop/app-server 的能力注册、插件对账、MCP 和线程路由。只修改仓库能证明属于 U 盘启动器的缺陷；不造私有 Browser/MCP 服务，不仿造 Codex host 工具，也不改授权或 durable/workspace 认证。

## 证据与症结

### 1. U 盘启动没有把已安装运行时放到 Codex 的命令搜索路径

`install-to-codex.ps1` 会在 `$CODEX_HOME/codex-agent-kit/bin` 创建 `python.cmd`、`pip.cmd`、`node.cmd` 等受管理的入口。审计前，Windows Codex 启动路径只注入媒体 MCP 环境变量，没有把该目录置于子进程 `PATH` 前面。因此 Codex 及其 shell 子进程可以解析到系统 Python（报告中为 3.11.5），而不是套件 Python 3.12.10；实际结果取决于机器全局 PATH。报告还复现了 CP936 下 stdout 和默认文本文件编码与 UTF-8 捕获/读取不一致。

**修复**：仅对 U 盘启动的 Codex 子进程，将 `$CODEX_HOME/codex-agent-kit/bin` 前置到继承的 PATH，并设置 `PYTHONUTF8=1`、`PYTHONIOENCODING=utf-8`。不改用户级/系统级环境变量；普通 Codex 启动不变。Python 写文件仍应显式 `encoding='utf-8'`，该环境变量不能替代代码的明确编码。

### 2. Browser 缺失属于 Codex Desktop 插件注册/会话注入链路

测试报告称安装包带有官方 Browser 插件资源，但当前 USB profile 的插件缓存没有 Browser；Codex 日志出现 `bundled_plugins_reconcile_skipped_features_unavailable reason=focus`，当回合也没有 Browser / `node_repl` 工具。仓库没有 Codex Desktop 的 bundled-plugin reconcile 实现。`reason=focus` 只能证明那次对账被跳过，不能推出窗口焦点损坏，也不能授权 U 盘管理器绕过 Codex 的 gate。

**处理边界**：保持官方插件资源和 Codex 启动方式；不手工复制插件缓存、不伪造 `node_repl` MCP、不篡改 Desktop 内部功能开关。当前证据不能证明 Browser 已修好，必须用相同 USB profile 和新会话确认插件缓存、工具注入与实际调用。

### 3. 模型元数据要保留原生缺省，不能把“已探测”当成“已验证全部工具”

- 本项目新增模型模板显式写 `supports_search_tool=false`。Codex 0.157.1 将它作为 MCP 延迟暴露/`tool_search` 能力条件；关闭时 MCP 保持直接暴露路径。`web_search_tool_type` 是另一个独立字段。对 404token 等第三方 Responses provider，不能为了“多开工具”直接将它改成 `true`：这会让 MCP 工具进入延迟暴露路径，而该 endpoint 的 `tool_search` 协议兼容性尚未被验证，存在让工具更不可见或请求被拒的风险。
- `node_repl_disabled` 在 Codex 0.157.1 默认 `false`；新模型条目省略字段可保留此默认。不得在无依据时写 `true`。
- `tool_mode` 省略时由 Codex 本地 feature/config 解析；不要给所有网关模型强行指定 `direct` 或 `code_mode_only`。工具模式会改变 shell、MCP 和延迟工具的暴露形态。
- `multi_agent_version` 与线程历史/配置共同决定 Codex 原生协作工具。不能把本地 `thread/list` 成功、模型元数据存在或提示词说明当作跨会话消息投递成功；也不能为所有第三方模型强制写 `v1/v2`。
- 当前模型模板已打开 skills/plugin/apps 的使用说明；说明文本只教模型使用宿主已提供的能力，不会创建缺失的 callable tool。

### 4. 跨会话失败须拆分本地线程、Codex 协作和 ChatGPT durable/workspace

报告称本地 `thread/start` / `thread/list` 成功，但当前工具表无跨会话消息/hand-off host 工具；durable/workspace 请求需要 ChatGPT 登录，`404token` 会话相关请求返回 HTTP 432。前两者是本地 Codex/app-server 与当回合工具注入问题，后者是产品认证/服务路由边界，不能由模型提示词或 USB MCP 注册代替。

**处理边界**：本次不添加自制线程代理、ID 重绑定、消息重放、delivery receipt 或云端认证绕行。把“本地线程接口可用”“Codex 原生 sub-agent 可用”“ChatGPT durable/workspace 路由可用”分开验收。

### 5. 本次不改的运行时/环境项

报告中的 Git 缺失可能影响依赖 Git 的可选工作流，但证据不足以说明它造成当前 Browser/跨会话工具表缺失；不为此把 Git 打包成新依赖。WSL 无发行版不是 Codex 基础工具注入故障。任何 Python 项目文件仍须显式使用 UTF-8。

## D/I/A 风险合同

- **D0**：目标为恢复已有基础行为；改动仅影响 U 盘启动的子进程环境，不写系统/用户环境，不触碰 Codex 配置、模型注册、MCP 注册、插件缓存或认证。
- **I1**：运行时 PATH/编码影响 Codex 与其子进程；范围收敛到 U 盘启动入口，并用纯函数测试确认顺序和变量。
- **A1**：Browser、跨会话 host tools 由 Codex Desktop/app-server 控制；本仓库静态检查不能宣称端到端修复。需 USB 测试机做真实调用验收。

## 实施与审计边界

1. 仅在 Windows USB Codex 启动时，把 `CODEX_HOME/codex-agent-kit/bin` 前置到当前 PATH，并给 Python 子进程提供 UTF-8 环境变量。
2. 保持现有模型模板中的 `supports_search_tool=false`、未显式禁用 `node_repl`、未强制 tool mode / multi-agent version 的策略；本次不改 MCP 注册和配置合并链。
3. 更新本开发文档，写清可控修复与上游边界。
4. 代码审计确认非 USB 启动的环境变量不变、MCP 媒体凭据环境可以继续合并、无持久 PATH 写入。

## 验收标准

- 本地单测证明 USB runtime bin 是 PATH 第一项、原 PATH 的其余项完整保留，Python 两个 UTF-8 环境变量正确。
- `cargo test` 的本次聚焦测试通过；`git diff --check` 通过。
- 检查差异确认没有改 `config.toml` 合并、MCP、Browser 插件目录、线程 ID、认证和 HyperFrames。
- 测试机另行确认：默认 `python` 解析到套件 Python，中文 stdout 与 UTF-8 文件可读；新 Codex 会话中 Browser/`node_repl` 实际注入；按本地线程、Codex sub-agent、durable/workspace 三类分别验证跨会话能力。
- 在测试机验收前，Browser、跨会话和各模型完整工具集状态标记为“未证实”，不可用本地单测冒充通过。

## 技术依据

- Codex 0.157.1 `ModelInfo` 字段缺省值：<https://raw.githubusercontent.com/openai/codex/rust-v0.157.1/codex-rs/protocol/src/openai_models.rs>
- Codex 0.157.1 MCP 工具暴露与 search/tool_search 条件：<https://raw.githubusercontent.com/openai/codex/rust-v0.157.1/codex-rs/core/src/tools/spec_plan.rs>
- 本机实测日志与 profile 检查来自用户提供的 `Codex内置浏览器不可用_诊断与修复建议(2).md`；当前会话未直接读取该机原始 Codex 日志和 USB profile，相关运行时结论仍需测试机复核。

## 实施记录

- 将 `usb_runtime_environment` 仅接入 USB Codex 启动选项，保留现有媒体 MCP 环境变量合并。
- 新增启动环境单测；运行结果和审计结论在本节补录。

### 旧模型目录 Node REPL 禁用标记迁移

测试机原始报告指出，旧版管理器写入的 `claude-sonnet-5-5` 条目显式含有 `node_repl_disabled=true`。此前仅修正了新条目模板；已存在的管理器目录仍会携带旧字段，因此旧模型切换后仍可能被 Codex 按元数据禁用 Node REPL。

修复限制在 `normalize_legacy_model_entries` 已有的精确旧模板签名内：描述、基础指令、`input_modalities=["text"]` 必须都匹配旧版生成值，且不得包含 `model_messages`。只有命中此签名时才删除 `node_repl_disabled=true`，并沿用既有迁移清除旧 text-only 标记；签名不匹配的条目和显式 `false` 值保留。此迁移不写 Codex `config.toml`，不改 Browser 插件缓存、MCP 注册、线程路由或认证。

回归测试应验证：命中旧签名后删除 `node_repl_disabled=true`、新目录再次规范化时保持幂等；任何一个签名字段被用户更改时，整条旧标记保持不动。Node REPL 真正进入当前会话仍依赖 Codex Desktop 的 Browser 插件注册和工具注入，模型目录迁移不构成端到端 Browser 修复证明。

审计限制：旧目录格式没有字段来源记录。如果用户在一条仍保留完整旧模板签名的模型上主动设置 `node_repl_disabled=true`，迁移无法区分该选择与旧版管理器写入的默认值，会清除此字段。为修复已确认的旧版 Sonnet 禁用问题，本次按旧模板签名迁移；不能声称覆盖此类人工定制时无影响。新目录不再写入该禁用字段。

### 本轮验证结果（2026-10-25）

- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`：790 通过、0 失败、6 忽略；首次并行运行有一个 Windows 临时文件锁失败，独立重跑通过，随后完整重跑全部通过。
- 两个旧模型迁移针对性 Rust 测试通过；包含模型目录归一化、MCP 配置保留的相关 Rust 覆盖也包含在上述全量库测试中。
- `npm run build`、`npm run lint`：通过。
- `npm test -- --reporter=dot`：39 个测试文件通过、2 个失败；480 项中 450 通过、29 失败、1 跳过。将 UI 测试单线程重跑后，`OfflineHome` 49 项通过，`Uninstall` 仍有 27 项失败，涉及当前卸载页文案/多路径展示与测试断言不一致。该失败未由本轮模型目录改动引入，但全量前端测试不能视为通过。
- `installer-pack/test-adapter-contracts.py`：58 项通过；`tools/test-installer-chain-window-static.py`：10 项通过；`docs/audit-capability-pack.py installer-pack`：无结构、清单、校验和差异。
- 只读独立审计确认补丁只改模型目录迁移和本文档，没有改写 `config.toml`、MCP 注册、Browser 缓存或会话路由；但上述旧目录用户定制歧义仍是已知边界。
- 本机没有 USB 测试机运行的 Codex Desktop 会话和设备工具桥接，不能做真实 Browser/Node 工具注入、跨会话调用或媒体生成端到端验证。必须在测试机验证新旧模型切换、MCP 生图、Browser 和跨会话工具是否实际注入。

### 代码审计补充：图像输入默认值

Codex 0.157.1 对缺省 `input_modalities` 的兼容默认值是 `text + image`。本项目的新网关模型模板没有序列化该字段，所以模型目录并没有像部分测试报告所述那样明确标记成 text-only；它沿用 Codex 的旧目录兼容默认。当前没有测试证据证明这一定造成问题，也没有证据证明所有 404token 模型都不接受图片。本轮保留原生缺省，避免仅凭未测试而全局关闭用户可能可用的图像能力；测试机应确认 UI 允许图片时，上游是否真正能理解。

## 本机实施和验证记录

- USB 启动环境：只在 USB `launch_codex` 分支前置 `CODEX_HOME/codex-agent-kit/bin`，并设置 `PYTHONUTF8=1`、`PYTHONIOENCODING=utf-8`；现有媒体 MCP 运行时环境继续追加到同一子进程环境。非 USB 分支仍传入空环境覆盖列表。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::win_update::tests::usb_runtime_environment_prefers_managed_bin_and_enables_utf8_python`：通过，1/1。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::win_update::tests`：通过，52/52。首次默认临时目录运行时有一项现有测试遭遇 Windows `PermissionDenied`；将 `TEMP`、`TMP` 指向仓库内临时目录后整组通过。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::model_catalog::tests`：通过，73/73，其中包含模型添加保留 media MCP 的回归断言。
- `git diff --check`：通过。
- 整文件 `rustfmt --check` 会报告 `win_update.rs` 中大量既存差异（含本次任务之前的 USB 卸载改动）；未对整文件执行格式化，以免改写无关工作区内容。
- 独立只读审计：代码边界 PASS；指出的两处文档描述已收窄。当前执行环境的 `LOCALAPPDATA` 是 Codex sandbox 私有路径；没有实际 USB profile 或受管 Python runtime，因此无法在本机做真实 Codex 启动/浏览器端到端验收。需在测试机确认默认 `python`、中文 stdout/文件读回、Browser 工具注入，以及三类会话能力。

审计结论：**代码安全审计通过；USB runtime 环境变量的注入逻辑与顺序已由单测验证。** PATH 注入为 U 盘子进程优先解析受管运行时提供了条件，但真实 `python --version` 解析和 Python 编码表现仍待 USB 测试机确认。本次没有证明 Browser 插件、跨会话 host tools、ChatGPT durable/workspace 路由或模型图像理解已恢复。HyperFrames 未纳入本次实现。

