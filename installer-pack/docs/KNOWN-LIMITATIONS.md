# 已知限制与未完成的外部条件

更新日期：2026-09-08  
适用版本：`codex-starter-kit 1.2.1`  
验证环境：Windows x64、CPython 3.12、Codex CLI 0.147.0

本文只列出完成 Codex 适配后仍然存在的限制。这里的“受限”不等于代码失败：有些能力需要私有运行时或凭据，
有些被明确隔离，还有些只能通过真实外部环境完成最终验收。

## 1. 状态总览

| 状态 | 数量 | 当前含义 |
|---|---:|---|
| `core-ready` | 25 | 只依赖 Codex 原生能力或说明性工作流，默认安装 |
| `auto-installable-runtime` | 14 | 包内已携带 Windows x64 / CPython 3.12 运行时和完整 wheelhouse，安装时仍需通过本机健康检查 |
| `guided-config` | 7 | 需要用户提供外部服务、凭据、模型或本地 MCP 包，默认不启用 |
| `unsupported` | 4 | 缺少可验证运行时、后端协议或再分发依据，保持禁用 |

复制 Skill 或 Agent 文件只代表文件安装完成，不代表运行时、网络、凭据、权限和数据源均已健康。

## 2. 运行时介质与平台边界

当前安装包已经包含以下可离线部署介质：

- Windows x64 CPython 3.12.10 安装器；
- CPython 3.12 / `win_amd64` 的完整离线 wheelhouse；
- Node.js 20.20.0 安装器；
- `@larksuiteoapi/lark-mcp@0.5.1` 及其离线 npm 依赖闭包。

这些介质的实际文件、字节数和 SHA-256 记录在 [`runtime/bundled/media-manifest.json`](../runtime/bundled/media-manifest.json)、
[`manifest/runtime-artifacts.json`](../manifest/runtime-artifacts.json) 和安装包的 `checksums.sha256` 中；安装器会在
部署前重新校验，不能把“文件存在”直接等同为运行时健康。

运行时契约目前只验证 Windows x64、CPython 3.12、`cp312`、`win_amd64`。Python 3.11、3.13、PyPy、Windows
ARM64、Linux 和 macOS 不在本版本验收矩阵中；不能因为脚本看似可移植就声称这些组合受支持。

## 3. 需要外部连接的 7 个 Skill

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

### Image-2 与 Seedance：2 个

`image-2` 和 `seedance2-0-video-gen` 面向用户提供的外部兼容网关，不代表 OpenAI、字节跳动或其他厂商的官方托管
服务。两者都要求专用 Key、根级 HTTPS 地址、实际模型和网关路由 ID。Seedance 还要求用户明确提供预扣积分、退款
规则和官方链接有效期。

本项目已经限制私网/本机远程 URL、重定向、文件大小、重复提交和安装目录写入，但仍不能保证第三方网关的可用性、
计费正确性、内容政策、数据保留方式或下载链接寿命。正式提交属于可能计费的外部操作，必须在执行前重新确认完整输入
和服务方政策。本轮未调用真实图像或视频接口。

全部连接字段及启用条件见 [`manifest/connection-fields.json`](../manifest/connection-fields.json) 和
[`manifest/api-services.json`](../manifest/api-services.json)。

## 4. 明确保持禁用的 4 个 Skill

| Skill | 保持禁用的原因 | 解除条件 |
|---|---|---|
| `canvas` | 能力包没有可验证的远程 Canvas 节点或画布运行时 | 提供可审计的运行时、协议、认证和健康检查 |
| `healthcheck` | 主机加固涉及管理员权限、系统策略和外部设备状态，静态 Skill 不能冒充真实审计 | 建立受控权限模型、逐项探测和可回滚实现 |
| `node-connect` | 设备配对协议和节点后端不在能力包中 | 提供协议、后端、认证、权限边界和端到端测试 |
| `pdf-processing-toolkit` | 上游声明专有许可证但缺少被引用的 `LICENSE.txt`；公开仓库只保留隔离占位，不含上游指南正文 | 获得可核验的再分发授权，或替换为许可证清晰且由仓库自行维护的实现 |

安装器默认排除这些 Skill。只有同时使用 `-IncludeNonDefault -IncludeUnsupported` 才会复制其隔离占位/审计源码；复制不
表示支持、授权或启用。PDF 占位也不会恢复任何上游正文或执行能力。

## 5. 数据源与高风险领域限制

- 新浪行情、AkShare、基金排行、公司公告、政策、宏观数据和第三方网页均会变化，也可能限流、改字段或暂时不可用；
- 离线回归使用受控假提供方验证契约，没有把测试数据当作真实行情；
- `partial` 或 `failed` 数据不能包装成完整成功，也不能把缓存描述为最新数据；
- 宏观、股票、基金、资产配置和创业脚本只做字段整理、显式场景和可复算计算，不提供自动买卖、目标价、产品选择、
  风险评级、收益承诺、投/不投决定或法律、税务、会计意见；
- 医疗、健身、营养、财务、法律和安全相关输出仍需用户结合现实约束，并在需要时交由有资质专业人士复核。

## 6. 安装与配置限制

- 安装器默认只安装 25 个 `core-ready` Skill 和 7 个 Agent；包根目录的 `Install-Codex-Starter.cmd` 会全量安装 50 个 Skill；
- `-IncludeNonDefault` 会把可补运行时或可配置的 Skill 扩展到 46 个，并自动准备包内运行时；外部连接仍需凭据；
- 安装器会备份并合并自己的托管配置区块；遇到用户已有未托管的同名 Feishu MCP 时报告冲突，不覆盖用户配置；
- 遇到同名 Skill、用户修改或用户新增文件时，安装器默认保留并跳过；用户需自行决定是否迁移内容；
- readiness 报告是本机探测结果，不是跨机器保证；复制安装包到另一台电脑后必须重新执行健康检查；
- 公开包不读取、备份或恢复用户的真实 Secret、会话、日志、缓存、草稿和生成物。

## 7. 版本与验证范围

当前契约最低目标为 Codex CLI 0.144.0，本地验证版本为 0.147.0。未来 Codex、Lark MCP、AkShare、DashScope 或第三方
兼容网关发生不兼容变化时，需要更新清单、锁定版本和回归测试后重新发布。

本版本已通过结构审计、包哈希、34 项离线契约回归、50 个 Skill 快速校验、飞书脱敏 dry-run、配置渲染和三种隔离
安装规模测试。这些测试证明当前仓库的契约和失败语义一致，但不替代真实租户、真实市场数据、真实付费 API 和生产权限
环境的验收。

## 8. 使用前检查

1. 在 [`manifest/skill-audit.json`](../manifest/skill-audit.json) 确认目标 Skill 的状态和依赖。
2. 运行 `scripts/verify-package.ps1`，确认当前源码和安装包未被篡改。
3. 对非核心能力运行本机 runtime/connection health check。
4. 检查数据来源、时点、费用、权限、隐私和输出目录。
5. 只有健康检查通过且用户明确授权时，才执行外部写操作或计费操作。
