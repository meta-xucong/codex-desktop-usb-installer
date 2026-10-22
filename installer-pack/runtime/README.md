# U 盘运行时闭包

本目录是 Windows x64 / CPython 3.12 的可复现运行时源码与介质清单。安装器先校验媒体 manifest 中的字节数和 SHA-256，再创建用户级隔离环境；不修改系统 PATH。

## 离线随包组件

- CPython 3.12.10 安装器；
- 77 个逐项校验 SHA-256 的 Python wheel，以及基础和 Manim 独立 requirements 锁；
- Manim Community 0.21.0 的 CPython 3.12 venv；
- Node.js 22.22.0 Windows x64；
- FFmpeg/FFprobe/FFplay 9.x LGPL shared Windows build；
- Feishu MCP 0.5.1 的 npm 依赖 closure。

## 联网初始化组件

HyperFrames CLI 固定为 `hyperframes@0.8.134`，通过随包 Node/npm 安装。若本机没有兼容 Chrome，`hyperframes browser ensure` 会联网准备浏览器。网络断开时，其 CLI/浏览器健康状态写成 `requires-network`，不会阻断本地 Manim、FFmpeg、Python 或 Codex 的其余安装。恢复网络后可以运行 `hyperframes.cmd browser ensure` 重试。

## 可选组件和账号

MiKTeX/LaTeX 仅在 Manim 需要 `Tex`/`MathTex` 公式时安装。Feishu 需用户自己的 App ID、App Secret 和租户授权；远程 OAuth MCP 需各自的账号/工作区授权。U 盘版的统一媒体 MCP 使用签名 License 中自动配置的设备 Key，不要求用户再填写媒体 Base URL 或 Key；具体模型可用性和调用额度仍由对应的 404token 上游配置决定。公开能力包不包含用户凭据。Python 3.12 安装器与依赖 wheelhouse 随包携带，初始化时离线安装并验证。

逐项介质、来源、许可证和完整性见 `bundled/media-manifest.json`、`wheelhouse-manifest.json` 和 `THIRD-PARTY-NOTICES.md`。
