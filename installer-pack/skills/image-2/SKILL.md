---
name: image-2
description: 通过 U 盘预装的统一媒体 MCP 使用签名授权生成图片；默认走 gpt-image-2.5 Responses API，支持文生图、参考图编辑、PNG 校验和自动保存。
---

# 统一 MCP 图片工作流

本 Skill 是 U 盘版默认的实际生图入口，不携带独立 HTTP 客户端。图片生成通过 Codex 的 `media_generation` MCP 完成；默认使用签名 U 盘授权已连接的 404token `gpt-image-2.5` Responses API。用户明确要求生成图片时直接走本 Skill，不先尝试另一条生图路径。

## 可用 provider

- `image-2`：默认模型 `gpt-image-2.5`，通过 `/v1/responses` 和 `image_generation` 工具调用；可文生图，或使用本地参考图编辑。U 盘启动器从签名 License 读取设备 Key 并在 Codex 启动时提供给 MCP，不要求用户再次填写。
- `grok-image`：xAI/Grok 兼容图片端点；可文生图，或使用最多 5 张参考图编辑。编辑参考图可使用 HTTPS URL 或本地图片。

先调用 `media_capabilities` 查看实际配置。Image-2 默认使用 `gpt-image-2.5`；Grok 等其他模型只有在上游确实配置后才使用，不猜测模型名、价格或供应商。

## 执行步骤

1. 调用 `media_capabilities`。如果没有默认模型，先调用 `media_refresh_models`；它只读取当前统一网关模型目录并保存非敏感模型 ID，不提交生图任务。然后再次调用 `media_capabilities` 确认签名授权和 `gpt-image-2.5`（或目录中可用的 Image-2 模型）已就绪。
2. 调用 `media_prepare`，传入 provider、完整 prompt、参考素材及参数。Image-2 文生图不传参考素材；图生图按用户给出的顺序传入 `{"kind":"image","local_path":"绝对路径"}`。Grok 编辑允许 `url` 或 `local_path`。
3. 向用户展示准备结果中的 provider、模型、完整 prompt、生成/编辑模式、参考图数量与顺序、尺寸/比例/分辨率和数量。明确告知价格估算为未知，用户需要先核对所用服务价格，并等待其明确确认。确认清单 ID 和指纹仅供后续 MCP 调用，不向用户展示。
4. 用户明确回复“确认”或“就按这份生成”后，以同一确认 ID 和指纹调用 `media_submit`，传 `user_confirmation` 为用户原话中的允许短语。不得改写已确认输入。
5. Image-2 Responses 同步返回时，MCP 会在这次提交中验证图片并自动保存，直接返回本机路径。若服务返回异步 task，之后只用相同确认 ID/指纹调用 `media_check`，完成后调用 `media_save`。

## 安全与能力边界

- `media_prepare` 不请求外部 API。只有 `media_submit` 会创建可能计费的任务。
- MCP 使用随包 CPython 3.12 标准库，无需安装 `openai` SDK、单独 Python 包或系统 Python；安装器会预装运行时并在写入 Codex 配置前做离线健康检查。
- 确认清单有效期 30 分钟；本地参考文件会锁定路径、字节数和 SHA-256。任何变化都重新准备并确认。
- Image-2 图生图只接受本地 PNG/JPEG/WEBP/GIF；Grok 图片编辑允许本地图片或公网 HTTPS URL。
- MCP 不读取 Codex 会话历史，不会从附件名推测模型或素材 URL。
- 如果提交状态未知、超时、收到 429/5xx 或解析失败，不自动重试，也不新建任务；告知用户先核查服务端任务/账单。
- 不编造成本、退款、额度或完成时间；不要把 MCP 的路由能力描述为官方账号授权。
- 每次首次使用或目录超过 24 小时，先调用 `media_refresh_models`。只用该 provider 已明确配置且精确出现在当前网关目录中的模型；目录命中不代表图片生成权限已实测。
- 如果收到 `UNRESOLVED_PRIOR_SUBMISSION`，展示旧任务编号和“可能已计费”提醒并停止。只有用户明确要求再次生成且确认“此前任务可能已计费”后，才可在新的 `media_prepare` 传入精确确认短语；不能自动填充或重试。
- MCP 错误结果是 JSON envelope；根据 `error.code`、HTTP 状态和 request ID 向用户解释，不要丢弃状态后改走另一条生成路径。
