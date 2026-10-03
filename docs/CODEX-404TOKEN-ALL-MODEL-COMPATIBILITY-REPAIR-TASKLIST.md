# U 盘版 Codex × 404token/Sub2API 全模型兼容性修复任务清单

文档版本：`0.1.0`<br>
日期：`2026-09-29`<br>
状态：`OPEN_FOR_404TOKEN_HANDOFF`<br>
主要责任侧：`404token/Sub2API`<br>
本地协作侧：`U 盘 Codex 管理器 / CCSwitch / Codex Desktop`

## 1. 目标

这份文档用于把后续用户实测问题集中交给 404token/Sub2API 开发会话，先完成上游模型路由和 Responses 兼容性修复，再做 U 盘端回归。

最终目标是：

1. 404token 的 `/v1/models` 中每一个被主动公布的模型，都有明确的上游模型 ID、供应商、账号/分组和协议适配状态。
2. 被公布为 Codex 可用的模型，至少能稳定完成普通文本、Responses 流式、单工具调用、工具结果续接和多轮对话。
3. GLM、DeepSeek、Claude、GPT、Kimi、Qwen、MiniMax、HY4 等不同供应商的模型不能只依赖一个通用透传路径；需要按实际上游协议完成转换和回归。
4. 模型目录中的“已发现”不再被误认为“工具调用已经兼容”。不支持某项能力的模型必须准确标记、隔离或从对应能力目录中移除。
5. 修复不能破坏目前已经正常工作的模型，尤其是 `deepseek-v4-flash`、`glm-5.2`、`claude-fable-5` 以及用户已实测正常的 GPT 模型。
6. U 盘默认安装和 CCSwitch 自动配置流程继续保持简单；完整模型探测仍是高级诊断功能，不得把未经验证的上游问题用本地安装流程掩盖。

## 2. 范围和边界

### 2.1 本轮主要修复范围

- 404token/Sub2API 的模型目录、模型映射和账号/分组路由。
- `/v1/responses` 非流式和流式转换。
- function call、tool result、`previous_response_id` 和多轮续接。
- 各上游供应商的错误码、限流、鉴权、模型 ID 和能力差异。
- 脱敏的请求/响应/事件日志，以及可重复的 fixture 和回归测试。

### 2.2 本轮不应采用的“伪修复”

- 不在 Codex 或 CCSwitch 中把工具调用改成纯文本模拟。
- 不删除 Codex 发出的工具、流式、多轮字段来换取 HTTP 200。
- 不把所有模型统一标记为 `unified_exec`、`responses_lite`、全推理等级或完整 Code Agent 能力。
- 不因某一次普通文本请求成功，就宣布该模型工具调用兼容。
- 不把 `/v1/models` 中出现模型 ID 当成该模型真实可用的证明。
- 不用错误的模型回退到 GPT、另一个供应商或另一个账号后继续返回成功。
- 不为了通过探测而放宽 JSON、`call_id`、事件顺序或工具结果校验。

### 2.3 职责划分

| 组件 | 应负责的内容 | 不应负责的内容 |
|---|---|---|
| 404token/Sub2API | 上游账号、分组、模型映射、协议转换、事件状态机、错误分类 | 修改 Codex 官方二进制或假造本地工具结果 |
| CCSwitch | Provider、`base_url`、鉴权注入和请求转发 | 替代 Sub2API 完成供应商语义转换 |
| U 盘管理器 | 安装、配置、模型目录刷新、高级诊断和日志展示 | 猜测或修复上游模型协议 |
| Codex Desktop | 消费标准 Responses 和工具事件 | 为上游模型补造缺失的事件身份 |

## 3. 当前证据和已知缺口

### 3.1 已有真实线上证据

404token 部署提交 `49e52b9d809c229f0bb638332d6595972c4d7304` 后，正式 USB-0013 绑定执行过以下矩阵：

- `glm-5.2`
- `deepseek-v4-flash`
- `claude-fable-5`

三模型各 8 个阶段，共 24 个 HTTP 200 响应；流式事件连续并以 `response.completed` 结束。Claude 的 `response.function_call_arguments.done.arguments` 已与增量累积值和 `response.output_item.done.arguments` 一致。

这证明该次部署和这三个模型的当次矩阵恢复，**不能外推到整个 `/v1/models` 目录**。`49e52b9d8` 是 Claude function-call arguments 修复，不是所有供应商的通用兼容证明。

### 3.2 用户 2026-09-29 实测报告

用户报告中已正常的模型：

- DeepSeek（含用户报告的 DeepSeek V4.1）
- `glm-5.2`
- Claude（含 `claude-fable-5`）
- GPT

用户报告中有问题或未完成验证的模型：

- GLM 5.3：无法访问工具。
- HY4：报告标记为未测试，不能写成失败或通过。
- HY4-Preview：工具调用失败。
- Kimi K3：工具调用失败。
- MiniMax：工具调用失败。
- Qwen 3.8：工具调用失败。

报告中的典型错误是 `Script error: exec cell [id] not found`。该错误没有附带 404token HTTP 状态码、原始 Responses SSE 事件或上游错误体，因此不能单独证明是上游协议错误。

### 3.3 证据质量要求

报告中的模型简称必须补齐为真实请求使用的精确 ID。尤其“GLM 首次测试”不能自动等同于 `glm-5.3`。每个失败案例必须保存：

- 请求的精确 `model`。
- 请求进入的 provider、账号、group 和映射后的上游模型 ID。
- HTTP 状态码、错误代码和脱敏错误体。
- 完整的 SSE event type、sequence、item_id、call_id 和 arguments 关系。
- Codex 客户端是否已经收到 `function_call` 后才启动本地工具。
- 本地出现 `exec cell not found` 的时间点和对应 session/exec ID。

## 4. 待修复模型台账

以下表格是当前任务的起始台账。没有真实证据的项目必须保持“未验证”，不能填写“已兼容”。

| 目标模型/别名 | 必须补齐的精确 ID | 当前状态 | 首要责任 | 放行条件 |
|---|---|---|---|---|
| `deepseek-v4-flash` | 已知 | 已有三模型矩阵通过 | 404token 回归 | 修复后回归不得退化 |
| `glm-5.2` | 已知 | 已有三模型矩阵通过 | 404token 回归 | 修复后回归不得退化 |
| `claude-fable-5` | 已知 | 已有三模型矩阵通过 | 404token 回归 | `done.arguments` 与 delta 完全一致 |
| GPT 相关模型 | 需从真实请求确认 | 用户实测正常，未纳入上述三模型矩阵 | 404token/本地联合回归 | 明确是 404token 路由还是原生 GPT |
| `glm-5.3` | 待确认 | 用户报告工具失败；未有本轮真实矩阵 | 404token | 文本、流式、工具和续接矩阵通过 |
| HY4 | 待确认 | 未测试 | 404token | 先确认是否仍在目录和可用分组，再决定是否探测 |
| HY4-Preview | 待确认 | 用户报告工具失败 | 404token | 真实事件证据显示可完成或被准确隔离 |
| Kimi K3 | 待确认（可能是 bare ID 或 `kimi-*`） | 用户报告工具失败 | 404token | 确认上游 ID、路由和工具事件后通过 |
| MiniMax | 待确认 | 用户报告工具失败 | 404token | 确认上游 ID、路由和工具事件后通过 |
| Qwen 3.8 | 待确认 | 用户报告工具失败 | 404token | 确认上游 ID、路由和工具事件后通过 |
| DeepSeek V4.1 | 待确认 | 用户实测正常，未纳入三模型矩阵 | 404token | 真实请求 ID 和工具/流式证据齐全 |

## 5. 404token/Sub2API 侧 P0 修复任务

### P0-01：建立真实模型和路由清单

- [ ] 导出目标 group 的完整 `/v1/models` 结果，保留原始 `id`、`owned_by`、展示名和模型状态。
- [ ] 对每个模型列出实际 provider、账号、group、channel、adapter 和上游 endpoint。
- [ ] 列出请求模型 ID 到上游模型 ID 的所有映射，包括精确映射、通配符、别名和日期版本。
- [ ] 检查不同供应商同名或别名是否会互相覆盖。
- [ ] 检查 `/v1/models` 是否公布了没有可调度账号或没有可用上游映射的模型。
- [ ] 确认“模型在菜单可见”和“模型具备工具调用能力”使用不同状态，不能混写。
- [ ] 产出脱敏的 `model-route-inventory.json`，字段至少包括：`requested_id`、`mapped_id`、`provider`、`group_id`、`adapter`、`account_class`、`capability_status`、`checked_at`。

### P0-02：统一 Responses 非流式契约

对每个目标模型检查：

- [ ] `POST /v1/responses` 的基本文本请求能返回标准 response envelope。
- [ ] `response.id`、`status`、`output`、usage 和错误结构稳定。
- [ ] 请求中的 `model` 不被静默改成另一个未告知的模型。
- [ ] reasoning、文本内容和工具输出不被错误丢弃或拼接到错误字段。
- [ ] 上游 401、403、404、429、5xx 映射为明确、可诊断、可重试边界的错误。
- [ ] 失败切换账号时不能把不同账号、不同模型的结果混在同一 response 中。

### P0-03：修复普通文本流式事件

每个模型必须检查以下事件的顺序、字段和终态：

```text
response.created
response.in_progress
response.output_item.added
response.content_part.added
response.output_text.delta
response.output_text.done
response.output_item.done
response.completed
```

要求：

- [ ] sequence 单调递增且不重复。
- [ ] UTF-8 分片可以任意边界拼接，不出现乱码或截断。
- [ ] 正常结束必有 `response.completed`。
- [ ] 上游中断、超时、拒绝和空响应不能伪装成正常完成。
- [ ] 不同请求的 response_id、item_id 和事件不能串线。

### P0-04：修复 function call 事件状态机

这是 GLM 5.3、Kimi、Qwen、MiniMax、HY4-Preview 等问题的核心检查项。

- [ ] `function_call` 的 `item_id`、`call_id`、函数名和 arguments 全程保持一致。
- [ ] `function_call_arguments.delta` 可按事件顺序累积为完整 JSON。
- [ ] `function_call_arguments.done.arguments` 必须是累积后的完整 JSON，不能是空字符串、额外 `{}` 前缀或截断 JSON。
- [ ] `response.output_item.done.arguments` 必须与累积值一致。
- [ ] 空参数、字符串参数、数组参数、嵌套对象和 Unicode 参数均有 fixture。
- [ ] 参数不是合法 JSON 时返回明确的结构化错误，不得把半成品交给 Codex 执行。
- [ ] 多个或并行工具调用必须按 `call_id` 分开，不能覆盖前一个调用。
- [ ] 相同 `call_id` 的重试不能导致工具重复执行。

### P0-05：修复工具结果续接和多轮对话

- [ ] Codex 提交 `function_call_output` 后，Sub2API 能继续同一 response/会话。
- [ ] `previous_response_id`、`response_id`、`item_id` 和 `call_id` 关系正确。
- [ ] 工具结果内容不被错误转换成用户文本。
- [ ] 工具成功结果、工具错误结果和空结果都能继续生成最终回答。
- [ ] 单工具、多工具、并行工具和多轮对话不发生上下文串线。
- [ ] 断线或上游重试时不会重复执行已经确认的工具调用。

### P0-06：逐供应商核对上游适配器

对每个未验证模型必须回答以下问题，不允许只修改一个通用转换器后直接宣布全部兼容：

- [ ] 上游真正接受的是 Responses、Chat Completions、Anthropic Messages 还是其他协议。
- [ ] 上游的工具字段名、参数格式、流式事件和终止标识是什么。
- [ ] 上游是否支持并行工具、工具错误结果、reasoning 和多轮续接。
- [ ] 上游模型 ID 是否需要日期后缀、provider 前缀或特殊 endpoint。
- [ ] 请求头、鉴权方式、上下文窗口和限流语义是否与其他供应商不同。
- [ ] adapter 转换后是否仍能满足 Codex Responses 契约。

优先顺序：

1. `glm-5.3`。
2. `kimi` / Kimi K3。
3. `qwen` / Qwen 3.8。
4. `minimax`。
5. `HY4` / `HY4-Preview`（先确认模型是否仍然存在和可调度）。

### P0-07：错误分类、failover 和可观测性

- [ ] 不可用模型返回明确的 `model_unavailable`、`model_not_mapped`、`upstream_protocol_error`、`tool_event_invalid`、`upstream_rate_limited` 等错误代码。
- [ ] 不得把“模型不支持工具”伪装成空文本成功。
- [ ] 不得把上游 4xx 永久重试成工具卡死。
- [ ] 失败切换只在同一请求边界内进行，并记录实际使用的 provider/model。
- [ ] 日志记录 `request_id`、`response_id`、requested model、mapped model、provider、group、adapter 版本、HTTP status、事件类型、sequence 和错误代码。
- [ ] 日志必须删除 API Key、Authorization、密码、完整 prompt 和完整模型敏感响应。
- [ ] 能根据同一个 request_id 将 Codex 请求、Sub2API 转换和上游响应串起来。

## 6. P1 兼容性和能力管理任务

- [ ] `/v1/models` 与能力清单分离：发现状态不等于 Code Agent ready。
- [ ] 为每个模型保存能力状态：`discovered`、`text_verified`、`stream_verified`、`tool_verified`、`multi_turn_verified`、`ready`、`quarantined`。
- [ ] 能力状态包含 adapter 版本、模型映射版本和验证时间。
- [ ] 未通过工具验证的模型仍可保留普通聊天能力，但不能被标记为完整工具模型。
- [ ] `/v1/responses/compact` 只在该模型真实支持时启用；未知状态不能伪造支持。
- [ ] 如果 Codex 使用 Responses WebSocket 或其他长期会话入口，必须对首轮和后续轮次分别测试。
- [ ] 对已知正常模型建立回归基线，后续修复不得改变其事件语义。

## 7. 测试方案

### 7.1 阶段 A：静态和 Mock 测试（不消耗上游额度）

- [ ] 模型映射、别名、provider 路由和 group 筛选的单元测试。
- [ ] 为 GLM、DeepSeek、Claude、Kimi、Qwen、MiniMax、HY4 建立最小 SSE fixture。
- [ ] 覆盖普通文本、stream、单工具、多工具、工具错误、工具结果续接和多轮。
- [ ] 覆盖空 arguments、非法 JSON、重复 sequence、缺失 done、错 call_id 和跨请求串线。
- [ ] 覆盖 401、403、404、429、500、上游超时、断线和空 HTTP 200。
- [ ] 使用脱敏 fixture，不允许把真实 token 或完整用户 prompt 纳入仓库。

### 7.2 阶段 B：线上非生成检查（不应产生模型生成费用）

- [ ] `/health`。
- [ ] `/v1/models`。
- [ ] 账号、group、model_mapping 和 provider 路由静态核对。
- [ ] 记录服务端 commit、容器 image、运行中二进制 SHA-256、健康状态和备份位置。
- [ ] 检查线上实际部署版本与本次修复 commit 完全一致。

### 7.3 阶段 C：未验证模型的最小线上探测

真实生成请求必须在执行前再次向用户说明并获得当次授权。建议先做最小集，不直接运行 12 请求完整矩阵。

每个模型最小 4 项：

1. 非流式普通文本：1 次。
2. 普通文本流式：1 次。
3. 单工具调用：1 次。
4. 工具结果续接并生成最终回答：1 次。

按当前未验证清单，若测试 `glm-5.3`、HY4-Preview、Kimi K3、MiniMax、Qwen 3.8、DeepSeek V4.1，共约 24 次生成请求；实际数量以确认后的精确模型 ID 为准。失败模型只在定位后补充对应阶段，不自动重复整轮。

### 7.4 阶段 D：完整兼容矩阵

单个模型完整矩阵建议 8 个阶段：

1. 普通文本非流式。
2. 普通文本流式。
3. 单工具调用。
4. 工具结果续接。
5. 多工具/并行工具。
6. 工具错误结果。
7. 多轮 `previous_response_id`。
8. 长文本/UTF-8/边界参数。

只有在阶段 C 通过后才运行阶段 D。若沿用 U 盘高级探测器的 12 项矩阵，必须提前披露每模型最多 12 次、最多模型数和可能额度消耗。

### 7.5 阶段 E：Codex 和真实 U 盘验收

对每个已通过上游矩阵的模型，在有效签名 U 盘上：

- [ ] 管理器能刷新并显示精确模型 ID。
- [ ] CCSwitch 选择正确 provider 和模型后 Apply 成功。
- [ ] Codex 启动后模型目录显示与 `/v1/models` 一致。
- [ ] 普通回答有增量流式输出并正常结束。
- [ ] 安全的文件读取或目录列出任务能触发工具调用、返回工具结果并继续回答。
- [ ] 连续两次工具调用不会出现 `exec cell not found`、卡死或重复执行。
- [ ] 失败时记录本地 Codex 日志和 404token request_id，禁止只凭 UI 文字判断根因。

## 8. 通过标准

### 8.1 单模型通过

单模型必须同时满足：

- 请求模型 ID、映射模型 ID 和实际 provider 可追溯。
- 普通文本和流式通过。
- function-call 参数合法且身份字段不丢失。
- 工具结果可续接并得到最终回答。
- 多工具、工具错误和至少一次多轮场景通过。
- 没有跨请求、跨模型或跨账号串线。
- 失败和限流有明确结构化错误。

### 8.2 全部模型放行

只有以下条件都满足，才能称为“404token 全模型兼容性修复完成”：

- 当前 `/v1/models` 目录中的每个主动公布模型都有路由清单。
- 每个模型有对应的能力状态和真实测试证据。
- 未支持能力的模型被准确隔离或明确标记。
- 已知正常模型回归没有退化。
- 404token 线上运行版本和测试证据对应同一个 commit/image。
- U 盘 Codex 原生界面完成至少一轮真实测试。

否则只能表述为：

> 404token 已修复部分模型的 Responses 兼容性；其余模型仍处于未验证、隔离或待修复状态。

## 9. 部署、回滚和版本记录

每次 404token 部署必须记录：

- Git commit SHA。
- 构建镜像标签。
- 运行中容器状态。
- `/app/sub2api` 或对应二进制 SHA-256。
- 部署前备份路径。
- `/health` 结果。
- 迁移是否执行以及回滚命令。
- 本次测试使用的 group、模型 ID 和请求数量。

部署顺序：

1. 在本地/CI 跑静态测试和 fixture。
2. 生成脱敏测试包并检查不含凭证。
3. 备份当前线上版本和配置。
4. 通过已验证的中继路径部署到 404token/AISelf。
5. 先做 `/health` 和 `/v1/models` 检查。
6. 只对授权的测试账号执行阶段 C/D。
7. 发现健康检查、路由或协议回归时立即回滚。
8. 记录最终 image、health、日志摘要和证据文件 SHA-256。

## 10. 后续问题登记区

用户会继续把实测问题追加到本节。新增问题不得直接改写已有结论，必须保留原始现象和证据。

### 问题模板

```text
问题编号：FUTURE-XXX
发现日期：YYYY-MM-DD
发现环境：U 盘包版本 / Codex 版本 / Windows 版本
模型精确 ID：
Provider / group / 账号类型：
复现步骤：
预期行为：
实际行为：
错误文本：
是否普通文本正常：是 / 否 / 未测
是否流式正常：是 / 否 / 未测
是否工具调用正常：是 / 否 / 未测
是否有 request_id：
附件日志路径：
额度/线上测试授权：未授权 / 已授权（次数上限）
严重级别：P0 / P1 / P2
责任侧：404token / U 盘管理器 / CCSwitch / Codex / 待定
状态：待诊断 / 待修复 / 待回归 / 已通过 / 已隔离
```

### 当前预留问题

| 编号 | 问题 | 当前状态 | 责任侧 |
|---|---|---|---|
| FUTURE-001 | GLM 5.3 工具调用失败 | 待取得精确 ID 和原始事件 | 404token 优先 |
| FUTURE-002 | HY4/HY4-Preview 能力和目录状态不明 | 待确认是否仍提供 | 404token |
| FUTURE-003 | Kimi K3 工具调用失败 | 待核对映射和 Responses 适配 | 404token |
| FUTURE-004 | MiniMax 工具调用失败 | 待核对映射和 Responses 适配 | 404token |
| FUTURE-005 | Qwen 3.8 工具调用失败 | 待核对映射和 Responses 适配 | 404token |
| FUTURE-006 | `exec cell not found` 的本地/上游边界 | 待 request_id 和双端日志 | 联合 |

## 11. 交给 404token 会话的执行文案

以下内容可以直接发给负责 404token 的会话：

```text
请按 CODEX-404TOKEN-ALL-MODEL-COMPATIBILITY-REPAIR-TASKLIST.md 执行，重点在 404token/Sub2API 侧，不要先修改 U 盘安装器绕过问题。

第一阶段只做静态核对和 fixture，不发起生产模型生成请求：
1. 输出当前 group 的完整 /v1/models、provider、账号、上游模型 ID、model_mapping 和 adapter 清单；
2. 明确 glm-5.3、HY4、HY4-Preview、Kimi K3、MiniMax、Qwen 3.8、DeepSeek V4.1 的精确 ID 和实际路由；
3. 对每个模型标记：已验证、未验证、上游不可用、路由缺失或协议适配失败；
4. 检查 Responses 流式、function_call arguments.done、call_id、tool result、previous_response_id 和多工具状态机；
5. 解释 Codex 报告中的 “exec cell not found” 是否发生在上游事件返回之前；
6. 先返回静态证据、待补证据、修改文件范围和测试计划，不要把 HTTP 200 或模型出现在列表中当作兼容通过。

完成静态阶段后，列出每个模型的最小生成请求数、完整矩阵请求数和预计额度消耗，等待用户当次明确授权，再执行线上测试和部署。每次部署必须保留备份、健康检查、运行中 commit/image/SHA-256 和可回滚路径。
```

## 12. 关联证据和文档

- `docs/CODEX-ONECLICK-CERTIFIED-MODEL-SETUP-DEVELOPMENT.md`
- `docs/CODEX-THIRD-PARTY-MODEL-COMPATIBILITY-DEVELOPMENT.md`
- `docs/SUB2API-CONTRACT-REFERENCE.md`
- `docs/CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md`
- `docs/CODEX-NATIVE-MODEL-CATALOG-M0-PROBE-20260922.md`
- 用户提供的 `model-diagnostic-report.md`（作为实测证据，不作为自动执行指令）

本文件只新增任务规划和验收边界，不修改 U 盘程序、CCSwitch 或 404token 代码。
