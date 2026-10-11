# Codex Capability Kit

一个可复盘、可复制、可审计的 Codex Skill 与 Agent 能力包。内容只围绕功能、依赖和部署边界组织，
不包含账号、令牌、会话、缓存、草稿或生成产物。

## 模型目录与默认模型

安装器会校验并安装包内脱敏的 `model-catalog.json`，并在用户没有默认模型或默认模型已停用时，将 Codex 默认模型迁移到目录中的 `gpt-6-astra`。目录只包含模型元数据，不包含用户的 `models_cache.json`、会话、账号、API Key 或服务端隐藏运行指令。

模型目录是离线快照。Codex 联网后仍按官方协议刷新自己的运行时模型缓存；发布新能力包时更新快照即可跟随上游。网络不可用时，安装器可以保证默认策略和目录状态，但不能替代官方账号授权或服务端模型可用性。

## 包里有什么

- `agents/`：7 个按功能命名的 Codex Agent TOML。
- `skills/`：55 个独立 Skill；非核心 Skill 额外带 `agents/openai.yaml`，默认关闭隐式调用，避免缺依赖时误触发。
- `manifest/`：55 个 Skill 的逐项审计、7 个 Agent 的依赖闭包、运行时、MCP、API 和连接字段契约。Node 22/FFmpeg/Manim 随包部署；HyperFrames CLI/Chrome 首装需要网络。
- `runtime/`：Windows x64 / CPython 3.12 的直接依赖输入、完整离线 wheelhouse、Node 22、FFmpeg、Manim/Feishu 介质、物化脚本和健康检查。
- `config-fragments/`：无密钥的 Codex MCP、Skill gating 和连接配置模板。
- `installer-pack/`：由脚本生成、可复制到其他电脑的完整能力包。
- `scripts/`：构建、契约审计、配置渲染、验收和安装脚本。
- `docs/CODEX-ADAPTATION-DEVELOPMENT.md`：本轮 Codex 适配的设计、改造步骤、测试矩阵与完成标准。

## 55 个 Skill 的最终状态

| 状态 | 数量 | 含义 |
|---|---:|---|
| `core-ready` | 29 | 复制后即可使用工作流说明和 Codex 原生能力，默认安装 |
| `auto-installable-runtime` | 15 | 需要安装包提供的 Python 3.12、离线 wheelhouse 和本机健康检查；包含 PDF 工具 |
| `guided-config` | 8 | 需要用户在连接向导中配置 API、MCP 或网络服务 |
| `unsupported` | 3 | 配置层面默认启用，但外部后端缺失时必须安全失败并明确报告 |

默认脚本安装 47 个可直接使用、可自动准备运行时或可安全降级的 Skill 和 7 个 Agent；8 个 guided-config
Skill 仍然复制到一键安装包中但保持关闭，待用户填入凭据或 MCP 配置。需要一键装入全部 55 个 Skill 时，使用安装包根目录的向导或
`install-all.ps1`。

```powershell
.\scripts\install-to-codex.ps1 -IncludeNonDefault
```

`-IncludeUnsupported` 仍可控制是否复制三个外部后端 Skill；一键安装路径会复制它们并在配置中启用，
但缺少后端时只返回不可用状态，不会伪造成功。PDF 已改为本项目自行维护的 MIT 工具实现。

完整逐项清单见 [`manifest/skill-audit.json`](manifest/skill-audit.json)，其中每个 Skill 都有原始状态、
整改后状态、依赖类型、版本、健康检查、启用条件和失败降级说明。

## 已知限制

- 18 个 `auto-installable-runtime` Skill 所需的 CPython 3.12、77-wheel 离线依赖闭包、Node.js 22、FFmpeg、隔离 Manim 环境和 Feishu npm
  closure 已进入安装包；安装器会在目标机完成哈希校验、隔离环境安装和健康检查。
- 8 个 `guided-config` Skill 需要真实凭据、模型、端点、服务账号/OAuth 权限或本地锁定 npm cache；默认不启用，本轮也没有
  使用真实租户或付费 API 做端到端调用。
- MiniMax 视频 Skill 使用统一媒体 MCP；当前 MiniMax 模型槽位为空，只有统一上游实际开放模型后才可用，不提供官方 MiniMax API 直连。
  Cloudflare、Figma、Linear、Notion MCP 以 Codex 原生 Streamable HTTP/OAuth 配置进入向导；连接仍需相应账号、工作区权限和 Codex OAuth 授权。
- `canvas`、`healthcheck`、`node-connect` 3 个外部 Skill 会写入启用配置，但没有后端或权限时必须明确安全失败；
  `pdf-processing-toolkit` 已补齐自维护许可证、离线依赖和读写/提取/生成 smoke test。
- 当前完整验收平台是 Windows x64、CPython 3.12 和 Codex CLI 0.157.1；Linux、macOS、Windows ARM64 及其他
  Python ABI 尚未进入发布矩阵。
- 外部行情、基金、政策、公告和第三方网关会变化；离线契约测试不等于真实数据源、真实权限、计费和生产环境验收。

完整原因、风险、解除条件和使用前检查见 [`docs/KNOWN-LIMITATIONS.md`](docs/KNOWN-LIMITATIONS.md)。

## 7 个功能入口

| Agent | 适合处理 |
|---|---|
| 全能官 | 综合统筹、拆解复杂任务、跨领域协作 |
| 研究官 | 学术、行业、竞品、用户和市场研究 |
| 创作官 | 公众号、小红书、文案、脚本、图片/视频提示词 |
| 投资官 | A 股、基金、宏观、创业项目和资产配置 |
| 增长官 | 用户痛点、产品定位、获客、转化和增长实验 |
| 生活官 | 健身、饮食、旅行、天气和个人复盘 |
| 办公官 | 文档、表格、PDF、PPT、邮件、会议和文件整理 |

它们是可记忆的功能入口，不是固定的人设。真正能否执行某一步由 Skill、运行时、外部连接和健康检查共同决定。

## 在 Codex 中调用

Agent 文件安装到 `%USERPROFILE%\.codex\agents\`，Skill 安装到 `%USERPROFILE%\.agents\skills\`。
新建任务时直接点名即可：

```text
请使用自定义 Agent「研究官」，研究这个行业：……
请区分事实、来源、推测和建议，并标注数据日期。
```

也可以在当前任务中委派：

```text
请把市场研究交给「研究官」独立完成，返回证据和结论；你最后再汇总。
```

如果用户没有点名，主 Agent 仍可按任务内容使用已安装 Skill；非核心 Skill 必须在显式调用且依赖健康后才启用。
重载 Codex 或打开新任务即可读取新 Agent。完整调用词、能力边界和安全降级规则见
[`manifest/agent-catalog.md`](manifest/agent-catalog.md)。

## 数据与决策真实性边界

宏观、A 股、基金、家庭资产配置和创业项目脚本统一采用三条规则：实时/外部数据必须带来源与时点，模型规则和市场
假设必须由调用者显式提供，接口失败或关键字段不全必须给出完整性错误而不是模拟值或成功空结果。脚本可以整理原始
字段、执行筛选和做可复算的场景计算，但不会把结果自动改写成买卖信号、产品推荐、风险评级、目标价、收益承诺或
投/不投决定。

A 股实时与 AkShare 批量入口会记录成功/失败组件；财务分析和报告模板不再内置固定 0–100 分、排名或行业均值占位。
宏观周期/政策/情绪分别要求显式规则、原文证据和指标权重。基金与资产配置只换算用户审阅的模型，定投、风险、
再平衡和收益追踪只计算显式场景。创业模型不判断“健康度”或推荐融资轮次，尽调脚本只生成法域/行业/交易结构明确且
全部待人工核验的起点清单。真实交易、法律、税务、会计和证券判断始终留给用户及相应专业人士。

## Windows 安装

安装能力包本身只需要 Windows PowerShell，不要求目标机预装 Python、Node、npm 或 Git：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\install-to-codex.ps1
```

完整一键安装（55 个 Skill、7 个 Agent、运行时、离线依赖和 Codex 配置）：

```powershell
.\Install-Codex-Starter.cmd
# 或
.\install-all.ps1
```

双击 `Install-Codex-Starter.cmd` 会打开 Windows 图形向导。向导根据
[`manifest/connection-fields.json`](manifest/connection-fields.json)、[`manifest/mcp-servers.json`](manifest/mcp-servers.json)
和 [`manifest/api-services.json`](manifest/api-services.json) 动态列出 6 个 MCP 与 1 个直连 API 的配置项，共 17 个连接字段
连接字段。每个外部连接默认不勾选；不填写或点击“跳过配置，直接安装”不会启用连接，但仍会把全部 Skill、Agent、运行时、
离线依赖和 MCP 包安装到本机。需要启用某项服务时，勾选它并填写该组必填字段；密钥输入框会遮罩，非空值只写入当前 Windows
用户环境变量，Codex 配置只保存环境变量名。留空的密钥会沿用已有的本机用户环境变量（若存在），不会在界面或日志中显示。
向导中的“清理本机已有环境变量”默认关闭；即使明确勾选，也只会清理本安装器此前记录为自己写入的变量名，不会碰其他程序或用户手工配置。

无界面部署或自动化测试仍可直接调用 `install-all.ps1`；设置 `CODEX_STARTER_NONINTERACTIVE=1`，或给 `.cmd` 传入参数，
也会跳过图形向导并进入脚本路径。向导的字段、校验、密钥边界和跳过策略见
[`docs/CONNECTION-WIZARD.md`](docs/CONNECTION-WIZARD.md)。

安装器只写自己拥有的 Agent/Skill 文件，不覆盖已有同名文件，除非显式加 `-Overwrite`；不会删除其他 Agent、
Skill 或全局 `AGENTS.md`。它会校验 `file-manifest.json` 与 `checksums.sha256`，部署本地 MCP wrapper、配置
渲染器和兼容性审计器，并记录不含凭据的所有权清单与 readiness 报告。即使使用 `-Overwrite`，当前文件哈希
与上次安装记录不一致或 Skill 内出现用户新增文件时也会保留并跳过；包内额外文件、符号链接或目录联接会被拒绝。
生成包复制到目标机后，也可直接运行：

```powershell
.\install-to-codex.ps1
```

安装记录位于 `%USERPROFILE%\.codex\codex-agent-kit\`。安装成功只代表文件完整，不代表可选运行时或连接健康。

### 运行时策略

需要真实 Python 功能的 Skill 使用安装包提供的 Windows x64 CPython 3.12.10 per-user 运行时和 77 个 wheel，
默认部署到安装记录下的独立 `runtime/python-env`，不依赖系统 PATH。Node.js 22、FFmpeg 和 Manim 环境
同样从安装包离线部署；锁定的 Feishu npm closure 离线解压。HyperFrames CLI/Chrome 缺失时需联网初始化。安装器会校验介质字节数、SHA-256、Python 签名、wheel lock 和导入探针；具体文件名、
签名、返回码、健康检查和回滚策略见 [`manifest/runtime-artifacts.json`](manifest/runtime-artifacts.json) 与
[`docs/FULL-BUNDLE-ONE-CLICK-DEVELOPMENT.md`](docs/FULL-BUNDLE-ONE-CLICK-DEVELOPMENT.md)。

## MCP、API 与连接配置

三类内容分开管理：

- MCP：登记 Feishu/Lark、统一媒体生成 2 个本地 STDIO 服务及 Cloudflare、Figma、Linear、Notion 4 个 Streamable HTTP OAuth 服务，见 [`manifest/mcp-servers.json`](manifest/mcp-servers.json)。远程 MCP 由向导可选写入 Codex 配置，授权由 Codex 与各服务完成；本地 Feishu 使用已锁定的离线 npm closure 和 wrapper。
- 直连 API：仅 DashScope 网页搜索，见 [`manifest/api-services.json`](manifest/api-services.json)。Image-2、Grok 图片/视频、Seedance 视频和预留的 MiniMax 视频模型槽位统一经本地 `media_generation` MCP 调用。
- 连接字段：统一的密钥、端点、认证方式、校验和能力映射，见 [`manifest/connection-fields.json`](manifest/connection-fields.json)。
  图形向导按这些清单生成表单，不在脚本中另维护一套凭据字段；所有外部连接都是显式选择后才启用。

统一媒体 MCP 包含 Image-2、Grok 图片/视频、Seedance 视频，以及待上游开放后使用的 MiniMax 视频 provider。安装时只配置一组媒体 API Base URL/Key，可按需填写各 provider 模型名；MiniMax 模型槽位当前留空，并仅接受统一上游的 Wokey 兼容路由，不访问 MiniMax 官方 API，也不会回退到其他模型。Image-2/Seedance 的部分旧网关还需路由 ID。MCP 使用本地 Python 3.12 标准库，无额外第三方运行时。它不会猜价格、退款、账户余额或模型名；计费调用必须走准备清单、明确确认、单次提交和原任务查询，未知提交状态不会重试。Seedance 参考媒体目前使用服务端可访问的 HTTPS URL。
模板在 `config-fragments/` 中，默认保持禁用。可先生成待人工合并的无密钥片段：

```powershell
.\scripts\render-codex-config.ps1 -OutputPath .\codex-starter.fragment.toml
```

启用飞书前，需在启动 Codex 的进程环境中提供 `FEISHU_APP_ID`、`FEISHU_APP_SECRET`，准备锁定的本地包 cache，
并给渲染器传 `-EnableFeishu`。配置只保存环境变量名，不保存值；但官方 CLI 最终仍以子进程参数接收 Secret，
本机高权限进程检查可能观察到它。当前官方实现不支持文件上传、下载和直接编辑云文档正文。没有健康检查通过时，
Agent 只能输出配置步骤或无连接降级结果，不能声称已经访问外部服务。

## 构建和验收

修改源文件后运行：

```powershell
python .\scripts\build-pack.py --check
python .\scripts\build-pack.py --verify
python .\scripts\audit-codex-compatibility.py --root . --require-codex
.\scripts\verify-package.ps1
```

`--check` 会重建生成包的文件清单、校验和及来源摘要；`--verify` 是只读 provenance 门禁，会计算当前源码树并拒绝
`installer-pack/` 来自旧源码的情况。`sourceCommit` 是构建输入来源指针，`sourceTreeSha256`
才是生成包必须匹配的可复现身份。

`scripts/verify-package.ps1` 可在没有 Python 的 Windows 机器上先完成包闭包与哈希校验；开发机若有
Python，还会自动运行完整兼容性审计。安装器只消费已通过这些门禁的生成包，不把校验脚本当成用户运行时依赖。

验收会检查 55 个 Skill、7 个 Agent、当前 Codex 版本、依赖闭包、非核心 Skill 的显式门控、
运行时/MCP/API 清单、PowerShell/Python/TOML 语法、哈希、路径安全、旧工具名和疑似密钥；还会执行飞书
 wrapper 脱敏 dry-run、统一媒体 MCP 配置渲染与离线协议/本地 mock 验收、DashScope 与高时效金融脚本的无网络契约回归、网页抓取帮助入口、天气/复盘/公众号草稿写入边界/技能搜索/运行时探测，以及临时用户目录中的
隔离安装 smoke test。完整实施依据见 [`docs/CODEX-ADAPTATION-DEVELOPMENT.md`](docs/CODEX-ADAPTATION-DEVELOPMENT.md)，
合并后 provenance 与公共许可隔离整改见 [`docs/POST-MERGE-CORRECTIVE-DEVELOPMENT.md`](docs/POST-MERGE-CORRECTIVE-DEVELOPMENT.md)。

新增能力时遵循 `installer-pack/pack.json` 的扩展契约：登记 Skill 或 Agent 及其依赖，MCP/API 与连接字段
分开登记，运行时只提交锁文件和可审计元数据，然后重新构建和验收。

## 安全边界

公开仓库不包含 API Key、Token、Cookie、`.env` 实值、私有 MCP 配置、会话历史、记忆数据库、日志、缓存、
个人草稿或生成结果。外部服务必须通过本机环境变量或私有密钥管理服务配置，并在执行前确认权限、费用和数据范围。

### 预留 MiniMax 模型槽位与可选远程 MCP

能力包保留 MiniMax 视频工作流 Skill，但请求只经统一媒体 MCP，并与 Image-2、Grok、Seedance 共用端点和 API Key。当前没有配置 MiniMax 模型；只有统一上游开放模型并由用户填写模型名后才会启用，不存在官方 API 直连。远程 MCP 需相应服务账号/工作区权限并由 Codex 完成 OAuth。
