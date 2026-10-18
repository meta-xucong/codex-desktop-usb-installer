---
name: minimax-video-gen
description: Prepare a MiniMax video-generation request and, only when the unified media gateway has explicitly configured a MiniMax video model, submit it through the shared media-generation MCP. Use when the user asks to generate or check a MiniMax video; use video-prompt-generator when they only need a prompt.
---

# MiniMax 视频工作流

所有媒体生成统一使用本地 `media_generation` MCP 和签名 U 盘授权提供的共享设备 Key。此 Skill 不调用 MiniMax 官方 API，不读取或要求单独的 MiniMax Key，也不自行猜测上游端点。

## 前置检查

1. 调用 `media_capabilities`，检查 `minimax-video` 是否显示 `configured: true`。
2. 若模型槽位为空或 MCP 未就绪，告诉用户当前统一上游没有开放 MiniMax 视频模型；停止操作。不得用 Grok、Seedance 或其他模型代替，除非用户另行明确选择该模型。
3. 不得通过请求参数临时覆盖已配置的 MiniMax 模型名。模型必须由用户在统一媒体连接配置中填写，并由 MCP 再次校验。

## 提交流程

1. 先将自然语言需求整理为简洁、可执行的视频提示词。用户只要求提示词时，直接返回提示词，不调用 MCP。
2. 用户要求实际生成且 MiniMax 已配置时，调用 `media_prepare`，`provider` 使用 `minimax-video`，同时提供提示词和模型支持的参数。
3. 向用户展示 MCP 返回的模型、时长、画幅、分辨率、提示词、费用未知提示和确认 ID。等用户明确确认这份具体请求后，才调用 `media_submit` 一次。
4. 使用 `media_check` 查询已提交任务；完成后可用 `media_save` 保存结果。提交状态未知时只能查询原任务，绝不能重复提交。

## 安全与可用性边界

- 当前没有预置 MiniMax 模型名。MiniMax 视频能力只有在统一媒体上游实际开放模型、并在 `MEDIA_MINIMAX_VIDEO_MODEL` 中配置后才可用。
- 设备 Key 由签名 U 盘授权提供；不设置 `MINIMAX_API_KEY` 或 `MINIMAX_API_BASE_URL`。
- 不宣称模型价格、余额、额度或任务成功。每次可能计费的提交都必须先获得用户对完整请求的明确同意。
- 当前 MCP 的 MiniMax 视频仅支持统一上游的 Wokey 兼容路由和文生视频参数；若请求需要未实现的参考素材或其他模式，说明限制并停止，不要猜测接口格式。
