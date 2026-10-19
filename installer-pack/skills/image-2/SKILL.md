---
name: image-2
description: 通过 U 盘预装的统一媒体 MCP 使用签名授权生成图片；用户要求 Image-2 请求参数清单、预检或“只准备、不提交”时也使用本 Skill。默认走 gpt-image-2.5 Responses API，支持文生图、参考图编辑、PNG 校验和自动保存。
---

# 统一 MCP 图片工作流

本 Skill 是 U 盘版默认的实际生图入口，不携带独立 HTTP 客户端。图片生成通过 Codex 的 `media_generation` MCP 完成；默认使用签名 U 盘授权已连接的 404token `gpt-image-2.5` Responses API。用户明确要求生成图片时直接走本 Skill，不先尝试另一条生图路径。

## 触发与路由边界

- 用户明确点名 `Image-2`、`gpt-image-2` 或 `gpt-image-2.5` 时，优先使用本 Skill。即使没有点名模型，只要用户同时要求“图片请求参数清单”并明确说“只准备、不提交生成”（或同义表达），也路由到本 Skill；只整理参数并停在确认前，不调用提交或模型目录刷新。对明确要求这一路径的 MCP 参数清单或预检，也使用本 Skill。
- 用户只要求撰写、改写或优化图片提示词，没有点名上述模型，也没有要求检查或准备 Image-2 的 MCP 请求时，使用 `image-prompt-generator`；不得仅因任务提到“图片”或“提示词”而调用本 Skill 的 MCP。
- 与图片无关的文案任务（例如咖啡店广告文案）不触发本 Skill，即使文案中出现了可能适合配图的主题。

## 可用 provider

- `image-2`：默认模型 `gpt-image-2.5`，通过 `/v1/responses` 和 `image_generation` 工具调用；可文生图，或使用本地参考图编辑。U 盘启动器从签名 License 读取设备 Key 并在 Codex 启动时提供给 MCP，不要求用户再次填写。
- `grok-image`：xAI/Grok 兼容图片端点；可文生图，或使用最多 5 张参考图编辑。编辑参考图可使用 HTTPS URL 或本地图片。

先调用 `media_capabilities` 查看实际配置。Image-2 默认使用 `gpt-image-2.5`；Grok 等其他模型只有在上游确实配置后才使用，不猜测模型名、价格或供应商。

如果 `media_capabilities` 调用失败、被审批策略拦截，或无法确认所需 provider/路由是否可用，立即说明能力状态无法核实并停止；不得把默认模型描述成已配置，不得根据猜测继续调用 `media_prepare`，也不得切换 provider。用户只要求参数清单/预检或明确说不提交时，始终不得调用 `media_refresh_models`。

## 执行步骤

1. 调用 `media_capabilities`。如果用户只要求准备/预检或明确说不提交，不调用 `media_refresh_models`；只用本地能力结果准备字段清单，缺失项标为未知并说明，不能猜测。若用户要求实际生成且没有默认模型，才可调用 `media_refresh_models` 读取统一网关目录，再调用 `media_capabilities` 确认模型和路由已就绪。
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
