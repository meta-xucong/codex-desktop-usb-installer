# U 盘版 Codex 基础能力分章回归矩阵（2026-10-25）

本矩阵供后续代码实现者与测试机执行者逐行填写，初始状态全部为 `NOT_RUN`。自动化可验证确定性契约，不能替代真实 app-server、真实 USB 会话或带副作用操作。执行前按[主开发规范](CODEX-USB-BASIC-CAPABILITIES-REPAIR-PLAN-20261025.md)建立基线，结果附到[证据台账](CODEX-USB-BASIC-CAPABILITIES-EVIDENCE-LEDGER-20261025.md)。

**本轮最小执行集合：**先处理有代码证据的模型基础指令问题；App Tools/PE 仅在同版离线对照证实缺陷时处理。只跑实际改动对应行的定向回归，再按主规范做一次适用最终套件。Browser/Search/Kimi/路由/Skill Hub/标题只在发现直接本仓缺陷时扩为实施项。已存在的 Node、Python/CDP、卸载行为只做受影响回归，不重复开发；USB 实机行仅在包候选可追溯后执行。

状态只能使用：`PASS`、`FAIL`、`NOT_RUN`、`ENV_BLOCKED`。每个 PASS 必须有日志/输出位置及候选版本/哈希；无证据留 `NOT_RUN`。`ENV_BLOCKED` 必须记录明确阻断而非归因猜测。

## 启动路径和执行依赖

章节号是文档组织顺序，不是串行执行门槛。`R-02` 到 `R-05` 可在第0章候选身份明确后分别推进；实机某条路被阻断不冻结独立 fixture/调用链修复。App Tools launcher 结果依赖先核清 bundled GUI Node PE 门槛（`R-19`），prompt 比较不得改变 Node 迁移 legacy signature（`R-05`）。最终完整包/测试机结论等所有独立必需项完成后再判。

启动环境必须分别记录四条路径，不能只跑普通启动后把其它路径设为 PASS：

| 路径 | 当前候选入口 | 对应矩阵 | 必须独立留档 |
|---|---|---|---|
| 普通 USB 启动 | `commands::win_launch_codex` → `win_update::launch_codex` | R-02 | preflight、builder、子进程 env、reload marker |
| 持久主题启动 | `commands` startup → `codex_theme::launch_with_active_theme`/`apply_with_restart_and_locale` | R-03 | 主题存在时环境/preflight；普通 profile 隔离 |
| 独立主题操作 | `try_on_with_restart`/`restart_debuggable_and_inject` → `launch_codex_with_cdp` | R-03 | CDP port、环境/preflight、try-on/keep 状态 |
| CDP 事务重启/回退 | `apply_with_restart_and_locale` 事务与 `launch_with_catalog_reload_preflight` | R-03 | commit/relaunch/rollback/restart 中 marker 生命周期和 env |

## 矩阵

| ID | 模块/验证层 | 最小步骤 | 预期 | 最小证据 | 自动化/实机/授权 | 当前状态 |
|---|---|---|---|---|---|---|
| R-01 | 文件与命令工具 | USB 新会话分别列工具、读目标 fixture 文件、执行无破坏命令；确认 host/app-server PID 与 stdio/退出码。 | 工具出现在回合中；文件内容和命令输出与 fixture 一致；无 CUI 窗口干扰工具结果。 | 当前回合工具清单、输入/输出 fixture 哈希、脱敏命令日志、退出码、进程树。 | 自动化合同 + USB 实机；fixture 不触碰用户数据。 | NOT_RUN |
| R-02 | 普通 USB Python 环境 | 普通启动后查询 `python` 解析路径/版本，输出中文，UTF-8 写入再读回临时文件。 | 优先受管 Python；中文 stdout 与文件可正确往返；媒体环境变量仍合并。 | 子进程 PATH、解释器路径/版本、编码结果、环境摘要。 | 自动化 + USB 实机；仅临时 fixture 文件。 | NOT_RUN |
| R-03 | CDP/主题 Python 环境 | 分别触发持久主题、独立应用主题和事务重启入口，重复 R-02 的非破坏检查。当前工作区 `launch_codex_with_cdp` 已接入 catalog preflight helper；先核对该路径，再单独跑四条入口。 | 四种启动入口均经过同环境 builder/preflight；非 USB 启动不获得 USB-only 环境。已有 theme 5/5、catalog reload 7/7 定向日志只覆盖自动化合同，不替代 USB 路径验收。 | 每入口调用链/进程环境、reload 状态、主题事务完成记录；日志见证据台账 E-11。 | 自动化 + USB 实机；不得仅凭 builder 单测 PASS。 | NOT_RUN（USB 实机未执行） |
| R-04 | Browser 插件 | 新 USB 会话记录插件缓存/对账日志、焦点事件和当前工具表；在 Browser 实际暴露后作只读页面/搜索调用。 | 只有实际注入且调用结果成功才 PASS；`reason=focus` 单独不构成根因。 | 同版本日志时间线、会话工具清单、工具调用返回、PID/前台窗口事件。 | USB 实机；不绕过宿主 gate。 | NOT_RUN |
| R-05 | Node REPL | 分别测新建模型、精确旧模板 `true`、旧模板 `false`/缺失/其他类型、非匹配 entry；同一旧 catalog 连续 preflight/normalize 两次，并重复加载。 | 仅精确模板布尔 `true` 被清除；第二次调用无写入/哈希变化；其他值不变；新 entry 省略字段；工具暴露另行验证。 | catalog 前后 diff/hash、备份、迁移命中原因、幂等记录及当前回合 Node 工具表/无副作用调用。 | 自动化 + USB 实机；手工 `true` 清除是用户已接受策略，先留原 catalog fixture。 | NOT_RUN |
| R-06 | hosted web search | 按 provider/model 查看最终 search 元数据，提交一条低成本/无计费 search 请求（若服务支持）。 | web search 与 MCP/tool_search 分开判断；按宿主协议返回，不强制改变全局字段。 | 请求类别、响应状态、工具清单、版本/provider/model。 | 自动化模拟协议 + 测试机；任何可能计费请求需另行授权。 | NOT_RUN |
| R-07 | tool_search 与 DIRECT/DEFERRED MCP | 用当前版本实际可搜索 deferred 工具检查 `tool_search`；另用直接暴露的 MCP 工具检查工具表。 | 只在宿主支持且存在可搜索 deferred 工具时期待 `tool_search`；`supports_search_tool=false` 不得推导所有 MCP 不可见。 | Codex 版本、配置元数据、工具注入清单和协议响应。 | 宿主解析/自动化 + USB 实机；不将 issue 44984 外推。 | NOT_RUN |
| R-08 | 模型添加/重复检测 | 测试同 slug 重复 Add、探测失败后重试、提交成功和写入失败；核对备份/原子提交。 | 重复条目不被意外覆盖；失败状态可重试且不宣称已提交；成功只追加/提交一次。 | UI 状态转换、请求 ID、catalog diff/备份、错误阶段。 | 自动化；使用隔离 catalog 和测试 endpoint。 | NOT_RUN |
| R-09 | 思考强度 | 对 provider 返回的 reasoning effort 候选做标准化、展示、选取、持久化并再次读取。 | 只展示确实支持的档位；缺失/未知值安全退化，不伪造 provider 能力。 | 原始探测响应脱敏、UI 候选、保存后的字段及重载状态。 | 自动化 + endpoint 环境；不靠模型生成文本猜支持情况。 | NOT_RUN |
| R-10 | 新旧模型目录与 MCP 字节保护 | 对新 catalog、旧模板迁移、未知字段、重复/损坏目录执行 preflight；迁移前后比对 `config.toml` 原始字节。 | 仅按精确迁移规则修改 catalog；`config.toml` 与 MCP 段字节完全不变；失败保留备份。 | 文件 SHA-256/字节 diff、目录输入输出、错误/备份记录。 | 自动化；不得改真实用户 CODEX_HOME。 | NOT_RUN |
| R-11 | MCP 安装/注册/握手 | 核对新包/旧升级包资源和路径；对 compatibility 和 MCP config patch 重复调用，比较第二次结果；观察 server spawn、initialize、ready、断开/重接、重启及新会话。 | 文件存在、注册、握手、重连和重启恢复分开标记；patch 第二次无非预期变化；JSON 除批准的 command/args 外保留原字段；失败定位阶段。 | package manifest、配置原始字节/字段 diff、PID/cwd/env 摘要、握手日志、重连前后工具表和退出码。 | 自动化 + USB 实机。 | NOT_RUN |
| R-12 | MCP `tools/list`、prepare、生成 | 先列工具；单测 `media_prepare` 参数 schema/prepare 返回；再按授权执行真实生成。 | prepare 成功只证明准备链；实际生成需确认请求已受理、任务完成及结果可取回。 | schema/工具名、响应状态、task ID、最终产物状态；脱敏保存。 | schema 自动化；真实生成可能计费，必须另行授权。 | NOT_RUN |
| R-13 | App Tools 跨会话传话 | 在同 USB CODEX_HOME 的 A 会话向明确 B 会话发送无副作用测试消息；核对 B 收件并回复。 | 发送返回、目标 ID、B 实际收到、回复回到 A 四项分别记录。 | A/B canonical ID、调用结果、目标端消息和回复回执。 | 自动化合同 + USB 实机；向他人/外部对象发送前需授权。 | NOT_RUN |
| R-14 | Codex 原生 multi-agent | 在当前 Codex 会话通过原生委派能力发一个可本地完成的只读子任务并等待其结果。 | native delegation 能力、委派动作、子任务结果分别可见；不得由 App Tools `send_message` 成功替代。 | 当前工具表、委派参数摘要、子任务 ID/结果/完成状态。 | USB 实机；不要求新增外部线程。 | NOT_RUN |
| R-15 | 会话 ID / 路由 | A→B→A 往返检查目标标识，跨会话读取接收回执；对无效 ID 做只读失败测试。 | canonical ID 不被重写；不会仅按标题或短 ID 路由；错误目标明确失败。 | 原始和 canonical ID、调用返回、接收端日志。 | 自动化 + USB 实机；不重放未确认副作用请求。 | NOT_RUN |
| R-16 | Skill Hub 首次/重试 | 在隔离缓存环境测首次刷新失败和后续重试，记录真实 HTTP、DNS、超时、解包、校验、落盘阶段。 | UI 分类对应实际失败阶段；fixture 改动不算生产下载逻辑修复；重试不留下半成品。 | 脱敏请求状态/HTTP、阶段日志、缓存/临时目录清单和重试结果。 | 自动化 fixture + 真实网络测试机；若网络被挡记 ENV_BLOCKED。 | NOT_RUN |
| R-17 | 自动标题 | 新建适用会话，记录是否发起标题生成、会话类型、失败/空结果后状态和最终标题。 | 按该会话类型现有产品合同生成/保留标题；标题生成失败不破坏会话身份/内容。 | 调用日志、session type/ID、标题变更前后。 | 自动化合同 + USB 实机。 | NOT_RUN |
| R-18 | 启动与新 payload | 在隔离目标上用同版新包做干净解包/安装；检查 compatibility 首次和重复调用，再执行普通 USB 启动。 | patch 前置条件、overlay/hash 和事务校验吻合；重复调用不产生非预期增量。 | 原包和 payload manifest/hash、PE/MCP 文件差异、两次调用前后 hash、启动日志。 | 自动化 + 专用 Windows 测试机；非用户安装。 | NOT_RUN |
| R-19 | 新候选 PE/Code Mode host | 检查用户生成的新候选的 PE 规则；针对 compatibility patch 调两次；触发 host stdio/文件命令/工具握手。 | host 不被改写；其他 sidecar 按现有规则；第二次 patch no-op；host 工具真实返回可复核。旧 payload 不恢复、不迁移。 | 候选 SHA/manifest、PE 头、字节 diff、父子 PID/句柄、host 请求返回。 | 自动化 parser + USB 新候选实测。 | NOT_RUN |
| R-20 | 卸载与保留 USB | 先做 UI/路径静态断言；真实清理由专用可恢复测试 profile 做两步确认，核对本机目标移除与 USB 介质保留。 | 按现行合同清理本机 Codex/CCSwitch/受管数据，保留 USB 安装介质；路径外不删。 | 展示步骤、清理计划、隔离前后目录清单、USB hash/可再启动检查。 | 自动化静态；真实删除要求专用环境和另行授权。 | NOT_RUN |
| R-21 | Git / WSL / HyperFrames 边界 | 检查是否存在明确产品承诺或目标问题相关性；避免默默纳入依赖。 | Git 只按明确可选工作流合同评估；WSL 不扩范围；HyperFrames 本计划排除。 | 范围决定及引用，不需声称功能运行通过。 | 文档/静态；无需实机。 | NOT_RUN |
| R-22 | USB 断开/重接/恢复 | 在专用介质与可恢复 profile 下记录正常退出/断开、重新接入、冷启动、热重启；覆盖一次未完成事务的安全恢复 fixture。 | 不残留错误锁/半完成 overlay；校验通过后才启动；身份/路径复核；非 USB 环境仍不受影响。 | 断开/重接时间线、事务/marker 状态、manifest/hash、进程关闭与重启结果。 | 自动化 fixture + 专用 Windows 测试机；不得拔除正在写入的用户介质。 | NOT_RUN |

## 填写规则

每次运行必须记录确切源码、包和 profile 身份；对计费、凭证、删改数据、对外发消息的行，先确认安全测试替代方式或取得独立授权。自动化测试缺失时把它列为待补测试，不为此临时构造会修改工作区或系统的命令。

