# Windows 环境、运行时与外部连接清单

这份清单是公开能力包的部署契约，不含任何密钥、Cookie、私有 URL、会话或用户数据。完整逐项状态见
`manifest/skill-audit.json`；运行时制品细节见 `manifest/runtime-artifacts.json`。

## 最终状态分布

| 整改后状态 | 数量 | 默认安装 | 启用方式 |
|---|---:|---:|---|
| `core-ready` | 25 | 是 | 安装后可直接使用 |
| `auto-installable-runtime` | 14 | 否 | 运行时安装器完成健康检查后启用 |
| `guided-config` | 7 | 否 | 连接向导保存配置并通过健康检查后启用 |
| `unsupported` | 4 | 否 | 保持禁用，给出替代路径或补齐许可证证据 |

## Windows 基线

| 组件 | 是否主链必需 | 检测 | 说明 |
|---|---|---|---|
| Windows 10/11 x64 | 是 | `Get-CimInstance Win32_OperatingSystem` | 安装目标环境 |
| PowerShell 5.1+ | 是 | `$PSVersionTable.PSVersion` | 安装器和原生降级脚本 |
| CPython 3.12.10 x64 | 按 Skill 需要 | `cpython-312` + `win-amd64` + x64 探针 | per-user，默认不改系统 PATH；3.11/3.13 不复用 |
| Python wheelhouse | 按 Skill 需要 | 生成 lock、完整导入检查和逐文件 SHA-256 | 公开状态为 pending，完整传递依赖由物化脚本生成 |
| Node.js 20.x | 仅 Feishu 可选 | `node --version; npm --version` | 不属于主安装链 |
| Git | 不属于运行时 | `git --version` | 仅用于仓库协作，不是能力包前置条件 |

Python 选择普通 per-user CPython，而不是 embeddable ZIP：数据、文档和 PDF 能力需要正常的 SSL、pip、
venv 和包导入布局；安装根为 `%LocalAppData%\Programs\Python\Python312`，使用
`/quiet InstallAllUsers=0 PrependPath=0 Include_test=0`，避免 USB 盘符变化和全局 PATH 污染。

## 离线 Python 依赖

直接输入在 `runtime/requirements-python-win-x64-py312.in`；`runtime/materialize-python-wheelhouse.ps1`
会生成完整传递闭包、带 `--hash=sha256:` 的 lock 和 machine-readable wheel manifest。公开
`runtime/wheelhouse-manifest.json` 当前是 `materializationStatus=pending`、`installReady=false`；制作介质时
必须完成物化、签名和导入检查，然后才能用 `pip --require-hashes --no-index --find-links` 安装。

| 直接包 | 锁定版本 | 主要用途 |
|---|---:|---|
| akshare | 1.18.94 | A 股和基金数据 |
| pandas | 3.0.5 | 表格和数据分析 |
| numpy | 2.5.2 | 数值分析 |
| matplotlib | 3.11.1 | 图表 |
| openpyxl | 3.1.5 | Excel |
| python-docx | 1.2.0 | Word |
| python-pptx | 1.0.2 | PowerPoint |
| pypdf | 6.17.0 | PDF |
| pdfplumber | 0.11.10 | PDF/表格提取 |
| reportlab | 5.0.1 | 文档/PDF 输出 |
| Pillow | 12.3.0 | 图像处理 |
| scrapling | 0.4.15 | 网页正文回退 |
| html2text | 2025.4.15 | HTML 转文本回退 |

## MCP 与直连 API

### MCP

当前只登记一个实际 MCP 候选：`feishu`。它属于 `guided-config`，需要用户提供 App ID、App Secret、服务域名
和认证方式，然后由渲染器生成待审阅的 `config.toml` 片段，默认保持 `enabled=false`，通过 `codex mcp list` 和只读探针
后才启用。官方实现使用 Node.js 20+；它不是包内的可执行文件。

候选对比和选择理由见 `manifest/feishu-mcp-research.md`，配置字段见 `manifest/connection-fields.json`，
无密钥模板见 `config-fragments/feishu-official-stdio.template.toml`，工具白名单见 `manifest/feishu-tools.json`。
Codex 只按 `env_vars` 转发进程环境变量名，本机 wrapper 读取值、检查域名和本地包后直接执行 cache 内的
`lark-mcp`；不会使用 `npx` 或联网下载。Secret 不得写入 Codex TOML、日志或安装所有权清单。由于官方 CLI
的 `-s/--app-secret` 是参数形式，上游子进程命令行仍可能被本机高权限进程检查观察到。当前 25 个白名单工具
中，文档工具只读；官方实现不支持文件上传、下载或直接编辑云文档正文。

### 直连 API

直连 HTTP 服务不伪装成 MCP：

| 服务 | 用途 | 状态 | 必需连接字段 |
|---|---|---|---|
| `api.dashscope-web-search` | 网页搜索摘要适配 | guided-config | 专用 API Key、`compatible-mode/v1` 根级 HTTPS Base URL、实际联网搜索模型 |
| `api.image-2` | 图片生成/编辑 | guided-config | 专用 API Key、根级 HTTPS Base URL、模型、外部网关路由 ID |
| `api.seedance` | 视频生成/任务查询 | guided-config | 专用 API Key、根级 HTTPS Base URL、模型、外部网关路由 ID、预扣积分、链接有效期、返还规则 |

对应模板是 `manifest/dashscope-web-search.template.env`、`manifest/image-2.template.env` 和
`manifest/seedance.template.env`；真实值只能存在于本机私有配置或密钥管理器。

DashScope 脚本只接受 `compatible-mode/v1` 根地址并自行拼接 `/chat/completions`；模型必须通过
`DASHSCOPE_MODEL` 或命令参数显式提供，不再沿用仓库中的示例模型名。未配置 Key 或模型时会在网络 I/O 前失败。

图像和视频脚本适配的是清单中写明的固定兼容路径，并非自动发现的供应商 API。`agent-id` 是外部网关的
路由字段，Codex 没有可替代它的标准任务 ID，因此必须由服务方配置。Image-2 不得回退使用
`OPENAI_API_KEY`；远程参考图只允许 HTTPS 公网地址并限制单张 25 MiB，本地参考图在确认清单中按
SHA-256 锁定。Seedance 不再内置模型、`20000` 积分或 `24` 小时等供应商策略；三项费用/链接字段必须
按服务方当前规则显式填写并在计费请求前展示给用户。任务记录与生成产物写入项目 `codex-data/`（或明确
配置的数据目录），不写入已安装 Skill。

## 明确禁用的能力

- `canvas`：没有可验证的远程画布运行时，只提供 HTML 原型说明或替代方案。
- `healthcheck`：主机、管理员权限、网络暴露和备份状态必须现场审计，不能由静态包伪造。
- `node-connect`：设备配对协议和节点后端不在当前 Codex 能力包边界内。
- `pdf-processing-toolkit`：原始元数据引用缺失的专有 `LICENSE.txt`，在再分发条款可核验前保持禁用；优先使用 Codex 已提供的 PDF 能力。

## 启用规则

1. `core-ready` Skill 在默认安装集合中。
2. `auto-installable-runtime` 只有在指定 Python 和 wheelhouse 健康检查通过后才启用。
3. `guided-config` 只有在连接字段校验、服务探针和权限确认通过后才启用。
4. `unsupported` 始终保持禁用；Agent 必须给出安全降级，不得声称拥有该能力。
5. 运行时、MCP 和 API 都有单独的健康检查与失败消息，互不混用。
