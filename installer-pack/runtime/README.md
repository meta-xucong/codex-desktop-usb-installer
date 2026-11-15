# U 盘运行时闭包

本目录是 Windows x64 / CPython 3.12 的可复现运行时源码与介质清单。安装器先校验媒体 manifest 中的字节数和 SHA-256，再创建用户级隔离环境；不修改系统 PATH。

## 离线随包组件

- CPython 3.12.10 安装器；
- 77 个逐项校验 SHA-256 的 Python wheel，以及基础和 Manim 独立 requirements 锁；
- Manim Community 0.21.0 的 CPython 3.12 venv；
- Node.js 22.22.0 Windows x64；
- FFmpeg/FFprobe/FFplay 9.x LGPL shared Windows build；
- Feishu MCP 0.5.1 的 npm 依赖 closure；
- HyperFrames 使用的 Chrome Headless Shell 152.0.7977.30 Windows x64。

## 联网初始化组件

HyperFrames CLI 固定为 `hyperframes@0.8.134`，完整 Windows x64 依赖已随包携带，从本地 ZIP 部署，安装时不执行 npm 下载。Chrome Headless Shell `152.0.7977.30` 已随包携带，从本地部署到 kit 私有目录；无需预装 Chrome，也不访问 Google 浏览器下载源。安装检查和 `hyperframes.cmd` 的后续渲染使用同一份本地浏览器。请通过 `hyperframes.cmd` 调用，避免绕过其浏览器路径设置。

查看 `runtime-state.json` 的 `hyperframes.cliHealth`、`hyperframes.browserHealth` 和 `hyperframes.error`；两项 health 均为 `passed` 才表示该组件就绪。本地 CLI 或浏览器部署失败会保留具体原因并记为 `install-failed`，不阻断其他已通过核心能力的配置与 readiness 收尾。按诊断修复后可重新安装能力包；不能把可选组件未就绪写成安装成功。

## 可选组件和账号

MiKTeX/LaTeX 仅在 Manim 需要 `Tex`/`MathTex` 公式时安装。Feishu 需用户自己的 App ID、App Secret 和租户授权；远程 OAuth MCP 需各自的账号/工作区授权。U 盘版的统一媒体 MCP 使用签名 License 中自动配置的设备 Key，不要求用户再填写媒体 Base URL 或 Key；具体模型可用性和调用额度仍由对应的 404token 上游配置决定。公开能力包不包含用户凭据。Python 3.12 安装器与依赖 wheelhouse 随包携带，初始化时离线安装并验证。

逐项介质、来源、许可证和完整性见 `bundled/media-manifest.json`、`wheelhouse-manifest.json` 和 `THIRD-PARTY-NOTICES.md`。

HyperFrames 默认 H.264 视频编码使用独立随包 GPL FFmpeg（含 libx264），由私有命令入口定位；其他组件继续使用原 LGPL FFmpeg。无需用户额外设置 PATH。许可证见 runtime/THIRD-PARTY-NOTICES.md。
