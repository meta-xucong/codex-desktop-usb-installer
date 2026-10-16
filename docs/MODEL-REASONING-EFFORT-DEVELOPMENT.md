# 第三方模型思考强度适配开发设计

状态：按设计完成实现；内部独立代码审计 PASS；本机模拟与运行时验收通过，未打包/发布
文档版本：1.0
审计基线：`45413fab1bab7b9697ef730ac992c711b940ab2`（分支 `codex/usb-fixes-20261004`，本次设计前工作区无未提交改动）

## 1. 目标与边界

修正安装器添加第三方模型时没有写入思考强度元数据的问题，并避免 Codex 向不支持的上游档位发送 `none` 或其他无效值。每个模型应根据当前网关路由能够证明的能力设置默认档位和可选档位。

本次只覆盖安装器的模型发现、测试证明、Codex 模型目录写入和相应 UI 呈现。Codex 本体不修改，不改 404token/Sub2API，不更改模型路由、API Key、费用策略、默认 GPT 模型，也不改变“添加只追加、复位才清理”的既有目录语义。

核心限制：上游模型厂商的官方能力说明只能说明原生模型支持什么；它不能证明 404token 当前路由、适配器和具体账号会接受什么参数。实现必须分别表达“厂商文档候选能力”和“当前网关已验证能力”。

## 2. 已确认的现状与因果链

1. 模型管理器实际用 `gateway_model_probe.rs::parse_model_catalog` 解析 `/v1/models`，当前 `GatewayProbeModel` 只保留 `id`、`owned_by` 和 `selectable`；响应中即使包含 reasoning capability 字段也会被忽略。`model_catalog.rs::parse_gateway_models` 是另一条目录路径，不是本功能的唯一接入点。
2. `UpstreamModelManagerSheet.tsx` 的“测试”调用 `gatewayModelProbe`。Rust 的 `gateway_model_probe.rs::run` 使用 `ProbeClient::quick_text` 发出单次文本请求。`model_probe.rs::text` 的 Responses 请求体只包括 `model/input/stream/store` 等字段，不指定 reasoning effort。
3. 这次成功只证明当前模型能完成一次默认路径的文本请求，不证明 `none`、`medium`、`xhigh` 等任一显式档位能通过。
4. 添加时后端重新读取 `/v1/models`、消费绑定到模型、账号、网关、目录和凭证摘要的短时 connectivity proof，然后调用 `add_tested_model_to_catalog`。
5. `conservative_tested_model_entry` 将新模型写成 `default_reasoning_level: null`、`supported_reasoning_levels: []`。因此该模型没有被配置为某个明确且经过验证的默认值。已观察到的上游报错明确拒绝 `none`，与目录元数据缺失导致客户端采用不兼容默认值的症状一致；但**Codex 究竟在哪个分支选择并发送 `none`，仍须用本机 runtime 和 mock gateway 请求证据确认**，不能只凭错误推测完整内部过程。

因果假设：安装器目前没有把上游档位能力和导入目录绑定；这与 `none` 被拒的现象吻合，但仅凭现有目录和报错，尚不能断言 Codex 因空元数据而发出了 `none`。实现前必须先用当前捆绑 Codex 和 mock gateway 建立基线证据（§7.3）。实现可修正目录声明和探测证据，不能修复或保证中转端把参数转换成原生模型参数。

## 3. 官方能力差异摘要

以下资料用于构建候选档位和测试矩阵，不可单独作为当前 404token 路由的通过证明。

| 系列 | 官方差异 | 设计影响 |
|---|---|---|
| OpenAI GPT | reasoning levels 和默认值按型号变化；例如 GPT‑6.1 Sol 不支持 `none`、`minimal`，默认 `medium`。 | 不把 GPT 档位规则自动套给所有厂商。 |
| Anthropic Claude | Opus 5.5 支持 `low/medium/high/xhigh/max`，默认 `medium`，不能关闭思考；很多其他型号默认 `high`；不同型号支持 `xhigh/max` 的范围不完全相同。 | 至少按精确型号配置；`none` 不得出现在 Opus 5.5 的能力列表。 |
| xAI Grok | Grok 4.5 支持 `low/medium/high`；4.6+ 支持 `xhigh`；通常默认 `high`，不能关闭思考。Grok 4.20 multi-agent 中 effort 还可代表协作 Agent 数量。 | 按精确版本识别；不能只用 `grok-*` 前缀；multi-agent 语义须单独标记。 |
| Google Gemini | 原生字段为 `thinking_level`；按型号有 `low/medium/high`、含 `minimal` 的集合、或仅 `low/high`；官方列出的型号没有 `xhigh/max`。 | Codex 的 OpenAI 风格参数是否能适配，取决于当前网关转换。 |
| DeepSeek | 原生/兼容格式的档位主要为 `none/low/high/max`；`medium` 与 `xhigh` 等会被映射到 `high`，`minimal` 映射到 `low`；具体 API 格式另有差异。 | 必须区分 API 协议和路由映射，不能把别名当成不同强度。 |
| Qwen、GLM、DeepSeek 等百炼兼容路由 | 百炼逐型号规定允许档位与映射。例如 Qwen3.8 将 `high/max` 映射到 `xhigh`；GLM‑5.3 支持 `low/high/max`，将 `medium` 映射为 `high`。 | 型号 ID 和 API 路由共同决定映射。 |
| MiniMax、Moonshot/Kimi 及其他转发线路 | 本轮没有足够的当前官方证据证明具体型号在当前 OpenAI-compatible 中转中的档位映射。 | 设为未知，需读取网关能力元数据或执行当前路由的明确兼容性探测。 |

一手来源：

- OpenAI: <https://developers.openai.com/api/docs/guides/reasoning>
- Anthropic: <https://platform.claude.com/docs/en/build-with-claude/effort>
- xAI: <https://docs.x.ai/developers/model-capabilities/text/reasoning>
- Google: <https://ai.google.dev/gemini-api/docs/thinking/>
- DeepSeek: <https://api-docs.deepseek.com/guides/thinking_mode/>
- Alibaba Cloud Model Studio: <https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses>

## 4. 推荐设计

### 4.1 能力证据分层

对每个精确 `model_id` 解析并存储以下来源之一；所有层都与当前 404token endpoint、授权账号绑定指纹和模型 ID 绑定：

1. **网关声明**：`/v1/models` 对该模型明确返回允许档位和默认档位。仅接受经过 schema、数量、枚举和默认值包含关系校验的字段。若来源字段不完整或冲突，不采信为完整能力。
2. **已审定型号配置**：安装器内置按精确型号维护的官方档位候选表，包含官方来源、抓取/审核日期、协议类型和已知默认值。型号族只可用于帮助检索，不得自动套用能力。该来源表达“候选”，不自动宣称当前网关线路已接受。
3. **当前网关探测**：对某模型某 endpoint 逐档位发送短小、限输出的 Responses 请求，记录“当前请求被路由接受”、明确参数拒绝或不确定。HTTP 成功只证明该请求在当前路由被接受并返回响应；除非网关提供可信转换/执行证据，否则不能证明模型实际按该档位推理。只有被当前路由接受的档位可以写入 Codex `supported_reasoning_levels`，界面和文档不得称作实际推理强度已验证。
4. **未知**：无法匹配可信声明或 profile，且没有足够实测结果时，不猜测档位、不写空列表后假称“无强度限制”。保持不能确定状态，要求用户进行明确的能力测试，或只允许使用经过验证的单一档位。

网关声明优先级最高，但必须来自同一 `/v1/models` 条目的明确、合法字段；若声明与型号 profile 冲突，标记冲突并停止自动导入 reasoning 元数据，不能静默二选一。型号 profile 不能覆盖当前线路实测出的明确拒绝。测试结果只对当前 endpoint、账号绑定、model ID 和短时目录指纹有效。

### 4.2 能力数据契约

内部新增一个可序列化的 reasoning profile（字段名可在审计后调整，语义不得丢失）：

```text
model_id                   精确上游模型 ID
default_effort             Codex 模型目录的默认档位
candidate_efforts          官方/网关声明候选值及来源
accepted_efforts           当前路由接受该请求的值；不表示上游实际应用了该强度
verification_status        gateway_declared | accepted_by_route | partial | unknown | conflict
route_fingerprint          绑定 endpoint、provider/account 的单向摘要
catalog_fingerprint        绑定当前 /models 目录摘要
profile_id / profile_version  内置 profile 身份及版本；无则为空
checked_at / expires_at    探测时间和一次性 Add proof 的有效期（不超过 5 分钟）
error_class                支持状态或拒绝原因的安全分类，不保留密钥/完整响应正文
```

所有数组去重并按 Codex 认可的顺序规范化。`default_effort` 必须存在于最后写入目录的 `supported_reasoning_levels` 集合中。`none` 是一档明确能力值，不能从空数组、空默认值或“thinking 关闭”推导出来；只有该精确模型和路由明确声明/接受 `none` 时才允许写入。目录字段表达“配置为可选/默认发送的 effort”，不构成运行时上游行为保证。

### 4.3 探测行为与费用边界

- 普通模型列表刷新只请求 `/v1/models`，不生成、不测试 reasoning，不增加费用请求。
- 单模型可用性测试目前会生成一次文本请求且没有单独费用确认。新逻辑不得在这个操作上追加多档位 sweep，也不得在添加、启动 Codex、安装、后台恢复时触发 reasoning 探测。
- 多模型协议矩阵已经要求单独 `costConsent`，但其目标是文本/工具/流式能力，不可悄悄把 reasoning sweep 塞入现有 12 请求矩阵，避免费用和 UI 声明失真。
- 新增明确的“检测思考档位”可选操作；开始前展示准确候选值、请求总数和“可能产生模型调用费用”，用户确认后才执行。Codex 目录探测全集固定顺序为 `none, minimal, low, medium, high, xhigh, max`；精确型号 profile 或合法网关 capability 可将全集收窄为明确候选子集。没有 profile/capability 的未知型号不得自动进行七档 sweep；用户需显式勾选要测的档位，候选为空则保持 unknown。每个勾选档位最多发一条相同的极短 prompt、尽可能小的输出限制和明确 effort 请求。只对该测试操作做有限并发、超时和取消；不得自动重试相同档位。
- 仅将明确 HTTP 成功且符合文本响应合同的档位记为 `accepted_by_route`。明确的参数不支持错误可列入 rejected set；超时、5xx、认证错误、额度错误、路由错误均列为 unknown/error，不可误记为“不支持”或接受。接受仅表示网关返回了兼容响应，不证明模型实际使用或保留该 effort。
- 如网关 `/models` 提供精确能力元数据且完整校验通过，可免于逐档测试而生成 `gateway_declared` 证据，但需标明声明来源；若模型导入测试已有明确拒绝结果，网关声明不能推翻它。
- 现有单次连通测试证明文本可用，不代表 reasoning 参数通过。UI 必须区分“文本连通”和“档位请求已测/当前路由已接受”，不暗示模型内部实际采用该强度。

**对未知型号的落法**：没有可采信能力声明时，不预置候选，不自动添加。用户明确选择要检测的 Codex 档位并确认请求数/费用后，只有被接受的档位可写入目录；默认选 `medium`（若其被接受），否则选所选且被接受集合中按固定顺序的首项。没有档位被接受时停止添加。若用户只做普通连通测试，只显示“文本测试可用，思考档位未测”，Add 不可用并提示选择档位检测。

### 4.4 写入 Codex 模型目录

- 将已声明或当前路由接受的 `default_effort` 与对应集合转换成 Codex 当前模型目录结构：默认值为字符串；`supported_reasoning_levels` 使用 Codex 运行时接受的 `{effort, description}` 对象数组。
- 默认值必须包含在数组中；数组不可为空；绝不可由未知能力写入 `none`。
- Description 使用短中文/英文说明或稳定短描述，不把模型供应商返回的任意长文本直接带入 Codex 配置。
- 如果仅有单档验证，目录只提供单个已验证档位；不能因某家原生文档列出更多档位便全部开放给一个未验证的中转线路。
- 不为修正单个导入模型而设置全局 `model_reasoning_effort` 或重写用户手动设置。必须先确认固定 Codex runtime 中全局配置与 per-model 默认/支持列表的优先级；若用户显式全局锁定了不支持的档位，应保留其配置并明确告知冲突，不能暗中改成 medium。
- 若当前用户配置显式设置了全局 `model_reasoning_effort`，且其值不在该模型当前路由接受的集合中，Add 和已有条目更新必须 fail closed，并明确说明全局值优先、需由用户自行调整全局设置后重测。若全局值兼容则可以添加；不得为导入单模型静默改写/清除全局值。
- 保持目录原子写入、备份、Codex 原生 `debug models` 最终验证、配置 marker、restart marker、并发锁和 additive add/reset 语义不变。
- connectivity proof 扩展为一次性承载并验证 reasoning evidence 摘要；消费时校验模型、endpoint、账户、凭据摘要、模型目录摘要、profile 版本和有效期。能力解析/候选产生须在 `gateway_model_probe.rs::parse_model_catalog` 主路径实现并同步 `GatewayProbeModel`、TS types/service tests；`model_catalog.rs::parse_gateway_models` 若被复用，须明确仅服务于另一条目录，不可替代主路径。禁止 UI 传入一份自称“接受”的客户端档位数组。
- 五分钟 proof 只能证明 Add 瞬间的网关绑定和请求接受状态，不能保证服务商之后不改路由。当前模型目录写入 Codex 持久配置，运行期不会自动知道 CCSwitch endpoint/account 或网关内部路由静默变化；本设计不承诺此类变化后阻止旧 effort 被发送。用户切换 endpoint/account 或上游路由变更后，必须回到管理器重新探测并更新 manager-owned 目录条目，或恢复默认。不得把历史接受结果宣传成永久保证。
- 模型已存在时，若模型目录归管理器所有，Add 不能无条件走 `already_added` 早退：新的有效 reasoning proof 应只更新该条目的 reasoning metadata，并保持其他字段/模型顺序；如果目录非本管理器所有，沿用现有拒写保护。相同能力未变化时仍返回 already-added。

### 4.5 兼容与恢复

- 只迁移本管理器拥有的模型目录。历史目录里的 `null` / `[]` 表示“未配置/旧版不完整”，不得自动解释为支持 `none`。
- 对已有手动修改或非管理器目录仍使用现有保护策略，不重写用户目录；无权属证据或 schema 不认识时停下并给现有“恢复 Codex 默认模型”入口。
- 增加 profile 版本和路由摘要可防止 URL、账号或 profile 更新后沿用旧的档位证明。
- 写失败、Codex runtime 不接受生成目录、账号绑定变化或证据过期时不得报告“已添加”。不修改旧模型，也不调用恢复默认操作。

## 5. 决策表与示例

| 能力来源 | 默认选择 | 可选项 | 是否准许添加 |
|---|---|---|---|
| 当前网关提供完整合法 capabilities，且默认在支持项中 | 网关声明默认 | 网关声明集合 | 是，保存声明来源；有路由拒绝时以拒绝为准 |
| 已审定型号候选 + 当前路由档位测试完成 | 首选被路由接受的 `medium`，否则首个被接受档位 | 当前路由请求接受集合 | 是，说明结果证明的是请求接受性 |
| 只有已审定型号候选，当前路由未验证 | 不把型号文档默认冒充路由默认 | 不暴露未验证档位 | 仅保留“文本可用、思考档未验证”状态；不可声称模型思考已就绪 |
| 未知型号，档位测试部分成功 | `medium` 若被接受，否则首个被接受档位 | 请求接受集合 | 是，明确标注“当前路由接受该档位请求” |
| 未知型号，没有档位测试或无任何已验证档位 | 无 | 无 | 不添加到可直接使用的受管理目录，给出继续测试说明 |
| 网关声明与内置 profile 冲突 | 无 | 无 | 阻止自动添加并显示冲突，等待修正资料/明确实测 |

Claude Opus 5.5 例：候选值为 `low/medium/high/xhigh/max`，默认候选 `medium`；不生成 `none` 候选。若当前网关测试只有 `low/medium/high` 通过，则 Codex 目录只列这三档，默认 `medium`。若 `medium` 未通过，不得因官方默认是 medium 强写 medium，应选择实际通过的默认并保留来源记录。

Gemini 型号例：某型号官方只有 `low/high`，候选列表不包含 `medium`；如果当前 404token `/responses` adapter 没有已知映射，需要测试实际 OpenAI-compatible 请求是否正确转成原生 `thinking_level`。未经转换/路由验证，不得把 OpenAI 五档塞进目录。

## 6. 拟改代码清单与复用评估

### 6.1 预计需要改动

| 文件 | 当前责任/可复用处 | 预计改动 |
|---|---|---|
| `src-tauri/src/app/gateway_model_probe.rs` | 已有 `/models` 查询、`parse_model_catalog` 的 ID/owner 解析、request lifecycle、取消管理、一次性连接 proof 和 request budget。 | 在真实管理器主路径扩展候选/能力解析；增加明确授权的档位操作；proof 绑定被接受档位证据；分类参数拒绝与网络/认证等未知失败。复用 `begin_management_run`、`ProbeClient`、binding_digest/catalog_digest 和 proof TTL。 |
| `src-tauri/src/app/model_catalog.rs` | `parse_gateway_models`、`conservative_tested_model_entry`、添加目录的原子写和权属校验、历史 migration、原生 Codex final validator、所有 Add/Reset 回归。 | 提取可复用的 profile resolver 和 Codex reasoning metadata builder；添加模型时要求已验证的非空 profile；迁移仅处理 manager-owned 旧的 null/empty 值；保持 Add additive，Reset 才清空。 |
| `src-tauri/src/app/model_probe.rs` | Responses 客户端、短文本请求、错误分类、脱敏 diagnostics、timeouts/cancel。 | 复用 HTTP client/timeout/diagnostic；增加显式 effort 请求构造及 bounded error classification；不改现有 quick text 默认语义，或仅在独立的 reasoning probe 方法中新增 effort 字段。 |
| `src/shared/types.ts` | 前端 Gateway probe contracts。 | 添加类型化的 reasoning candidate/status/result、确认费用的显式请求字段、proof 引用。 |
| `src-tauri/src/commands.rs` / `src/services/managerApi.ts` | Rust Tauri command 仅转发 request/report；前端 service 包装 Tauri command。 | 优先复用已有 `gateway_model_probe` command，不加新命令；检查字段透传/异常映射，若合同扩展不需要 wrapper 逻辑，则只加相应序列化测试。 |
| `src/app/views/UpstreamModelManagerSheet.tsx` | 现有发现/测试/添加状态机、短时 proof 校验、UI session/cancel 和模型目录状态显示。 | 加入明确、独立的“测思考强度”动作和费用确认；展示“文本已通/档位未测/档位实测”状态；只有拿到后端 proof 后才启用 Add；添加成功后用状态回读确定结果。 |
| `src/app/views/UpstreamModelManagerSheet.test.tsx` | 已覆盖测试 proof、过期、绑定变化、Add/reset/cancel 状态。 | 加入 reasoning evidence 过期、成本确认、状态显示、添加所需 proof、同名但不同 route 的证据隔离测试。 |
| `src-tauri/src/app/gateway_model_probe.rs` 单元测试区 | 已覆盖 request validation、目录、proof 生命周期。 | 覆盖能力字段 parsing、unknown/unsupported/error 分类、明确费用确认、proof 绑定和一次性消费。 |
| `src-tauri/src/app/model_catalog.rs` 测试区 | 已覆盖目录构造、添加/复位、权属、恢复、原生验证。 | 将测试从“supported []”改为显式强度元数据；覆盖 known/unknown/conflict/profile-version stale、empty rejection、default-membership、additive 保留旧模型。 |
| `src-tauri/src/app/model_native_probe.rs` | `native_schema::ModelList`、`ReasoningOption` 已校验 Codex 原生目录 reasoning 字段；final Codex validator 对写入后的 model catalog 做真实 CLI 验证。 | 复用现有结构/validator 作为格式合同；仅在测试暴露缺口时改动。不能把本模块的原生 Codex `/models` metadata 误当成上游 404token capability。 |

### 6.2 先复用、不再造

- 复用现有 `ProbeClient` 连接、超时、取消、状态脱敏和请求计数；不新建第二套 HTTP client。
- 复用现有账户、provider、endpoint、credential、catalog fingerprints 与短时 proof lifecycle；为 proof 加字段，不复制一套异步证据管理。
- 复用 manager-owned catalog validator、原子写、备份、配置锁、restart marker 和 `native_default_catalog/validate_final_catalog`。
- 复用 `UpstreamModelManagerSheet` 已有模型状态与取消框架。新增 UI 仅为能力检测和说明，不重做整个管理页。
- 复用官方链接作为审核资料；不要打包用户原始 `models_cache.json`、测试账号、key 或从本机抓到的个人模型目录。

### 6.3 不应修改

- 不改 Codex runtime / vendor binaries / provider authentication 配置。
- 不改 CCSwitch、统一网关 API key 注入、404token/Sub2API 服务端路由或账单。
- 不在列表刷新、Codex 启动、Add/Reset、能力包安装时隐式发起生成请求。
- 不仅凭 `owned_by`、模型名模糊前缀、客户端传入字段或一次无 effort 的文本响应推断全部强度。
- 不删旧模型，不恢复默认，除非用户明确点击当前已有复位入口。
- 不修改 `models_cache.json` 或把其中内容打进公开包。

## 7. 测试与验收矩阵

### 7.1 Parser/能力解析

1. `/models` 仅含 id/owned_by：状态为 unknown，不编造支持项。
2. 合法 capability 结构：接受允许档位、默认值和来源；默认不在集合时拒绝该 capability 记录。
3. 未知 effort enum、重复项、超长数组、错误字段类型、伪造多个 capability、凭据内容混入字段：拒绝/脱敏，不写入目录。
4. 同一 model ID、不同 owner/route fingerprint：不得共享已验证结果。
5. 供应商 profile 例：Claude Opus 5.5 排除 none；Gemini low/high 例不增加 medium；DeepSeek medium/xhigh alias 不宣称为新增原生强度；Grok 4.5 不暴露 xhigh 作为独立强度。

### 7.2 Probe/proof/费用

1. 普通刷新：只调用 `/models`，generated requests = 0。
2. 旧单次 connectivity test：仍只有当前既有的一次文本 request，不隐式做多档 sweep。
3. reasoning sweep 未勾选明确费用确认：本地拒绝且 generated requests = 0。
4. 已确认测试：每个所选候选最多一条请求；被网关接受的档位被记录为 `accepted_by_route`，unsupported 与 timeout/auth/credit/5xx 正确区分；不自动重试，也不宣称模型真实推理强度。
5. 取消/超时/模型目录刷新/凭证更新/endpoint 变化/账户变化/过期：proof 失效，不可 Add。
6. proof 不可重放；不能将 A 模型的证据用于 B 模型、另一个 endpoint 或不同账户。
7. 日志只记 model、effort、status、请求 ID、错误分类；不记 API key、prompt 隐私正文或完整上游错误响应。

### 7.3 Catalog 与 Codex 运行时

1. 实现前先对基线捆绑 Codex + mock Responses gateway 做黑箱回归：空元数据/单个 supported effort 与全局 effort 存在/不存在四种组合，捕获实际发出的 reasoning 值。2026-10-15 已用随包 `codex-cli 0.151.0-alpha.7.1` 和本地 mock 完成：空目录且无显式全局 effort 时，CLI 日志显示 `none`，请求 `reasoning` 对象未带 `effort`；加入仅 `medium` 的单模型目录后请求发送 `effort=medium`；空目录配全局 `medium` 也发送 `medium`；即使模型目录只支持 `medium`，显式全局 `none` 仍发送 `effort=none`。四次均单请求完成，没有真实 API key/网络费用。该结果证明目录元数据能约束未被全局设置覆盖的请求，也证明全局不兼容值优先；因此实现必须做配置冲突阻断。脱敏摘要保存在 `.test-tmp/reasoning-baseline-confirmed/baseline-results.json`（该目录为本机测试临时文件，不进入发行包）。若将来 runtime 版本变化，应重新跑该测试；不得把目录 validator 当作替代证据。
2. 可添加模型的目录条目 `default_reasoning_level` 非空，且包含在 supported list 中；list 非空、值唯一。
3. 每个未被当前路由接受的 effort 都不出现在 Codex selector；`none` 只在被声明/接受时存在。
4. `codex debug models` 或当前已复用的本机 native validator 能读取最终目录，且解析出的默认/可选强度与预期一致。
5. 修复后在 mock gateway 捕获 outgoing request：初始默认档位与单模型目录配置一致；Claude Opus 5.5 fixture 不会发 `none`；未知能力条目不可添加，不能发送未声明 effort。
6. 全局 reasoning 配置与每模型默认/列表交互：不破坏用户现有全局值；当全局值不在模型被接受集合中时，Add/update 被阻止并说明修复方式；不能悄悄退回 `none` 或自动覆写用户设置。测试全局 none 对仅接受 medium 的模型必须被阻断。
7. Add 后原有模型仍在，旧配置与用户拥有的 catalog 不被覆盖；仅 Reset 清理。
8. 已存在的 manager-owned 模型经新 proof 重新检测后，只更新其 reasoning metadata；其它模型 ID、顺序、字段完全保留；非 manager-owned 文件仍拒写。
9. 历史 manager-owned null/empty entry 可确定性迁移或提示复位；外部/用户目录仍被保留并拒绝覆盖。
10. Codex 原生校验失败、写入中断、账号切换和并发添加均无“已添加”假阳性，旧目录可以恢复。

### 7.4 UI

- 模型列表刷新和普通连接测试不额外发费用请求；展示“文本测试通过”与“思考档位验证”两个独立状态。
- 档位测试要显示请求数量和费用提示，并在确认前不发请求。
- 探测成功只显示“当前路由接受该档位请求”。Add 需要有效的后端 proof；添加完成后读取状态并显示已添加。
- 退出/返回/重开页面后，UI 不伪造或恢复过期的 reasoning proof；本地短期结果显示过期并要求重新测。

## 8. 执行顺序与放行门

1. 内部独立只读设计审计已通过；随后按本文件实现。实现审计复核确认此前发现的输出上限过低、错误分类过宽问题已修复，未发现 P1/P2 阻断。
2. 实际改动范围包含第 6 节列出的实现文件、对应测试，以及本设计和审计交接文档；没有新增跨边界模块。
3. 已完成定向 React（19/19）、完整前端（484 passed、1 skipped）、完整 Rust lib 单测（740 passed、6 ignored）、TypeScript check、目标文件 ESLint、Vite build，以及 Windows Codex CLI 0.162.0-alpha.17.2 的隔离模型目录解析验证。
4. Windows Codex CLI 验证使用隔离 `CODEX_HOME` 和本地目录 fixture，成功读回 `vendor/claude-opus-5-5` 的 `medium` 默认值与 `low/medium/high/xhigh/max` 支持列表；未更改日常 Codex 配置。
5. 真实上游档位探测没有执行；本轮使用 mock Responses gateway，不访问 404token、不产生上游费用。上游静默换路依旧超出该客户端证明能保证的范围。
6. `cargo fmt --all -- --check` 会被仓库既存的 `crates/codex-usb-license/src/lib.rs:396` 格式差异阻断；本次修改的 Rust 文件单独 `rustfmt --check` 通过。Vite 仍提示现有主 chunk 超过 500 kB，属于非阻断建议。

## 9. D/I/A 与冻结契约

- **目标**：在安装器添加第三方模型时写入有来源的非空思考强度默认值/支持集合；未知/拒绝时不把该模型标成档位就绪。运行期静默换路不在此机制可保证范围内。
- **非目标**：修改 Codex 本体、网关服务端/模型路由、密钥注入、收费、卸载/许可、其他插件/模型列表 UX。
- **硬约束**：不增加隐式生成请求；所有 reasoning sweep 需明示成本和用户确认；不把厂商文档视为网关实测；Add additive/Reset 才清除；不覆盖用户目录；显式全局 effort 与模型接受集冲突时阻止 Add/update 并保留全局值；不把 secret/log raw error 写入产物。
- **允许文件**：第 6 节 6.1 中的 Rust/TS 文件，以及用于型号 profile 的单一受版本控制 JSON/模块（具体新增位置须经独立审计同意）；`docs/MODEL-REASONING-EFFORT-DEVELOPMENT.md` 的冻结修订。
- **D/I/A**：D0（未知型号 fail-closed、明示费用确认、路由能力分层和验收边界均已写入本文）；I2（跨 UI、probe proof、Codex model catalog 和网关 route evidence 的一致性）；A2（Reasoning sweep 可能产生上游计费请求，必须审查费用授权与无隐式网络请求约束；不改账单服务端）。
- **实现放行证据**：内部独立固定版本审计 PASS；第 7.3 第 1 项的基线请求捕获通过；测试覆盖第 7 节；实际 Codex catalog validator 通过；模拟网关请求证明默认/可选 effort 不会越界；若无真实 404token route，必须把 route 状态明确保留为未实测，不能声称上游全路线已实测。
