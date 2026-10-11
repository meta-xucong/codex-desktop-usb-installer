---
name: image-2
description: 使用统一媒体生成 MCP 调用已配置的 Image-2 或 Grok 图片模型，支持文生图与参考图编辑；采用离线确认清单、单次提交、任务查询和本地保存。
---

# 统一 MCP 图片工作流

本 Skill 是用户侧图片工作流入口，不再携带独立 HTTP 客户端。所有外部图片生成均通过 Codex 的 `media_generation` MCP 完成；普通图片请求优先使用 Codex 原生图像能力，只有用户明确选择 Image-2 或 Grok 外部模型时调用本 Skill。

## 可用 provider

- `image-2`：服务方配置的 Image-2 兼容网关；可文生图，或使用本地参考图编辑。
- `grok-image`：xAI/Grok 兼容图片端点；可文生图，或使用最多 5 张参考图编辑。编辑参考图可使用 HTTPS URL 或本地图片。

具体模型名从 `media_capabilities` 的配置状态读取；没有配置模型时先让用户在安装器中填写，不猜测模型名、价格或供应商。

## 执行步骤

1. 调用 `media_capabilities`。确认 API Base URL、API Key 和目标模型已配置。该工具不联网、不计费。
2. 调用 `media_prepare`，传入 provider、完整 prompt、参考素材及参数。Image-2 文生图不传参考素材；图生图按用户给出的顺序传入 `{"kind":"image","local_path":"绝对路径"}`。Grok 编辑允许 `url` 或 `local_path`。
3. 向用户展示准备结果中的 provider、模型、完整 prompt、生成/编辑模式、参考图数量与顺序、尺寸/比例/分辨率和数量。明确告知价格估算为未知，用户需要先核对所用服务价格，并等待其明确确认。确认清单 ID 和指纹仅供后续 MCP 调用，不向用户展示。
4. 用户明确回复“确认”或“就按这份生成”后，以同一确认 ID 和指纹调用 `media_submit`，传 `user_confirmation` 为用户原话中的允许短语。不得改写已确认输入。
5. 若返回异步 task，之后只用相同确认 ID/指纹调用 `media_check`。完成后调用 `media_save` 保存结果。

## 安全与能力边界

- `media_prepare` 不请求外部 API。只有 `media_submit` 会创建可能计费的任务。
- 确认清单有效期 30 分钟；本地参考文件会锁定路径、字节数和 SHA-256。任何变化都重新准备并确认。
- Image-2 图生图只接受本地 PNG/JPEG/WEBP/GIF；Grok 图片编辑允许本地图片或公网 HTTPS URL。
- MCP 不读取 Codex 会话历史，不会从附件名推测模型或素材 URL。
- 如果提交状态未知、超时、收到 429/5xx 或解析失败，不自动重试，也不新建任务；告知用户先核查服务端任务/账单。
- 不编造成本、退款、额度或完成时间；不要把 MCP 的路由能力描述为官方账号授权。
