# Codex 上游模型按需发现、验证与启用开发文档

**文档版本：** 1.1<br>
**状态：** `CONTRACT_AUDITED_PASS`<br>
**日期：** 2026-10-09<br>
**任务分类：** `D0 / I2 / A2`<br>
**基线提交：** `c35112c50caa1880f686b4f946c1821477601547`

## 1. 用户目标与开发边界

用户要求废弃“把上游目录中的模型自动加入 Codex”的旧行为，改为：用户主动打开二级弹窗发现上游模型；可以逐个测试连通性；只有测试通过后才允许点“添加进 Codex”；页面提供“一键复位”，恢复 Codex 自带 GPT 模型目录。现有实现能复用的部分须优先复用，并通过独立文档审计、实现审计和功能测试后再交用户验收。

本任务的唯一写入者是主控会话。文档审计和代码审计由不同于写入者的只读 Agent 执行。源码参考来自当前仓库，不涉及外部原型迁移。

### 1.1 硬约束

1. `/v1/models` 发现结果不得自动进入 Codex 可选模型目录。
2. 打开弹窗、刷新目录、安装后启动、管理器启动均不得发起模型生成请求。
3. 连通性测试必须由用户对具体模型主动触发；每次只发一条轻量 Responses 文本生成请求，不重试生成请求，并明确提示可能计费。
4. 只有该模型本次 Responses 文本调用通过后，UI 才显示“添加进 Codex”。添加只扩展菜单，不改当前默认模型、CCSwitch Provider、API Key 或账户绑定。
5. “一键复位”移除本管理器拥有的第三方模型目录覆盖，使 Codex 回到其随当前版本提供的原生模型列表；不按 `gpt-` 前缀猜测官方模型，不触碰 CCSwitch 连接、凭证、MCP、Skills 或其他用户配置。
6. 新发现模型不会在后台刷新时自动重新加入。旧版本已加载的目录在升级时保留，直到用户点击复位；复位后自动刷新不能重新加回任何模型。
7. 仅真实 Responses 文本兼容性通过的模型可以添加。图片、视频或其他非 Codex 对话接口即使出现在 `/v1/models`，也不能因名称或目录存在而加入。
8. 保留本任务开始前的所有工作区改动；特别是 `OfflineHome.tsx`、`OfflineHome.test.tsx`、`i18n.tsx`、`styles.css` 和 `types.ts` 已有的未提交改动。
9. 不调用真实上游、不发起付费生成、不改 Sub2API/404token 服务端、不打包发布、不提交或推送。

### 1.2 非目标

- 不把一次文本调用宣传为完整 Codex Agent 兼容性认证；工具调用、流式、多轮、Shell、Skills 能力仍属于现有深度诊断/兼容性工作。
- 不修改模型别名的既有归一化规则，不在上游名单建立厂商白名单。
- 不改媒体生成 MCP；图像/视频能力仍由媒体 MCP 单独负责。
- 不更换当前 404token Provider 为 OpenAI 官方直连。复位的是 Codex 模型目录，不是 Provider 路由。

## 2. 当前实现核对

当前有两条含义不同的路径，不能混为一谈：

### 2.1 自动目录同步会自动加载模型

- [OfflineHome.tsx](../src/app/views/OfflineHome.tsx)：安装后的 CCSwitch 成功分支会调用 `refreshNativeModels(true)`；已安装首页启动后也会延迟调用。高级功能中另有“刷新模型列表”按钮。
- [model_catalog.rs](../src-tauri/src/app/model_catalog.rs)：`refresh_native_model_catalog()` 使用已授权 U 盘绑定、provider 地址和用户 Key，读取 `/health`、`/v1/models`，归一化上游目录后调用 `native_catalog()`，再经 `write_catalog_and_config()` 写入管理器拥有的 JSON 和 `config.toml` 托管块 `model_catalog_json`。现有目录中不存在时会把发现到的模型集合写入 Codex，因此等于自动启用。
- `native_model_entry()` 为目录项填充统一的通用字段；当前目录流程不会逐模型发送 Responses 请求，所以 `ready`/可见不能代表上游模型真实可调用。该目录路径也没有按视频或其他非 Responses 模型类型过滤。
- `CODEX-THIRD-PARTY-MODEL-COMPATIBILITY-DEVELOPMENT.md` 已把 `native_model_entry()` 的统一 image/tool 等能力标记列为能力误报风险。新 Add entry 不盲目复制这些声明；只给本次验证过的 text input 和 Codex schema 必需字段，所有未验证能力保持不声明/禁用。
- [model_catalog.rs `cleanup()`](../src-tauri/src/app/model_catalog.rs)：已有清理管理器自有目录和配置块的逻辑，目前用于卸载收尾，没有页面上的复位命令。

### 2.2 高级诊断可以测模型，但不添加模型

- [GatewayModelProbePanel.tsx](../src/app/views/GatewayModelProbePanel.tsx)：位于“高级功能”展开区。目录检查调用 `gatewayModelProbe({ selectedModelIds: [], costConsent: false })`，只读取 `/v1/models` 并展示结果；它本身不修改 Codex/CCSwitch 配置。
- [gateway_model_probe.rs](../src-tauri/src/app/gateway_model_probe.rs)：解析上游目录、检查重复 ID、最多允许选 3 个模型。选定模型后要求显式费用同意，再调用 `ProbeClient::probe_model()` 做协议矩阵；最多每模型 12 次真实生成请求、最长 10 分钟。矩阵结果只用于诊断，不触发“添加进 Codex”。
- [model_probe.rs](../src-tauri/src/app/model_probe.rs)：`ProbeClient` 已具备 HTTPS/Bearer 安全客户端、请求预算/取消、Responses 请求与响应校验、随机挑战文本校验和脱敏诊断。`receive()` 每个请求只发送一次，不自动重试；`text()` 已是可复用的单次文本请求实现，但当前为私有方法。

### 2.3 现有 UI 和配置写入复用件

- [Sheet.tsx](../src/app/Sheet.tsx)：标准二级弹层，已有焦点陷阱、Escape、无障碍语义及关闭动画，应复用，不另造 Dialog。
- [managerApi.ts](../src/services/managerApi.ts)：已有 `gatewayModelProbe()` / `cancelGatewayModelProbe()` Tauri 封装。
- [commands.rs](../src-tauri/src/commands.rs) 与 [lib.rs](../src-tauri/src/lib.rs)：分别提供现有命令薄包装和 invoke 注册模式。
- [model_catalog.rs](../src-tauri/src/app/model_catalog.rs)：已有 `FILE_LOCK`、路径普通文件/重解析点检查、有限长度读取、`owned_json()`、托管配置块解析、原子写入、provider URL 校验、模型解析/归一化、Codex 目录条目生成、重载标记，以及卸载清理。应在这些边界上扩展，禁止另造一套配置写入/所有权判断。

### 2.4 旧文档关系

- `docs/CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md` 中“把 `/v1/models` 的全部发现模型写入 Codex 菜单”和“自动刷新”的产品策略在本任务完成后被本文件替代；其官方 `model_catalog_json` 接入证据、目录 schema、原子写入和 Codex 版本注意事项继续有效。
- `docs/CODEX-MODEL-REFRESH-ALIAS-MERGE-DEVELOPMENT.md` 针对 CCSwitch Provider 编辑器下拉，不是 Codex 原生菜单启用策略；别名归一化实现可复用，但不得把 CCSwitch 编辑器状态当成原生目录持久状态。
- `docs/CODEX-THIRD-PARTY-MODEL-COMPATIBILITY-DEVELOPMENT.md` 中“发现不等于兼容”继续有效。本功能快速测试只证明一条 Responses 文本请求成功，不替代深度协议/Agent 能力验证。

## 3. 目标交互

### 3.1 页面入口

在 Codex 已安装且当前 USB 授权有效的页面，把 Advanced 中原“刷新模型列表”入口改为“管理上游模型”。点击后打开 `Sheet` 二级弹窗。高级诊断仍保留，但定位为“深度协议诊断”，不要再承担模型启用 UI。

页面入口旁显示当前状态概览，例如“已添加 N 个上游模型”；N 来自本地管理器拥有的活动目录，不通过再次生成请求计算。

### 3.2 弹窗打开和目录发现

打开弹窗时：

1. 请求本地活动目录状态，并执行一次 `/v1/models` 目录读取。
2. 不执行 `/responses`，不写 Codex 配置。
3. 展示模型 ID、`owned_by`、当前状态（未测试/测试通过/测试失败/已添加/已不在上游目录）。
4. 重复的完全相同 ID 标记为不可测试/添加；上游返回为空、错误或格式异常时保留当前活动目录，不得清空或覆盖。
5. 只对 Codex Responses 文本模型开放快速测试。模型名不能作为是否支持 Responses 的判据；最终资格由真实 Responses 文本测试决定。

### 3.3 单模型快速连通性测试

每行提供“测试连通性”。触发前展示明确提示：“每次测试发送 1 条真实 Responses 文本请求，可能产生用量或费用；取消请求时上游可能仍已受理。” 用户主动点击该行测试即授权本次单条请求。弹窗打开和列表加载不需要费用确认，因为它们只读目录。

测试成功条件：

- 模型 ID 在当前有效账户、当前 provider 返回的最新目录中唯一存在；
- `/v1/responses` 返回可解析的完成响应，响应模型 ID 与测试 ID 一致；
- 随机挑战文本回显正确；
- 发送请求数恰为 1；不重试生成请求。

通过后该行显示“测试通过”和“添加进 Codex”；失败、超时、取消、鉴权错误或上游不支持 Responses 时不显示添加按钮，并给出安全错误码。成功只代表该时间点的一次文本调用通过，不代表工具、流式、多轮、图片输入或未来可用性。

成功状态仅在本弹窗会话有效，并绑定模型 ID、设备凭证指纹、provider 身份和目录检查时间；账户/Key/provider 改变、重新发现目录、弹窗关闭或测试过期时清除成功状态。不要把它写入持久能力 sidecar，避免把短期连通性误作长期能力证明。

后端必须在测试成功后签发随机、短时、一次性的内存证明票据；只有 UI 成功态不构成 Add 授权。票据绑定本次测试的精确上游模型 ID、provider/base URL、有效 USB 账户绑定标识、凭证摘要、模型目录摘要、当前 proof epoch 和签发时间，默认 5 分钟过期。不得保存 Key、prompt 或响应正文。目录重新发现、弹窗会话结束、取消/失败或绑定变化时使关联票据失效；Add 消费票据时必须原子地标记为已用，并重新检查当前绑定、provider、目录摘要及该精确 ID 唯一存在。票据不得写入磁盘、日志或返回的诊断文本。

响应同时返回票据失效时间、连接绑定摘要和凭证摘要。UI 每 10 秒检查本地票据 TTL，并在弹窗重新获得焦点/可见时通过 `state` 比较当前目录状态、连接指纹和凭证指纹；到期时用定时器移除 Add 状态并撤销票据。连接或凭证摘要变化、状态校验失败、目录变为 orphaned 时清除 Add 状态。`state` 不返回 proof ID，也不能代替后端验证票据；Add 消费时重新验证票据。无需为 UI 状态检查额外请求上游目录。

### 3.4 “添加进 Codex”

- 只在本次弹窗中该模型的快速测试通过后展示。
- 点击时提交模型 ID 和快速测试返回的一次性证明票据。后端先消费并核验票据，再重新检查授权绑定、provider、目录状态及该精确上游 ID 的唯一性；不再发生成请求。若目录/绑定在测试后改变、票据过期/已用，拒绝写入并提示重测。
- 持久化 ID 必须与刚刚通过测试的精确上游 ID 完全一致。不要调用会把 `-stable`、`-latest` 或日期后缀合并到另一模型的 `normalize_models()` 来替换已测试 ID；该归一化只继续用于旧目录展示/兼容逻辑。重复添加幂等。
- 合并时保留既有 manager-owned catalog entry 原文，只追加本次经验证的新 entry；不得重新从完整上游目录生成并覆盖已选集合。
- 不改变 Codex 当前默认模型、不自动选中、不重启 Codex、不改 CCSwitch 的 `model`、base URL、Key 或账户配置。
- 写入成功后回读目录/托管配置确认；设置现有 reload marker，并提示“模型已加入；重启 Codex 后生效”。失败时保留之前目录和其他 `config.toml` 设置。

### 3.5 “一键复位”

弹窗底部提供“一键复位为 Codex 原生模型”。点击后：

- 复位当前 Codex Home 中由本管理器拥有的第三方模型目录覆盖；管理器自有目录为空且没有外部 `model_catalog_json` 或 ownership 冲突时，该动作幂等成功。
- 移除管理器拥有的 `model_catalog_json` 托管块、目录 JSON 和对应模型能力 sidecar；Codex 回到当前版本自带的 GPT 原生模型列表。
- 先用现有托管块解析移除 manager block 的内存副本，再用 TOML 解析器检查剩余有效配置是否定义用户自己的 `model_catalog_json`。解析失败或该 key 存在时，整个 Add 首次激活/Reset 都 fail-closed；Reset 不改配置、catalog、sidecar 或 reload marker，并提示“检测到外部模型目录配置或配置无法安全解析，已保留原配置；未恢复 Codex 原生目录”。不能用字符串前缀扫描替代 TOML 语义检查，也不能移除管理器块后才发现外部目录。
- 仅当管理器所有权、路径、托管块和内容校验通过时清理。若用户改写配置块、路径是重解析点、文件不属于本管理器或并发修改，停止并保留原样，展示冲突原因。
- Add 和 Reset 必须持有同一把异步 catalog mutation lock，从 Add 的票据校验前一直覆盖到 catalog commit；Reset 等待先取得锁的 Add 到达提交边界并完成，再移除其结果。Reset 取得锁后先撤销所有未消费票据；排队在 Reset 后的 Add 因票据已失效而拒绝写入。Reset 返回后不得有先前 Add 迟到写回目录。
- UI 发起 Reset 前先取消当前管理弹窗的目录/快速测试/Add request IDs，并等待取消 IPC 返回；随后再调用 Reset。这样已完成但未消费的测试票据会被撤销，在途测试观察到取消后不得签发票据。若上游已受理生成，费用仍可能产生；不得因取消而重试。
- Add 的请求取消与提交共用 `FILE_LOCK` 线性化：若取消先取得锁，Add 必须在任何文件写入前观察到取消并终止；若 Add 已取得锁并开始提交，取消返回“已过提交点/无法撤回”，Add 需报告准确的 applied 状态。关闭弹窗后重开时必须从 `state` 重新读取，不用旧请求结果推测当前状态。
- Reset 开始时使旧自动应用/内存能力状态及所有未消费快速测试票据失效。它等待 Add 完成其确定的提交边界，再按序复位；不得在 reset 之后让迟到任务重新写回目录。
- 后端同时维护内存 `proof_epoch`。快速测试开始时捕获 epoch，签票前在同一票据锁内确认 epoch 未变且请求未取消；Reset 在 mutation lock 内递增 epoch 并清空全部票据。Reset 开始前已发出的测试即使晚返回成功，也不得在新 epoch 签发可消费票据。epoch 只存在进程内，不写日志或文件。
- 写入 reload marker 并提示重启 Codex 生效。
- 只清模型目录及对应模型能力状态；保留账户绑定、CCSwitch Provider/Key、MCP、Skills、API 连接、其他 Codex 配置和用户选择的非模型设置。
- 不尝试删除上游平台模型，不切换 OpenAI 官方 Provider，也不承诺名称为 GPT 的模型一定走 OpenAI 官方线路。

实现前确认 Codex 当前版本在移除管理器 `model_catalog_json` 覆盖后会回到随包原生目录；卸载清理路径已按此原理实现，但本任务的本机测试仍须验证目录文件/托管块的复位结果。不要以静态 `gpt-*` allowlist 替代原生目录。

验收必须使用安装包锁定/测试的 Codex 版本（当前 `installer-pack/pack.json` 声明 `testedCodexVersion: 0.157.1`）；若最终发行载荷包含独立 `resources/codex.exe`，优先对该确切二进制测试并记录 `--version`。使用全新的临时 `CODEX_HOME`：先用官方 `debug models` 入口确认 manager catalog 覆盖可见，再删除覆盖并确认当前版本自带的 GPT 原生列表恢复。不得使用或改写用户真实 Codex Home。若确切随包版本无法以可重复方式验证原生目录恢复，则复位能力不得标为通过。

### 3.6 启动和升级行为

- 删除“安装成功后自动刷新并写入目录”和“首页启动后自动刷新并写入目录”的调用。
- 移除旧的独立“刷新模型列表”写入按钮，改为显式打开模型管理弹窗。
- 首次安装默认不写任何上游模型目录，Codex 显示原生 GPT 模型。
- 已有安装若已有旧版管理器拥有的目录，升级时原样保留现有用户可见模型；不突然删除、不扩展。用户点击复位后清空；后续启动/刷新不会重新添加。
- 弹窗内目录刷新仅更新当前弹窗的临时发现列表；只有一次性测试成功且用户点击添加才更新持久活动目录。
- Sheet 的 `open` 状态一变为 `false`，立即取消目录/测试请求并使当前会话票据失效，不等待退出动画结束后卸载内容。

## 4. 状态、IPC 与安全契约

### 4.1 数据来源

- **discovered models**：本次 `/v1/models` 解析结果，仅保存在 UI 会话内。
- **active models**：已有管理器拥有的 `codex-usb-native-model-catalog.json` 中实际启用的模型集合。该文件继续充当显式用户选择集，不新增第二套冗余 allowlist 文件。
- **test pass**：本弹窗内的短期状态，不写入 Codex catalog、能力 sidecar、日志或 USB 许可证。
- **credentials**：仅从有效签名 USB 绑定读取；不得返回前端或写目录。诊断只返回现有短指纹，不记录 Key、prompt、完整响应或 Authorization。

### 4.2 Tauri 接口建议

复用现有 `gateway_model_probe`：在请求中增加可选 `connectivity_model_id`，保持现有 `selected_model_ids + cost_consent` 深度矩阵契约兼容。

模式约束：

| 请求形态 | 行为 | 计费生成 |
|---|---|---:|
| `selected_model_ids=[]`、无 `connectivity_model_id` | 只请求目录 | 0 |
| `connectivity_model_id` 有值、无矩阵选择 | 验证该精确模型 ID 的单条 Responses 文本请求；仅成功时签发短时一次性内存票据 | 1 |
| `selected_model_ids` 非空、`cost_consent=true` | 保留现有深度协议矩阵 | 最多 12/模型 |
| 快速测试和矩阵参数同时出现 | 拒绝请求，不发送生成 | 0 |

删除旧的 `refresh_native_model_catalog` Tauri 命令及其 UI 自动/手动调用，不保留一个可被调用后再次写入全量上游目录的旁路。另封闭 `model_catalog::refresh_requested(..., Some(native_verification))` 旧自动应用路径：该请求必须在发网络请求前以 `legacy-auto-activation-disabled` 安全错误拒绝；删除 `activation::publish()` 的生产调用，验证代码库不存在其他可自动写入 Codex 目录或设置默认模型的调用者。保留独立的 `gateway_model_probe` 深度诊断；不得把测试矩阵接回模型自动应用。

Rust 内部 `NativeModelCatalogReport` 若仍被能力诊断/存储测试使用则保留；只删除全量自动刷新函数、命令注册、前端 API 方法和已经无调用者的 TS 类型。旧的 `docs/CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md` 须加醒目的 `SUPERSEDED` 说明：其中全量同步/自动激活策略废弃，本文件为现行行为契约；Codex schema、ownership 和安全写入证据仍可参考。

新增本地管理命令 `manage_native_model_catalog`，operation 仅支持 `state`、`add`、`reset`：

- `state` 返回 `status`（`ok`/`orphaned`）、当前管理器目录 `modelIds`、`restartRequired`、安全提示 `message`，以及当前 `connectionFingerprint` 与 `credentialFingerprint`；不存在有效账号/Key 绑定时两个指纹均为空。该响应不返回 proof ID；UI 只比较短期票据本地 TTL 与当前目录/绑定状态，真正的 proof 消费和有效性仍由 Add 后端复核。
- `add` 接收独立的 `request_id`、精确测试模型 ID 和 `connectivity_proof_id`。后端用 Add 的 request ID 进入共享 gateway probe 门禁，复查现行绑定和最新 `/v1/models`，再原子消费未过期票据并核验其模型 ID、provider/base、USB 绑定标识、凭证摘要、目录摘要；否则拒绝写入并要求重测。该调用自身不触发生成；新目录 GET 和文件提交期间保持该门禁，取消 Add request ID 可阻止后续写入。
- `reset` 仅执行上述有所有权校验的原生目录复位。

快速测试成功响应含 `connectivity_proof_id`；测试失败/取消/列表读取/矩阵报告不得签发票据。刷新目录和关闭弹窗要让对应会话票据失效。后端证明票据实现设有限容量并回收过期项；并发 Add 对同一票据只能一个成功。Reset 递增 proof epoch，拒绝任何在旧 epoch 启动的迟到测试签票。

扩展现有 `cancel_gateway_model_probe(request_id)`：取消运行中的目录/测试/Add 请求，并同时撤销由该 request ID 签发但尚未消费的票据；弹窗关闭时即使测试已经完成，也要对所有已通过测试的 request ID 及当前 Add request ID 发此撤销调用。目录刷新先撤销上一轮目录/测试 request IDs，再建立新轮次。该撤销不代表撤回上游已经接收的生成请求。

新增命令必须在 `commands.rs` 和 `lib.rs` 注册；前端通过 `managerApi` 调用，不直接操作文件系统。

### 4.3 并发和错误处理

- 复用 gateway probe 的全局单请求门禁、取消登记和 request ID 校验；弹窗 open 变为 false 时立即取消请求，不等 Sheet 的退出动画/组件卸载。迟到响应不得更新新的弹窗会话。
- 目录读取、模型测试和 catalog 写入之间不允许因旧 request ID、旧账户绑定或旧 provider 结果污染新状态。
- 使用 `FILE_LOCK` 和现有原子文件写入。add/reset 写之前比对当前文件快照；并发修改时返回冲突，不覆盖。凡写入用户 `config.toml`，一律使用 `atomic_file::write_atomic_preserving_backup()`，不得使用会轮转 `.bak` 的通用 `write_atomic()`；manager-owned catalog/sidecar 才使用通用 writer。Add 的已激活 catalog 更新不得无必要重写 config；首次激活时先落 manager-owned catalog，再原子添加托管配置块，若配置未激活则回收安全可删的孤儿 catalog。Reset 先完成所有路径/ownership/外部覆盖预检，再解除托管配置并清理文件。
- Quick test 与 Add 的最新目录重验共用 gateway probe 的全局单请求门禁，防止并行请求造成 ticket/目录快照竞态。前端为每个成功测试保存其 `requestId` 与 proof；Add 另生成自己的 `requestId`。目录刷新或关闭弹窗时对所有尚未消费的测试 `requestId` 及当前 Add `requestId` 调用取消以撤销票据/阻止写入。Add 前的目录 GET 与 catalog 文件提交处于同一门禁生命周期内。
- Add/Reset 另共用异步 catalog mutation lock；Reset 等待任何先行 Add 完成后再清理，并在释放锁前撤销所有剩余票据。排队 Add 在 Reset 之后拿锁时必须发现票据已撤销并退出。取消若晚于 `FILE_LOCK` 提交点，不承诺回滚；返回结果和下一次 `state` 都必须反映实际已提交状态。
- 跨文件操作必须区别 `not_applied`、`applied_restart_pending` 和 `cleanup_pending`。每次写失败后回读 catalog/config/marker 判断实际状态；若模型目录已可见但 marker 或清理失败，UI 显示“已应用，需重启/检查”，不能显示普通失败或声称回滚。Reset 若已解除 catalog 引用但文件/sidecar 删除失败，显示“Codex 已切回原生目录，托管文件清理待重试”；若 marker 写入失败则同时指出需手动重启/检查。不得把未确认的跨文件结果报告为完全成功。
- Add/Reset 无论返回成功、部分成功状态或 IPC 错误，UI 都必须再次请求 `state`，以实际文件状态更新 `activeIds` 和 `catalogStatus`。若复位已解除引用但留下孤儿 JSON，状态重读须切为 `orphaned`，禁用测试/Add 并保留 Reset 入口；若状态重读失败则 fail closed，禁用测试/Add，直至重新取得 `ok` 状态。
- 上游 generation POST 不自动重试。失败响应只暴露安全错误类别，不显示服务端返回正文。
- 目录请求超时/空模型表/非法响应时保留已有 active catalog。仅显式 reset 能清空。

## 5. 可复用代码清单

| 目标 | 复用内容 | 位置 | 本任务如何使用 |
|---|---|---|---|
| 弹窗与无障碍 | `Sheet`、`useFocusTrap` | `src/app/Sheet.tsx` | 新建 `UpstreamModelManagerSheet` 复用标准 Sheet，不自建 Dialog/focus trap。 |
| 当前目录探测 | `gatewayModelProbe()`、请求 ID、取消 API | `src/services/managerApi.ts`、`src-tauri/src/app/gateway_model_probe.rs` | 保留 `GatewayModelProbePanel` 作为独立深度诊断；复用只读 `/v1/models` 与安全请求门禁，扩展单模型快速测试分支。 |
| HTTPS、身份校验、Responses | `ProbeClient::new/models/response/text/receive`、`validate_response`、`require_text` | `src-tauri/src/app/model_probe.rs` | 将单次挑战文本封装成窄的公开方法；不复制 HTTP/响应解析，不复用 12 请求矩阵做 quick test。 |
| 目录解析/别名处理 | `parse_model_catalog`/`resolve_selected_models`、`safe_identifier`；旧目录读取时复用 `split_alias`/`normalize_models` | `src-tauri/src/app/gateway_model_probe.rs`、`src-tauri/src/app/model_catalog.rs` | 探测与 Add 均精确保留用户实际测试的原始 ID；禁止在选择性 Add 时把 alias 归一到未测试的另一个 ID。发现列表本身不写文件。 |
| Codex 目录项 | 现有 catalog schema/`native_model_entry` 的必需结构 | `src-tauri/src/app/model_catalog.rs` | 新增 entry 不显式写 `input_modalities`，采用 Codex 原生缺省；不声称上游视觉质量已验证，不声明搜索/工具协议能力。既有 manager-owned entry 默认原样保留；仅 [`CODEX-MODEL-CAPABILITY-ADAPTATION-REPAIR-20261024.md`](CODEX-MODEL-CAPABILITY-ADAPTATION-REPAIR-20261024.md) 所定义的精确旧模板由启动 preflight 迁移。schema 必需但上游未知的数值尽可能省略/用 Codex fallback，必须填写时用当前官方 catalog 的最低有效值，不把它说成上游真实能力。 |
| 安全配置文件处理 | `FILE_LOCK`、`regular_or_absent`、`read_optional`、`owned_json`、`remove_managed_config`、`deactivate_owned_catalog`、`atomic_file::write_atomic`、新增 `write_atomic_preserving_backup`、现有 `toml::Value` 解析依赖 | `src-tauri/src/app/model_catalog.rs`、`src-tauri/src/app/atomic_file.rs`、`src-tauri/src/app/codex_config_readback.rs` | catalog/sidecar 继续用带 ownership 检查的原子写入；用户 `config.toml` 用不轮转/删除 `.bak` 的原子替换。Add/reset 先剥离 manager block，再语义解析 unmanaged TOML key；解析失败或外部 `model_catalog_json` 都 fail-closed。 |
| Provider 安全校验 | `provider_base()`、`usb_guard::authorized_account_binding()` | `src-tauri/src/app/model_catalog.rs`、`src-tauri/src/app/usb_guard.rs` | 后端每个测试/添加操作都重新读取有效 USB 绑定，不信任前端 endpoint 或凭证。 |
| reload 状态 | `mark_restart_required()`、`restart_required()` | `src-tauri/src/app/model_catalog.rs` | 添加/复位后提示 Codex 重新加载。复位不能照搬卸载 `cleanup()` 的 `clear_restart_required()`。 |
| IPC 注册和类型 | command wrapper / `invoke_handler` 注册范式 | `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs` | 新增一个窄管理命令；不创建平行网络客户端或第二套配置接口。 |
| 安全清理 | `cleanup()` 中的 owned JSON 清理模式 | `src-tauri/src/app/model_catalog.rs` | 复用其中身份验证/模型 sidecar 失效策略；不把整个卸载流程直接暴露给 UI，因为 reset 需要保留 reload marker。 |
| 本地化与页面状态 | `useI18n`、现有 GatewayProbe strings、`OfflineHome` 页面入口 | `src/app/i18n.tsx`、`src/app/views/OfflineHome.tsx` | 扩展中英关键文案并保持现有多语言 fallback，不重做连接设置 UI。 |

## 6. 详细代码清单与允许写入范围

以下文件是本任务计划写入边界。若实现需要触及列表外文件，主控先检查是否确实必要，再更新本合同；执行中不得擅自扩范围。

| 文件 | 计划改动 | 验收关注点 |
|---|---|---|
| `src/app/views/OfflineHome.tsx` | 删除安装后和启动时自动 `refreshNativeModels(true)`；把旧刷新入口改为打开模型管理弹窗；保留本任务开始前的连接与扩展 UI 改动。 | 组件挂载/安装成功均不自动发现或写模型；已安装入口正常。 |
| `src/app/views/GatewayModelProbePanel.tsx` | 保留现有高级深度协议诊断面板和 `selected_model_ids + cost_consent` 行为；只做共享 API 新字段/类型所需的兼容调整，不将它改造成上游管理 UI。 | 现有矩阵测试体验与 12 请求/模型费用确认契约不变。 |
| 新建 `src/app/views/UpstreamModelManagerSheet.tsx` | 复用 `Sheet`、共享 discovery/quick-test/cancel API 与 `manageNativeModelCatalog`；实现目录状态、逐行测试/Add、票据过期/连接变化失效、复位及会话并发保护；orphaned 状态仅允许复位。 | 列表只读；失败/过期/绑定变化不显示 Add；添加/复位结果与后端状态一致。 |
| `src/app/views/OfflineHome.test.tsx` | 添加无自动刷新、打开弹窗发现、复位入口回调等前端契约断言；不覆盖已有连接页测试和基线改动。 | 保证自动装载被废弃。 |
| 新建 `src/app/views/UpstreamModelManagerSheet.test.tsx` | 覆盖 modal 生命周期、退出动画前取消、费用提示、通过后显示 Add、失败隐藏、刷新失效、票据过期/绑定变化、Add 失败撤销测试 request、孤儿状态复位后的状态恢复、Add/Reset 返回 `cleanup_pending` 后仍孤儿时继续禁用测试/Add 和迟到响应。 | 不需要真实网络/Key。 |
| `src/shared/types.ts` | `GatewayModelProbeRequest` 增加可选快速测试字段；manager add action 增加独立 `requestId`、model ID 和 proof ID；增加 catalog state/action response 类型；删除无其他消费者的 `NativeModelCatalogReport` TS interface。 | 前后端字段 camelCase 一致，旧矩阵调用继续有效。 |
| `src/services/managerApi.ts` | 扩展现有 `gatewayModelProbe()` 参数；删除 `refreshNativeModelCatalog()`；新增含 Add request ID/proof ID 的 `manageNativeModelCatalog()` Tauri IPC 薄封装和测试。 | Web preview 安全降级；无凭证记录；Add 可被 request ID 取消。 |
| `src/services/managerApi.test.ts` | 新增命令/参数映射测试。 | invoke 参数不漏字段、不泄密。 |
| `src/app/i18n.tsx` | 中文/英文新增发现、一次生成费用、短期连通状态、添加、复位、重启和冲突文案。 | 不复用错误的“完全兼容”措辞；保留现有用户未提交文本。 |
| `src/app/styles.css` | 仅新增模型弹窗/行状态必要样式，优先复用 `.sheet`、`.actions`、`.status-*`。 | 不重排现有连接设置样式。 |
| `src-tauri/src/app/model_probe.rs` | 为随机挑战文本增加窄的 `pub` 测试方法，内部复用现有私有 `text()`。 | 每次快速测试恰好 1 次 generation POST；不重试。 |
| `src-tauri/src/app/gateway_model_probe.rs` | 为 request/report 增加快速测试模式、一次性内存证明票据、票据到期时间与连接摘要；复用当前最新目录解析、唯一 ID 校验、取消/运行门禁和 `ProbeClient`。矩阵路径行为不变。 | 列表 0 次生成；快速测试恰好 1 次；票据正确绑定/失效/单次消费；矩阵兼容现有 max 3 / cost consent。 |
| `src-tauri/src/app/model_catalog.rs` | 删除 `refresh_native_model_catalog()` 全量写入函数；封闭 `refresh_requested(..., Some(native_verification))` 自动 publish 路径；新增本地 `state/add/reset`、safe text-only entry、reset 外部 override preflight 及明确部分失败报告；复用路径/ownership/原子文件 helper。 | Add 精确增量；旧目录升级保留；reset 仅删 manager-owned 项；启动无自动补回；不将一次文本测试宣传为图像/工具兼容。 |
| `src-tauri/src/app/atomic_file.rs` | 新增不轮转 `.bak` 的原子文件替换方法，专供用户 `config.toml`；保留现有 `write_atomic` 给 manager-owned catalog/sidecar。 | Windows 使用 replace-existing/write-through；其他平台用同目录原子 rename；新增回归测试证明用户 `.bak` 字节不变。 |
| `src-tauri/src/app/model_catalog.rs` 内测试 | 使用显式临时 home 路径调用 path-parameterized catalog state/Add/Reset helper，不设置进程级 `CODEX_HOME`；覆盖正常 Add/Reset、原配置与 `.bak` 保留、外部 override fail-closed、孤儿状态、提交前/后的取消和 mutation lock 两种排队次序。gateway proof 测试另覆盖 Reset 清票后无法消费。 | 实际文件事务可验证，测试不读取/修改真实用户 Codex Home，不需要 Key。 |
| `docs/CODEX-NATIVE-MODEL-CATALOG-INTEGRATION-DEVELOPMENT.md` | 文件开头添加醒目的 `SUPERSEDED` 注记，指向本文件；标记全量同步/自动应用策略废弃，保留 schema 与安全写入参考价值。 | 后续实现人员不会继续按旧自动加载策略开发。 |
| `src-tauri/src/commands.rs` | 移除旧 `refresh_native_model_catalog` command；暴露窄的管理器 state/add/reset command。 | command 只转发/授权，不复制目录算法。 |
| `src-tauri/src/lib.rs` | 移除旧 refresh command 注册并注册新管理 command。 | Tauri 构建能解析 invoke handler。 |

不应修改：`CCSwitch/**`、`installer-pack/**`、`media-generation-mcp.py`、API Key 注入/账户注册、用户全局 Codex 配置（开发过程）、release/version/signing 文件，以及本任务开始前不相关的变更。

## 7. 实施顺序

1. **文档审计先行**：只读审计本文件的行为边界、复用准确性、复位所有权安全、费用语义和测试覆盖。审计 `PASS` 后才进入实现；`FAIL` 先修文档并冻结新版本。
2. **锁定工作区基线**：再次查看完整 `git status` 和本任务文件 diff；确认既有连接页/媒体 MCP 改动保留。记录初始与最终 diff，不还原其他未提交改动。
3. **先切断自动加载**：删除旧 `refresh_native_model_catalog` 命令/function 和首页安装后/启动时的调用，落实首次安装不加载网关模型；先加前端回归，证明启动不触发模型目录 invoke。
4. **实现后端单次测试**：在 `ProbeClient` 上暴露窄文本测试，扩展 gateway probe 的请求校验和 response；保持 12 请求矩阵不变。
5. **实现显式 Add/Reset**：新命令复用 model_catalog 现有写入、路径/所有权、原子提交和 reload marker；状态从 manager-owned active catalog 派生，不新增重复 allowlist。
6. **接入弹窗**：使用 `Sheet`，显示 discovered/active/test 状态；Add 仅在测试通过后出现；reset 成功/冲突结果明确，保持页面原有修改。
7. **实现专项测试**：先跑 Rust 和 React 单元/fixture 测试，再跑 lint/typecheck 和 Tauri build。所有网络使用 loopback mock，不调用真实 API。
8. **固定代码版本并独立审计**：代码 diff 固定后由不同 Agent 只读审计，检查合同、并发、错误分支、费用请求数、Codex 文件 ownership、基线保留与测试证据。
9. **通过后交用户验收**：本任务不自动打包/签名/发布；提供功能行为、测试命令与结果、审计结论、剩余边界。

## 8. 功能测试和审计验收矩阵

### 8.1 前端交互

- 首页加载、有效 USB 状态轮询和安装完成均不调用 `refresh_native_model_catalog`，也不把发现结果写入 Codex。
- 点击“管理上游模型”才触发列表读取；列表请求调用一次，无生成请求。
- 有效目录显示模型 ID/owner/状态；重复 ID 不可测试/添加；空目录、401、超时不清空 active models。
- 快速测试 pending 状态禁用重复触发；open 变为 false 时在 Sheet 退出动画卸载前取消目录/测试/Add request IDs；迟到结果不覆盖重开的弹窗，也不能签发可用 Add 票据或提交 catalog。
- 只有测试通过的同一精确模型 ID 显示 Add；失败、取消、测试过期、目录刷新或凭证指纹变化时隐藏 Add 并使票据失效。
- Add 成功显示已添加和重启提示；对已添加模型再次 Add 幂等。
- Reset 成功后活动列表为空，Codex 原生模型说明可见；失败/冲突不假报成功；弹窗再次打开不会自动重新加入模型。
- Add/Reset 即使存在用户 `config.toml.bak` 也必须逐字节保留该文件；配置替换不可通过通用 writer 轮转或删除它。
- orphaned catalog 只展示状态和复位动作；在复位清理孤儿目录前不允许测试或 Add。Reset 后 UI 重新读取 state；若仍 orphaned，则继续只允许复位；若恢复 `ok`，测试入口恢复。
- 快速测试返回服务端票据过期时间、连接摘要和凭证摘要；UI 到期、连接/Key 变化、恢复焦点校验失败时清除 Add 状态并撤销票据。
- 同一票据只允许一次 Add；并发重复消费只有一个成功。Reset 后之前签发的全部票据失效。
- Add/Reset 共用 mutation lock，按获得顺序串行；Reset 在持锁后撤销 proof epoch 并清除既有票据，确保 Reset 前启动的在途 quick test 不能晚签票，先 Reset 后 Add 的排队请求会因 proof 已撤销而在写文件前退出。自动化测试分别覆盖两种 mutation lock 获得次序、Reset 清票后消费失败、Add 文件提交边界的取消前/后行为；另通过 UI 测试确认 Reset 后按 state 结果恢复或继续限制操作。
- 点击 Reset 时若有目录/测试/Add 请求，先等取消请求返回，再执行 reset；被取消的在途 quick test 不能在 reset 返回后签发可消费票据。
- 取消与文件提交按 `FILE_LOCK` 线性化：取消赢在前时零写入；提交赢在前时报告已应用/重启状态且不假装回滚；关闭后重开从后端 state 重读。

### 8.2 Rust 网关探测

- list-only: 一个 GET `/v1/models`，0 次 POST `/responses`。
- 快速测试: 最新目录唯一模型、随机挑战文本、1 次 POST `/responses`，`generatedRequests == 1`。
- 快速测试拒绝未知/重复/不安全 model ID、同时使用矩阵字段、无授权或非 404token binding。
- 上游 4xx、非完成响应、response model 不匹配、challenge 不匹配、超时、取消均失败，且不重试 POST。
- 成功测试票据绑定精确 model ID、provider/base、USB 绑定标识、凭证摘要、目录摘要和有效期；目录/绑定变化、超时、取消、过期或重复消费均拒绝 Add。
- Add 的最新目录 GET 处于同一 request gate；取消 Add request ID 后不得写入 catalog；Add 路径 0 次 Responses POST。
- 已有矩阵测试仍覆盖 max 3、单独 `cost_consent`、最大预算和取消行为。

### 8.3 Catalog 生命周期

- 首次空状态下打开目录/刷新不写入 `model_catalog_json`；Codex built-in 目录保持可用。
- Add 只写该模型并保留原有管理器模型、`raw_id` 和无关 TOML 内容；API Key 不出现在 catalog/config/report/log。
- 已有旧版全量目录升级后不静默清空；自动后台刷新不会扩展其模型集合。
- Reset 删除管理器自有 JSON、模型 sidecar 和唯一受管配置块，并写 restart marker；无关 `config.toml` 字段、CCSwitch auth/provider、MCP/Skills 配置不变。
- 文件被用户修改、ownership identity 不匹配、重解析点、托管块损坏、并发写入时 fail closed；不会删除非管理器文件或覆盖较新的内容。
- Reset 后再次首页启动/目录刷新，不重新写回任何上游模型。
- `refresh_requested(..., Some(native_verification))` 在发请求/写盘前拒绝；源码无任何生产调用者可把发现全集或深度验证结果自动发布到 Codex catalog 或改默认模型。
- 外部 unmanaged `model_catalog_json`（包括 TOML 兼容引号 key）或无法解析的 config 使 Add 首次激活/Reset 在首次写入前冲突退出，config/catalog/sidecar/reload marker 字节均不变。
- Add/Reset 更新用户 `config.toml` 时使用保留既有 `.bak` 的原子替换方法；回归测试验证用户备份在 Add、Reset 前后均字节一致。
- Reset 与一个在途 quick test 并发：UI 取消后即使上游迟到返回成功，旧 epoch 也不能签发 Add 票据；Reset 返回后 ticket store 无可消费的旧票据。
- Add 或 Reset 返回 `cleanup_pending` 后，重新读取 state 若发现孤儿目录，UI 只保留 Reset；后端 state 对“存在托管 JSON 但配置未引用”的临时 home 返回 `orphaned`。两个 UI 路径分别有回归用例。
- 对 Add/Reset 的写入、回读、marker、清理失败路径做结果状态审查；可稳定注入的文件系统错误由专项测试覆盖，其余不可移植的 OS 故障注入作为手动代码路径验收，不将未执行的注入用例标为测试通过。
- 注入 Add helper 在最后一次提交检查前取消，要求零写入；在提交边界回调后取消，要求按已越过提交点继续并反映真实文件状态。用隔离 mutation lock 测试 Add 先行/Reset 排队以及 Reset 先行/Add 排队两个次序；用 gateway ticket 测试证明 Reset 清票后排队 Add 无法消费票据并在提交前失败。
- 使用当前安装包内 Codex 可执行文件和全新临时 `CODEX_HOME`，记录准确版本，验证“manager catalog 覆盖可见 -> 移除覆盖 -> 当前版本原生 GPT 列表恢复”；测试过程不触碰用户真实 Codex Home。
- 新添加 entry 仅声明已测试的 text input，不声明图像/搜索或工具 Agent 能力；升级保留的既有 entry 不会被静默重写。

Add 的跨文件写入要特别处理：沿用历史先写 catalog JSON、再写 config 的顺序，但 config 用 `write_atomic_preserving_backup()`，不轮转用户 `.bak`。`state` 必须只把同时满足 manager identity、托管块路径匹配且配置指向该文件的目录视为 active。Add 必须对初次配置提交失败保持孤儿 JSON 不激活；已有激活目录只原子更新 JSON，不无必要重写 config。每步写入后回读实际文件，若 reload marker/最终回读失败，返回“已应用但需检查/重启”的明确状态，不能向用户声称完全回滚或完全未改变。Reset 预检全部路径和 ownership 后先解除管理器引用，再清理拥有的 catalog/sidecar；若解除引用已成功但文件清理失败，要报“已恢复原生目录，托管文件清理待重试”，不得报整个 Reset 未应用。外部 override 冲突必须发生在任意写操作之前。

### 8.4 验收放行标准

- 文档独立审计：`PASS`。
- 代码独立审计：`PASS`；`FAIL` 必须先修正、冻结新 diff 并重审。
- 上述专项测试、项目规定的前端检查、Rust 检查与 Tauri 构建全部通过。
- 不以 mock 或目录列表成功宣称第三方模型完整 Agent 兼容。
- 明确记录实际模型/努力路由若不可观察则为 `ROUTE_UNVERIFIED`；Hook 状态独立记录为 `HOOK_UNVERIFIED`，不伪造来源。

## 9. 当前已知风险和处理

| 风险 | 处理 |
|---|---|
| `/v1/models` 不代表模型有可用线路 | 对每个模型分别做一次真实 Responses 文本调用，Add 只在通过后显示。 |
| 一次测试可能计费 | 测试前明确显示“一条真实生成请求，可能计费”；不在自动启动或打开目录时生成，不自动重试。 |
| 通过文本不代表工具/Skills 兼容 | UI 只称“连通性测试通过”；高级协议矩阵继续单独存在。 |
| 媒体模型混进 Codex 菜单 | 只有能返回合规 Responses 文本的条目才显示 Add；不通过名称前缀过滤。 |
| 用户配置与托管块冲突 | 复用当前 marker/identity/compare-before-write；有冲突就不覆盖。 |
| 当前 Codex 运行时加载旧 catalog | 添加/复位统一写 reload marker，并提示重启；不强制终止 Codex。 |
| 历史自动目录已被用户使用 | 升级保留当前目录；只有用户主动复位才移除；此后后台刷新不再自动加新模型。 |
