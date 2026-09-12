---
name: seedance2-0-video-gen
description: 通过显式配置的外部 Seedance 2.0 兼容网关提交视频任务。适用于用户明确调用本技能，并已配置专用端点、模型、路由 ID、凭据及服务方费用策略的场景；严格保留媒体顺序和重复项，以不可变确认清单、幂等提交、历史恢复和本地产物对账降低重复扣费风险。
---

# Seedance 2.0 外部视频网关

本技能是 `guided-config` 的外部 HTTP 适配器，不是 Codex MCP，也不代表 OpenAI 或 Seedance 官方 SDK。只有用户明确调用 `$seedance2-0-video-gen`，并明确选择已配置的兼容网关时才使用。

## 脚本定位

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。所有示例都应展开为绝对脚本路径：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" ...

不要假设当前目录是仓库根目录，也不要向已安装 Skill 目录写任务数据。

## 启用条件

以下配置必须来自用户、服务方文档或私密运行环境；Starter Kit 不提供猜测默认值：

- `SEEDANCE_API_BASE_URL`：服务根地址，只允许 HTTPS，不得包含 API 路径、凭据、查询参数或 fragment。
- `SEEDANCE_API_KEY`：视频服务专用凭据。
- `SEEDANCE_MODEL`：端点实际支持的模型名。
- `SEEDANCE_AGENT_ID`，或 `--agent-id`：外部网关要求的路由 ID。它不是 Codex 官方任务 ID，不得从任务、线程或其他元数据猜测。
- `SEEDANCE_PRECHARGE_POINTS`：服务方当前声明的正整数预扣积分。
- `SEEDANCE_OFFICIAL_LINK_TTL_HOURS`：服务方当前声明的正整数链接有效小时数。
- `SEEDANCE_REFUND_RULE`：服务方当前返还/结算规则的原文。

缺少模型、路由 ID或任一费用策略字段时，连确认清单也不创建；缺少端点或专用凭据时不访问远端服务。`--help` 和离线契约测试不需要这些值。

可选数据路径：

- `SEEDANCE_DATA_DIR`：本技能任务记录与默认产物的专用根目录。
- `CODEX_DATA_DIR`：未设置前项时使用其下的 `seedance2-0-video-gen/`。
- `SEEDANCE_TASK_RECORDS_PATH`：任务记录 JSON 的显式绝对路径。
- `SEEDANCE_MAX_VIDEO_BYTES`：服务结果下载上限，默认 512 MiB。
- `VIDEO_OUTPUT_DIR` 或 `--output-dir`：视频输出目录。
- `CODEX_ARTIFACT_DIR`：未指定视频目录时使用其下的 `seedance2-0-video-gen/`。

## 网关契约

脚本固定适配：

- 创建任务：`POST /api/llm/doubao/contents/generations/tasks`
- 查询任务：`GET /api/llm/doubao/contents/generations/tasks/{task_id}`

请求使用 `Authorization: Bearer ...`、`agent-id` 和幂等键。若服务方路径、字段或计费语义不同，应先修改并测试适配器，不得绕过脚本手写计费请求。

请求 `content` 顺序固定为：完整 prompt、全部图片、全部视频、全部音频。映射如下：

| 已确认输入 | API 内容项 | role |
|---|---|---|
| prompt | `{"type":"text","text":"..."}` | 无 |
| 图片 | `{"type":"image_url","image_url":{"url":"..."}}` | `reference_image` |
| 视频 | `{"type":"video_url","video_url":{"url":"..."}}` | `reference_video` |
| 音频 | `{"type":"audio_url","audio_url":{"url":"..."}}` | `reference_audio` |

媒体只接受无内嵌凭据、无 fragment 的 HTTPS URL，并拒绝 localhost 或显式非公网 IP。脚本不会读取 Codex 会话历史，也不会从附件名、资源面板或 session 文件自动重建 URL；调用者必须从当前用户消息中取得并原样显式传入完整 URL。下载服务返回的视频时还会逐次校验 DNS 和重定向目标为公网地址，并执行响应长度与流式字节上限；任一校验失败就停止，不保留部分文件。

## 不可变输入规则

- 图片、视频、音频分别按用户提供顺序传入；禁止排序、改写 attachment 路径或按文件名重排。
- 相同 URL 重复出现时也必须保留；重复次数是已确认输入的一部分，禁止静默去重。
- `--expected-image-count`、`--expected-video-count`、`--expected-audio-count` 必须显式给出；无素材时传 0。
- duration 必须为 4–15 秒整数；ratio 为 `16:9` 或 `9:16`。
- 当前兼容契约要求 `generate_audio=true`；参考音频只表示参考素材。
- URL 字符串由清单锁定，但远端 URL 背后的字节内容不受本地清单控制。需要字节级固定时，先使用用户认可、不可变且服务可访问的对象存储地址。

## 强制确认流程

所有真实生成都必须经过“离线准备 → 人工确认 → 使用同一清单提交”。

### 1. 准备确认清单

有媒体的示例：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" "<完整提示词>" --agent-id "<外部网关路由 ID>" --duration 11 --expected-image-count 2 --image "<图片1 URL>" --image "<图片2 URL>" --expected-video-count 1 --video "<视频1 URL>" --expected-audio-count 1 --audio "<音频1 URL>" --prepare-confirmation

纯文本生成仍要显式声明零素材：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" "<完整提示词>" --agent-id "<外部网关路由 ID>" --duration 8 --expected-image-count 0 --expected-video-count 0 --expected-audio-count 0 --prepare-confirmation

准备阶段不请求 API、不创建 task_id、不扣费。核对 `api_request_sent=false`，并核对 prompt、三类媒体列表及数量、duration、ratio、watermark、`generate_audio=true`、模型、输出目录和服务方策略。确认清单有效期为 30 分钟。

`confirmation_file`、`confirmation_fingerprint`、`prompt_sha256`、`submission_key` 是内部执行/审计数据，不向用户展示。

### 2. 向用户确认

必须完整展示：

- prompt 正文；
- 图片、视频、音频各自的完整 URL、数量和顺序；
- duration、ratio、watermark 和“音频生成：是”；
- 实际模型；
- 当前配置的预扣积分、`SEEDANCE_REFUND_RULE` 原文、官方链接有效期；
- 本地产物目录，以及取得 task_id 后只查询同一任务、不自动重发的规则。

只有用户明确回复“确认”或“就按这份生成”后才能提交。任一字符、URL、顺序、重复次数、模型或生成参数变化，都要创建新清单并重新确认。

### 3. Dry-run

    <python> "<skill-directory>/scripts/seedance_video_gen.py" --agent-id "<外部网关路由 ID>" --confirmation-file "<内部清单绝对路径>" --confirm-fingerprint "<内部完整指纹>" --dry-run

Dry-run 校验清单并返回实际 payload，但不请求 API。不得携带 `--confirm-charge`、`--user-confirmation` 或任何输入覆盖参数。

### 4. 正式生成

    <python> "<skill-directory>/scripts/seedance_video_gen.py" --agent-id "<外部网关路由 ID>" --confirmation-file "<内部清单绝对路径>" --confirm-fingerprint "<内部完整指纹>" --confirm-charge --user-confirmation "确认"

正式命令只使用已确认清单。不要在对话中展示内部路径、指纹、哈希、幂等键、凭据或完整命令。长任务外层超时建议不少于 2400 秒。

一旦响应包含 `task_id`，只轮询或查询该 task_id。命令仍在运行时只轮询同一进程；不得再次执行创建命令。提交遇到超时、连接中断、429 或 5xx 时按“服务端受理状态未知”处理，不自动 POST。

## 恢复与对账

恢复受理状态未知的提交，只查询，不创建：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" --agent-id "<外部网关路由 ID>" --recover-submission "<submission_key>"

查询历史任务：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" --agent-id "<外部网关路由 ID>" --query-history --task-id "<task_id>"

本地产物对账：

    <python> "<skill-directory>/scripts/seedance_video_gen.py" --agent-id "<外部网关路由 ID>" --reconcile-artifacts

同一确认清单只能创建一次。未知提交后若用户仍要求新建任务，必须创建新清单，明确说明可能重复扣费，并在新确认后增加 `--confirm-retry-risk`。

## 数据与输出

默认任务记录：

    ./codex-data/seedance2-0-video-gen/seedance_video_tasks.json

默认视频目录：

    ./codex-data/seedance2-0-video-gen/video/

非 `main` 路由 ID 默认写入 `agents/<安全化路由 ID>/video/`。确认清单位于对应视频目录下的 `.confirmations/`。所有路径均展开为绝对路径返回，任何记录都不写入 Skill 的 `scripts/` 目录。

任务记录可能包含 prompt 和媒体 URL，应按项目敏感数据管理；不要提交到源码仓库。服务返回的官方 URL 按配置的有效期计算，到期后优先使用已下载的本地产物；链接过期不等于需要自动重新生成。

## 失败处理

- 参数、配置、确认或数量不匹配：在请求前失败，不删素材、不降低 expected count。
- 已取得 task_id 后轮询失败：保留并查询原任务，不重新创建。
- 用户要求更改或重新生成：新建清单并重新确认。
- 不得使用 curl、PowerShell HTTP 或其他客户端绕过确认、幂等和任务记录边界。
