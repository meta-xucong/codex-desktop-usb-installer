---
name: hyperframes-video
description: Build template-driven, programmatic videos with Hyperframes and web technologies; use when a video should be assembled from HTML/CSS/React-like scenes and rendered through a local video pipeline.
---

# Hyperframes 程序化视频

HyperFrames 是本地视频编排/渲染工具，不是 MCP，也不是托管式生视频 API。U 盘安装器会预装 Node.js 22、FFmpeg/FFprobe，并尝试联网安装锁定版本的 HyperFrames CLI（0.8.174）及准备 Chrome。检查 `%USERPROFILE%\.codex\codex-agent-kit\runtime-state.json`：`hyperframes.cliHealth`、`hyperframes.browserHealth` 均为 `passed` 才能确认本机渲染链路就绪。也可运行 `hyperframes.cmd doctor`；CLI 已安装但浏览器未就绪时，联网后重试 `hyperframes.cmd browser ensure`。

首次安装 HyperFrames CLI/Chrome 需要访问 npm 和浏览器分发源；离线安装时其状态可能是 `requires-network`，此时仍可使用本包里的 Manim 和 FFmpeg，但不要声称 HyperFrames 已可渲染。常规本地创作/渲染不需要 API Key；发布到云端或使用特定在线 AI 功能时，才需要对应服务账号/凭据，并应另行确认。

把需求拆成有时长的镜头、可复用版式、字体和运动提示，优先用本地素材。渲染前检查画布、素材路径、字体、总时长和音频节奏。先预览，再交付项目源文件或实际导出的视频；未经用户要求，不覆盖源媒体、不发布。
