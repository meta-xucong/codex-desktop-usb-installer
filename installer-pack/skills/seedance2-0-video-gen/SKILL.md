---
name: seedance2-0-video-gen
description: 通过统一媒体生成 MCP 调用已配置的 Seedance 视频模型，保留参考图/视频/音频顺序，并采用两阶段确认、单次提交、异步查询与本地产物保存。
---

# 统一 MCP 视频工作流

本 Skill 只负责 Seedance 视频创作流程；HTTP 请求、凭据、确认记录、幂等提交、任务查询和下载均由统一 `media_generation` MCP 处理。用户明确选择 Seedance 时可使用；用户未指定 provider 时，只有 `media_capabilities` 将 Seedance 列为可用且适合作为默认线路时才可选择。若用户明确指定其他 provider 或生成方式，以用户指定为准。

## 启用条件

先调用 `media_capabilities` 查看 `seedance-video` 是否已配置。签名 U 盘授权会自动提供共享设备 Key；Seedance 实际模型名和当前网关要求的 `MEDIA_GENERATION_AGENT_ID` 仍须来自上游配置。不要猜测模型、价格、预扣积分、退款条款或链接有效期。

Seedance 参考素材当前必须是模型服务端可访问的公网 HTTPS URL；MCP 不会代替用户上传本地媒体。通过 `media_prepare` 的 `reference_media` 数组按原顺序传入 `kind` 为 `image`、`video` 或 `audio` 的 URL 项。相同 URL 重复出现时也保留，不排序、不去重。

## 执行步骤

1. 调用 `media_capabilities`，确认签名授权、Seedance 模型和路由 ID 已配置。该工具不联网、不计费。
2. 调用 `media_prepare`，使用 `provider="seedance-video"`，传入完整 prompt、素材数组，以及实际支持的 `duration`、`ratio` 和 `watermark` 参数。纯文本生成传空数组。当前兼容契约会启用音频生成并返回最后一帧；不要传 `generate_audio=false`。
3. 展示完整 prompt、模型、视频参数、所有参考素材 URL/类型/数量/顺序，并明确价格估算未知，要求用户先核对服务方当前计费规则。等待用户明确回复“确认”或“就按这份生成”。确认 ID 与指纹是 MCP 内部执行数据，不向用户展示。
4. 用户确认后，以同一确认 ID 和指纹调用 `media_submit`。提交不得改变任何已确认的输入。
5. MCP 返回 task ID 后只查询同一任务：重复调用 `media_check`，直到完成或服务返回终态。完成后调用 `media_save` 将结果保存到本机，并向用户给出本地绝对路径。

## 计费与失败处理

- 准备阶段不发 API 请求；只有用户确认后的 `media_submit` 会创建可能计费的视频任务。
- 确认清单 30 分钟过期。本地记录仅锁定 URL 字符串，远端 URL 内容可能变化。
- 提交状态未知、网络超时、HTTP 429/5xx 或响应无法解析时，不自动重试或新建任务；先让用户核查网关任务与账单状态。
- 查询失败时保留原 task ID 并继续查询原任务；不能以重新生成代替查询。
- MCP 不读取 Codex 会话历史，也不会从附件面板或文件名猜测媒体 URL。
