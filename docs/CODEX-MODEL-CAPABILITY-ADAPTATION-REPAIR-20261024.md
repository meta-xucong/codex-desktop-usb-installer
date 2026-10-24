# Codex 自定义模型能力适配修复（2026-10-24）

## 1. 目标与判定

恢复 USB 版为自定义 Provider 模型写入目录时对 Codex 原生宿主能力的兼容，不改 Product Design。修复应让模型在 Codex 声明的默认输入模态下使用宿主提供的工具，同时通过简短、通用的模型说明降低外部 Provider 的协作和 Windows 运行时误用；另修复本地模型目录准备时“数据已成功取得，却因临时目录删除失败而中断”的错误门。

核心判定：Codex 的 `view_image` handler 在访问文件前会先检查模型目录的 `input_modalities`。若条目明确写成 `['text']`，Codex 就返回 `view_image is not allowed because you do not support image inputs`；文件权限不能改变这一判定。USB 当前构造的新模型条目明确写入 `['text']`，因此是对 Codex 原生默认能力的主动收窄。OpenAI Codex 源码的 `default_input_modalities()` 注释说明默认值兼容为 text 与 image，只有 preset 明确收窄时才覆盖默认值。修复采用“省略 `input_modalities` 字段，让 Codex 使用自身默认值”，不另造能力枚举、网络探测器或权限绕过。

## 2. 证据与来源

| 证据 | 来源 | 对设计的约束 |
|---|---|---|
| `view_image` 先检查 `model_info.input_modalities`，不含 image 时在读取文件前拒绝 | Codex 官方源码 [`view_image.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/handlers/view_image.rs) | 精确错误应修模型能力元数据；不能通过放宽文件权限修复 |
| Codex 默认输入模态为 text + image，显式 preset 可收窄 | Codex 官方源码 [`openai_models.rs`](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/protocol/src/openai_models.rs#L194-L200) 和 [`ModelInfo` 反序列化](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/protocol/src/openai_models.rs#L477-L480) | 对自定义 catalog 删除人为 text-only 覆盖，采用原生缺省 |
| 当前 USB 新条目明确序列化 `input_modalities: ['text']` | `src-tauri/src/app/model_catalog.rs` 的 `conservative_tested_model_entry` | 这是可直接修复的本地能力收窄点 |
| Skills 是工作流指令，MCP 提供工具/动作；长而矛盾的提示会降低遵循效果 | [Codex Skills 官方文档](https://developers.openai.com/plugins/concepts/skills)、[官方提示设计文章](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra) | 只增加一段短而通用的自定义模型基线指令，不修改技能内容或 MCP |
| 旧安装日志记录 `native-final-cleanup`，但当前直接添加合同不再调用最终 CLI 校验；仍可达的 `native_default_catalog` 在读取/序列化成功后因 `area.cleanup()` 失败而报错 | `docs/MODEL-ADD-DIRECT-COMMIT-20261024.md`、`model_native_probe.rs`、`model_catalog.rs` | 只修当前仍可达的首次原生目录提取清理门；不恢复 post-probe CLI 校验 |

上游问题报告可作为现象参考，不能当作 Codex 官方行为承诺：[`#32267`](https://github.com/openai/codex/issues/32267) 报告与 text-only modality 相同的 `view_image` 拒绝；[`#37822`](https://github.com/openai/codex/issues/37822)、[`#36586`](https://github.com/openai/codex/issues/36586) 报告自定义 Responses Provider 下 delegation/encrypted handoff 的失败。该项目没有实现解密 `agent_message` 的协议层，禁止自行添加解密代理或 Provider 协议仿真。

## 3. 风险轴与工作协议

- **D0**：本轮范围和修复方向由用户明确确认；Product Design 已明确排除。
- **I2**：修改自定义模型 catalog 的缺省能力和启动时的管理器目录迁移，并调整首次原生目录提取的临时清理处理；与 MCP TOML/MCP 启动链路保持隔离。
- **A1**：不访问或写入真实 Provider，不发起收费图像/模型请求；使用离线 fixture、Rust 单测、前端既有测试与本地 Codex CLI（如本机可用）。
- 先独立审计本文件，PASS 后才实施；实施后冻结差异，再独立代码审计，缺陷修复后重审最终差异。
- 仓库原有未提交改动属于用户基线，逐项保留。禁止全文件格式化、清理临时目录、重置或覆盖无关改动。

## 4. 产品与技术规则

### 4.1 新增模型：恢复 Codex 默认输入能力

1. `conservative_tested_model_entry` 不再输出 `input_modalities`。
2. 不以“已测试过文本响应”为由把模型写成 text-only。思考档位探测仍只决定 reasoning 支持集合，不声称已验证 Provider 的视觉质量。
3. 不主动设置 `supports_image_detail_original` 等与该错误无关的字段，不扩大搜索/插件权限，不修改 `config.toml` 中的 MCP 配置。`include_*_usage_instructions` 是 Codex 宿主能力使用说明的注入开关，不授予工具权限；旧条目的迁移只恢复这些说明。
4. Codex 按原生默认值提供图像输入能力。若某 Provider/模型本身不接受图像请求，错误由该 Provider 返回；本次不新增图像探测或动态 capability 框架。对实际纯文本模型，使用者应选择支持视觉的模型，或由后续有上游能力证据的单独需求细分处理。

### 4.2 已添加模型：只迁移管理器生成的精确旧模板

管理器启动时现有 `preflight_usb_catalog()` 已在 Codex 启动前读取、验证并以原子写事务修复管理器拥有的目录。本次复用该路径，不重建目录、不触碰 `config.toml`。

仅当一个条目同时满足以下已核对的旧生成模板签名时迁移：

- `input_modalities` 恰为字符串数组 `['text']`；
- `description` 恰等于旧生成器描述 `Text model supplied by the configured gateway; only one text response was verified.`；
- `base_instructions` 恰等于旧 USB 单句模板 `You are Codex, a coding agent. Help the user with software engineering tasks by inspecting the workspace, making focused changes, and verifying your work.`；
- 不存在 `model_messages` 字段，避免覆盖由 Codex 原生模型模板提供的指令。

命中时只删除该条目的 `input_modalities` 字段；更新 `description` 为 `Gateway model supplied by the configured provider; text response verified, image understanding not separately tested.`，并更新 `base_instructions` 为 4.3 的新基线。其余模型字段（包括 `include_skills_usage_instructions`、`include_plugin_usage_instructions`、`include_apps_usage_instructions`、slug、reasoning 集合、优先级、顺序、未识别字段及其他可选字段）原样保留。任一签名字段被用户改过、字段类型不符或存在 `model_messages` 时，不把它当成旧模板；本次能力迁移器不得改变该条目的 capability、description 或 instructions 字段。通用旧版 JSON 形状修复仍遵守已有逻辑，但不得被本次能力迁移测试冒充为“整条 entry 未变化”。迁移幂等；第二次启动不再写目录。

**明确的迁移限制：**仓库旧模板把三个宿主使用说明开关设为 `false`，当前工作区模板设为 `true`；已有 catalog 没有记录来源版本或用户改动历史。因此 `false` 既可能来自旧默认，也可能是用户主动选择，无法安全地区分。本次不自动改写既有条目的三个开关。新添加模型继续使用当前生成器的 `true` 值；恢复旧条目的这些开关需要另行设计明确、用户可控的操作，超出本合同。

已有通用 JSON 形状修复若遇到缺少 `base_instructions` 的条目，仍补回简短 fallback `LEGACY_FALLBACK_MODEL_BASE_INSTRUCTIONS`。它与旧生成模板的精确签名 `LEGACY_MODEL_BASE_INSTRUCTIONS` 不同，避免该结构性补齐在下一次启动意外触发能力迁移；只有 4.2 精确匹配的条目可得到新的长指令。不得借通用 schema 修复把新提示词写入未命中的旧模型。

继续使用现有 ownership 检查、catalog JSON/schema 校验、配置快照冲突检查、原子替换、`.bak` 保留/恢复。无法通过管理器目录身份检查时不迁移。迁移失败按现有 fail-closed 启动错误显示，保留当前文件及备份供复位/恢复。迁移只改 `model_catalog.json`，不得读改 `config.toml` 的 MCP 段。本节是 `MODEL-ADD-DIRECT-COMMIT-20261024.md` 中“活动目录重复 Add 保持旧条目原样”的唯一例外：仅由启动 preflight 对精确旧生成模板执行上述兼容迁移；用户点击重复 Add 时仍保持原条目不变。

### 4.3 短小通用的自定义模型指令

更新 `MODEL_BASE_INSTRUCTIONS`，用紧凑、可执行的规则补充现有编码代理身份：

- 只使用 Codex 当前会话实际提供的工具和能力；工具不存在、拒绝或无结果时，不得声称操作成功，也不得编造图像观察或文件内容。
- 需要观察图像时使用宿主提供的图像能力；若 `view_image` 明确报告模型不支持图像输入，说明当前模型/目录能力阻断，停止猜测视觉内容，继续完成不依赖视觉的部分或请用户切换模型。
- 外部 Provider 的子代理/协作消息若表现为不透明编码内容、无法确认子任务收到或返回结果，不尝试解码、不循环重派；默认优先在当前会话顺序完成，确有必要时最多尝试一次并在失败后本地继续。
- Windows 环境使用当前会话实际 shell 的语法；运行 Python 优先使用已配置的便携运行时路径（如 `CODEX_PYTHON` 或 USB 包装器提供的解释器）；文本/JSON 按 UTF-8 读写，不混用 Bash heredoc 与 PowerShell 语法。
- 只依据命令/工具的可见结果汇报完成状态；证据不足时明确说明未验证。

新指令如下，约 100 个英文词；避免产品技能细则、强制工具、全局权限打开、模型名称硬编码或与 Codex 官方系统指令重复的大段说明：

> You are Codex, a coding agent. Help with software engineering by inspecting the workspace, making focused changes, and verifying your work. Use only tools and capabilities available in this session. Claim image observations or successful actions only when supported by tool output. If image input or a tool is explicitly unsupported, report the limitation, do not invent results or repeat the same failing call, and continue only with supported work or ask for usable input. For external-provider delegation, prefer local sequential work; if a handoff is opaque or unconfirmed, do not decode or loop-retry it, and continue locally. On Windows, use the active shell syntax, configured USB Python runtime, and UTF-8 for text/JSON. State completion only from visible evidence.

新增目录项使用该常量；旧目录只按 4.2 精确签名迁移。不要把这些规则写进 Product Design 或任何其他 skill 文件。

### 4.4 首次原生目录提取：安全校验保留，删除失败降级为待清理

`native_default_catalog` 在 Codex `debug models` 成功、输出解析成功、目录序列化和大小限制检查通过后，所需数据已全部留在内存。后续临时目录删除失败不应阻止模型添加准备。

- 先继续执行严格的临时路径安全校验：根目录必须可 canonicalize，canonical 根路径必须等于对象创建时记录的路径，目录名必须以 `codex-native-probe-` 开头，父目录必须等于系统临时目录；目录内的 owner marker 必须包含该随机根目录名。无法验证路径归属、路径被替换/重建、marker 缺失或越界时仍返回错误，不能把安全失败降级。
- 路径验证通过后，`remove_dir_all` 失败视为 `cleanup_pending`：记录不含路径、凭据或用户内容的 warning，继续返回已验证目录数据；`TemporaryArea::drop` 保留重试删除。
- 目录读取、子进程/Job 清理失败、非零退出、JSON/模型目录解析失败、大小超限和取消/超时仍按原错误处理；这条规则只覆盖成功提取后的临时文件系统删除失败。
- 不重新启用 `validate_final_catalog`，不恢复探测完成后的 `Codex debug models` 校验，不改变直接添加合同里的网络请求数和凭证规则。

## 5. 禁止范围

- 不改、不读取后重写、不安装或测试 Product Design 插件及其 skill 文件。
- 不改 MCP 注册、MCP 服务启动、`config.toml` 合并算法、CCSwitch、404token 凭据、模型探测/票据合同、thinking effort 推导、安装器、卸载器、签名和发行逻辑。
- 不添加 `agent_message` 解密、Responses 协议代理、自动开启所有工具、全局权限修改、图像/模型联网探测、并发兼容性框架或额外安装步骤。
- 不恢复探测后的 CLI 校验门；直接添加合同继续生效。
- 不打包、不签名、不复制到 U 盘。

## 6. 允许写入文件与逐项落地

| 文件 | 改动 | 必要回归证据 |
|---|---|---|
| `docs/CODEX-MODEL-CAPABILITY-ADAPTATION-REPAIR-20261024.md` | 本实施合同、审计记录 | 审计状态及最终证据如实更新 |
| `docs/CODEX-SELECTIVE-UPSTREAM-MODEL-MANAGEMENT-DEVELOPMENT.md` | 将旧“仅 text 输入”规则改为引用本合同的 Codex 原生缺省规则 | 不再存在相互矛盾的有效开发说明 |
| `src-tauri/src/app/model_catalog.rs` | 新条目省略 `input_modalities`；定义新 description/base instructions；只迁移签名完全匹配的旧模板；更新定向单测 | 新条目字段缺省；精确旧模板迁移；用户编辑条目保留；幂等；现有 config/MCP 字节保留测试通过 |
| `src-tauri/src/app/model_native_probe.rs` | 区分 cleanup path ownership validation 与删除操作；对成功捕获后的删除失败警告并继续，保留 Drop retry；更新定向单测 | unsafe path 仍失败；仅 safe remove failure 允许成功返回；解析/进程错误仍失败 |

除上述文件外不写其他文件。若审计发现实现必须扩大范围，先暂停并更新本合同、再重新审计。

## 7. 测试与验收矩阵

| 测试 | 预期结果 |
|---|---|
| 新增模型条目构造 | 没有 `input_modalities`；保留支持/默认 reasoning 集合；无搜索/插件 capability 外扩 |
| 官方兼容行为静态断言 | 自定义条目缺省后不再显式限制为 text-only；记录 Codex upstream default 是 text+image |
| 精确旧模板迁移 | 删除 `['text']`，更新描述/指令，所有宿主说明开关、slug、reasoning、顺序、优先级及其他字段不变 |
| 用户编辑签名测试 | 分别修改 description、base instructions 或 modalities；添加 `model_messages.instructions_template` 或任意 `model_messages` 时跳过迁移。宿主说明开关的全 false、全 true、混合/缺失等组合都按原值保留，不管其他迁移是否命中 |
| 非签名旧条目提示词 | 缺少 `base_instructions` 的非命中条目仅回填旧短指令；只有精确旧模板迁移获得新长指令 |
| 旧规则混合目录 | 仅迁移命中的条目；其他条目保持原字节语义 |
| 幂等/启动写入 | 首次迁移一次；再次 preflight 不写、不轮转 backup |
| MCP/config 不变 | 启动迁移前后 `config.toml` 字节完全一致，含 `[mcp_servers.media_generation]`；目录迁移不触及配置文件 |
| Codex 本地解析（若随包 CLI 可用） | 在临时 `CODEX_HOME` 载入省略 `input_modalities` 的 catalog 并执行本地 `debug models`/等效 parser 检查；若无法从 parser 输出确认解析后的 image 默认值，只能报告“目录解析通过、图像能力运行时未验证”，不能把静态字段断言称为端到端图像测试 |
| 目录 ownership/schema/备份 | 非管理器目录拒绝迁移；迁移写失败时既有备份恢复/保留行为不退化 |
| 清理边界 | 目录身份校验失败仍返回错误；不同路径替换、同路径重建/缺 marker 均拒绝；安全路径删除失败只记录 cleanup pending 并返回已解析目录；Drop 重试；Codex 进程/解析失败仍失败 |
| 全量静态检查 | 定向 Rust tests、相关 React tests/typecheck（如文件未变可说明不适用）、格式检查、代码搜索确认无 Product Design diff 与无 text-only 强制字段 |

所有测试均使用临时 CODEX_HOME、临时目录、注入的 I/O 结果或 mock；不写真实用户 Codex 配置、不请求 404token、不做付费模型调用。若 Cargo/编译器/本地 Codex CLI 缺失，记录具体不可执行项和环境证据，不能以静态检查代替并宣称通过。

## 8. 回滚与兼容

- 新安装模型直接采用 Codex 缺省输入模态。
- 既有管理器目录只迁移严格命中旧生成签名的条目。迁移前使用已有原子写/备份机制；用户修改过的模型元数据不被改写。
- 如遇目录校验或写入失败，按现有模型目录 preflight 错误处理，可由用户通过模型目录复位回到 Codex 原生模型目录；不自动删除 MCP 或其他配置。
- 临时目录删除失败会由析构重试；若仍残留，只留下本次随机命名的临时探测目录，不影响已提取的内存目录或 MCP 配置。

## 9. 独立审计放行条件

实施前审计必须检查：证据是否直接支持 capability 修复；迁移签名是否足以避免覆盖用户定制；是否存在重写 config/MCP 的间接调用；cleanup 安全检查是否仍在 remove 前且 path failure 不会被吞；测试是否覆盖当前未提交基线及用户硬约束。任一实质问题为 FAIL，修订合同后再审。没有实际模型/effort 路由来源时标记 `ROUTE_UNVERIFIED`，不得声称审计模型路由已验证。

实施后审计只读检查最终 diff、调用链和测试证据；范围外文件变化、MCP 写入路径变化、用户内容覆盖、cleanup 越权或无依据声称测试通过均为阻断缺陷。

## 10. 实施记录

| 阶段 | 结果 |
|---|---|
| 证据/范围冻结 | 完成；未改 Product Design |
| 独立开发文档审计 | PASS（第 3 轮）；审计内容结论通过。Agent 模型/effort 路由来源不可核实，标记 `ROUTE_UNVERIFIED`，不声称路由已验证 |
| 实施 | 已完成；按本文件第 6 节修改模型条目缺省模态、精确旧模板迁移与原生目录临时清理处理 |
| 独立代码审计 | 第四轮静态复审未发现新的代码阻断；结论 `INSUFFICIENT_EVIDENCE / ROUTE_UNVERIFIED`，因 Windows Rust 单测未执行且审计运行路由不可核实，不记作流程 PASS |
| 测试 | `git diff --check` 通过；随包 Codex 0.162.0-alpha.2 在隔离临时 `CODEX_HOME` 解析省略模态字段的 catalog，并输出 `input_modalities: ["text", "image"]`。Rust 单测未执行：本机无 `cargo`、`rustc`、`rustfmt`；未做真实 MCP 会话或上游付费请求 |
| 2026-10-24 当前状态复审 | 独立只读审计结论 `INSUFFICIENT_EVIDENCE / ROUTE_UNVERIFIED`：静态审查未发现明确代码阻断，但不能据此验收。重新运行 `git diff --check` 通过；Rust 工具链缺失，当前源码单测无法执行。尝试用本机 Codex CLI 的 `debug models --bundled` 解析临时自定义目录时，CLI 按该选项语义只输出随包目录，未包含临时模型，因此本轮没有重新验证自定义 catalog 的默认 image 模态。旧记录的隔离 CLI 解析结果仅保留为历史证据，不作为本轮复测。真实 MCP 会话和 Windows `Drop` 重试测试仍待测。 |
| Rust 环境复查及针对性复测 | 查明 Rust 实际安装在 `D:\CargoMoved\rustup-home\toolchains\stable-x86_64-pc-windows-msvc\bin`；用户 PATH 原先只指向不存在的 `D:\CargoMoved\cargo-home\bin`。已将真实 stable `bin` 持久加入用户 PATH（新终端生效）。直接调用工具链首次编译发现 `model_native_probe.rs` 将 `remove_dir_all` 函数项传给泛型闭包时产生 `FnOnce` 生命周期编译错误，已改为显式闭包。将 `TEMP`/`TMP` 指向工作区可写临时目录后，模型目录测试 73/73、原生探测/清理测试 27/27 通过。全量 795 项库测试未完成：受限环境下出现其它模块测试失败，并卡在联网夹具，已停止且没有完整汇总；`rustfmt --check` 未通过，提示两文件存在格式差异，本轮未整文件格式化。最终独立定向代码复审 PASS（限相关代码与本次小修）；模型/effort 路由仍 `ROUTE_UNVERIFIED`，真实 MCP 会话仍待环境验证。 |
