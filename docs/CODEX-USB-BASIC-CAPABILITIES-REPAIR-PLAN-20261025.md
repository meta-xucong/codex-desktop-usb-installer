# U 盘版 Codex 基础能力修复开发规范（2026-10-25）

**实施状态：**本轮工作树已最小移除一条“优先本地顺序执行”的模型模板措辞，并把 `codex-code-mode-host.exe` 排除在 GUI 子系统改写名单外；对应定向测试通过。完整 Rust 测试在沙箱内受文件系统权限阻断，前端全量测试在获批的临时 fixture 环境通过。以上源码候选尚未打包、无 USB 实机证据；App Tools launcher 因缺同版原包对照保持不变。章节和测试证据见[证据台账](CODEX-USB-BASIC-CAPABILITIES-EVIDENCE-LEDGER-20261025.md)、[回归矩阵](CODEX-USB-BASIC-CAPABILITIES-REGRESSION-MATRIX-20261025.md)和[测试机验收手册](CODEX-USB-BASIC-CAPABILITIES-TEST-MACHINE-20261025.md)。

**当前授权与范围：**用户已授权本轮窄幅源码实施、独立审计、自动化测试，以及只向专用 GitHub 任务分支提交/推送本轮相关内容。本授权不包含实际打包发布、覆盖已安装/USB 程序、真实安装/卸载、修改用户真实配置、登录/读取凭证、对外发消息或可能计费的生成；这些仍须单独授权。执行遵循最小变更原则，不增加功能、额外校验层、通用框架或 UI 开关。

## 目标与判定原则

目标是定位并修复测试机报告的 Codex 基础能力问题，降低修复引入相邻回归的风险；不能承诺零新 Bug，也不把范围无限扩大。只有仓库可控且证据支持的缺陷进入代码修复。上游服务、宿主、第三方 provider 或产品未承诺能力要标清边界，不能用本地模拟伪装为已修复。

每一项使用统一状态：

- `PASS`：本项预期有对应证据支持；注明证据层级和适用的版本、provider/model、profile。
- `FAIL`：观察到违反预期的可复现结果；附原始日志/截图/请求或测试结果，不能只写推断。
- `NOT_RUN`：尚未执行，或执行证据不全。
- `ENV_BLOCKED`：已尝试但被权限、缺包、网络、设备或产品限制阻断；记录动作、目标、错误和阻断范围。

每项另标证据类别：`CODE_CONFIRMED`（静态调用链/契约已证明）、`A_B_REQUIRED`（可能根因，需单变量对照）、`EXTERNAL_BOUNDARY`（本仓不可控）或 `USER_POLICY`（用户已确定的产品策略）。静态确认的窄代码缺陷可以先修并补自动化，不必等待无关测试机能力；某条路径 `NOT_RUN`/`ENV_BLOCKED` 只阻断该路径的运行时/发布结论，不阻断有独立证据的代码工作。A/B 假设在得到区分证据前不得驱动大范围改动。当前规范不把所有问题判为 `CODE_CONFIRMED`；实施者先基于候选源码重新分类。

“代码结构正确”“自动化通过”“Windows 测试机通过”“目标 USB 介质通过”是不同证据层级。不能把前者替代后者。

## 本轮最小实施范围与停止条件

1. **优先目标：**只处理本机源码可定位的两类候选问题：模型管理器基础指令中明确抑制委派的管理器文本；App Tools launcher/PE 改写中经同版本机包和离线行为对照能证明的语义缺陷。模型修复优先保留用户自定义内容与旧 Node 匹配；只有目标 schema 接受且同版 fallback 已证实时，才考虑省略 `base_instructions`。
2. **不重复开发：**当前工作树已有的 Node 迁移、统一 Python/UTF-8 环境、CDP 主题目录预检、两步卸载并保留 USB 契约只做受影响回归确认，不重写已完成路径。
3. **条件性范围：**Browser、Search、Kimi/视觉、跨会话/ID、Skill Hub 和自动标题只定位直接本仓调用链。没有可复现的本仓缺陷就不改代码，不新增权限、重试框架、观测平台或宿主机制。
4. **最小验证：**每个实际改动先跑关联定向测试；完成后执行一次适用的 Rust workspace 和前端检查/测试，并按需要使用仓库已有 release 测试脚本。只修本轮新增失败；不删测试，不运行会发布/上传的 release workflow。
5. **停止条件：**证据确认的优先缺陷已修、相关定向及适用最终回归通过，独立 diff 审计未发现非目标改动后即停止。无法取得同版或真实运行环境时保留最小改动并标明待实机，不扩展猜测性修复。

## 章节导航与状态

| 章 | 范围 | 状态 | 主要入口 |
|---|---|---|---|
| 0 | 基线、来源、profile、授权边界 | 待执行 | Git 状态、包 manifest、签名与哈希记录 |
| 1 | 故障分层与定位顺序 | 待执行 | 启动日志、MCP/app-server、回合工具表、请求与结果 |
| 2 | 模型基础指令和 Node 默认迁移 | 已移除一条管理器顺序执行偏好；运行时影响待验证 | `src-tauri/src/app/model_catalog.rs`、模型 preflight/写回调用链 |
| 3 | App Tools MCP 启动和跨会话 | launcher 不变；同版证据待取得 | `crates/codex-win-engine/src/portable.rs`、实际 `.mcp.json` 与 server 日志 |
| 4 | 普通/主题/事务启动环境一致性 | CDP 预检接线已实现；实机待验，其余路径待核对 | `win_update.rs`、`codex_theme.rs`、`commands.rs`、启动和重载入口 |
| 5 | PE/签名/隐藏窗口/恢复边界 | host 不再被重写；旧 payload 和运行时待验 | `portable.rs`、`sys.rs`、`windows_process.rs`、同版原包 |
| 6 | Browser、搜索、媒体、会话、Skill Hub、标题及范围边界 | 待执行 | 分项代码入口与测试机证据 |
| 7 | 分章回归、审计、构建与交付门槛 | 待执行 | 当前源码、测试结果、release manifest/checksum、测试机记录 |

## 阅读顺序与执行依赖

章节编号是文档顺序，不是必须串行完成的执行依赖。先做第0章身份登记；第1章按问题拆证据。第2章、第4章和第6章可并行做互不冲突的只读调用链审查；证实独立代码缺陷后可分别窄修并补自动化。第3章 App Tools launcher 决策必须先经过第5章 bundled Node 子系统、实际启动父级和 stdio 核对，因为当前改写要求 Node 为 GUI subsystem。第2章 prompt A/B 不能改写 Node 迁移用作精确匹配的 legacy 签名。第7章整合各章结果。某项 `ENV_BLOCKED` 不冻结其他有独立证据、可用隔离 fixture 验证的工作。

## 第0章：基线、来源和边界

**前置决策：**确定受测候选源码、目标发行包和测试机 profile 的身份。缺少精确版本时记 `NOT_RUN`，不得拿公开 `main`、旧 MSIX 或旧测试日志代替当前候选。

**涉及记录：**当前工作区 Git 状态与完整 diff；`installer-pack/pack.json`、`file-manifest.json`、`checksums.sha256`；官方同版本/架构/发布者的原始包及其签名验证与哈希；测试机当前包、USB payload、CODEX_HOME 身份。精确哈希须从实际文件采集，未采集写“待采集”。

**执行与交付：**

1. 在工作开始和结束时记录 `git status --porcelain=v1 --untracked-files=all`、tracked staged/unstaged diff 的身份/哈希、所有 untracked 文件路径及内容哈希。只记 HEAD 不代表当前候选；普通 `git diff` 不含 untracked。不得 stash、reset、clean、覆盖或整理无关文件。
2. 默认在当前 checkout 上协作。新 worktree 从 commit 开始，不带入当前 tracked dirty diff 或 untracked 文件，不能默认新建后继续。若隔离确有必要，须审阅并转移 tracked diff 和所有相关 untracked 内容、逐项核验哈希后再修改。多人改同一文件时先停写、重读文件和 diff、协商唯一写入者；冲突或状态不明时保持文件不动。
3. 对源码候选记录基线 commit（如有）、完整 tracked diff 身份和 untracked 清单/哈希；对包记录 manifest、checksum 和实际文件哈希。源码、打包产物、USB 文件必须能以版本和哈希串联。
4. 官方对照仅用同版本、同架构、同发布者并验证签名的原包。公开上游源码/issues 可作线索，不等于测试机安装版本，也不能证明因果；证书表非空不等于签名验证。
5. 使用隔离测试 profile；USB `CODEX_HOME` 与用户标准 Codex 数据、凭证分离。不得为测试合并会话、配置或密钥。
6. 不删除测试来凑绿，不改写历史测试结果。历史结果按原时间、源码/测试二进制指纹、命令和环境单独入账。

**正例：**有同版可信包、当前 USB 文件哈希、候选源码 diff 和 profile 身份，比较结果可复核。**反例：**只凭文件名认定版本相同；把旧全量数字写成本候选通过；覆盖现有脏文件。

**回退范围：**尚未改代码前不回退任何既有工作区变更。未来每一代码章须列出仅对应文件的回退策略，并保留受测前副本/哈希；不得一键回滚整工作区。

**接受标准：**基线信息可复核、受测身份无歧义、隔离 profile 生效、无关脏改动保全。任一不满足时后续运行时结论保持 `NOT_RUN` 或 `ENV_BLOCKED`。

## 第1章：故障定位顺序和证据链

按以下顺序逐项找断点，不能从最终“模型没完成”反推所有前置层正常：

1. **服务/进程启动**：记录实际 executable、版本、父子 PID、cwd、相关环境变量、退出码和 stderr/stdout 采集方式。
2. **协议启动/注册**：记录 MCP initialize、server ready、超时/退出及重启；配置存在不等于 server 成功启动。
3. **当前回合工具暴露**：保存该新会话/模型的工具清单与来源，区分 Browser、Node REPL、文件/命令 host、媒体 MCP、App Tools 与 native multi-agent。
4. **模型调用与工具协议**：保存脱敏后的请求类别、provider/model、工具名/参数结构、上游响应/工具调用结果和错误；不得记录 token、密码或无关用户内容。
5. **最终结果**：核对调用工具的返回、会话目标、收件/委派回执及用户可见结果。`thread/list`、配置项、MCP initialize 或“请求已发送”均不单独证明最终交付。

**涉及文件：**根据断点选择 `sys.rs`、`portable.rs`、`model_catalog.rs`、provider probe、Browser/媒体能力包实现及对应测试。先由证据台账标明哪一层已核实，再提最小改动。

**正例：**工具未出现在当前回合清单时先查注入层；server 未握手时不改模型提示词；HTTP 请求被拒时保留请求/响应状态，不伪造工具成功。**反例：**凭本地 MCP 配置存在宣布 MCP 可用；凭有 thread ID 宣布消息送达。

**接受标准：**每项至少有当前 profile 的工具/服务证据和明确断点；若根因仍多解，先设计单变量判别，不扩大修改面。无证据的 issue/公开 main 结论标为“线索/假设”。

若设备/服务证据不可用，代码审查可继续完成：列出当前函数入口、调用者、配置/数据写点、能由 fixture 验证的合同，并明确未知项。`ENV_BLOCKED`/`NOT_RUN` 不得阻断与该运行时假设无关的确定代码修复，也不得将代码修复升级为运行时 PASS。

## 第2章：模型基础指令与旧目录 Node 策略

**前置决策：**先确认模型条目实际发送的最终 `base_instructions`、`model_messages`、provider/model、当回合工具清单。没有最终提示词/工具回执，不能声称是指令导致委派故障。

**涉及文件：**`src-tauri/src/app/model_catalog.rs`；模型目录加载、preflight、添加和持久化调用链；相应隔离 catalog fixtures。不要改 `config.toml` 的 MCP 配置来模拟提示词效果。

**执行与交付：**

1. 在 prompt A/B 前冻结旧模板签名：`normalize_legacy_model_entries` 的匹配所用旧描述、旧 `base_instructions`、text modalities 和缺少 `model_messages` 必须由独立 `LEGACY_*` 常量/fixture 表达。不得用新 `MODEL_BASE_INSTRUCTIONS` 覆盖旧签名。若需要改变匹配谓词，应单独版本化并补旧/新模板的命中、非命中和幂等测试，避免 prompt 策略改变导致 Node 迁移漏命中。
2. 同 provider/model、相同隔离 profile 和请求，建立唯一变量对照：当前自定义基础指令 vs 目标 Codex 版本接受的省略字段候选。**先用目标版本 CLI/parser/schema 确认 `base_instructions` 可省略并观察真实 fallback**；如 schema 要求字段，不得盲删，应选择有该版本依据的默认值再设计对照。当前文档不猜具体 parser 命令，执行者须从目标版本入口核实并记录实际命令。
3. 记录最终解析指令或可审计摘要、工具表、委派请求、目标端实际收件和结果回执。优先检查可能压制原生委派的指导；**只删少数句子不等于原生默认**。不得覆盖用户条目，使用可恢复隔离候选。只有 schema 和 A/B 结果支持，才提出生产策略；若宿主注入正常但 provider 不遵循工具协议，按 provider/model 兼容边界处理，不把 prompt 改写说成能力保证。
4. 保留用户确认的 Node 迁移政策：仅匹配精确旧模板的条目，且 `node_repl_disabled` 严格为 JSON 布尔 `true` 时删除该字段；`false`、缺失值、其他类型和不匹配条目不改该字段。新条目继续省略它。用户已知并接受：完全匹配模板上的手工 `true` 也会被移除；这是策略取舍，不是来源识别，不是待决策项。
5. 验证目录归一化、添加、切换、启动加载和再次写回入口一致；同一 catalog 连续 preflight 两次，第二次不写文件/哈希；确认只改预期 catalog，MCP `config.toml` 原始字节不变。旧结论“匹配条目保留 `true`”不得作为当前策略。

**正例：**精确旧模板 + 布尔 `true` → 仅移除该字段并保留 MCP 配置；非匹配或 `false` → 原样保留；隔离 prompt A/B 得到当前版本 fallback 与实际委派回执。**反例：**把全局 `true` 强加给 Search/Node；把 `thread/list` 成功当委派回执；把旧测试数字视为当前完整验证。

**回退范围：**只回退新建的迁移/提示词策略代码与相应新条目，不重置其他模型目录、MCP 配置或用户数据。匹配手工 `true` 删除后不能从字段本身恢复；需先保留原 catalog 备份和可复原步骤。

**接受标准：**自动化覆盖命中/不命中、类型边界、幂等和 MCP 字节不变；测试机证实省略字段 fallback、实际工具表和委派回执。只完成前者时运行时状态仍为 `NOT_RUN`。

**本轮最小变更：**仅从新/精确匹配旧模板共用的 `MODEL_BASE_INSTRUCTIONS` 删除“prefer local sequential work”偏好；不删 `base_instructions` 字段、不改变旧签名常量或迁移谓词、不重写用户自定义条目。新增/扩展的定向断言通过，但这只能证明模板字符串变更，不证明它是原委派故障根因或恢复了宿主委派能力。

若 fallback 验证被 `ENV_BLOCKED`，可继续与 prompt 假设独立的旧模板迁移/幂等自动化；不得上线“删除 base 字段”的生产变更或声称原生委派恢复。

## 第3章：App Tools MCP 启动与跨会话

**前置决策：**取得目标发行包内官方 `.mcp.json`、launcher `.cmd`、server、Node 候选路径和当前便携改写后的配置；通过第0章确认它们是同版本文件。

**涉及文件/函数：**`crates/codex-win-engine/src/portable.rs::patch_console_subsystem_executables`、`patch_codex_app_tools_mcp_config`、`ensure_portable_codex_startup_compatibility_inner` 及安装/启动调用点；插件 `.mcp.json`、官方 `launch_codex_app_tools_mcp.cmd`、Node runtime；MCP/app-server 启动日志和跨会话调用方。

**执行与交付：**

1. 比较官方 cmd wrapper 和直接 bundled Node：候选 Node 优先级/回退、固定 cwd、env allowlist、argv 与 quoting、startup/tool timeout、退出码和错误输出。当前改写已知保留 cwd 与其他 JSON 字段；不得写成“全部启动语义丢失”。
2. 当前便携 patch 的执行顺序是 PE sidecar 处理先于 `patch_codex_app_tools_mcp_config`，后者要求 bundled Node 为 GUI subsystem。两者有直接耦合；任何停止该 Node PE 改写的方案须同步重设/验证 MCP 命令、父进程窗口策略与 stdio，不能只移除名单项导致前置条件失败。以当前源码调用链为准。
3. 对全新解包 payload 和旧 manager 已安装 payload 分别验证；对 compatibility patch、MCP config patch 重复调用，核对第二次为 no-op。JSON 只允许预期的 `command/args` 改变，`cwd`、env allowlist、enabled、审批和 timeout 字段应保持原值。
4. 只有真实证据支持时提出最小 launcher 修正；评估 bundled Node 缺失、cmd fallback、父进程窗口行为及启动失败的可诊断性。不得无证据恢复可能重新弹出控制台的 cmd 启动。
5. 测试机依次记录 MCP initialize、server ready、`tools/list`、一个无计费/可逆工具调用结果、断开/重接、server 重启及新会话中的再次发现。每项保存时间、profile、会话 ID、工具名和结果。
6. 将 App Tools 的跨会话 `create/send/handoff` 与 Codex 原生 multi-agent 分开测试；记录准确目标 ID、调用返回、目标会话实际收件回执和回复回执。拒绝、未找到工具、已发送无回执分别记账。

**正例：**启动和 `tools/list` 成功后单独验证真实调用；跨会话 target 与接收端日志可一一对应。**反例：**只检查 `.mcp.json`；把本地 Node 路径存在当握手成功；把 `thread/list` 或 `api_request_sent=true` 当消息已送达；重发状态未知的有副作用请求。

**回退范围：**仅恢复此 MCP server 的 `command/args` 或相应 launcher 选择，不改其他 `mcpServers`、批准模式、凭据、线程数据或标准 Codex profile。恢复官方配置也必须先留存当前便携配置和观察到的控制台/Node 行为。

**接受标准：**命令、cwd/env/argv/超时/退出行为与设计相符；握手、工具表、调用与跨会话收件/回复均有独立证据。配置保留测试不替代运行时验收。

**本轮 App Tools 决策：**目前本地只读到 Codex `26.1002.7124.0` MSIX 样本；不能证明它与当前 USB 候选同版。该样本的 `.cmd` wrapper 有多个 Node 路径候选、系统 `node` fallback 和明确缺失 Node 错误；当前直接 bundled Node 改写保留 `cwd` 及其他 MCP JSON 字段，但会收窄候选/fallback 路径。现有证据不能证明该差异对目标 USB 版本构成缺陷，也没有同版握手/stdio 对照，因此本轮不改 launcher、不触发真实 MCP；先完成有明确源码证据的 Code Mode host 变更，缺少同版证据时将 App Tools 留作实机/同版候选验证。

**本轮 host 子项：**`codex-code-mode-host.exe` 已从 GUI subsystem sidecar 名单移除；代码现在保留该文件自身 subsystem，已有 CUI/GUI fixture 定向测试验证不改写和重复调用幂等。该代码改动消除一项不必要 PE 修改，但没有直接跑 host stdio/文件工具，因此不宣称已恢复工具运行，也不自动修复旧 USB payload。

若测试机 MCP 被 `ENV_BLOCKED`，可先完成官方包/当前源码的差异梳理、JSON 字段保真、Node PE 前置条件及旧/新 payload patch 幂等自动化；这些通过不代表 initialize、工具注入或跨会话已通过。

## 第4章：启动路径环境一致性

**前置决策：**列出并逐一定位普通 USB 启动、持久主题启动、独立应用主题启动、主题事务重启入口，确认每条实际调用关系；不得假定函数共用就等于所有入口已调用。

**涉及文件/函数：**`src-tauri/src/app/win_update.rs::launch_codex`、`codex_launch_environment`、`prepare_model_catalog_launch`、`complete_usb_model_catalog_launch`；`src-tauri/src/app/codex_theme.rs::launch_with_active_theme`、`apply_with_restart_and_locale`、`try_on_with_restart`、`restart_debuggable_and_inject`、`launch_codex_with_cdp`、`launch_codex_plain`、`launch_with_catalog_reload_preflight`；`src-tauri/src/commands.rs` 普通启动和主题命令入口。确认符号和 caller 以当前工作树源码为准。

逐路径核对以下四类，不用“主题路径”合并代替：

| 路径 | 可核对入口 | 独立证据 |
|---|---|---|
| 普通 USB 打开 | `commands::win_launch_codex` → `win_update::launch_codex` | USB gate、preflight、环境 builder、启动、marker completion 的顺序及子进程环境 |
| 持久主题启动 | `commands` 启动编排 → `codex_theme::launch_with_active_theme` → `apply_with_restart_and_locale` | 持久主题设置与 CDP relaunch 共用 USB 环境/preflight；失败时 marker 保留 |
| 独立主题/CDP 操作 | `codex_theme::try_on_with_restart`/`restart_debuggable_and_inject` → `launch_codex_with_cdp` | 独立 theme action 调用环境 builder 和 reload preflight；try-on 与 keep 状态不混淆 |
| CDP 事务重启与回退 | `apply_with_restart_and_locale` 事务分支、`launch_with_catalog_reload_preflight`、`launch_codex_plain` | 事务提交后 CDP 启动及回滚后普通启动都核对环境；成功路径才完成 marker |

**执行与交付：**

1. 逐一追踪四条路径的实际 caller/分支；确认所有 USB 分支共用 builder，且启动前运行 model catalog preflight。启动失败不得清除未完成 reload marker；只测试函数本体不能代替路径覆盖。
2. 核对受管 Python `bin` 实际路径与 PATH 优先次序，继承其余 PATH 项；保留 UTF-8 环境与已有媒体环境变量合并。每条启动路径分别记录传入子进程的环境摘要。
3. 用非 USB profile 对照，确认 USB-only PATH/CODEX_HOME/环境覆盖不泄漏、不持久写全局 PATH。检查 preflight 失败、CDP 失败、事务回滚、重启和重复操作下 marker 保留/清理。

**正例：**四条路径的进程环境可观察、受管 Python 在 PATH 优先、中文输出与 UTF-8 文件可读；非 USB 环境不被改变。**反例：**仅测试 builder 单元而未核对 CDP 调用点；因 USB 环境失败而全局修改用户 PATH。

**回退范围：**仅回退新增入口接线；保留已验证的统一环境构造、PATH 优先级、UTF-8 与媒体变量合并，不回滚其他 USB/CCSwitch 逻辑。

**接受标准：**四条路径各有自动化/代码链证据，并在测试机逐路径对照；不能由一次普通启动替代。若调用链静态确认某路径漏接，可先窄修并补 fixture；设备阻断只阻断真实启动结论。非 USB 隔离须在普通和主题入口共同验证。

**当前已实现子项（范围限定）：**`codex_theme.rs::launch_codex_with_cdp` 现在调用 `launch_with_catalog_reload_preflight`，先运行 `prepare_usb_model_catalog_for_theme_launch`，启动成功后才调用 `complete_usb_model_catalog_launch`。helper 的 `prepare()?` 或 `launch()?` 失败会提前返回，不调用完成回调，因此重载标记保持待处理。当前日志有 5 项主题 gate/preflight 定向测试和 7 项目录重载相关定向测试通过；另有一次 Tauri library 测试日志报告 802 passed、0 failed、6 ignored。日志未记录可复核的命令行、源码/二进制指纹，故只登记为当前工作区日志证据，不绑定发布候选。该修复只关闭 CDP 主题路径接线，不证明普通/持久主题/事务四路全覆盖；USB 实机仍待执行。

## 第5章：官方二进制、PE、签名和窗口行为

**前置决策：**对照同版本原始 PE 文件、签名验证结果、文件哈希和便携文件哈希；当前本机历史 MSIX 样本不是测试机包的替代品。

**涉及文件：**`portable.rs` PE 清单/偏移检查/USB overlay 与事务 hash；`sys.rs` 子进程创建和 stdio；`windows_process.rs` 窗口 watcher、一次性前台激活；打包 manifest/checksum。

**执行与交付：**

1. **未来变更**和**既有 payload 恢复**分开。评估每个 sidecar 的真实父进程、创建标志、控制台和继承句柄；PE subsystem 单独不是 stdio 故障的充分条件。未来只停止有证据显示不需要且产生副作用的修改，不做全名单批量回退。
2. 旧 payload 中发现 GUI host 时不得按名字推定旧管理器改过。只允许从版本、架构、发布者相同且签名/哈希经核验的可信完整原文件恢复；不允许仅 `2→3`、重写 PE 子系统或用不同版本文件替换。
3. 若宿主未来官方版本本身为 GUI 子系统，保留该官方文件，不强制改成 CUI。对签名文件做任何字节改写均须重新评估签名有效性、manifest/checksum、USB overlay/事务 hash 和包来源；不得把源码 fixture 结果称为包签名验证。
4. 找出实际控制子进程可见性的父级；窗口可见性轮询和一次用户显式激活不等同持续抢焦点。改动窗口逻辑前先保留启动可见性与显式“打开窗口”行为。

**正例：**A/B 单项比较中用同一原文件和同一 profile，记录 parent PID、spawn flags、stdin/stdout/stderr 句柄和窗口事件。**反例：**仅因 subsystem=2 判断损坏；把管理器控制所有后代当既定事实；把恢复标准输出效果或所有工具故障根因写成已证明。

**回退范围：**未来 PE 改写只回退对应文件/清单规则；既有二进制恢复使用完整同版备份且先确认目标哈希。源码回退不能自动修复已部署 payload，文档不得承诺两者相互替代。

**接受标准：**原包来源和哈希明确；签名验证结果留档；目标文件与候选版本匹配；每项二进制改动有必要性和相邻 app-server/工具回归证据。未匹配的 USB payload 标 `NOT_RUN`。

## 第6章：其他定向问题与范围边界

**Browser/focus：**`reason=focus` 是对账线索，不是焦点导致故障的因果证明。检查当前版本日志、focus gate 条件和插件缓存/工具清单；保留必要窗口可见性与用户显式激活。公开 issue 48522 作者的 focus 解释为 hypothesis；issue 44984 涉及 Bedrock delayed dynamic tools，不能直接证明 USB/404token 故障。

**Search/Node：**拆分 hosted web search、`supports_search_tool`/`tool_search`、DIRECT MCP 和 DEFERRED MCP。按实际 Codex 版本、provider/model、回合工具表核对；不全局强制 `supports_search_tool=true`。Node 默认字段按第2章策略处理；metadata 不等于当回合工具注入。

**Kimi/跨模型视觉：**按输入是否抵达、工具是否请求、MCP 返回/媒体任务 ID、上游 response、最终图像/文本结果分层。历史 `api_request_sent=true` 但最终任务状态不明时，先按请求 ID 查任务状态；不得盲目重发可能计费的生成。

**会话路由：**用同一 USB CODEX_HOME 建 A→B→A 测试，记录 canonical session ID、目标会话类型、实际收件和回复回执；不重写/猜测 ID，不合并官方会话、凭证或数据。

**Skill Hub 与自动标题：**Skill Hub 首次刷新必须从实际网络/下载/校验/落盘阶段定位，不能把测试 fixture 超时清理改动当生产修复。自动标题先定位当前代码入口及 session type，再用新会话验证标题请求/最终标题；不从旧标题记录推定当前实现。

**Git/WSL/HyperFrames：**Git 是否打包取决于明确产品承诺和可选工作流；不因 Browser/MCP 线索自动增加依赖。WSL 不扩展为本轮 USB 基础能力目标；HyperFrames 明确排除。

**卸载契约：**现行产品行为是两步确认后清理本机 Codex、CCSwitch 和受管能力包数据，同时保留 USB 安装介质；不改回可选清理。真实卸载/删除用户数据必须在专用可恢复测试环境并另行授权，本开发规范不授权操作真实用户数据。

**涉及文件/函数和每项执行合同：**

| 子项 | 初始代码入口/边界 | 必做步骤与产出 | 停止/接受条件 |
|---|---|---|---|
| Browser/focus | 当前仓库未找到 Codex Desktop bundled Browser reconcile 实现；本仓相邻入口只有 `portable.rs::ensure_portable_codex_startup_compatibility_inner` 和 `windows_process.rs::wait_for_usable_window_for_root`/`show_windows_for_root`。 | 对照当前 app 版本日志、插件缓存和回合工具表；同时间线记录焦点事件/PID、reason、插件加载与调用结果。保留入口和未发现的 owner 作为证据。 | 未证明本仓改动因果前不改 gate/窗口行为。只以真实插件加载+工具调用判断运行时；reason=focus 单独不是 PASS/FAIL 根因。 |
| Search/Node | `src-tauri/src/app/model_catalog.rs::{conservative_tested_model_entry,normalize_legacy_model_entries}`、`model_native_probe.rs` 的候选解析与标准化；目标 Codex host 决定实际注入。 | 对每个版本/provider/model 导出最终 metadata 和工具表，分 hosted web search、`supports_search_tool/tool_search`、DIRECT MCP、DEFERRED MCP；Node 严格按第2章 legacy 策略。 | 不全局写 `supports_search_tool=true`。metadata 测试通过不表示当前回合工具出现；工具出现不表示调用成功。 |
| Kimi/跨模型视觉 | 本仓可见入口 `src-tauri/src/app/capability_pack.rs` 安装/环境、`media_generation.rs` 授权环境、`codex_agents_hint.rs` 工作流说明和 `commands.rs` 媒体设置；MCP 服务/上游模型执行不必然由这些代码实现。 | 逐请求区分输入抵达、工具调用、prepare、用户费用确认、submit、异步 task/check、save、最终结果。未知 `api_request_sent=true` 任务先按 task/request ID 查状态，记录 response 和费用状态。 | 不盲目重发、不靠 prepare/schema 说生成成功；真实生成/计费前另授权。若仅上游模型不遵循工具协议，记录外部边界。 |
| App Tools 与原生 multi-agent | 本仓只直接改 `.mcp.json` 的 launcher（第3章）；Codex App Tools server 和 native delegation host 是外部运行组件。`model_catalog.rs` 的基础指令会影响模型输入但不提供工具本身。 | 用相同 USB `CODEX_HOME` A→B→A；分别核对 `create/send/handoff` 工具、canonical ID、A 调用返回、B 收件、回复回执；另外独立提交 native delegation 并读子任务结果。 | `thread/list`、配置存在、请求已发送均不作收件证据；目标不明/收件未知时停止，不重试副作用请求。 |
| Skill Hub 首刷/重试 | `src-tauri/src/commands.rs` Skill Repo check/apply/rollback 命令；`src-tauri/src/app/skill_repo_update.rs::{check_update,apply_update,rollback}` 和实际网络阶段。 | 用隔离缓存重现首刷，分 DNS/connect/HTTP/body/parse/hash/extract/write，记录首次结果和相同输入重试；fixture 与实际下载实现分别登记。 | 不把测试服务器 fixture 清理写成生产下载修复。HTTP/网络阻断时保存错误阶段并记 `ENV_BLOCKED`，不凭 404/timeout 猜 token。 |
| 自动标题 | 当前 `src-tauri/src`、`crates`、`src` 搜索未找到 Codex conversation title 生成 owner；起点是目标宿主新会话日志/接口，而非本仓虚构标题服务。 | 先识别会话类型、谁发起 title 请求、当前及最终标题、空/失败/重试语义；如果最终证明经过本仓调用再补准确入口和单测。 | 无本仓 owner 时标 `EXTERNAL_BOUNDARY`/`NOT_RUN`，不新造标题 API，不把模型任务内容当自动标题证据。 |
| 卸载 | `src-tauri/src/app/uninstall_cleanup.rs`、`commands.rs` 与 `src/app/views/Uninstall.tsx`/测试；现行两步确认产品策略。 | 静态审查清理目标清单及 USB 介质排除；自动化只用临时 fixture 验路径。 | 不把“清理改成可选”列为修复。真实用户数据删除需专用可恢复环境及单独授权。 |

对无仓库控制权的 Codex Desktop/网关/插件行为记录 `EXTERNAL_BOUNDARY` 或 `NOT_RUN`，不造替代机制。上述局部静态修复只需本章列出的源代码/自动化证据；实机/网络/计费环境阻断不得冻结互不依赖的章节。

**前置决策：**逐项从用户故障单/日志选择具体能力和会话类型；不因同属“工具”而混合 Search、Browser、MCP、native delegation。付费、外发和真实清理先检查授权。

**具体交付：**一条断点、一条最小对照、当前源码入口、相邻回归和单项回退方案。Browser、Search、媒体、路由、Hub、标题和卸载分别登记状态；秘密、凭证、完整用户 prompt 不进入报告。

**正例：**某 Search provider 的 tool_search 缺失只在该版本/provider/model 单独核对；Hub HTTP 404 与 fixture timeout 分开定位。**反例：**用 Browser 的 `reason=focus` 证明所有插件失败；未知媒体任务直接重发；通过修改工具开关伪造可用。

**回退范围：**仅回退对应模块的单项代码，保留已验证的 Python 环境、USB profile 隔离、两步卸载产品契约及其他无关模块；真实配置/数据变更不由源码回滚代替。

**接受标准：**每项有目标版本、profile、实际工具/请求/结果证据和相邻回归；无本仓控制权的结果标为 `NOT_RUN`/`ENV_BLOCKED`，不要求无关模块通过来代替本项。

## 第7章：回归、审计和交付

**前置决策：**各章实现者提供当前工作树 diff、源文件指纹、自动化命令和完整日志；测试资源和任何外部授权满足后才执行对应测试。

**回归顺序：**章节定向测试 → 相邻模块测试 → Rust 全量 → 前端全量 → check/lint/format/build → release manifest/checksum/源包关联 → 测试机新会话模块矩阵。每步记录实际命令、开始/结束时间、exit code、机器/环境、源码或二进制指纹、完整日志位置、通过/失败/忽略数量及逐项原因。

**仓库中已核实的命令入口（仅供后续获授权阶段执行，本轮未运行）：**

| 目标 | 当前仓库定义/CI 使用的命令 | 写入性与边界 |
|---|---|---|
| 安装前端依赖 | `npm ci` | 会写 `node_modules`；仅在获授权且使用指定候选工作区时执行。 |
| 前端类型检查 | `npm run check` | `package.json` 脚本。 |
| 前端 lint | `npm run lint` | `package.json` 脚本。 |
| 前端单测 | `npm test`（脚本映射 `vitest run`）；CI 用 `npm run test --if-present` | CI 的完整调用见 `.github/workflows/ci.yml`。 |
| 发布工作流定向测试 | `npm run test:release` | `package.json` 定义的本地测试脚本；只在触及发布工作流/manifest 相关代码时执行，不等于触发 GitHub release。 |
| 前端构建检查 | `npm run build` | 包括 `tsc --noEmit`、Vite build 和 renderer entry 检查；不是 Tauri 安装包发布。 |
| Tauri workspace clippy | `cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings` | `.github/workflows/ci.yml`。 |
| Tauri workspace 测试 | `cargo test --manifest-path src-tauri/Cargo.toml --workspace` | `.github/workflows/ci.yml`。 |
| Windows 引擎单测 | `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --all-targets` | CI 仅在该 crate 有变更时运行。 |
| 主题引擎单测 | `cargo test --manifest-path crates/codex-theme-engine/Cargo.toml --all-targets` | CI 仅在该 crate 有变更时运行。 |

仓库 `package.json` 与上述 CI workflow 未定义/执行 Rust 格式检查命令；实施者应先核对适用项目约定或维护者指定命令，再记录实际调用，不要从历史“fmt通过”数字推定本轮格式检查已跑。`.github/workflows/release.yml` 属发布/签名/分发工作流，不作为普通本地回归命令；任何实际 release dispatch、签名或发布需另有明确授权。本规范不提供未经仓库定义核实的打包命令。

**源码修改权限与推进规则：**本文件是实施蓝图，不自动授予执行权限。取得明确代码实施授权后，先做 `CODE_CONFIRMED` 可复现窄缺陷及隔离自动化；A/B 假设可保持 `NOT_RUN` 并独立推进其它已证实工作。不可因一项需实机/网络/账户授权的验收阻断所有独立章节；但相应章节不得记运行时或发布 `PASS`。每章开始前记录本章拟改文件与基线；完成后只核对/回退该章文件和新增产物，不回滚既有无关 dirty 文件。源码恢复只影响源码候选，不能声称恢复了已安装文件或 USB payload。

不得把旧版 `796/0/6`、`478/0`、定向 `77/11` 或更早 `248` 等历史记录套用到后续源码。若没有日志和源码指纹关联，写“历史报告/当前无法绑定”，不声称结果不一致，也不声称当前通过。发布包的 manifest/checksum 必须和最终源码候选及产物绑定；未打包不称“包验收”，本机通过不称“USB 测试机通过”。

**章节接受标准：**无未解释失败；失败有归因证据或明确阻断；危险/副作用路径的正反例覆盖；独立文档/代码审计完成；相邻功能未退化。任何一章不满足则保持该章未通过，不能用其他章节绿灯抵消。

**具体交付：**逐章 diff、自动化和实机原始记录、review 发现及处理、候选源码/产物/测试机版本绑定表、未解决项和签收状态。

**正例：**本候选全量日志可绑定测试二进制和工作树；release checksum 对应最终包；USB 新会话覆盖关键模块。**反例：**用旧计数、单测或本机开发环境替代当前 release/测试机验收；删掉失败项；无 source-to-package 关联就发布。

**回退范围：**每章依据其审查过的 diff 单独回退；包回退使用已验证的上一版本完整产物和 manifest，不能仅复用源码回滚推断包安全。测试机 profile 恢复按手册执行，严禁把整机或用户数据清理当作一般回退。

**最终接受标准：**必须满足本章前述测试、审计、manifest/checksum 和测试机门槛；残余 `NOT_RUN`、`ENV_BLOCKED`、未授权动作或来源不明的失败明确列出，不得标完整验收 PASS。

**真实操作授权：**真实卸载/用户数据清理、写入或替换真实配置/已安装二进制、凭证使用/外发和可能计费的真实生成均不由本文默许。需专用可恢复环境或对应单独授权。本轮允许的源码测试、临时 fixture 和任务分支提交不得扩展为这些操作。

## 文档可执行性审计记录（2026-10-25）

本记录只关闭本组开发文档的编排缺口，不代表任一功能代码已实施或验收。章节编号明确为阅读/文档顺序；实际依赖仅在“阅读顺序与执行依赖”和章节前置条件中列出。第3章 launcher 决策依赖第5章 Node PE/父进程/stdio 证据；第2章 prompt 修改必须保持 Node 迁移 legacy 匹配签名独立且稳定。普通 USB、持久主题、独立主题/CDP、事务重启已逐路径列在第4章和矩阵 R-02/R-03。

本轮文档复核关闭项：命令从 `package.json`、`.github/workflows/ci.yml` 核对；格式命令未核实则保留为执行前确认项；工作区身份包含 tracked 与 untracked 且明确新 worktree 不继承 dirty 状态；每项测试/实机/授权边界与 `NOT_RUN`/`ENV_BLOCKED` 的局部阻断规则已写明；旧版 payload、重试/幂等、断开恢复、配置字节不变和同版可信完整二进制约束已纳入执行合同。复核仅确认文档相互一致和仓库入口存在，未运行测试、服务、打包或目标机操作。

