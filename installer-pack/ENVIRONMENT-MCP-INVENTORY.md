# Windows 环境、运行时与外部连接清单

这份清单是公开能力包的部署契约，不含任何密钥、Cookie、私有 URL、会话或用户数据。完整逐项状态见
`manifest/skill-audit.json`；运行时制品细节见 `manifest/runtime-artifacts.json`。

## 最终状态分布

| 整改后状态 | 数量 | 默认安装 | 启用方式 |
|---|---:|---:|---|
| `core-ready` | 26 | 是 | 安装后可直接使用 |
| `auto-installable-runtime` | 18 | 是 | 运行时安装器完成健康检查后启用 |
| `guided-config` | 8 | 否 | 连接向导保存配置并通过健康检查后启用 |
| `unsupported` | 3 | 是（安全降级） | 外部后端缺失时明确报告不可用 |

## Windows 基线

| 组件 | 是否主链必需 | 检测 | 说明 |
|---|---|---|---|
| Windows 10/11 x64 | 是 | `Get-CimInstance Win32_OperatingSystem` | 安装目标环境 |
| PowerShell 5.1+ | 是 | `$PSVersionTable.PSVersion` | 安装器和原生降级脚本 |
| CPython 3.12.10 x64 | 按 Skill 需要 | `cpython-312` + `win-amd64` + x64 探针 | per-user，默认不改系统 PATH；3.11/3.13 不复用 |
| Python wheelhouse | 按 Skill 需要 | 生成 lock、完整导入检查和逐文件 SHA-256 | 公开状态为 pending，完整传递依赖由物化脚本生成 |
| Node.js 22.22.0 x64 | 是 | `node.cmd --version` | 随包部署；HyperFrames 与 Feishu 共用 |
| FFmpeg/FFprobe 9.x | 是 | `ffmpeg.cmd -version; ffprobe.cmd -version` | 随包部署；LGPL shared build |
| Manim CE 0.21.0 | Manim Skill 需要 | `manim.cmd --version` / runtime-state | 独立 CPython 3.12 环境，离线轮子安装 |
| HyperFrames CLI/Chrome | 程序化视频需要 | `hyperframes.cmd doctor` | npm/Chrome 首装没有本机可用副本时需联网 |
| Git | 不属于运行时 | `git --version` | 仅用于仓库协作，不是能力包前置条件 |

Python 选择普通 per-user CPython，而不是 embeddable ZIP：数据、文档和 PDF 能力需要正常的 SSL、pip、
venv 和包导入布局；安装根为 `%LocalAppData%\Programs\Python\Python312`，使用
`/quiet InstallAllUsers=0 PrependPath=0 Include_test=0`，避免 USB 盘符变化和全局 PATH 污染。

## 离线 Python 依赖

直接输入在 `runtime/requirements-python-win-x64-py312.in`；`runtime/materialize-python-wheelhouse.ps1`
会生成完整传递闭包、带 `--hash=sha256:` 的 lock 和 machine-readable wheel manifest。本版 U 盘运行包已随包锁定 77 个 Python wheel，并包含 Manim 0.21 的独立哈希锁；安装器校验每个 wheel 的 SHA-256 后离线安装。

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

## 本地媒体工具链

`local-video-editing` 使用随包的 FFmpeg/FFprobe。`manim-animation` 使用独立 Manim 0.21 环境；其基础场景无需额外 FFmpeg，`Tex`/`MathTex` 公式才需要可选 MiKTeX/LaTeX。`hyperframes-video` 需要 Node 22、FFmpeg/FFprobe、HyperFrames CLI 和 Chrome；前两者随包离线部署，CLI/浏览器缺失时安装器联网补齐，失败只降级该 Skill 并写入运行时状态。

## MCP 与直连 API

### MCP

本包登记 2 个本地 MCP（`feishu`、`media-generation`）和 4 个远程 OAuth MCP（Cloudflare、Figma、Linear、Notion）。`feishu` 属于 `guided-config`，需要用户提供 App ID、App Secret、服务域名
和认证方式，然后由渲染器生成待审阅的 `config.toml` 片段，默认保持 `enabled=false`，通过 `codex mcp list` 和只读探针
后才启用。官方实现要求 Node.js 20+；本安装器统一预装 Node.js 22，Feishu npm closure 本身随包提供。

远程服务在向导中默认关闭，选择后写入官方 Streamable HTTP URL 并开启 Codex OAuth 配置；用户仍需完成对应账号授权。Cloudflare Docs 提供公开文档检索，Cloudflare API、Figma、Linear 和 Notion 按账号/工作区授权访问数据，部分工具可写入数据。选择连接不代表已登录或服务健康；授权前应审核服务权限。

候选对比和选择理由见 `manifest/feishu-mcp-research.md`，配置字段见 `manifest/connection-fields.json`，
无密钥模板见 `config-fragments/feishu-official-stdio.template.toml`，工具白名单见 `manifest/feishu-tools.json`。
Codex 只按 `env_vars` 转发进程环境变量名，本机 wrapper 读取值、检查域名和本地包后直接执行 cache 内的
`lark-mcp`；不会使用 `npx` 或联网下载。Secret 不得写入 Codex TOML、日志或安装所有权清单。由于官方 CLI
的 `-s/--app-secret` 是参数形式，上游子进程命令行仍可能被本机高权限进程检查观察到。当前 25 个白名单工具
中，文档工具只读；官方实现不支持文件上传、下载或直接编辑云文档正文。

### 本地 MCP：统一媒体生成

| 服务 | 用途 | 状态 | 必需连接字段 |
|---|---|---|---|
| `mcp.media-generation` | Image-2 / Grok 图片与视频 / Seedance 统一准备、确认、提交、查询、保存 | auto-installable-runtime | U 盘签名授权自动提供设备 Key；安装器从上游模型目录自动选择最新匹配项，没有匹配项时留空 |

统一媒体服务以安装器预装的 CPython 3.12 标准库程序运行，不需要系统 Python、`openai` 包或其他额外 Python 依赖；安装器会检查 Python、MCP 版本及含中文工具清单的严格 UTF-8 协议。签名 U 盘授权中的设备 Key 会以当前 Windows 用户 DPAPI 加密副本保存在 `connections/media-generation.credential.dpapi`；MCP 启动包装脚本优先使用 USB 启动器注入的短时进程环境，缺失时读取 DPAPI 副本，并把 Key 只放入 MCP 子进程环境。明文不会写入 Codex TOML、普通连接文件、用户环境变量、命令行或日志；普通连接信息与模型目录缓存不含密钥，授权缺失或卸载时会清理受管文件。首次使用或缺少默认模型时，`media_refresh_models` 仅调用认证后的 `/v1/models` 读取目录并缓存可识别的图片/Grok 默认模型，不创建生图/视频任务；匹配不到的模型槽位保持为空。Image-2 使用已配置模型走 `/v1/responses`，同一次提交会校验并保存 PNG。异步视频任务只在上游明确完成且解析到输出地址后标记完成；相对 `content_url` 会补成当前网关地址，`media_save` 随后下载并保存。下载只对与网关协议、主机、端口完全相同的地址带授权；外部主机、不同端口和重定向不会收到 Key。每笔可能计费的生成都先落一份有时效、带 HMAC 的确认清单；未知提交结果不会重试。Seedance 参考素材目前要求公网 HTTPS URL；MCP 不代替用户上传。详情见 `manifest/media-generation-tools.json`。

### 直连 API

| 服务 | 用途 | 状态 | 必需连接字段 |
|---|---|---|---|
| `api.dashscope-web-search` | 网页搜索摘要适配 | guided-config | 专用 API Key、`compatible-mode/v1` 根级 HTTPS Base URL、实际联网搜索模型 |
DashScope 是当前唯一单独的直连 API。Image-2、Grok、Seedance，以及未来由统一媒体上游开放后的 MiniMax 视频模型，都使用一个媒体 MCP 和签名 U 盘授权中的共享设备 Key。MiniMax 模型槽位 `MEDIA_MINIMAX_VIDEO_MODEL` 当前留空；不设置 MiniMax 官方 API Key 或官方 API Base URL。

DashScope 脚本只接受 `compatible-mode/v1` 根地址并自行拼接 `/chat/completions`；模型必须通过
`DASHSCOPE_MODEL` 或命令参数显式提供，不再沿用仓库中的示例模型名。未配置 Key 或模型时会在网络 I/O 前失败。

## 外部后端缺失时安全失败的能力

- `canvas`：默认允许调用，但没有远程画布后端时只提供 HTML 原型并报告不可用。
- `healthcheck`：默认允许调用，但主机、管理员权限、网络暴露和备份状态必须现场审计，不能由静态包伪造。
- `node-connect`：默认允许调用，但设备配对协议和节点后端不在当前 Codex 能力包边界内。
- `pdf-processing-toolkit`：已补齐自维护 `LICENSE.txt`、pypdf/pdfplumber/reportlab 离线依赖和 smoke test。

## 启用规则

1. `core-ready`、`auto-installable-runtime` 和 `unsupported` Skill 在默认配置中写入 `enabled=true`；运行时和外部后端仍按实际健康状态决定能否完成操作。
2. `auto-installable-runtime` 只有在指定 Python 和 wheelhouse 健康检查通过后才执行真实操作。
3. `guided-config` 只有在连接字段校验、服务探针和权限确认通过后才启用。
4. `unsupported` 只在配置层面启用；Agent 必须给出安全降级，不得声称拥有缺失的外部能力。
5. 运行时、MCP 和 API 都有单独的健康检查与失败消息，互不混用。
