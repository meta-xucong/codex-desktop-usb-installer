---
name: minimax-video-gen
description: Use for MiniMax video availability checks and request preparation, including when the user says not to submit yet; actual generation is unavailable until a protocol-compatible adapter and upstream are verified. Use video-prompt-generator when the user only wants a standalone prompt.
---

# MiniMax 视频工作流

已实现的媒体生成走本地 `media_generation` MCP。MiniMax provider 当前被硬禁用；共享设备 Key 与官方 MiniMax API Key 不是同一凭据。此 Skill 不调用官方 API、不读取密钥，也不猜测端点。

## 前置检查

1. 用户要求检查 MiniMax 能力或准备但暂不提交时，调用 `media_capabilities`；不调用 `media_refresh_models`。
2. 当前此 provider 是保留槽位，能力应报告不可用。告知用户目前没有已验证的 MiniMax 上游/兼容适配；停止操作。不得仅凭模型目录或手动填写模型名继续准备，也不得用 Grok、Seedance 或其他模型代替，除非用户另行明确选择该模型。
3. `MEDIA_MINIMAX_VIDEO_MODEL` 是预留字段；填写模型名、刷新共享模型目录或设置 Wokey profile 都不能启用 MiniMax。不要让用户把这些操作当作修复步骤。

## 当前可执行操作

1. 先将自然语言需求整理为简洁、可执行的视频提示词。用户只要求提示词时，直接返回提示词，不调用 MCP。
2. 当前没有可执行的 MiniMax 检查、prepare、submit、query 或 save；用户要求实际生成时明确说明暂不可用并停止。只有用户另行选择其他模型后，才可转到对应模型的 Skill。

## 未来适配后才可启用的提交流程

只有独立官方适配器实现、离线契约测试及安全审计通过后，才能重新启用 `media_prepare` → 展示完整请求 → 用户明确确认 → 单次 `media_submit` → 原任务查询/保存。当前版本不得执行这些步骤。

## 安全与可用性边界

- 当前没有 MiniMax 上游账号，也没有经验证的统一网关适配；`MEDIA_MINIMAX_VIDEO_MODEL` 不能单独启用此 provider。
- MiniMax 官方 API 使用独立 API Key、`api.minimax.io` 的 `/v2/video_generation` 与 `/v2/query/video_generation/{task_id}`，其 H3/H3 Max 请求结构和参数规则不同于当前共享媒体网关。除非未来实现并审计专用适配，否则不得使用签名设备 Key 直连官方 API、猜测 Wokey 兼容性或显示可用。
- 设备 Key 由签名 U 盘授权提供；不设置 `MINIMAX_API_KEY` 或 `MINIMAX_API_BASE_URL`。
- 不宣称模型价格、余额、额度或任务成功。每次可能计费的提交都必须先获得用户对完整请求的明确同意。
- H3 官方能力范围与当前统一 MCP 的实现状态见审计证据；官方文档参数不能直接套用为当前 MCP 参数。
