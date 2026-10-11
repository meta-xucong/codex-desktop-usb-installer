# 已知限制与未完成的外部条件

更新日期：2026-10-06
适用版本：`codex-starter-kit 1.5.0`
验证环境：Windows x64、CPython 3.12、Codex CLI 0.157.1

本文只列出完成 Codex 适配后仍然存在的限制。这里的“受限”不等于代码失败：有些能力需要私有运行时或凭据，
有些被明确隔离，还有些只能通过真实外部环境完成最终验收。

## 1. 状态总览

| 状态 | 数量 | 当前含义 |
|---|---:|---|
| `core-ready` | 29 | 只依赖 Codex 原生能力或说明性工作流，默认安装 |
| `auto-installable-runtime` | 15 | 包内已携带 Windows x64 / CPython 3.12 运行时和完整 wheelhouse，安装时仍需通过本机健康检查 |
| `guided-config` | 8 | 需要用户提供外部服务、凭据、模型或本地 MCP 包，默认不启用 |
| `unsupported` | 3 | 外部后端或权限不在包内；配置中可启用，但缺失时安全失败 |

复制 Skill 或 Agent 文件只代表文件安装完成，不代表运行时、网络、凭据、权限和数据源均已健康。

## 2. 运行时介质与平台边界

当前安装包已经包含以下可离线部署介质：

- Windows x64 CPython 3.12.10 安装器；
- CPython 3.12 / `win_amd64` 的完整离线 wheelhouse；
- Node.js 22.22.0 安装器；
- `@larksuiteoapi/lark-mcp@0.5.1` 及其离线 npm 依赖闭包。

这些介质的实际文件、字节数和 SHA-256 记录在 [`runtime/bundled/media-manifest.json`](../runtime/bundled/media-manifest.json)、
[`manifest/runtime-artifacts.json`](../manifest/runtime-artifacts.json) 和安装包的 `checksums.sha256` 中；安装器会在
部署前重新校验，不能把“文件存在”直接等同为运行时健康。

运行时契约目前只验证 Windows x64、CPython 3.12、`cp312`、`win_amd64`。Python 3.11、3.13、PyPy、Windows
ARM64、Linux 和 macOS 不在本版本验收矩阵中；不能因为脚本看似可移植就声称这些组合受支持。

## 3. 需要外部连接的 8 个 Skill

### 飞书：4 个

`feishu-doc`、`feishu-drive`、`feishu-perm`、`feishu-wiki` 依赖官方 Lark MCP、本地锁定 npm cache、Node.js、
App ID/Secret、飞书侧应用权限和可访问租户。

当前限制：

- 文档正文按只读能力适配；不承诺直接编辑云文档正文；
- 当前官方实现不支持文件上传和下载；
- 删除协作者、删除文件、转移所有者等高风险写操作仍需 Codex 显式审批；
- 官方 CLI 最终仍以子进程参数接收 App Secret，受信本机的高权限进程检查可能观察到它；本项目 wrapper 只能减少
  日志和 shell 转义泄漏，不能消除上游参数传递方式本身的风险；
- 未使用真实租户和凭据执行端到端读写测试。

详见 [`manifest/mcp-servers.json`](../manifest/mcp-servers.json) 和
[`manifest/feishu-tools.json`](../manifest/feishu-tools.json)。

### DashScope 网页搜索：1 个

`web-search-extraction` 是用户显式选择的外部兼容适配器，不是 Codex 原生网页能力，也不是 MCP。它要求专用 API Key、
`compatible-mode/v1` 根地址、实际可用模型和网络访问。公开包不猜测模型；提供方模型、价格、配额或接口行为变化时，
必须重新核验。Codex 原生网页能力可用时仍优先使用原生能力。

### 统一媒体生成 MCP：Image-2、Grok、Seedance

本地 STDIO MCP 统一提供 Image-2 图片、Grok 图片/视频、Seedance 视频，以及待上游开放后使用的 MiniMax 视频模型槽位的准备、确认、提交、查询与结果保存工具。它使用随包 Python 3.12 标准库，需用户自行提供一组媒体 API Base URL、共享 Key 和实际模型名；Image-2/Seedance 的部分网关路由需要用户配置 agent-id。MiniMax 当前模型槽位为空，仅接受统一上游的 Wokey 兼容模式；不会访问官方 MiniMax API 或回退其他模型。网关 API 路径必须符合随包 provider adapter 的路由契约。

每次可能计费的任务都要求离线准备、展示完整输入并等待用户确认；MCP 写入 HMAC 校验的单次确认记录，已有或未知提交不会自动重发。它不查询或猜测价格、余额、退款规则。Image-2 图生图接受本地参考图；Grok 图片/视频接受其当前适配的参考素材；Seedance 参考媒体目前要求服务端可访问的公网 HTTPS URL。第三方 API 兼容性和任务状态仍须用用户自己的端点与账号验证，本轮只做离线协议与本地模拟服务验收，不发真实计费请求。

### MiniMax 视频：预留模型槽位

`minimax-video-gen` 保留为工作流 Skill，实际调用只经过统一媒体 MCP，并共用其 Base URL/API Key。当前统一上游未配置 MiniMax 视频模型，因此该 provider 会显示未配置并在任何网络请求前停止。不得改走 MiniMax 官方 API，也不得自动回退 Grok 或 Seedance。

全部连接字段及启用条件见 [`manifest/connection-fields.json`](../manifest/connection-fields.json) 和
[`manifest/api-services.json`](../manifest/api-services.json)。

## 4. 远程 OAuth MCP 连接

向导另提供 Cloudflare Docs、Figma、Linear、Notion 四个远程 Streamable HTTP MCP 连接。用户必须拥有相应服务账号、空间访问权并完成 Codex OAuth。工具权限由供应商和工作区策略决定；部分工具可以写入数据。只把 URL 写入配置不代表登录完成或连接健康，连接健康状态需在 Codex 侧查看。

## 5. 配置启用但受外部后端约束的 3 个 Skill

| Skill | 当前边界 | 缺失时行为 |
|---|---|---|
| `canvas` | 依赖外部 Canvas 节点、画布网关和设备桥接 | 明确报告后端未连接，改用本地 HTML 原型 |
| `healthcheck` | 依赖 Windows 安全工具、管理员权限、系统策略和设备状态 | 明确报告权限/后端不可用，不伪造审计结果 |
| `node-connect` | 依赖设备配对协议、节点后端和远程网关 | 明确报告未配对/后端不可用，不把二维码或网页可达当成配对 |

安装器的一键路径会复制并在配置层面启用这些 Skill；启用只表示允许调用安全降级说明，不表示外部后端已存在。
`pdf-processing-toolkit` 已替换为本项目自维护实现，许可证见其 `LICENSE.txt`，并通过离线 PDF smoke test。

## 6. 数据源与高风险领域限制

- 新浪行情、AkShare、基金排行、公司公告、政策、宏观数据和第三方网页均会变化，也可能限流、改字段或暂时不可用；
- 离线回归使用受控假提供方验证契约，没有把测试数据当作真实行情；
- `partial` 或 `failed` 数据不能包装成完整成功，也不能把缓存描述为最新数据；
- 宏观、股票、基金、资产配置和创业脚本只做字段整理、显式场景和可复算计算，不提供自动买卖、目标价、产品选择、
  风险评级、收益承诺、投/不投决定或法律、税务、会计意见；
- 医疗、健身、营养、财务、法律和安全相关输出仍需用户结合现实约束，并在需要时交由有资质专业人士复核。

## 7. 安装与配置限制

- 安装器默认安装并在配置中启用 47 个 Skill（26 core-ready、18 auto-installable-runtime、3 unsupported），另复制 8 个 guided-config 但保持关闭；
- `-IncludeNonDefault` 会把 8 个 guided-config 也复制到 Skill 根目录并自动准备包内运行时；外部连接仍需凭据；
- 安装器会备份并合并自己的托管配置区块；遇到用户已有未托管的同名 Feishu MCP 时报告冲突，不覆盖用户配置；
- 遇到同名 Skill、用户修改或用户新增文件时，安装器默认保留并跳过；用户需自行决定是否迁移内容；
- readiness 报告是本机探测结果，不是跨机器保证；复制安装包到另一台电脑后必须重新执行健康检查；
- 公开包不读取、备份或恢复用户的真实 Secret、会话、日志、缓存、草稿和生成物。

## 8. 版本与验证范围

当前契约最低目标为 Codex CLI 0.144.0，本地验证版本为 0.157.1。未来 Codex、Lark MCP、AkShare、DashScope 或第三方
兼容网关发生不兼容变化时，需要更新清单、锁定版本和回归测试后重新发布。

1.3.0 基线曾通过结构审计、包哈希、34 项离线契约回归、50 个 Skill 快速校验、飞书脱敏 dry-run、配置渲染和三种隔离
安装规模测试。1.4.0 增加 Skill、远程 MCP 和媒体适配器；这些本轮没有在真实服务账号、Codex OAuth 或付费 API 上验证，
应先运行本地安装包校验并手动完成 OAuth / API Key 配置，再进行低风险连接检查。上述限制不替代真实租户、实时数据和付费 API 验收。

## 9. 使用前检查

1. 在 [`manifest/skill-audit.json`](../manifest/skill-audit.json) 确认目标 Skill 的状态和依赖。
2. 运行 `scripts/verify-package.ps1`，确认当前源码和安装包未被篡改。
3. 对非核心能力运行本机 runtime/connection health check。
4. 检查数据来源、时点、费用、权限、隐私和输出目录。
5. 只有健康检查通过且用户明确授权时，才执行外部写操作或计费操作。


## HyperFrames 首次部署边界

Node.js 22、FFmpeg/FFprobe 和 Manim 0.21 离线包含在能力包中。HyperFrames CLI 通过 npm 固定版本安装；若本机没有可用 Chrome，浏览器初始化会下载 Chrome。网络不可用时 `runtime-state.json` 会将 `hyperframes.cliHealth` 或 `hyperframes.browserHealth` 标记为 `requires-network`，不会阻断 Python、Manim、FFmpeg、飞书或 Codex 配置安装。联网后可用 `hyperframes.cmd browser ensure` 重试。
