---
name: image-2
description: 通过显式配置的外部 Image-2 兼容网关生成或编辑图片。适用于用户明确调用本技能且已配置专用端点、模型、路由 ID 和凭据的场景；使用不可变确认清单、输入完整性校验和人工确认边界，禁止把它误认为 Codex 原生图片工具。
---

# Image-2 外部图像网关

这是一个 `guided-config` 的外部 HTTP 适配器，不是 Codex MCP，也不是 Codex 原生图片生成能力。普通图片生成优先使用 Codex 原生图片工具；只有用户明确调用 `$image-2`，或明确选择已配置的 Image-2 兼容网关时才使用本技能。

## 脚本定位

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。调用脚本时必须使用：

    <python> "<skill-directory>/scripts/image_2_gen.py" ...

不要假设当前目录是仓库根目录，也不要把已安装 Skill 目录当作产物目录。

## 启用条件

正式请求前必须由用户或私密运行环境显式配置：

- `IMAGE_2_API_BASE_URL`：服务根地址，只允许 HTTPS，不得包含 API 路径、凭据、查询参数或 fragment。
- `IMAGE_2_API_KEY`：该图像服务的专用凭据。不得读取或回退到 `OPENAI_API_KEY`。
- `IMAGE_2_MODEL`：服务方实际支持的模型名；本技能没有猜测默认值。
- `IMAGE_2_AGENT_ID`，或准备命令中的 `--agent-id`：外部网关要求的路由标识。它不是 Codex 官方任务 ID，不得从 Codex 任务、线程或其他元数据中猜测。

可选配置：

- `IMAGE_2_DATA_DIR`：本技能确认清单和 base64 输出的专用目录。
- `CODEX_DATA_DIR`：未设置前项时使用其下的 `image-2/`。
- `IMAGE_2_MAX_REFERENCE_BYTES`：单张参考图上限，默认 25 MiB。
- `IMAGE_2_MAX_REFERENCES`：单次参考图数量上限，默认 8。
- `IMAGE_2_TASK_POLL_INTERVAL`、`IMAGE_2_TASK_POLL_TIMEOUT`：轮询间隔和总超时。

缺少模型或路由 ID 时不得创建确认清单；缺少端点或专用凭据时不得提交请求。`--help` 和离线契约测试不需要凭据。

## 网关契约

脚本固定适配以下兼容路径；服务根地址必须实际提供这些路径：

- 文生图：`POST /api/llm/openai/v1/images/generations`
- 图生图：`POST /api/llm/openai/v1/images/edits`
- 轮询：`GET /api/llm/openai/v1/images/tasks/{task_id}`

请求使用 `Authorization: Bearer ...`、`agent-id`、`X-Async: true` 和由确认指纹派生的确定性 `Idempotency-Key`。创建接口既可返回带 `task_id` 的异步响应，也可直接返回 `data[].url`、`data[].b64_json` 或顶层 `image_url` 的同步响应；脚本对一次生成只发送一次 POST，收到同步结果时不会重复提交。

图生图以 `image[0]`、`image[1]`……的 multipart 字段按确认顺序发送。若服务方契约不同，应先修改并测试适配器，不能绕过脚本手写请求。

## 强制确认流程

所有真实生成都必须经过“离线准备 → 人工确认 → 使用同一清单提交”。不得直接把 prompt 或参考图传给正式生成模式。

### 1. 准备确认清单

文生图：

    <python> "<skill-directory>/scripts/image_2_gen.py" "<完整提示词>" --agent-id "<外部网关路由 ID>" --model "<实际模型>" --size "2048x2048" --prepare-confirmation --json

图生图：

    <python> "<skill-directory>/scripts/image_2_gen.py" "<完整编辑要求>" --agent-id "<外部网关路由 ID>" --model "<实际模型>" --image "<参考图1>" --image "<参考图2>" --size "2048x2048" --prepare-confirmation --json

`--image` 可重复，顺序和数量必须与用户输入完全一致；相同 URL 或相同文件重复出现时也必须保留，禁止去重、排序或按文件名重排。

准备阶段不请求图像 API。核对返回值中的 `api_request_sent=false`、prompt、参考图列表、数量、顺序、尺寸、模型和外部路由 ID。`confirmation_file`、`confirmation_fingerprint`、`prompt_sha256` 是内部执行数据，不向用户展示。

### 2. 向用户确认

完整展示并确认：

- prompt 正文；
- 文生图或图生图模式；
- 每张参考图的定位符、数量和顺序；
- 尺寸与实际模型；
- 这是可能计费的外部服务请求。

远程 URL 只锁定 URL 字符串，不能离线保证 URL 背后的字节内容不变化；准备结果会给出该提醒。本地文件会在清单中记录绝对路径、大小和 SHA-256，并在 dry-run/正式执行时复验。用户修改任一输入后必须重新准备清单并重新确认。

### 3. Dry-run

    <python> "<skill-directory>/scripts/image_2_gen.py" --confirmation-file "<内部清单绝对路径>" --confirm-fingerprint "<内部完整指纹>" --dry-run --json

Dry-run 只校验清单和本地文件完整性，不发送 API 请求。不得同时传 `--confirm`，也不得覆盖 prompt、参考图、模型、尺寸或路由 ID。

### 4. 正式生成

用户明确确认后，执行：

    <python> "<skill-directory>/scripts/image_2_gen.py" --confirmation-file "<内部清单绝对路径>" --confirm-fingerprint "<内部完整指纹>" --confirm --json

正式命令只使用已确认清单。不要在对话中展示清单路径、指纹、哈希、凭据或完整执行命令。外层超时建议不少于 300 秒。首次 POST 前脚本以排他方式写入 `submissions/` 账本；已记录的异步提交只恢复原 task_id，同步响应未完成持久化或受理状态未知时阻止重发，完成记录则复用既有结果。

## 参考图安全边界

- 本地和远程参考图只接受 PNG、JPEG、WEBP 或 GIF，单张默认不超过 25 MiB，单次默认不超过 8 张。
- 远程参考图只接受 HTTPS；拒绝内嵌凭据、fragment、localhost、非公网 IP，以及解析到非公网地址的主机。
- 每次重定向都会重新校验目标。下载失败、类型不符或任一参考图不可用时，整个编辑请求失败，不得静默删图后继续。
- URL 查询参数可能包含短期签名；确认和日志中按敏感定位符处理，不要无必要扩散。

## 尺寸与产物

允许的尺寸为脚本列出的 2K 预设：`2048x2048`、`1360x2048`、`2048x1360`、`1536x2048`、`2048x1536`、`1632x2048`、`2048x1632`、`1152x2048`、`2048x1152`、`2048x864`。

服务返回 URL 时原样返回；服务返回 base64 时，解码后的图片写入 `IMAGE_2_DATA_DIR/outputs/`，或默认的 `./codex-data/image-2/outputs/`。确认清单和一次性提交账本默认写入同一数据根目录下的 `.confirmations/` 与 `submissions/`，不写入 Skill 安装目录。

## 失败处理

- 配置、确认、完整性或网络校验失败：原样报告，不改输入、不自动重试。
- 已取得 `task_id`：只轮询该任务；超时后保留 task_id 供人工查询，不重新创建。
- 输入需要变化：生成新清单并重新取得用户确认。
- 不得使用 curl、PowerShell HTTP 或其他客户端绕过脚本的确认和校验边界。
