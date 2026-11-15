---
name: hyperframes-video
description: Build template-driven, programmatic videos with Hyperframes and web technologies; use when a video should be assembled from HTML/CSS/React-like scenes and rendered through a local video pipeline.
---

# Hyperframes 程序化视频

HyperFrames 是本地视频编排/渲染工具，不是 MCP，也不是托管式生视频 API。HyperFrames CLI 固定为 `hyperframes@0.8.134`，完整 Windows x64 依赖已随包携带，从本地 ZIP 部署，安装时不执行 npm 下载。Chrome Headless Shell `152.0.7977.30` 已随包携带，从本地部署到 kit 私有目录；无需预装 Chrome，也不访问 Google 浏览器下载源。安装检查和 `hyperframes.cmd` 的后续渲染使用同一份本地浏览器。请通过 `hyperframes.cmd` 调用，避免绕过其浏览器路径设置。

查看 `runtime-state.json` 的 `hyperframes.cliHealth`、`hyperframes.browserHealth` 和 `hyperframes.error`；两项 health 均为 `passed` 才表示该组件就绪。本地 CLI 或浏览器部署失败会保留具体原因并记为 `install-failed`，不阻断其他已通过核心能力的配置与 readiness 收尾。按诊断修复后可重新安装能力包；不能把可选组件未就绪写成安装成功。 常规本地创作不需要 API Key；云端功能需要另行确认账号与费用。

把需求拆成有时长的镜头、可复用版式、字体和运动提示，优先用本地素材。渲染前检查画布、素材路径、字体、总时长和音频节奏。先预览，再交付项目源文件或实际导出的视频；未经用户要求，不覆盖源媒体、不发布。

HyperFrames 默认 H.264 视频编码使用独立随包 GPL FFmpeg（含 libx264），由私有命令入口定位；其他组件继续使用原 LGPL FFmpeg。无需用户额外设置 PATH。许可证见 runtime/THIRD-PARTY-NOTICES.md。
