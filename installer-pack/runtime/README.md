# U 盘运行时闭包

本目录是 Windows x64 / CPython 3.12 的可复现运行时源码与介质清单。安装器先校验媒体 manifest 中的字节数和 SHA-256，再创建用户级隔离环境；不修改系统 PATH。

## 离线随包组件

- CPython 3.12.10 安装器；
- 77 个逐项校验 SHA-256 的 Python wheel，以及基础和 Manim 独立 requirements 锁；
- Manim Community 0.21.0 的 CPython 3.12 venv；
- Node.js 22.22.0 Windows x64；
- FFmpeg/FFprobe/FFplay 9.x LGPL shared Windows build；
- Feishu MCP 0.5.1 的 npm 依赖，随能力包安装部署；
- HyperFrames 使用的 Chrome Headless Shell 152.0.7977.30 Windows x64。

## 联网初始化组件

HyperFrames CLI 固定为 `hyperframes@0.8.134`，完整 Windows x64 依赖已随包携带，从本地 ZIP 部署，安装时不执行 npm 下载。Chrome Headless Shell `152.0.7977.30` 已随包携带，从本地部署到 kit 私有目录；无需预装 Chrome，也不访问 Google 浏览器下载源。安装时启动这份内置浏览器校验版本和空白页。请通过 `hyperframes.cmd` 渲染，它使用同一份本地浏览器。

查看 `runtime-state.json` 的 `hyperframes.cliHealth`、`hyperframes.browserHealth` 和 `hyperframes.error`；两项 health 均为 `passed` 才表示该组件就绪。`hyperframes.browserCheckMs` 和 `environmentApplyMs` 分别记录浏览器验证与应用运行环境的耗时。本地 CLI 或浏览器部署失败会保留具体原因并记为 `install-failed`，其他核心能力继续配置和记录安装结果。

## 可选组件和账号

MiKTeX/LaTeX 仅在 Manim 需要 `Tex`/`MathTex` 公式时安装。飞书部署完成后等待用户在管理器填写 App ID 和 App Secret，保存后启用。实际访问文档、云盘或知识库仍取决于应用的飞书授权范围。远程 OAuth MCP 需各自的账号或工作区授权。U 盘版的统一媒体 MCP 使用签名 License 中自动配置的设备 Key；具体模型可用性和调用额度由对应的 404token 上游配置决定。Python 3.12 安装器与依赖 wheelhouse 随包携带，初始化时离线安装并验证。

逐项介质、来源、许可证和完整性见 `bundled/media-manifest.json`、`wheelhouse-manifest.json` 和 `THIRD-PARTY-NOTICES.md`。

HyperFrames 默认 H.264 视频编码使用独立随包 GPL FFmpeg（含 libx264），由私有命令入口定位；其他组件继续使用原 LGPL FFmpeg。无需用户额外设置 PATH。许可证见 runtime/THIRD-PARTY-NOTICES.md。
