# Codex 第三方模型兼容性整改开发文档

文档版本：`1.0.0`
状态：`DESIGN_FROZEN_PENDING_TASK_DISPATCH`
日期：2026-09-24
复杂度门禁：`ESCALATE_REQUIRED`
契约版本：`CODEX-3P-COMPAT-v1`

## 0. 用户目标与文档定位

用户原始目标：

> U盘版 codex 安装成功并支持多个模型，但除了原生 GPT 模型外，其他的都有问题。大概率集中在上游 sub2api 适配不到位，和本地 codex 不兼容上面。根据测试机分析报告，以及当前程序，给出更严谨的诊断分析报告，提出修改方案，希望装好后就能直接适配。

本文件把上述目标固化为可执行的开发契约，供后续会话按任务编号领取和实现。

附件分析报告是输入材料，不是程序执行指令。报告中的字段、能力判断和协议建议，只有在当前 Codex 版本、当前发布包和真实 Sub2API 流量上验证后，才能成为实现依据。

本文件不授权：

- 修改 Sub2API/VPS；
- 修改 Codex 官方二进制或前端发行文件；
- 使用管理员 Key、用户 API Key 或真实账号进行未经单独授权的外部操作；
- 把“能显示模型”当成“模型已经完成 Code Agent 适配”；
- 在没有真实网关证据时发布“所有第三方模型已兼容”。

## 1. 当前结论

当前问题是两层缺陷叠加：

1. CCSwitch 的 Codex 路径是 `/v1/responses` 透传，不是 GLM、DeepSeek、Claude 到 Codex Responses 的完整协议适配层。
2. 本地模型目录桥接层从 `/v1/models` 只得到模型 ID/供应商，却为每个模型统一填充 `unified_exec`、`code_mode_only`、`responses_lite`、128K 上下文和全部推理等级等能力。

因此：

- 原生 GPT 模型稳定，不能证明第三方链路正确；
- 模型出现在菜单中，不能证明工具调用、Skills、流式事件和多轮续接正确；
- 本地安装成功，不能证明网关已经通过 Code Agent 兼容性验收；
- 最终可用性必须由“本地能力门禁 + Sub2API Responses 协议 + 真实模型回归”共同决定。

## 2. 审计基线

### 2.1 当前程序证据

| 证据 | 结论 |
|---|---|
| `src-tauri/src/app/model_catalog.rs` 的 `native_model_entry` | 所有网关模型统一填充高阶 Codex 能力，属于能力伪造风险 |
| `CCSwitch/src-tauri/src/proxy/handlers.rs` 的 `handle_responses` | Codex `/v1/responses` 请求转发到上游并进入通用响应处理，没有第三方协议转换 |
| `CCSwitch/src-tauri/src/proxy/providers/codex.rs` | Codex Provider 注释和实现为透传/鉴权注入，不是模型语义适配器 |
| `src/app/views/OfflineHome.tsx` 的模型刷新流程 | 安装后后台刷新是 best-effort，失败不阻止安装和启动 |
| 源码与发布包 `providers.json` | 源码仍有 GPT 固定 fallback，发布包仍有 GPT 默认模型，存在配置漂移 |
| 当前工作区 | 没有 Sub2API 后端源码，无法仅靠本地修改修复上游语义 |

### 2.2 当前文件基线

本次设计基于以下文件快照；无 Git 仓库时使用 SHA-256 作为可比对基线：

~~~text
codex-third-party-model-compatibility-plan.md
SHA256 7A0C9A15F4024626647D3C62CE1298D686B71D84674097EBB746BC46F6CA703D

src-tauri/src/app/model_catalog.rs
SHA256 E96FD2E4806ECD1603222088C5A481124415DDD42FA50DD926D1C5FAD26A052B

CCSwitch/CCSwitch/src-tauri/src/proxy/handlers.rs
SHA256 A393CC579BC1040924C687980C7057C959E2CD816B12609A805E0029FB67A1A2

CCSwitch/CCSwitch/src-tauri/src/proxy/providers/codex.rs
SHA256 03B322BCCD50B66F3E1C9F759D09662A3B84CACB17498898838D0F5D7D819039

release/usb-oneclick-g39.5-20260924-r23-codex917-consolefix/manifest.json
SHA256 CC20886FF21494EEA3A238921245BB7A9A8052208F504CE74BF0326C8D0E5949
~~~

后续每个执行任务必须记录自己的起始基线和最终差异，不得覆盖其他会话的未提交改动。

## 3. 根因矩阵

| 编号 | 根因 | 证据级别 | 影响 | 优先级 |
|---|---|---:|---|---:|
| R-01 | 本地目录把所有模型标记为完整 Code Agent 能力 | 已确认 | Codex 会向不支持的模型发送工具、Shell、推理和 Responses Lite 行为 | P0 |
| R-02 | 本地目录明确关闭 Skills/插件/App 使用说明 | 已确认 | 第三方模型可能看不到 Skills 相关 instructions | P0 |
| R-03 | CCSwitch Codex 路径是 Responses 透传 | 已确认 | 上游必须自己实现 Responses 与各模型协议转换 | P0 |
| R-04 | Sub2API 的 call_id、tool_result、response_id、事件顺序可能不完整 | 与症状高度吻合，尚无脱敏包级证据 | PowerShell 失败、Claude 循环、工具续接失败 | P0 |
| R-05 | `/v1/models` 只做发现，没有真实能力探测 | 已确认 | 模型显示状态被误当成兼容状态 | P1 |
| R-06 | 安装成功与网关适配成功没有硬门禁 | 已确认 | 用户看到“安装完成”，但模型实际不可用 | P1 |
| R-07 | 源码、发布包和模板存在固定 GPT 默认/回退 | 已确认 | 第三方模型选择和回归结果不稳定 | P1 |
| R-08 | 原生目录曾以较早 Codex 版本做过探测，当前发布包版本不同 | 文档证据，需复测 | 可能出现目录字段或加载行为不兼容 | P1 |

## 4. 目标架构

~~~text
Codex
  -> CCSwitch /v1/responses
  -> Sub2API Responses compatibility layer
  -> provider adapter: OpenAI / GLM / DeepSeek / Anthropic / ...
  -> upstream model
~~~

本地模型发现与能力门禁：

~~~text
GET /v1/models
       |
       v
Model discovery              只确认模型 ID/供应商
       |
       v
Capability probe             逐模型验证文本、工具、流式、多轮、Skills
       |
       v
Capability manifest          unknown / verified / quarantined
       |
       v
Codex native catalog         只写当前 Codex 接受的官方字段
~~~

职责边界必须保持：

- Sub2API：负责跨模型协议和 Responses 语义转换；
- CCSwitch：负责本地路由、鉴权注入和透传，不伪装成上游语义适配器；
- 安装器：负责安装、启动、模型目录刷新和就绪状态；
- Codex 原生目录桥：负责模型展示，不替代 Sub2API；
- 能力探测器：负责证明模型是否能作为 Code Agent 使用。

## 5. Sub2API P0 协议契约

Sub2API 必须把不同上游协议归一到稳定的 Responses 语义。至少保留以下身份字段：

~~~text
response_id
previous_response_id
item_id
call_id
tool_use_id
tool_name
arguments
tool_result
stream_event_type
~~~

必须支持：

- `POST /v1/responses`；
- `POST /v1/responses/compact`；
- 流式和非流式；
- 单工具、多工具和并行工具；
- `function_call` / `function_call_output`；
- Chat Completions `tool_calls`；
- Anthropic `tool_use` / `tool_result`；
- `previous_response_id` 多轮续接；
- 工具错误结果回传；
- UTF-8、长上下文和 reasoning 内容保留；
- 重试时不重复执行同一 `call_id`；
- 断线恢复和明确的结构化错误；
- 当前响应结束与整个 Agent 任务结束的区分。

推荐状态机：

~~~text
READY
  -> MODEL_RUNNING
  -> TOOL_CALLS_RECEIVED
  -> TOOL_EXECUTING
  -> TOOL_RESULTS_READY
  -> MODEL_CONTINUING
  -> COMPLETED
~~~

流式事件必须保持合法顺序，至少验证：

~~~text
response.created
response.in_progress
output_item.added
function_call_arguments.delta
function_call_arguments.done
output_item.done
response.completed
~~~

这部分不能由本地安装器“猜测修复”。如果没有 Sub2API 源码，应以脱敏的真实请求/响应/事件包作为证据，不能直接宣称上游已修复。

## 6. 本地程序整改契约

### 6.1 模型目录能力改为保守、可验证

修改范围：

- `src-tauri/src/app/model_catalog.rs`；
- 相关模型目录类型、测试和 UI 状态；
- 不修改 Codex 官方二进制。

要求：

- `/v1/models` 只负责发现 `raw_id`、`owned_by` 和展示名称；
- 不再对所有模型默认写入 `unified_exec`；
- 不再默认写入 `use_responses_lite=true`；
- 不再默认宣称 128K 上下文和全部推理等级；
- 不再统一关闭 Skills，必须由当前 Codex 字段能力和真实探测决定；
- 未验证能力必须是 `unknown`、禁用或隔离状态；
- 同一模型不能因为菜单可见就自动变成 `ready`。

建议能力状态：

~~~text
discovered
text_verified
tool_verified
shell_verified
skills_verified
stream_verified
multi_turn_verified
ready
quarantined
~~~

### 6.2 官方目录与诊断能力分离

官方 `model_catalog_json` 只写当前 Codex 已确认接受的字段。不得未经验证直接加入自定义字段。

自定义状态另存为本地 sidecar，例如：

~~~text
codex-usb-model-capabilities.json
~~~

sidecar 可以保存：

- provider 身份；
- 模型 ID；
- 能力状态；
- 探测时间；
- Sub2API adapter 版本；
- Codex 版本；
- 失败原因代码。

sidecar 不得保存：

- API Key；
- Authorization header；
- 账号密码；
- 完整用户请求正文；
- 模型私密响应内容。

### 6.3 自动探测与就绪门禁

安装后在已获得合法设备 Key 的前提下自动执行：

1. `/health`；
2. `/v1/models`；
3. 每个模型的最小文本请求；
4. 单工具调用；
5. 工具结果续接；
6. `unified_exec`/PowerShell；
7. 多工具调用；
8. 流式事件顺序；
9. Skills instructions 可见性；
10. 连续五轮对话；
11. 工具错误结果；
12. 重复调用和断线恢复。

安装成功与适配就绪必须分开显示：

~~~text
Codex 已安装
网关已连接
模型已发现
模型已通过 Code Agent 兼容性验证
~~~

任何未通过的模型只能显示为“已发现，未验证”或“已隔离”，不能显示为完整可用。

### 6.4 默认模型和回退策略

修改源码、发布包和模板中的固定 GPT 默认/回退逻辑：

- 默认模型只能从 `ready` 模型中选择；
- 没有 `ready` 模型时不得静默回退到 GPT；
- 固定 fallback 列表必须删除或改为经验证动态列表；
- 源码资源和正式发布包必须由同一生成步骤产生；
- provider、base URL 或账户绑定改变时，旧模型目录必须隔离。

### 6.5 版本门禁

当前发布包的 Codex 版本必须重新执行原生目录字段探测。此前较早版本的探测结果不能直接作为当前包的验收证据。

发现版本变化且未重新验证时：

- 保留 Codex 默认启动能力；
- 不写入未知目录格式；
- 保留旧目录但标记兼容性未知；
- 记录 `native_model_catalog_unsupported`；
- 不宣布第三方模型就绪。

## 7. 任务拆分与唯一写入边界

以下任务由后续会话按编号领取。每个任务只能有一个写入负责人；审计会话不得修改被审文件。

### T0：Sub2API Responses 契约与真实流量证据

- 负责人：待用户指派；优先分配给掌握 Sub2API 源码或真实测试环境的会话。
- 允许修改：Sub2API 仓库中的协议适配层和对应测试；若没有上游源码，本任务只产出脱敏抓包和契约文档。
- 不允许修改：本地 Codex 安装器、CCSwitch、模型目录。
- 前置：无。
- 交付：请求/响应结构、流式事件序列、call_id/response_id 关联、各模型差异、失败样例。
- 验收：GLM、DeepSeek、Claude 各完成普通文本、单工具、多工具、工具结果续接和五轮对话。

### T1：本地模型目录能力模型

- 负责人：待用户指派。
- 允许修改：`src-tauri/src/app/model_catalog.rs`、模型目录类型、Rust 单元测试。
- 不允许修改：Sub2API、Codex 官方二进制、CCSwitch 协议语义。
- 前置：T0 可以暂未完成，但所有未知能力必须安全降级。
- 交付：能力状态模型、保守目录生成、sidecar、无密钥测试。
- 验收：未经探测的模型不能获得 `unified_exec`、`responses_lite`、全推理等级和 `ready` 状态。

### T2：安装后自动探测与就绪门禁

- 负责人：待用户指派。
- 允许修改：`src/app/views/OfflineHome.tsx`、manager command/API、状态展示和相关测试。
- 不允许修改：模型协议转换、Sub2API 远程行为。
- 前置：T1 的能力状态接口冻结。
- 交付：首次启动自动探测、超时/失败恢复、隔离模型显示、重试入口。
- 验收：安装失败、网关失败、单模型失败和全部模型失败均不伪称适配完成。

### T3：provider 和默认模型配置收口

- 负责人：待用户指派。
- 允许修改：`src-tauri/resources/ccswitch/providers.json`、发布资源生成链、`codexTemplates.ts`及对应测试。
- 不允许修改：模型目录核心算法、Sub2API。
- 前置：T1/T2 的 `ready` 状态定义。
- 交付：移除固定 GPT fallback、源码/发布包一致、动态默认模型策略。
- 验收：无已验证模型时不会静默选择不存在或未验证的模型。

### T4：脱敏协议诊断

- 负责人：待用户指派。
- 允许修改：本地日志/诊断模块及测试。
- 不允许修改：请求正文语义、密钥存储和上游协议。
- 前置：T0 确认诊断字段。
- 交付：request_id、response_id、call_id、事件序号、模型、adapter 版本和错误代码。
- 验收：日志不包含 API Key、Authorization、密码和完整 prompt/response。

### T5：fixture、回归和发布门禁

- 负责人：待用户指派，必须与 T1/T2 执行者分离。
- 允许修改：mock gateway、Rust/TypeScript 测试、验收脚本和发布检查清单。
- 不允许修改：被审实现以制造通过。
- 前置：T0–T4 的接口冻结。
- 交付：本地可重复 fixture、三模型兼容性矩阵、当前 Codex 版本字段探测记录。
- 验收：正常、失败、边界和回归路径均有命令、退出码和产物证据。

### T6：集成、打包与独立审计

- 负责人：主控会话或用户指定的集成会话。
- 允许修改：集成配置、发布包、版本清单；不得改动未授权模块。
- 前置：T0–T5 均完成并固定版本。
- 交付：新 U 盘包、SHA256、安装后首次启动证据、独立审计结论。
- 验收：只有“符合”才能进入发布；“不符合”返工；“证据不足”补证。

## 8. 推荐并行顺序

~~~text
T0  Sub2API 契约/抓包 ─────────────┐
                                   ├─ T1 能力目录 ── T2 启动门禁 ─┐
T3 provider 配置收口 ─────────────┘                              ├─ T5 回归
T4 脱敏诊断 ─────────────────────────────────────────────────────┘
                                                                  ↓
                                                            T6 集成审计
~~~

T0 与 T1/T3/T4 可在不同写入边界内并行；T2 依赖 T1 的状态契约；T5 依赖所有接口冻结；T6 必须串行进行。

## 9. 统一任务派发模板

后续给会话派任务时，必须明确以下内容：

~~~text
任务编号：T1
目标：只实现本开发文档第 6.1 节的本地模型能力状态与保守目录生成。
依据：CODEX-THIRD-PARTY-MODEL-COMPATIBILITY-DEVELOPMENT.md，T1、6.1、6.2。
唯一写入边界：src-tauri/src/app/model_catalog.rs、对应类型和 Rust 测试。
禁止修改：Sub2API、CCSwitch 协议转换、Codex 官方二进制、其他任务文件。
前置：读取当前文件基线；未知能力必须安全降级。
验收：执行指定测试；证明未探测模型不会被标记为 unified_exec/ready；报告文件差异、命令、退出码和未验证项。
交接：返回 WRITER_STATUS、修改文件、基线/最终哈希、测试证据和待审问题。
~~~

执行会话不得自行扩大任务范围。发现需要改变公共字段、状态、协议或验收标准时，先返回 `DESIGN_QUESTION`，暂停依赖该决定的路径。

## 10. 验收矩阵

| 验收项 | GLM | DeepSeek | Claude | 证据要求 |
|---|---:|---:|---:|---|
| 普通文本 | 必须 | 必须 | 必须 | 非流式响应正确 |
| 单工具调用 | 必须 | 必须 | 必须 | call_id 与参数一致 |
| PowerShell/unified_exec | 必须 | 必须 | 必须 | 工具结果可续接 |
| Skills 可见性 | 必须 | 必须 | 必须 | instructions 未丢失 |
| 多轮续接 | 必须 | 必须 | 必须 | previous_response_id 正确 |
| 多工具调用 | 必须 | 必须 | 必须 | 多个 call_id 不串线 |
| 工具错误恢复 | 必须 | 必须 | 必须 | 错误返回后可继续 |
| 流式事件 | 必须 | 必须 | 必须 | 事件顺序完整 |
| 连续五轮 | 必须 | 必须 | 必须 | 无循环、无重复工具调用 |
| 断线恢复 | 建议 | 建议 | 建议 | 恢复后状态不重复执行 |

“模型能显示”“普通聊天成功”“HTTP 200”都不能单独作为 Code Agent 兼容性通过证据。

## 11. 发布门禁

只有同时满足以下条件，才能把 U 盘包标记为“第三方模型已适配”：

- T0 有真实 Sub2API 协议证据或源码测试；
- T1 不再伪造未知能力；
- T2 安装后会自动探测并正确隔离失败模型；
- T3 源码、发布包和模板无固定模型漂移；
- T4 有脱敏诊断且没有秘密泄露；
- T5 在当前 Codex 发布版本上完成真实或等价 fixture 验收；
- T6 由独立会话完成只读审计；
- 当前发布包经过真实设备 Key 和真实网关回归；
- 三类目标模型均通过工具、Skills、流式和多轮验收。

否则只能发布为：

~~~text
Codex 安装可用；第三方模型目录已发现；Code Agent 兼容性未完成验证。
~~~

## 12. 当前未决问题

| 编号 | 问题 | 责任任务 | 阻断范围 |
|---|---|---|---|
| Q-01 | 是否能取得 Sub2API 源码或脱敏 Responses 流量 | T0 | 上游修复和最终兼容性结论 |
| Q-02 | 当前 Codex `26.917.62051` 接受哪些模型目录字段 | T5 | 原生目录发布 |
| Q-03 | GLM/DeepSeek/Claude 的真实模型 ID 和 provider owner | T0/T5 | 能力探测参数 |
| Q-04 | 当前真实 USB Key 是否能访问 `/v1/models` 和 `/v1/responses` | T5 | 现场回归 |
| Q-05 | `responses_lite` 是否适合第三方模型 | T0/T5 | 目录能力字段 |

在 Q-01、Q-02、Q-04 未解决前，不得宣布最终兼容完成。

## 13. 关联文档

- `docs/CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md`
- `docs/CODEX-NATIVE-MODEL-CATALOG-M0-PROBE-20260922.md`
- `docs/SUB2API-CONTRACT-REFERENCE.md`
- `docs/USB-404TOKEN-CCSWITCH-LINKAGE-DEVELOPMENT.md`
- `docs/G39.5-R7-FIELD-ISSUES-REPAIR-DEVELOPMENT.md`
- 用户提供的 `codex-third-party-model-compatibility-plan.md`

本文件针对第三方模型 Code Agent 兼容性整改，优先级高于旧文档中“目录已接入”的描述；旧文档中已实现的模型发现能力不能替代本文件规定的能力探测和真实网关验收。



