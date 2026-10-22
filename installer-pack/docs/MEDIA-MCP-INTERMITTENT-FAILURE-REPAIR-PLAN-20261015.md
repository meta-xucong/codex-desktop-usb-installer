# 统一媒体 MCP 偶发失败修复开发文档

日期：2026-10-15  
状态：实现已落地；离线验收与独立代码/包审计通过，等待授权实机 smoke test  
目标：在不盲目重试、不暗示“已验证可生成”、不泄露凭据的前提下，减少错误模型付费提交、保留上游错误语义、让已接受任务可恢复、确保完成产物可保存。

## 0. 执行摘要与边界

本轮先把 starter-kit 的媒体 MCP 从 1.3.3 定点同步到 USB 包固定来源 `45413ffab1bab7b9697ef730ac992c711b940ab2` 的 1.3.4。同步清单和来源保真边界见同目录 `source-fidelity-media-mcp-1.3.4-20261015.md`。这只是当前版本对齐，不包含本开发文档提出的功能修复。

后续按本规格实施的 1.3.5 修复记录见第 12 节。原始需求范围、阶段和验收标准仍保留为设计基准；第 12 节记录的是实际实现及已执行的离线证据，不代表真实上游生图/视频已通过。

第 1–11 节保留为开发规格和验收基线；实施结果、测试证据和未完成的实机验收见第 12 节。不得把一次真实生成、一次模型目录命中或本地 `--healthcheck` 单独解释为完整可用证明。

### 范围

- 仅修改统一媒体 MCP、其 starter-kit 启动包装器/安装清单/配置片段、媒体专项测试与文档。
- 保持现有六个工具名称和显式付费确认门槛；如需扩展工具契约，须同时改 JSON-RPC schema、清单、skill 指引及测试。
- 保留单次提交持久化、HMAC 确认、提交前落盘和“未知提交绝不自动重发”的安全原则。
- 保留 starter-kit 当前的环境变量、连接 JSON、DPAPI 凭据注入；不得换成 USB 专属包装器。
- 仅使用假 HTTP 服务、固定 fixture、临时目录和注入式时钟/网络故障测试。开发阶段不调用真实 provider、不生成计费媒体、不使用真实 Key。

### 不在范围

- 不猜测或新增上游查询、dry-run、取消、幂等或计费端点。只有服务商正式契约/可重复的脱敏响应能证明支持时，才可启用相应能力。
- 不在无法确认提交是否到达上游时自动重新 POST。
- 不保证 `/v1/models` 列出的 ID 已开通生成权限，也不把只读目录检查描述为生成测试。
- 不更换 provider 路由、不修改 API key 注入来源、不改价格或扣费策略。
- 不把本地健康检查标记为图片/视频能力通过。

## 1. 固定基线与现状判定

### 1.1 基线

- USB 权威源：仓库 `codex-desktop-usb-installer`，提交 `45413ffab1bab7b9697ef730ac992c711b940ab2`，媒体服务 1.3.4。
- starter-kit 同步前：提交 `1ee1a9640175710fff52a3c98b0e6cf9fe459c09`，媒体服务 1.3.3；本仓库存在预先未提交的广泛变更，本开发不得清理或覆盖它们。
- 本次同步文件、SHA-256、包装器适配约束详见来源保真记录。
- 现存 `docs/media-mcp-reliability-v1.3.3.md` 保留为历史记录；本文件是本次间歇性失败的实现规格，阶段完成后追加实现证据，不覆盖旧记录。
- 本轮同步前未保存逐文件工作树快照。当前能核对同步目标与来源相符、同步后包 manifest/checksum 有效；不能据此证明此前所有未提交文件内容都未被覆盖。修复实现开始前，开发者应在独立/干净工作树继续，并记录其基线状态。
- 后续功能修复使用下一个补丁版本 `1.3.5`；在服务源码、两个 wrapper、两个 manifest 和配置包版本检查中统一版本。若基础同步实际校验不通过，停止，不在 1.3.3 或未对齐分支上叠加修复。

### 1.2 代码已证实的现状

以下依据固定 USB 1.3.4 服务源码，starter-kit 同步后代码位置保持一致：

| 位置 | 当前行为 | 对开发设计的影响 |
|---|---|---|
| `scripts/media-generation-mcp.py::_model`（约 270 行） | 优先采用显式模型名，否则采用配置模型；仅做字符串长度/控制字符检查。MiniMax 有额外固定槽位限制。 | 显式模型不受已缓存目录约束；目录检查必须在准备阶段完成。 |
| `media_refresh_models`（约 105 行） | 显式发起认证 GET `/v1/models`，保存 ID 列表、选定默认值、网关与时间；声明不发生成请求。 | 可复用现有目录；不要在 `tools/list` 或每次启动时隐式联网。 |
| `media_prepare` / `_validate_request`（约 311、522 行） | 验证 provider、通用字段和部分 provider 参数；模型由 `_model` 解析；准备阶段不联网。 | 目录强校验应明确要求刷新或验证目录，且在 prepare 前失败，不到 submit 才暴露。 |
| `_http_json`（约 601 行） | 4xx 错误变成普通 `MediaError` 文本；POST 5xx/408/425/429 与网络异常变成“提交状态未知”；错误对象不保留统一字段。 | 报告中的“400 已被 MCP 映射成 502/503”不能仅凭会话文字认定。必须用 fixture 重现本层状态与最终 MCP 工具结果后，再决定映射修复。 |
| `_media_submit_locked`（约 824 行） | 先落盘 `submitting` 与 `api_request_sent=false`；请求开始前落盘 `true`；未知不重试；幂等摘要只在获得正常响应后写入。 | 持久化意图和不重试已存在，修复不能回退；增加可观测性和有证据的对账能力。 |
| `media_check`（约 935 行） | 仅能查询已取得 `task_id` 的 `task_created`；未知提交返回错误；completed 无 artifact 会继续留在 task_created。 | 区分“任务未知”和“已终态但无产物”；勿凭空对未知任务查询。 |
| `_save_artifacts` / `media_save`（约 1101、1146 行） | Responses image-2 同步完成会自动保存；视频等异步流程由调用者再次 `media_save`；下载受 URL、重定向和大小限制。 | 统一产物状态与持久化；检查内容类型/格式/哈希和已存在文件复用。 |
| `_configured` / `media_capabilities`（约 1160 行） | 配置存在与最近目录状态可见，generation capability 明确为未实测。 | 不得把 configured/目录命中映射为生成 ready。 |

报告提供的 Grok 猜测模型在 `media_prepare` 放行、正式请求后失败，以及 image-2 上游 400 被用户侧看成 502/503，是重要排障线索，但不是对该失败响应的原始 wire 证据。实现前先补脱敏夹具或日志证据；本计划要保证未来可准确区分，并阻止已知目录外模型进入提交。

## 2. 优先级和交付门

顺序固定如下。P0-1 与 P0-2 属于先行保护；P1-1 的对账只能在验证供应商契约后实现，不允许因此阻塞已定义的 `submission_unknown` 安全状态。每一阶段的测试通过后再进入下一阶段。

| 顺序 | 优先级 | 改动目标 | 发布门 |
|---|---|---|---|
| 1 | P0 | 基于显式刷新目录验证 `provider + model`，提交不可绕过 | 目录外模型在无网络 POST 的情况下失败；过期/网关不匹配目录有确定策略 |
| 2 | P0 | 结构化上游错误与脱敏，消除错误原因丢失 | 4xx、429、5xx、超时及非 JSON 响应的代码、状态、可重试语义可预测 |
| 3 | P1 | `submission_unknown` 可识别、可重复查询证据；有正式接口才做对账 | 未知提交永不重新 POST；不虚构 task id / reconciliation 能力 |
| 4 | P1 | 统一任务终态、产物落盘状态与可恢复保存 | completed 不能冒充已交付；重复保存幂等且校验现存文件 |
| 5 | P2 | provider/model/operation 参数 schema 与轮询建议 | 不支持参数在 prepare 阶段拒绝；状态未知值不能无限伪装成运行中 |
| 6 | 发布 | 镜像、构建、离线验证、可选人工授权的真实 smoke test | 版本、包清单、哈希、凭据边界全部一致；无密钥入包 |

## 3. 阶段 1：模型目录成为提交前门禁（P0）

### 3.1 设计原则

1. 唯一可用模型证据是用户主动调用 `media_refresh_models` 后，当前同一网关 `/v1/models` 返回的模型目录。配置的默认模型是候选值，不是可用性证明。
2. `media_prepare` 必须匹配当前 route 的 gateway、profile、provider、model 和该 provider 支持的 operation。模型目录接口若只返回模型 ID 而不返回媒体能力，则目录只能证明 ID 存在，不能证明 operation 权限；在此情形使用经本地维护、明确标记来源的 capability map，或拒绝未经验证的 operation，不得推断。
3. 目录检查是本地读取，不发起生成请求。目录缺失、网关不同、超过规定 TTL、格式损坏均不能静默当作有效。
4. 不得自动把另一网关缓存带到当前网关。复用现有 `allow_gateway_mismatch=True` 仅允许 refresh 时保留用户指定候选值供交叉匹配；最终保存的目录必须关联规范化 gateway URL 和 profile。
5. `media_submit` 再次验证 confirmation 中的 route/model 仍与创建时一致；目录缺失/变化不能改写已确认模型。发现 route 已变化时拒绝提交，要求重新 prepare/confirm。

### 3.2 文件和代码职责

- `scripts/media-generation-mcp.py`：
  - 新增目录数据结构/读取校验 helper，返回 `verified_at`、规范化 gateway 指纹、profile、`catalog_ids`、非敏感来源信息与 schema 版本。
  - 修改 `_model`/`_validate_request`/`media_prepare`：精确检查目录 ID，拒绝大小写猜测/别名/前缀匹配；错误 `MODEL_NOT_AVAILABLE`，含 provider、requested model、目录更新时间、是否需刷新及经长度限制的候选 ID（不得含密钥/完整响应）。
  - freshness 默认 24 小时，可用常量覆盖测试时钟。TTL 到期返回 `MODEL_CATALOG_STALE`，提示先调用刷新；不得在 prepare 里隐藏网络 GET。
  - `media_submit` 从确认记录读取目录指纹并验证 route/model 未变；若无法证明一致则 `CONFIRMATION_STALE`，禁止 POST。
  - `media_capabilities` 区分 `configured_model`、`catalog_verified_model` 和 `generation_capability=not_tested`。
- `manifest/media-generation-tools.json` 与根/包两份 `manifest/mcp-servers.json`：记录目录刷新是只读网络访问、TTL、失败码和不代表 generation-ready。
- 涉及的媒体生成 skills：删改“可自由猜模型”式指导，先 `media_refresh_models`，再选精确返回 ID，保持用户确认流程。

### 3.3 阶段 1 验收

- 缓存里存在精确目录 model + 相同 gateway/profile：prepare 成功并固化目录指纹。
- 目录未刷新、时间超过 TTL、格式损坏、网关变更、目录 ID 不存在、provider 未支持：prepare 返回规定的结构错误；测试 fake server 断言无生成 POST。
- refresh 返回新 ID 后，旧配置模型不在目录：不继续选择旧模型；配置默认可置空或选确切允许的首选模型，不能把 ChatGPT 文本模型误当媒体模型。
- 缓存只有媒体 model ID、无 operation 元数据：不能由模型 ID 自动推断图像/视频/编辑能力。
- refresh 的 GET 返回 401/403/5xx/坏 JSON：不覆盖上一次有效记录为新鲜状态；capabilities 清楚标记目录失败/上一次成功时间。
- submit 时 route 或确认绑定不一致：0 次 POST，状态保持可恢复失败，用户须重新 prepare。

## 4. 阶段 2：统一结构化错误契约（P0）

### 4.1 错误类型

引入 `MediaError` 数据对象，至少包含：`code`、`safe_message`、`provider`、`model`、`http_status`（若存在）、`provider_request_id`、`gateway_request_id`、`retryable`、`submission_state`、`api_request_sent`。新增字段对 `None` 省略；不把完整请求头、URL query、prompt、base64、API Key、DPAPI 数据写到 error/log。

建议 code 映射：

| 条件 | code | retryable | submit 状态 |
|---|---|---:|---|
| provider 返回 400，确认内容政策特征 | `UPSTREAM_CONTENT_POLICY` | false | `submission_rejected` |
| provider 400 的其他校验拒绝 | `UPSTREAM_INPUT_REJECTED` | false | `submission_rejected` |
| 401/403 | `UPSTREAM_AUTH_FAILED` | false | `submission_rejected` |
| 404/明确模型不存在 | `UPSTREAM_MODEL_NOT_FOUND` | false | `submission_rejected` |
| 429 | 默认 `SUBMISSION_UNKNOWN`；只有 provider contract 明确证明请求在接受前被拒绝时才用 `UPSTREAM_RATE_LIMITED` | 默认 false；明确未接受时可 true，仍只允许用户确认后新建请求 | 默认 `submission_unknown`；有明确拒绝契约时 `submission_rejected` |
| 非 POST 查询的 5xx | `UPSTREAM_UNAVAILABLE` | true | 原任务状态不变 |
| POST 5xx/408/425、超时、连接断开或响应 JSON 无法解析 | `SUBMISSION_UNKNOWN` | false | `submission_unknown` |
| 本地 schema/模型/文件校验失败 | 精确本地 code | false | 未提交 |

错误类别识别只能使用明确 HTTP 状态和白名单字段（如标准 error.code/type）；不得从自由文本推断“内容政策”并对外暴露敏感原文。上游正文最大读取 8 KiB，JSON 解析失败也要返回安全的通用消息和 request IDs。`retryable=true` 只表示服务端契约能证明请求未被接受；它仅提示用户等待/修正后创建新的确认记录，绝不授权自动重试或复用已消费的 confirmation。`Retry-After` 只是建议等待时间，不是服务端拒绝/未计费的证明。429 没有明确 provider contract 时按提交未知处理，必须 `retryable=false`。

### 4.2 兼容工具返回

- 当前 MCP 可将工具错误作为 `CallToolResult.isError=true` 与 text 内容返回。升级后所有错误 text 必须是单行合法 JSON envelope，方便 Codex 稳定读取；若 MCP 框架支持 structuredContent，可同时写结构化内容，但 text JSON 保留兼容。
- envelope 至少形如：`{"success":false,"state":"submission_rejected","error":{"code":"UPSTREAM_CONTENT_POLICY","safe_message":"上游图像服务拒绝了本次请求。","http_status":400,"retryable":false},"provider":"image-2","model":"gpt-image-2.5","api_request_sent":true}`。
- stdout 仍只能写合法 JSON-RPC；调试记录写 stderr/脱敏日志，不改变现有 UTF-8 原始字节协议。
- provider 原始消息只保留经过脱敏、限长的字段；危险字段 key、Authorization、Bearer 字符串、API key 本文、reference URL query 全部屏蔽。测试用独特 canary secret 断言输出、stderr、状态文件和 zip 无泄漏。

### 4.3 准确错误映射验收

- fake server 返回 400 JSON、400 非 JSON、401、403、404、422、429（含 Retry-After）、500、502、503；验证 `http_status`、code、`retryable` 和 submission 状态。
- 模拟实际会话所述“上游 HTTP 400 但 UI 看见 502/503”的路径：从 fake HTTPError 进入 `_http_json`、提交持久化、tool dispatcher、最终 JSON-RPC。若实现只产生 `UPSTREAM_INPUT_REJECTED`/400，则报告中的旧映射不能宣称重现；记录为外层客户端展示问题或未复现。
- 对每个响应携带不同测试 Request-ID，验证映射字段保留；无 ID 时为空，不自行伪造 provider request ID。
- JSON 错误正文含 canary token / prompt / URL query 时，所有用户输出与日志必须脱敏；不含完整上游正文时仍保留状态、白名单错误类型、关联 ID。
- POST 400 被明确拒绝后状态是 rejected；POST 429 fixture 分别验证“默认/未知接受语义→unknown + retryable=false”和“provider contract 明确保证接受前拒绝→rejected + retryable=true”。POST 5xx/408/425 仍按可能已接受处理。不得因认为 429 可重试或收到 Retry-After 就自动再次 POST。

## 5. 阶段 3：未知提交的安全恢复（P1）

### 5.1 状态和持久化

当前本地确认记录已保存 confirmation、请求摘要、`submitting`、`api_request_sent`、提交时间和响应摘要；不要重造持久层。需补齐：

- 在第一次网络 I/O 之前持久化 `submission_id`（本地随机 UUID）、`provider`、`model`、gateway/profile 指纹、规范化请求的 HMAC/SHA-256 摘要、幂等键摘要、是否发起网络请求和时间戳。
- 本地 `submission_id` 是本地跟踪标识，不等于上游幂等保证。明文幂等 key 只在进程内使用，不写入磁盘、错误、诊断包或日志。现有 `_secret_key()` 只用于 confirmation HMAC；provider 幂等键输入和稳定性须单独定义且不可暴露。
- 新增固定的 provider contract 表，明确每个 provider/profile 是否正式支持 `Idempotency-Key`、header 名、值的稳定范围、冲突响应和重复请求语义。当前 `_http_json` 的条件式 header 不能单独证明上游支持。只有有合同/可审计 fixture 的项才转发并在状态中记录 `idempotency_forwarded=true`；否则只依赖本地 single-use 状态机，并记录 `false`。不得声称本地 idempotency hash 能阻止上游重复计费。
- 如果原子写入失败，网络请求不得发出。
- 记录任何未知结果均进入终态 `submission_unknown`，禁止 `media_submit` 再次 POST；包括服务进程重启后重新调用同一 confirmation。

### 5.1.1 同一生成意图的未决提交保护

- 仅按 confirmation ID 去重不够：调用方可能在未知结果后再次 `media_prepare`，拿到新 confirmation ID 并对同一生成意图提交。必须跨确认记录保护。
- 生成 `intent_fingerprint = HMAC(_secret_key(), canonical_json({gateway_fingerprint, profile, provider, model, operation, normalized_parameters, prompt, reference_hashes_or_full_reference_url}))`。不把 prompt 原文、URL 或 API key 放进索引；只持久化 HMAC 指纹。规范化 JSON 使用固定 UTF-8、固定键排序和紧凑分隔符。URL 的完整规范值（包括 query）只参与进程内 HMAC 计算，以区分不同签名素材链接；禁止把它写入意图索引、日志或诊断结果。不得通过剥除 query 把不同素材误判成同一意图。
- **原子预留**：新增每个 `dataRoot` 唯一的跨进程索引互斥锁，Windows 命名为 `Local\\CodexMediaIntent-v1-<SHA256(canonical_dataRoot)>`，保证同一交互登录会话的多个 Codex/MCP 进程竞争同一锁。获取锁最多等待 10 秒，超时返回 `INTENT_LOCK_TIMEOUT` 并 fail closed；遇到 abandoned mutex 视为已取得锁，必须运行崩溃恢复扫描。持锁范围覆盖“扫描/恢复同意图记录→决定是否允许→写入新 confirmation”全过程。必须在写 confirmation 文件成功后才释放；写失败则返回错误、不产生可提交记录。不得用进程内 `threading.Lock` 代替。
- **锁顺序**：所有需要同时拿锁的路径统一按 `intent-index mutex → confirmation-operation mutex` 获取，释放反向进行。不得有路径先持有 confirmation 锁再等待 intent-index 锁。网络请求期间不持有全局 intent 锁；提交开始和结果状态迁移都在释放网络锁前将状态原子落盘，后续 prepare 扫描可据此阻断。
- **提交状态事务**：submit 开始时持两锁、重读 confirmation，并要求 `state=prepared`、未过期、无活动 operation marker、意图预留指向该 ID；只有一次原子 compare-and-swap `prepared → submitting` 能成功。获胜者生成 operation token 并原子写 `api_request_sent=false` 后释放两锁；其他并发调用必须返回 `SUBMISSION_ALREADY_STARTED`，不得更换 token、改写状态或发请求。HTTP 前 callback 再按相同顺序拿锁，只在记录仍为 `submitting`、token 仍是当前赢家且请求未标记时 CAS 写 `api_request_sent=true`；验证失败返回错误并且零 POST。回调落盘成功、释放锁后才允许打开连接。响应/异常后再次按同顺序持锁，仅当前 token 所有者可提交终态。落盘失败不允许进入下一网络步骤；旧进程迟到结果不得覆盖新状态。
- **查询/保存事务**：`media_check` 与 `media_save` 不能只靠 per-confirmation mutex 保护而绕过 intent 锁。启动网络操作前，在两锁内重新验证 task/状态，并要求 `operation_state` 为空；然后写一个独占 token 与 `operation_state=checking|saving`，再释放锁做 GET/下载。任何活动标记都阻止任何另一个 check/save（包括另一种操作）和 submit；返回 `OPERATION_IN_PROGRESS`，不发重复请求、不覆盖 token。结束时按同顺序持锁，仅 token 所有者能提交结果并清除 marker。启动恢复时，checking marker 可安全清除并重试只读 GET；saving marker 可清理 `.part` 后恢复 `completed_pending_save`，然后只重试结果下载。保存失败不得退回生成态；`media_save` 不得在任务未 completed 时开始。
- **占用状态**：扫描状态为 `prepared` 且未过期、`submitting`、`task_created`、`in_progress`、`submission_unknown`、`completed_pending_save` 的同指纹记录都占用该意图。相同意图的第二次 prepare 不创建新 confirmation：`prepared` 返回 `INTENT_ALREADY_PREPARED` 和原 confirmation ID；提交中/有 task/unknown 返回 `UNRESOLVED_PRIOR_SUBMISSION` 和 submission/task ID、时间、可能已计费信息。用户可继续使用原 confirmation 的既有任务流程。
- **崩溃恢复**：持锁扫描发现过期 `submitting`，若 `api_request_sent=true` 或字段缺失/损坏，按 `submission_unknown` 处理并锁住意图；若完整记录证明 `api_request_sent=false`，可标记为 `interrupted_before_send` 并释放占用。处于 `prepared` 且过期的记录，只有能验证 HMAC、确认状态未变且 `api_request_sent=false` 才可转为 `expired` 并释放。损坏状态一律 fail closed。
- `media_prepare` 在写入新 confirmation 前扫描全部有效/未决记录，并按上述状态表判定占用。命中 `submission_unknown` 时返回 `UNRESOLVED_PRIOR_SUBMISSION`，不创建新 confirmation、不发网络请求；返回先前 submission ID、创建时间、脱敏 request ID 和“可能已计费”提示。
- 指纹记录带 `fingerprint_key_id` 和 schema version。复用当前稳定保存的本地 HMAC secret，不做自动轮换；若指纹密钥丢失、损坏、版本未知，或无法比较某条 unknown 记录，prepare 必须返回 `UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE` 并阻止提交，直到恢复该索引或用户明确执行人工归档确认。不得因 key 缺失自动创建新 key 后把旧记录视为无关。
- `submission_unknown` 只有被受支持的 reconciliation 正式解决为 completed/failed/not_found、或按用户明确操作归档后才不再阻止同意图。时间经过或重新启动不能自动清除阻断。
- 用户确实要再次付费生成时，必须先看到未决提示并明确授权。扩展 `media_prepare` 参数 `repeat_after_unknown_ack`：无字段时一律拒绝；只有精确等于 `确认此前任务可能已计费且仍要求新生成` 才能新建确认。图片/视频 skill 必须先向用户展示未决 ID 和可能重复计费，再等待用户明确回复该句的同义确认，不能由 skill 默认为用户填入此字段。
- 接受显式复做后，在 intent-index mutex 内以 compare-and-swap 方式消费旧 unknown 的一次性复做授权，并创建唯一的新 `prepared` 记录；新 confirmation 记录 `supersedes_unknown_submission_ids`、接受的旧 submission ID、确认时间和 acknowledgement 标志；不得修改/删除旧记录。并发 ACK 只能有一个获准，其余返回 `INTENT_ALREADY_PREPARED`。该字段不是“旧请求未成功”的证明。
- 不同 intent fingerprint 可继续准备；已明确 rejected/failed/completed 的记录不构成 unknown 拦截。用户明确修改 prompt/参数会得到新 fingerprint，但仍须经过普通 prepare + submit 确认。

### 5.2 对账能力开关

- 先建立 provider contract 表（固定 URL/method/查询键/鉴权范围/响应状态映射/重复计费说明/文档或 fixture 来源）。默认所有 provider `reconciliation_supported=false`。
- 仅当官方/网关合同或可审计接口定义明确支持以 `task_id`、provider request ID 或幂等键读取任务时，才实现 `media_reconcile`；没有正式接口就保持不支持。不得用 `/v1/models`、猜测 `/tasks/{id}` 或幂等 POST 来冒充查询。
- 若新增工具，`media_reconcile` 必须只允许 GET，接受 confirmation/submission ID，不接受新的 prompt/生成参数；只能查同一已知 submission。状态包括 `accepted/task_created`、`completed`、`failed`、`not_found`、`still_unknown`。`not_found` 只有上游明确提供强一致性/终态语义时才可用于用户选择新任务；默认不允许自动重试。
- 对账查询无接口/失败/无确定答案时，返回 `SUBMISSION_UNKNOWN`、可能已计费提醒、最后提交时间与脱敏 request IDs；不得保证“未计费”。
- 现有 `media_check` 仍只查询已确认 task_id 的任务；不能把未知提交自动转成它支持的 task ID。

### 5.3 阶段 3 验收

- 注入在 DNS、连接、写入请求、上游接受后丢响应、非 JSON 响应、写记录失败、进程崩溃等故障点；检查状态记录和发出的 HTTP 请求数。
- 每一条 unknown 路径再次调用 submit，发出 POST 总数仍为 1；重启进程后同样成立。
- unknown 后以相同 gateway/profile/provider/model/operation/参数/prompt/参考 hash 创建第二份 prepare：无 ACK 时拒绝且不创建 confirmation；提供精确 acknowledgement 时创建一条关联旧记录的新 confirmation，但仍不发 POST，直到新一轮显式确认；另一意图正常准备；旧任务已解决后相同意图可新建。
- 两个独立进程 barrier 同时对完全相同意图调用 prepare：只能一个创建 confirmation，另一个拿到 `INTENT_ALREADY_PREPARED`；第二进程必须能读取第一个进程的持久状态。
- 在第一次 prepare 写文件前后、submit 的 `prepared→submitting` 落盘前后、`submitting`→request_started、响应处理前后注入终止/异常；重启后不可出现两个同意图活跃 confirmation，`api_request_sent` 不确定时必须锁为 unknown。
- 两个进程同时 submit 同一个 confirmation、两个 confirmation 尝试同一 intent、submit 与 prepare/check/save 交错；验证两锁顺序无死锁、活跃/unknown 状态始终唯一占用，且生成 POST 最多一次。
- 对 check/save 的 GET/下载并发与进程崩溃注入：marker 不造成重复写或错误终态；重启只可重试只读 GET/原结果下载，绝不重发 generation POST。
- 对同一个 confirmation 并发调用两次 submit：只有首个 `prepared→submitting` CAS 成功，第二个返回 already started 且无第二次 POST；首个 request callback token/state 校验失败时两次 POST 均为 0。迟到响应不能覆盖新 token/状态。
- 同一 confirmation 上 check/check、save/save、check/save、save/check 并发：单 marker 只能被一个操作持有，其余均返回 `OPERATION_IN_PROGRESS` 且不发网络请求；不同 task 的操作可并行但各自持不同 confirmation token。
- 并发重复 ACK：最多一个新 confirmation 获准，旧 unknown 的一次性授权只被消费一次，其他调用不能绕过预留。
- 相同 URL 但不同 query 的参考素材生成不同 HMAC 指纹；工具输出、索引和日志仍不暴露 query。
- 删除/损坏指纹 key、未知 key ID、损坏 confirmation 或旧 schema：不能静默忽略未决记录或放行同意图；返回 index unavailable，且没有 POST。
- 构造支持与不支持对账的 provider fixture；不支持时明确无 GET 路径或上游猜测；支持时只能 GET 现存 ID，且重复 reconcile 不产生 POST。
- 对 provider contract 声明支持和不支持幂等的 fixture，验证 header 是否按 contract 发送、稳定性是否匹配，且本地状态不会把未认证的上游幂等能力宣传成保证。
- 校验磁盘记录含 submission ID、request digest、状态、时间、可能计费字段与 IDs，但不包含 prompt 全文（除当前 confirmation 必需记录外，不复制）、key、明文幂等 key。

## 6. 阶段 4：任务状态与媒体交付闭环（P1）

### 6.1 统一状态

每个工具成功或失败都按稳定 envelope 返回：`success`、`state`、`provider`、`model`、`confirmation_id`、`submission_id`、`task_id`（可空）、`api_request_sent`、`artifacts`、`saved`、`error`（失败时）及 `timestamps`。现有工具字段可以保留为兼容别名，但不得让错误只能从自然语言推断。

状态明确区分：`prepared`、`submitting`、`submission_rejected`、`submission_unknown`、`task_created`、`in_progress`、`completed_pending_save`、`completed_saved`、`failed`、`completed_without_artifact`、`expired`。对已有历史记录 schema 做向后兼容读取；只在下一次成功写入时迁移，保留未知字段。迁移失败则只读报错，不能重发。

### 6.2 完成状态

- provider status 归一化必须显式列出成功、执行中、失败、取消、过期和未知映射；未知字符串不作为成功。
- provider 说 completed 但没有可识别 artifact，变为 `completed_without_artifact` 或 `result_unavailable`，不可继续无限查询，也不声称生成并交付成功。
- 图片同步 Responses、同步图片 API 与异步视频完成后，均将 artifact 保存状态返回清楚。是否自动保存由本地媒体类型/路径契约决定；最小可接受行为是 `completed_pending_save` 明确提示下一步 `media_save`，不能返回含糊的 `completed` 且 `saved=[]`。
- 确认记录不得在保存前清理；重启后可通过 confirmation ID 继续保存。

### 6.3 安全下载及本地完整性

- 沿用现有 HTTPS/公网地址校验、禁重定向、最大 512 MiB、临时 `.part` 原子替换和仅对完全匹配网关 origin 附加 Bearer 的逻辑。任何修复不得放宽授权传播。
- 远端下载结果应验证与 artifact kind 对应的 MIME、扩展名与 magic bytes；视频至少检查允许格式签名和容器头，不声称做完整编解码器验证。响应 MIME 与文件类型冲突时返回 `ARTIFACT_TYPE_MISMATCH`，删除临时文件。
- 保存记录增加 MIME、字节数、SHA-256、保存时间和来源类型（base64/URL）；敏感 URL query 不保留在工具响应或记录。
- 如果目标文件已存在，验证现有文件 hash 与记录匹配才复用；不匹配时拒绝覆盖，写入新唯一目标或返回 `SAVED_ARTIFACT_TAMPERED`。不得仅凭文件存在就认定已经下载完整。
- 保存失败不回滚远端 `completed` 状态；将交付状态保留为 pending，允许只调用 `media_save` 重试下载，不允许重发生成。

### 6.4 阶段 4 验收

- 对同步 image-2 Responses、Wokey image、异步 Grok/Seedance 视频覆盖 submit→poll→save 生命周期；成功且未保存要显式 pending，保存后路径、MIME、大小、SHA-256齐全。
- 假服务器模拟空 body、截断 body、伪 PNG/JPEG/MP4、MIME 错配、超限、重定向、跨域下载 URL；断言不会把错误文件标为 saved，不泄露 Bearer 到非网关 host。
- 保存后改写磁盘文件，再次 save 必须发现 hash 不一致；同内容重复 save 为幂等；保存网络失败后再次 save 只 GET 原 artifact URL，不 POST。
- `completed_without_artifact` 有稳定终态和用户可理解的修复指引。

## 7. 阶段 5：参数与轮询边界（P2）

- 将当前 `_validate_request` 的 provider 分支提炼为 provider/model/operation 的 capability schema，包含 operation、引用素材类型/数量、参数名/类型/枚举/上下限/默认值，以及网关 profile 限制。
- 先按已验证 model ID 选 schema；缺省时只应用明确默认参数；未知参数拒绝并指出字段，不默默丢弃。当前不同 provider 的已知限制不得在重构中放宽。
- 若远端模型目录不携带能力数据，schema 来源标记 `local_adapter_contract`，并在 capabilities 显示“适配器已知，不代表上游授权”。变更 schema 必须有 fixture/文档依据。
- 参考文件的准备 hash 检查保持；提交前再次校验。查询和保存已提交任务不得要求参考文件仍存在。
- `media_submit` 返回配置化 `poll_after_seconds`；`media_check` 对 in-progress 指数退避建议并设最大值。MCP 不在一次工具调用里阻塞长轮询；Codex/skill 按建议间隔单独调用。
- 对查询状态增加最大任务保留/过期策略，仅本地标记 stale，不删除可追溯的错误与提交记录；过期后仍可按已知 task ID 手动检查，不能创建新任务。

### 阶段 5 验收

- 每个 provider/profile/model operation 的参数正例、缺省值、边界值、非法类型、未知参数、越界 duration、无效 aspect ratio、过多/错误 MIME reference 均覆盖。
- 未知 provider status 返回 `provider_status_unknown`，不会被误标 in_progress 或 completed；轮询建议有上下限和可测试时钟。
- 重新打开 confirmation 查询/保存成功，即使参考文件已被移动；只有新提交和原始确认完整性核验才要求文件存在。

## 8. 代码文件清单和职责

| 文件 | 必做内容 |
|---|---|
| `scripts/media-generation-mcp.py` | 所有状态、目录、错误、对账开关、参数、下载/落盘核心实现；保持 stdout JSON-RPC 协议，不含凭据日志。 |
| `scripts/start-media-generation-mcp.ps1` | 仅负责 Python/env/凭据适配与健康检查；版本 1.3.5；不得写入 Codex TOML 明文 key。 |
| `manifest/media-generation-tools.json` | 工具参数/付费属性/错误与状态契约、目录 freshness、支持的对账状态和健康分层。 |
| `manifest/mcp-servers.json` | media-generation 节点版本、健康预期、工具/超时描述；保留无关节点。 |
| `config-fragments/media-generation-stdio.template.toml` | 维持 UTF-8 wrapper/stdin 约束、1500 秒外部工具超时、媒体 submit 要求 prompt approval。 |
| `skills/image-2/SKILL.md`、`skills/seedance2-0-video-gen/SKILL.md`、`skills/grok-video-prompt-compiler/SKILL.md` | 按流程刷新目录、精确选模型、解释 unknown 和 pending、不可自动重复提交。只改与媒体契约有关的指引。 |
| `scripts/test-media-generation-mcp.py`（新增专项） | 标准库 unittest + fake HTTP server + 临时 HOME/config/data dir；不使用真实 key/provider。 |
| `scripts/build-pack.py` | 把新设计/来源记录纳入 docs 镜像列表；构建前检查源/包 MCP、wrapper、清单和版本相同。不要通过全量构建测试来改写未审的用户变动。 |
| `docs/MEDIA-MCP-INTERMITTENT-FAILURE-REPAIR-PLAN-20261015.md` | 本规格及实施记录；实施完成后追加 commit、测试命令、结果与未验收项，不抹去规格。 |
| `docs/source-fidelity-media-mcp-1.3.4-20261015.md` | 保留来源 SHA、目标映射和凭据差异记录；新来源同步后追加新条目，不能覆盖历史。 |

根目录是编辑源，`installer-pack/` 是发布镜像。每次代码变更后必须镜像同一内容；只有模板需按 Kit 需求微调时，须在文档中列出差异和验收，不能静默漂移。

## 9. 专项自动化测试清单

1. **模型目录**：精确 ID 命中、模型不在目录、目录为空、过期、网关切换、刷新失败不覆盖旧快照、provider-operation 不支持、提交时确认指纹漂移。
2. **错误 envelope**：400 policy/input、401/403、404/model missing、422、429 有/无明确未接受 contract、500/502/503、超时、连接拒绝、坏 UTF-8/JSON、过大 body；字段稳定、状态持久化正确。
3. **请求计数与 unknown**：before_request 标记、post 已发/未发、响应丢失、重启恢复、重复调用；unknown 每个 confirmation 永远不超过一个 POST。
4. **错误和日志脱敏**：API key canary、prompt canary、URL query canary、DPAPI/authorization 内容；不进入 stdout 错误、stderr、日志、状态文件、manifest、ZIP。
5. **异步状态**：queued/in_progress/completed/succeeded/done/failed/cancelled/expired/unknown、完成无 artifact、错误内含 request id。
6. **落盘**：base64 PNG/JPEG 正误 fixture；URL PNG/JPEG/MP4/WEBM；文件头、MIME、长度、哈希、临时文件清理、重复保存和本地篡改。
7. **网络边界**：私网/localhost/非 HTTPS/重定向/跨 origin；外域下载不含 Authorization，API host 的 Bearer 仅对严格 origin 匹配发送。
8. **MCP 协议**：模拟 Windows 中文/非 UTF-8 locale，initialize、tools/list、工具错误和健康检查 stdout 无额外文本；stdio EOF 正常退出。
9. **镜像和包**：root 与 installer-pack 的服务源码 hash、版本、工具定义、wrapper expectation、manifest、模板配置一致；解压到随机临时目录从包内启动；manifest/checksums 验证通过；zip 不含测试 key。

测试必须报告每条断言，不可把“unittest 启动成功”视为通过。回归先跑原有 MCP 契约测试，再跑新专项；检查 Python 编译、PowerShell parser、Git diff whitespace。禁止使用真实 provider 进行自动化。

## 10. 发布前验收与证据格式

开发者提交以下可重放材料：

1. 固定提交 SHA、工作树状态、源/包双方版本和文件 hash 表。
2. 本节第 9 组测试逐项 pass/fail、命令、测试环境，不附 key 或用户素材。
3. 离线安装包解压测试记录：实际解压路径、wrapper `-HealthCheck` 的脱敏 JSON、MCP initialize/tools/list 结果；该结论只能标记 `local_runtime_protocol=passed`。
4. gateway catalog GET 仅在有测试凭据授权时单独测，标签 `gateway_catalog=passed`，不自动标成 `generation_capability=passed`。
5. 真实图片/视频 smoke test 仅由用户明确授权后执行，输出模型、provider、request id、耗时、产物 hash 和状态；不要把 API key/完整 prompt 放入报告。没有该测试时 `generation_capability=not_tested`。
6. 检查复制出来的正式 ZIP/EXE 的包内文件 hash 与发布 manifest 对应，并做 secrets scan。

### 最终通过标准

- 对未目录验证/目录失效模型，在任何生成请求前拦截。
- HTTP 状态和安全错误类别可稳定区分，原始请求 ID 可追溯，密钥永不进入日志/包。
- POST 状态未知时一次提交最多一笔 POST；能查询只限有正式契约证明的 provider，其他情况如实标 unknown。
- 任务成功与本地保存状态分开报告；用户能恢复已完成任务的结果下载，损坏结果不会显示保存成功。
- 所有根/包镜像一致，Starter-kit wrapper 凭据边界未改变，离线协议健康通过。
- 没有经过授权的真实生成时，报告明确写“本地实现验收通过，真实上游生成未测”，不得写“所有媒体能力已证明可用”。

## 11. 开发者执行清单

- [ ] 验证来源保真记录的基线和 hash，工作树外的未提交文件没有被更改。
- [ ] 完成阶段 1 并运行模型目录/无 POST 测试。
- [ ] 完成阶段 2 并运行 HTTP fixture、最终 JSON-RPC 和脱敏测试；核实报告中的 400→502/503 是否可复现。
- [ ] 完成阶段 3：同意图 unknown 跨 confirmation 去重；先为每个上游填写对账/幂等合同，无合同就实现明确“不支持”状态，绝不猜 endpoint。
- [ ] 完成阶段 4 状态机、保存完整性与可恢复性测试。
- [ ] 完成阶段 5 schema 与轮询测试，保留当前 provider 参数约束。
- [ ] 提升全链路版本至 1.3.5，更新 root/package 镜像和构建映射。
- [ ] 在隔离干净工作树执行 builder 和 package verifier；核对包哈希与无密钥扫描。不能在含有大量用户未提交更改的工作区直接盲跑 builder。
- [ ] 由非实现者独立审计规格映射、回归覆盖及构建证据；所有 P0/P1 必须关闭后才可申请用户验收。

## 12. 实施记录与当前验收状态

### 实施基线

- 实施基线提交：`1ee1a9640175710fff52a3c98b0e6cf9fe459c09`。该工作区在实施前已有广泛未提交修改；本次只对媒体 MCP、对应测试/说明及其安装包镜像进行定点编辑，未重建或清理整个 `installer-pack/`。
- 当前媒体 MCP 版本：`1.3.5`。根目录与发布镜像的服务端、wrapper、测试、媒体工具清单、MCP registry、UTF-8 配置片段、来源记录和相关 skills 已同步；发布包 `file-manifest.json` 与 `checksums.sha256` 已按实际包内容重算。
- 当前源码树 SHA-256 记录在 `installer-pack/pack.json`；`python scripts/build-pack.py --verify` 返回 `verified=true`。此操作只验证来源元数据，没有删除/重建包。

### 已落地的行为

- 提交前要求 provider 明确配置模型、模型目录在 24 小时有效期内且网关/profile 指纹匹配；模型目录只由显式刷新取得，目录刷新失败保留上次已验证快照。
- 上游响应映射为稳定错误码、HTTP 状态、脱敏 request ID 和重试语义；POST 响应不明时保留 `submission_unknown`，不转发未经验证的幂等头，也不自动重发。
- 以全局意图互斥和持久化 HMAC 指纹阻止相同意图跨 confirmation 重复提交；提交前再次校验意图预留，历史重复 prepared 单据由本次明确提交者原子胜出并将其余单据标为 superseded；未知提交的重复请求要求一次明确确认，且不声称已对账或已取消。
- `submitting` 状态迁移、请求开始标记和跨 confirmation 扫描/恢复使用同一全局互斥；扫描到仍持有 confirmation 操作锁的活动提交时拒绝新准备，不把运行中的请求误判为可重复。
- 参考素材及 provider 结果中的签名 URL，完整内容仅在内存解密/使用；Windows 持久化采用当前用户 DPAPI。工具结果只返回 origin，不返回签名 query。非 Windows 测试模式使用显式标识的测试保护 envelope，正式非 Windows 环境对此 fail closed。
- 查询/保存状态区分上游完成、产物待保存、已保存、完成但无产物等结果；下载校验 origin、拒绝重定向、验证 MIME/文件签名/大小/hash，并保留可恢复保存状态。
- 继续使用 Starter Kit 原有 wrapper、进程环境与当前用户 DPAPI 凭据回退方式；不把 key 写入 Codex TOML、普通连接 JSON、诊断输出或包内文本。

### 最新补充修复

- 独立审计发现旧版本可能留有同一意图的多份有效 `prepared` 确认单；提交时现重新扫描并验证唯一预留。若仅存在重复 `prepared` 单据，本次用户明确提交的确认单原子胜出，其他单据写为 `superseded`；混有未知/正在处理等状态时仍失败关闭。新增回归证明最多一个 POST。
- provider 结果中的签名 artifact URL 现使用 DPAPI 加密落盘；提交、查询响应只给 URL origin，保存时才在内存解密。新增 canary 覆盖状态文件、submit/check 工具响应和恢复下载路径。

### 已执行验证（历史快照，已由第 13–14 节更新）

下表记录最初 1.3.5 实施时的验收收据；它没有覆盖随后发现的 Wokey 源/包目录请求差异。请以第 14 节的 29 项双份测试、当前包自校验结果和全局来源验证状态为准。下表中的 28 项与 `build-pack.py --verify` PASS 不代表当前整包状态。

| 检查 | 结果 | 范围 |
|---|---|---|
| `python -m py_compile scripts/media-generation-mcp.py scripts/test-media-generation-mcp.py scripts/test-adapter-contracts.py` | PASS | Python 语法，不代表上游可用 |
| `python scripts/test-media-generation-mcp.py` | PASS，28/28 | 假网络/临时目录/状态机与协议用例；含双进程 prepare、submit/prepare 交错、历史重复确认单、参考 URL 与结果 URL 的 DPAPI 脱敏，不产生 provider 请求 |
| `python scripts/test-adapter-contracts.py` | PASS，52/52 | Starter Kit 当前离线契约总套件 |
| `python scripts/media-generation-mcp.py --healthcheck` | PASS | CPython 3.12.10；本地 runtime/protocol；gateway catalog `not_checked`，generation capability `not_tested` |
| `python scripts/build-pack.py --verify` | PASS，`verified=true` | 包来源树指纹；未运行会重建包的 `--check` |
| 根目录与包内媒体文件镜像、manifest/checksums | PASS | 独立来源保真审计通过；310 个 manifest 条目和 311 个 checksum 条目均匹配，`build-pack.py --verify` 通过 |

### 尚未验收

- 没有真实 GET `/v1/models`、真实图片生成或真实视频生成；因此 `gatewayCatalog` 与 `generationCapability` 的真实服务状态仍未测试。
- 尚未用目标 Codex 安装器完整安装到干净 Windows 用户目录，也未用实体 U 盘冷启动 Codex 并调用 MCP。此项需要用户选择模型/场景并明确授权；会产生上游费用，不属于自动离线测试。
- 独立代码审计已 PASS；此结论只覆盖固定源码和发布包一致性，不代表目标 Codex 安装器或实体 U 盘已实测。任何真实 smoke test 另行记录 provider、model、状态、request ID、耗时和产物 hash，报告中不写 key 或完整 prompt。

## 13. 补充发布审计：Wokey 媒体模型目录请求未同步进安装包

### 发现

2026-10-15 对当前 starter-kit 根目录和 `installer-pack/` 做逐文件比对时发现：根目录 `scripts/media-generation-mcp.py` 的 `media_refresh_models` 已针对 `api.wokey.ai` 请求 `/v1/models?output_modalities=all`，并有 `test_wokey_catalog_refresh_includes_media_models` 回归；发布包 `installer-pack/mcp/media-generation-mcp.py` 仍请求不带参数的 `/v1/models`，包内测试也缺少该 Wokey 用例。两边仍标为 1.3.5，但 SHA-256 不同。

Wokey 默认模型目录可能只列文本模型。媒体 MCP 又要求显式配置模型命中当前目录才允许 `media_prepare`，因此开发目录的修复没有进入实际安装包时，图片/视频模型会在目录发现或准备阶段缺席。该缺口可解释“统一网关直连能看到/使用媒体模型，而安装后的 MCP 模型目录没有它们”的现象。它不证明生成权限已开通；目录命中只证明 ID 可见。

### 本轮冻结的最小修复

- 根目录是编辑源；不更改其已存在的 Wokey 分支逻辑和回归断言。
- 将 `scripts/media-generation-mcp.py` 字节同步到 `installer-pack/mcp/media-generation-mcp.py`，将 `scripts/test-media-generation-mcp.py` 同步到 `installer-pack/test-media-generation-mcp.py`。
- 重新计算 `installer-pack/file-manifest.json` 与 `installer-pack/checksums.sha256`，只更新本轮实际改动文件的包内 manifest/checksum 条目；不改包 ID、版本、默认安装列表或其他包内容。
- **发布来源指纹保护**：当前工作树存在大量与本轮无关的修改、删除和未跟踪文件。不得为了让 `scripts/build-pack.py --verify` 通过而把整棵当前工作树的 SHA 写入 `installer-pack/pack.json`。该字段描述完整 source tree；只有先完成所有 source→pack 映射的全量一致性审计，并能证明包确实由该完整来源树构成，才允许更新。否则保留原 `sourceTreeSha256`，把全包来源验证标为 blocked/stale，并明确这不等于定点文件镜像或包内 checksum 失败。
- 实施前后保存 `git status --short`、本节涉及文件 hash 和包元数据快照；最终审计确认除本节列明的服务、专项测试、包内文档及 `file-manifest.json`/`checksums.sha256` 外，既有文件均未被本轮改动。不得执行会删除重建整个 `installer-pack/` 的 builder。
- 更新安装包副本中的本开发文档，记录本节发现、修改和验证结果。
- 禁止运行会先删除并重建整个 `installer-pack/` 的 `scripts/build-pack.py` 普通构建流程；本轮工作区有大量既存修改。应复用 builder 的规范化、canonical hash 和 manifest helper 做定点镜像/元数据重算，并逐项检查最终 diff。

### 必须验收

1. 根目录与安装包服务文件 SHA-256 完全相同；根目录与安装包媒体专项测试文件 SHA-256 完全相同。
2. 根目录与安装包各自独立运行媒体专项离线测试；Wokey fixture 必须断言 query 精确为 `output_modalities=all`、三个媒体模型映射正确且生成 POST 数为 0；非 Wokey fixture 必须断言 query 为空。
3. 运行语法检查、starter-kit adapter contract tests、包内 `file-manifest.json`/`checksums.sha256` 验证；运行只读 `build-pack.py --verify` 并如实记录结果。若它仅因既有混合工作树的全局 `sourceTreeSha256` 不匹配而失败，保留原指纹并将全包来源验证标为 blocked/stale，不通过改写全局指纹掩盖；不得运行真实生成请求作为自动化测试。
4. 将包解压到随机临时目录，从包内 MCP 服务完成 local health/protocol 初始化；报告只能标记本地协议可用，不得标记真实生图/视频 ready。
5. 独立代码审计确认只有上述定点源/包镜像、对应包 manifest/checksum 条目和本节文档发生变化；`installer-pack/pack.json` 的全局来源指纹保持原值，除非全量来源映射审计另行证明可安全更新。已有非媒体文件修改保持不动。

### 当前发布门状态

发现此差异之前的 1.3.5 离线测试和审计结论仍有效，但它们没有证明根目录与包内 MCP 相同。本节补齐并完成两份副本的独立测试与 hash 验证后，才可认定 starter-kit 安装包包含 Wokey 媒体模型目录修复。全局 `build-pack.py --verify` 若因既存来源树漂移失败，则完整来源溯源仍未验收，不能由定点 package self-integrity 代替。真实图片/视频 smoke test 仍单独记录，不把模型目录刷新成功冒充生成成功。

## 14. 定点修复实施记录（2026-10-15）

本节记录 10 月 15 日的离线实施快照。后续实际网关目录复测见第 15 节。

### 变更

- 将根目录 `scripts/media-generation-mcp.py` 定点复制到 `installer-pack/mcp/media-generation-mcp.py`；两者 SHA-256 均为 `5ef6fc74d6e8d6a17e1159007d13f1e29e50f24d465450120c8164ed45497a96`。
- 将根目录 `scripts/test-media-generation-mcp.py` 定点复制到 `installer-pack/test-media-generation-mcp.py`；两者 SHA-256 均为 `1100a28332cfa00ce6a710a6f2fe802906e0776dde478ab62585f8869c466d21`。包内新增 Wokey 精确查询参数回归，根/包专项测试现在同为 29 项。
- 更新包内本文件副本以及包 `file-manifest.json`、`checksums.sha256`。未改 `installer-pack/pack.json` 的 `sourceTreeSha256`、版本、包 ID、安装清单或其他来源文件。

### 验证

| 检查 | 结果 | 说明 |
|---|---|---|
| 根目录媒体专项测试 | PASS，29/29 | `python -B scripts/test-media-generation-mcp.py`；使用假网关与临时状态，不调用 provider。 |
| 安装包媒体专项测试 | PASS，29/29 | `python -B test-media-generation-mcp.py`（工作目录 `installer-pack`）；证明发行镜像含同一 Wokey 回归。 |
| Python 语法 | PASS | 根/包 MCP 服务、根/包专项测试及 adapter 测试 `py_compile`。 |
| Adapter contract suite | PASS，52/52 | 初次运行受 Windows 沙箱默认 Temp ACL 拒绝；设置 `TEMP`/`TMP` 到工作区新建临时目录后全套通过。 |
| 根/包 healthcheck | PASS | 两份均报告 `runtimeStatus=passed`、`protocolStatus=passed`、版本 `1.3.5`、6 工具；`gatewayCatalogStatus=not_checked`、`generationCapability=not_tested`。 |
| 临时目录 stdio MCP 协议 | PASS | 从包内服务副本启动；`initialize`、`tools/list` 均为合法 JSON-RPC，6 工具，stdout 仅两条协议消息、stderr 为空。 |
| `file-manifest.json` / `checksums.sha256` | PASS | 310 个 manifest 文件项、311 个 checksum 项均逐项与实际包文件匹配；根/包镜像字节一致。 |
| `scripts/build-pack.py --verify` | BLOCKED（既存全局指纹漂移） | 实测仍报告 pack 的原指纹 `53e570…` 与当前来源树指纹不一致。工作区含大量既有修改/删除/未跟踪文件；按 Section 13 保留原 `sourceTreeSha256`，不以混合工作树覆写来源身份。该结果不代表包内 manifest/checksum 失败。 |
| 真实 gateway 目录、图片/视频生成 | 见第 15 节 | 本表为 10 月 15 日快照；下一日已执行真实只读目录 GET，图片/视频生成仍未提交。 |

### 范围说明

本轮只对 MCP 服务、其专项测试、两份开发文档和包内清单/校验和做定点变更。`installer-pack/pack.json` 全局来源指纹和其余预存工作区内容保持不动。独立代码审计应检查该范围及 manifest/checksum 自洽性；完整干净来源构建和真实 provider smoke test 仍是独立发布验收项。

## 15. Wokey 实际目录复测（2026-10-16）

- 通过 `start-media-generation-mcp.ps1` 读取当前 Windows 用户 DPAPI 凭据，并经 MCP stdio 调用 `media_refresh_models`；没有绕过 MCP 直接请求上游，也没有提交生图/视频 POST。
- 真实只读目录请求成功：`api_request_sent=true`、`generation_request_sent=false`、`gateway_catalog_status=verified`、模型数 46。
- 同次 MCP 会话调用 `media_capabilities`，确认目录新鲜且以下映射均命中目录：`image-2=gpt-image-2.5`、`grok-image=grok-imagine-image-2.0`、`grok-video=grok-imagine-video-1.5`。Seedance、MiniMax 槽位为空/未配置，因此不宣称可用。
- 此结果验证统一网关在 Wokey profile 下能通过 MCP 返回媒体模型目录；不证明模型生成路由、配额、实际产物保存或 WorkBuddy 对话内调用成功。
- WorkBuddy 客户端验收本轮受限：Remote Desktop Commander 显示设备离线，本机应用清单及进程检查没有发现 WorkBuddy。此次调用使用同一启动脚本和 MCP 服务，但由本机 MCP stdio 驱动，不能标成 WorkBuddy 对话内验收。恢复 WorkBuddy 连接后仍需在其对话中重启客户端、调用 `media_refresh_models` 和 `media_capabilities`。
- `generation_capability` 继续保持 `not_tested`。实际图片/视频提交应在明确具体内容、模型和计费范围后单独验收；当前未发生计费生成请求。

