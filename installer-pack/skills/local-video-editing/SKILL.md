---
name: local-video-editing
description: Plan or perform local video editing with FFmpeg, including trimming, concatenation, subtitles, audio, and export; use when the user wants an actual media file edited locally.
---

# 本地视频剪辑

U 盘安装器会在用户级运行时中预装 FFmpeg 9 和 FFprobe，并提供 `%CODEX_HOME%\codex-agent-kit\bin\ffmpeg.cmd`、`ffprobe.cmd`。先检查运行时状态或调用这些包装命令，避免依赖系统 PATH 上不确定的版本。若运行时检查未通过，说明状态并交付剪辑决策表或命令草案，不要声称已输出视频。

先探测输入文件的轨道、编码、时长与帧率，再按目标平台决定尺寸、帧率、编码、音轨和字幕。兼容时优先流复制；确需变换时再重编码。保留源文件并把成品写到单独路径，检查输出的时长、轨道信息和音画同步。Whisper 转录和模型是可选依赖，不在用户未要求时下载。不得推断用户同意发布或上传。
