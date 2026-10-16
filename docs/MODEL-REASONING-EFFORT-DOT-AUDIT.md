# 发给 dot 的独立审计任务：第三方模型思考强度适配实现

请对本地仓库 `D:\AI\AGENT_U盘\codex-desktop-usb-installer` 中已实现的代码和开发设计文档进行一次**只读、独立、深度审计**：

- 主设计：`docs/MODEL-REASONING-EFFORT-DEVELOPMENT.md`
- 固定基线：`45413fab1bab7b9697ef730ac992c711b940ab2`
- 审计范围：当前未提交 diff，重点审阅新增能力探测、proof 绑定、catalog 写入与全局 effort 冲突保护、错误分类和测试；实现已完成，尚未打包/发布。

## 问题背景

用户导入 `claude-opus-5-5` 等第三方模型后，Codex 请求出现类似：`does not support reasoning effort "none"; use low, medium, high, xhigh or max`。现有代码发现模型时只保留 `/v1/models` 的 model ID 和 owner；连通性测试是未显式设置 reasoning effort 的一次文本请求；添加到 Codex 时写入 `default_reasoning_level: null`、`supported_reasoning_levels: []`。这些现状吻合症状，但目前尚未证明空目录元数据正是 Codex 发出 `none` 的原因，设计将当前捆绑 Codex + mock gateway 的基线请求捕获列为实现前门槛。

安装器接的是 404token/Sub2API OpenAI-compatible 统一网关。即使 Anthropic、OpenAI、xAI、Google、DeepSeek、Qwen/GLM 等官方文档说明了原生支持档位，也不能因此假设当前中转 endpoint/adapter 路由能一一对应。

## 要审的核心判断

1. 文档是否正确区分“厂商原生能力”“网关 `/models` 声明”“当前 404token endpoint 实测能力”，并阻止把名称匹配当作路由能力证明？
2. “无能力数据时不能让空数组隐式落到 `none`”的原因是否成立？设计不得把这一点当作已证实根因；它将先用固定 Codex + mock gateway 对空元数据、单档 metadata、global effort 存在/不存在做四组黑箱捕获。请检查本项目固定 commit 的实际 schema/CLI 语义与该验收是否可行。
3. 推荐的流程是否能在不增加未授权收费请求的前提下，让普通刷新/连接测试/Add/启动不暗中 sweep？明确档位测试的 UI consent 和后端 enforcement 是否足够？
4. 对未知型号仅开放当前路由接受的请求档位、默认值必须包含在支持集、`none` 只可显式声明/接受后写入，是否安全且符合当前 Codex catalog schema？显式全局 effort 与模型接受集合冲突时阻断 Add/update、保留用户全局值是否正确？
5. 能否复用现有 short-lived connectivity proof、绑定 fingerprint、catalog digest、cancellation、request lifecycle、atomic catalog write/native validator；文档是否漏列了关键既有服务、类型、命令或测试？
6. 模型 capability evidence 是否需要写入磁盘？短期 proof 与 Add 之后写入永久 Codex catalog 的生命周期是否充分？设计已收窄保证为 Add 时绑定的路由接受状态，并承认运行期静默换路不能自动失效；endpoint/账号或上游静默变更后，重测/更新是否可行且边界是否清楚？同模型 re-add 是否安全且不丢其它条目？
7. 固定 Codex runtime 下全局 reasoning setting、model catalog default 和 UI 支持项的优先级是什么？文档是否遗漏了用户全局设为 `none` 等冲突配置，以及是否应保留用户设置？
8. 代码清单是否覆盖从 `gateway_model_probe.rs::parse_model_catalog` 真实管理器解析主路径、Rust request struct / serialization / Tauri command / TS shared type / manager UI / backend validation 到 `codex debug models` acceptance 的完整流？注意 `model_catalog.rs::parse_gateway_models` 是另一条目录路径，不可误认为唯一入口。指出遗漏/冗余文件及可复用符号。
9. 官方来源的型号/默认值/档位是否有错、过时或范围过宽？特别关注 Claude Opus 5.5、Grok 4.5/4.6/4.20 multi-agent、Gemini per-model `thinking_level`、DeepSeek 映射及百炼映射。对缺少 MiniMax/Moonshot/Kimi 官方证据的地方，是否应保持未知？
10. 测试矩阵是否覆盖 malformed metadata、unsupported vs transient error、费用拒绝、过期/重放 proof、绑定切换、profile conflict、历史目录迁移、非 manager-owned 文件保护、Add additive/Reset only、Codex runtime 没有 false positive？
11. D/I/A、实现范围、放行门是否合理，有无对外宣称过度或设计范围漂移？

## 当前作者侧验证收据

- 内部独立设计审计和两轮代码复审均 PASS；这不是 dot 的结论，请独立复核。
- 完整前端测试：484 passed，1 skipped；定向模型管理器 UI：19/19。
- TypeScript check、目标文件 ESLint、Vite production build 通过。
- Windows Codex CLI `0.162.0-alpha.17.2` 在隔离 `CODEX_HOME` 下读取了本地 model catalog fixture，成功解析 `medium` 默认及 `low/medium/high/xhigh/max` 列表。
- 完整 Rust lib 测试：740 passed，6 ignored；当前仅 mock/离线验证，不调用真实上游。

## 输出格式

给出一个明确结论：`PASS`、`FAIL` 或 `INSUFFICIENT_EVIDENCE`。逐条写：

1. 发现列表（严重级别、仓库路径及行号、复现条件/影响、必须修订项）。
2. 独立核实哪些请求路径、proof 绑定、目录写入/更新和 Codex catalog schema；不要只复述作者说明。
3. 检查 `max_output_tokens=1024`、结构化 400 拒绝分类、incomplete→unknown 与 UI 提示是否可靠，是否存在错误 Add 或假阳性。
4. 明确哪些验证只覆盖 mock/当前 CLI，哪些仍需真实 404token endpoint 或正式发行包验证。

## 审计边界

- 只读：不要编辑源代码或开发文档，不要运行真实模型、不触发付费请求、不读写 API key、不操作签名/许可/USB/VPS。
- 不要假设没有在审计输入中给出的 Sub2API route mapping，也不要把厂商文档支持等同于转发线路支持。
- 若无法检查固定 commit 的 Codex runtime schema 或完整证据，返回 `INSUFFICIENT_EVIDENCE`，不要用猜测替代验证。
- 返回结果后等待主任务根据 findings 修订；不要自行实施修复。
