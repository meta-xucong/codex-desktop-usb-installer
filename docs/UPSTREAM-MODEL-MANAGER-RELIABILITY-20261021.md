# 上游模型管理：交互与添加可靠性修复

日期：2026-10-21  
基线：`e09a541`；工作区已有未提交改动。本任务只修改模型管理相关区域，保留其他所有改动。  
风险：I2（模型测试证明与配置文件事务跨模块）；A2（已有多轮 UI 与后端测试，需独立审计）。

## 1. 问题与证据边界

用户报告：旧包未测前出现添加入口；点击测试后还要再点才开始；通过后添加路径重复/不清楚；一轮 7 项通过、重测 6 项后添加失败；提示“无法读取当前 Codex 原生模型目录，未写入模型”。

当前工作区源码与用户运行包可能不同：

- 当前工作树的主按钮点击后先调用 `requestUserConfirmation`，确认后立即调用档位探测，并同时展开检测卡片；这与用户观察到的“还需再点一次”不一致。当前运行包版本、二进制哈希与源码 build commit 尚未核对，不能断定用户实际观察来自旧包。实施和验收均以明确记录运行二进制 SHA/build commit 对应源码 revision 为准。
- 当前档位测试通过时，Add CTA 在展开卡片中；关闭卡片后行内会显示“查看确认”并需重新打开卡片，属于冗余回跳，不是同屏两个 Add 按钮。
- 当前组件使用 `window.confirm` 包装确认；用户日志出现 `Command plugin:dialog|confirm not allowed by ACL`，但无组件/request ID 关联，不能确定来源于此页面。按确认桥接风险处理，并验证模型页点击不会再调用该接口。
- `gateway_model_probe.rs::add_tested_model` 在目录合并前调用 `consume_connectivity_proof`，立即删除一次性证明。
- `model_catalog.rs::add_tested_model_to_catalog` 在没有可复用的非空托管目录时，通过已校验随包 Codex CLI 执行 `debug models` 得到默认清单；候选目录再经随包 CLI 验证后才写入。该流程不是读取用户 Codex 主目录中的“原生模型目录”。失败可能来自可信包/CLI校验、CLI执行/退出、输出限制或解析等阶段；当前泛化提示与错误码不足以判定具体原因。
- Add 前端 catch 当前会删除 ticket 并调用 `cancelGatewayModelProbe(ticket.requestId)`；后端 cancel 会撤销该请求的 proof。即使只保留前端 ticket，调用 cancel 仍会让后端凭证失效。
- proof TTL 为 5 分钟。延迟到首次写入前消费时，必须处理 Add 开始前过期与本地校验运行中到期的边界。
- 用户提供日志 `codex-app-manager(7).log`：49,023 bytes，SHA-256 `944B780428064C89E6DCD7CA47CDBDEEB3C1DA4B83BDE0F6CF91B10B7F27F181`，本地修改时间 2026-10-21 12:36:41 +08:00。日志含 `kimi-k3` 多轮档位事件，至少一条为 `effort-rejected`；没有足够关联信息证明它就是 7→6 的两轮，因此波动根因仍待同模型/同绑定结构化结果验证。日志也没有运行 EXE SHA/build commit，不能直接与当前工作树对应。
- 日志没有 `manage_native_model_catalog(operation=add)`、`native_default_catalog` 或原生默认清单失败阶段事件，不能用它确定那次 Add 失败的具体 CLI 根因。

## 2. 目标交互与费用提示

每个模型只保留一条主动作路径：

| 状态 | 唯一主动作 | 行为 |
|---|---|---|
| 未测/失败 | 测试/重试 | 一次点击立即发完整档位探测；不先进入二级卡片或等待第二次点击 |
| 检测中 | 检测中… | 禁用重复操作并显示进度/状态 |
| 完整检测通过且 proof 有效 | 添加到 Codex | 同一行按钮直接 Add，不经过“查看确认”回跳；关闭/重新进入检测卡片后仍在同一位置显示此按钮 |
| 添加中/成功 | 添加中… / 已添加 | 禁止重复提交或显示成功状态 |
| 后端明确返回 precommit 可重试且 proof 仍有效 | 重试添加 | 重用同一 proof；不重复发网关档位请求 |
| proof 过期/绑定变化/Reset/取消 | 重新测试 | 撤销旧资格后要求重新检测 |

全量连通性测试只作展示，不能解锁 Add。只有当前模型、当前 endpoint/账户/凭据指纹、当前目录摘要和当前 reset epoch 绑定的完整档位 proof 可解锁 Add。

按用户后续明确要求，单模型行不再重复显示“最多发送 N 次请求/可能产生费用”，也不额外弹费用确认框；用户点击该模型的“自动检测”按钮即启动本轮档位探测，后端请求明确标记 `reasoningCostConsent=true`。页面级说明仍通过管理页提示提供。批量测试潜在请求量更大，保留显示模型数/总请求数范围的应用内确认；不得调用受 ACL 拒绝的 `window.confirm`，且必须在打包 Tauri 环境做集成验收。不得以此扩大本轮重置流程。

## 3. proof 生命周期与原子写入

不得降低设备绑定、账户/密钥指纹、endpoint、模型 ID、目录摘要、epoch、TTL、取消/Reset 互斥、一次性消费、签名 MSIX/可信 CLI 校验或最终目录验证；不得用空/硬编码目录兜底。已有模型累加、配置备份、原子替换和 Reset 语义必须保持。

### 状态与 API 契约

Add 命令的前端响应必须结构化。建议在 `NativeModelCatalogActionReport` 增加下列明确字段（或等价 tagged union），并为所有 Add 失败路径定义完备映射，不以错误字符串 contains 分类：

- `status`: `precommit_retryable`、`precommit_revalidate`、`proof_invalid`、`commit_started`，以及现有成功/清理/不确定状态；
- `errorCode`: 稳定安全码，如 `native-cli-unavailable`、`native-defaults-rejected`、`proof-expired`、`binding-changed`、`catalog-write-failed`；不得含路径/响应正文；
- `commitState`: `not_started`、`started`、`uncertain`；
- `proofState`: `valid`、`pending_validation`、`consumed`、`invalid`、`expired`；
- `retryable`: `precommit_retryable` 仅在 `commitState=not_started && proofState=valid` 时为 true；`precommit_revalidate` 表示未开始提交且 proof 未消费，但 fresh catalog digest 因网关刷新失败暂不能核验，重试时必须先重新拉取并匹配 digest，匹配才继续，不匹配则清除资格。

发生 `reset`、用户取消、绑定或上游目录摘要变化、proof 失效/过期时必须撤销 proof 并返回 `proof_invalid`。非用户取消的本地 CLI/临时区 precommit 故障，只有在 fresh catalog digest 已确认不变且 proof 仍有效时才返回 `precommit_retryable`。网关 `/models` 刷新失败时不能声称 digest 未变：proof 保持未消费，返回 `precommit_revalidate`/`proofState=pending_validation`；下次 Add 先重新获取目录，只有摘要与 proof 相同才继续，否则撤销资格并要求重新测试。

### 处理顺序

1. Add 开始时非消耗式验证静态绑定、epoch 与 expiry。重新获取 `/models` 后再比较 fresh `catalog_digest`，提交前再次全量核验。Add 总 deadline 固定为不超过 180 秒；Add 专用 `/models` 复核请求上限为 30 秒，失败时保留尚未消费的 proof 并返回可重试状态，避免网络等待耗尽本地随包 CLI 校验预算。当前 proof TTL 为 300 秒，考虑秒级时间戳，启动时剩余 TTL 少于 190 秒则返回 `proof_invalid/proof-expired`，不发起后续网关请求或文件写入。
2. 重拉并核对当前网关目录、加载/合并默认清单、最终随包 CLI 校验、取消检查和文件快照检查；这些阶段失败不写文件、不消费仍有效 proof。
3. 使用单个有界操作 deadline 覆盖 Add 的网关确认及本地 CLI 阶段；最终提交前再次检查 expiry/reset/binding/epoch。测试中覆盖 proof 在 Add 期间到期，确保拒绝提交且 `write_count=0`。
4. 在现有模型目录 `FILE_LOCK` 仍持有、全部快照检查通过、第一次原子写入前，再次校验并原子消费 proof。证明提交回调改为可失败 `Result`；消费失败必须阻止写入。锁序维持 `FILE_LOCK → proof state lock`，新增并发/reset/cancel 竞态回归。
5. Add 返回 `precommit_retryable` 或 `precommit_revalidate` 时，前端保留未过期 ticket 与 origin request ID，不能调用 `cancelGatewayModelProbe(originRequestId)`；该取消接口会撤销 proof。`precommit_revalidate` 的重试必须先验证 fresh catalog digest。其它失败、用户取消、失效或提交开始/不确定时清除 UI ticket，并只在需要时撤销后端 proof，随后刷新 catalog state。
6. precommit 重试只重复本地 Add 前置步骤/安全验证；测试断言不会再次发档位网关请求。proof 过期后只显示重新测试。

## 4. 档位分类和 7/6 结果

- 档位探测只对 reqwest 明确标记为连接建立阶段失败（`is_connect()`；HTTP 请求尚未发送）的错误，在剩余 deadline 允许时自动重试一次，等待 120 ms；两次尝试都计入实际请求数。发送阶段超时、已收到 HTTP 响应、HTTP 4xx/5xx、响应不完整、取消和预算耗尽均不得自动重发，避免把可能已计费的请求重复提交。
- 保留 `accepted_by_route / rejected_by_route / unknown` 三态，不跨不同运行/绑定合并。
- 一个档位被明确拒绝、其他档位明确通过时，用本轮 accepted 子集生成 proof；显示准确 accepted / rejected 数。若有 unknown 但本轮至少一个档位明确 accepted，proof 仅包含 accepted，unknown 标为“未确认并跳过”；不得将 unknown 视为 rejected 或加入 Codex。
- 没有 accepted、请求整体未完成、取消、预算耗尽或主响应缺失时，不签发 Add proof。局部档位 unknown 不阻断已确认 accepted 子集的添加；unknown 显示“未确认并跳过”，不能算 rejected。
- 同一完整 report 中至少一个 accepted 即可发仅含 accepted 的 proof；局部 unknown 不入 proof。整体响应不完整或取消仍不得发 proof。
- 全量连通探测结果不得污染档位列表或发 proof。UI 回归要验证 bulk connectivity pass 后仍不可 Add。

## 5. 原生默认清单错误诊断

把误导性提示改为“读取随包 Codex 默认模型清单失败”，并附稳定错误码。增加有限、脱敏阶段诊断，至少覆盖：MSIX 定位、身份/版本/成员哈希校验、Authenticode、已安装/随包 CLI 哈希比较、CLI 提取、临时区准备、Windows Job 创建/附加、进程启动、子进程退出码、stdout 超限、JSON/目录结构拒绝、清理。只记阶段枚举、错误码、退出码、输出字节数、耗时与 request/run ID；不记密钥、完整 stdout、响应正文或完整用户路径。保留可信执行链，不因旧发行版曾工作就绕过校验。

只在可获得精确随包签名 MSIX 的隔离环境里复现 CLI 后，才针对确认的输出/兼容差异修复。该集成验证列为发行前验收，不用单测冒充实测。

## 6. 修改边界

允许修改：`src/app/views/UpstreamModelManagerSheet.tsx`、对应测试、必要的页面中英文文案、`src-tauri/src/app/gateway_model_probe.rs`、`src-tauri/src/app/model_catalog.rs`、仅用于上述阶段诊断/有证据 CLI 修复的 `src-tauri/src/app/model_native_probe.rs`。`commands.rs`、`src/shared/types.ts`、`src/services/managerApi.ts` 仅为结构化状态传递必需时改动；`commands.rs` 已有其他未提交改动，必须原样保留并限定模型管理分支。

不改媒体 MCP、skills、签名器、license、CCSwitch、网络预检、relay/VPS/Sub2API/Wokey；不打包、签名、操作真实 U 盘或用户 Codex 目录、不调用真实 provider、不推 GitHub；不清理/覆盖任务开始前已有改动。

## 7. 验收

### 前端

- [ ] 未测模型只显示“测试”，没有添加入口。
- [ ] 一次点击后立即调用档位探测；检测中状态可见；无重复“启动检测”步骤。
- [ ] 通过后唯一同一行 CTA 是“添加到 Codex”；关闭/重新进入不出现冗余“查看确认”回跳。
- [ ] 全 unknown 或整体不完整批次没有 Add CTA；mixed accepted+unknown 可 Add accepted 子集，unknown 清楚标示并跳过；明确 rejected 与 accepted 准确显示；bulk connectivity 不能解锁 Add。
- [ ] precommit_retryable 保留 ticket 且不调用 origin cancel；点击“重试添加”不发新的 gateway reasoning probe。
- [ ] precommit_revalidate 保留未消费 ticket 且不调用 origin cancel；重试只重新获取 `/models` 并核 digest，不发新的 reasoning probe；摘要变化则清除 Add 资格。
- [ ] proof_invalid、取消、reset、binding/catalog 变化、到期后清资格并要求重新测试；成功后显示已添加，原生模型仍保留。
- [ ] 单模型一次点击后立即出现 reasoning probe IPC，且 `window.confirm` 调用次数为 0；模型行不重复显示档位数量/费用提示，页面级提示仍可查看。批量确认在打包 Tauri 集成环境中验证无 ACL 拒绝。

### 后端

- [ ] 默认清单读取失败、最终随包 CLI 拒绝均 `write_count=0`、commit 未开始、有效 proof 未消费；恢复故障后可用同 proof 成功 Add 一次。
- [ ] 加入前取消、Reset、绑定/目录变化、过期、最终 consume 失败均 `write_count=0`；`/models` 临时刷新失败不消费 proof，下一次 retry 先核对 digest，变化时不得提交。
- [ ] 两个并发 Add 共用 proof，最多一个越过提交边界；另一个不写入。
- [ ] 写入启动后任何失败都不能返回 precommit_retryable；保留现有事务恢复语义。
- [ ] proof TTL/有界 deadline 测试覆盖“开始时不足 TTL”“运行中跨 expiry”。
- [ ] 推理档位的连接建立失败至多自动重试一次；超时、HTTP 响应、未完成响应等结果不自动重试，并由请求数与诊断码准确反映。
- [ ] mixed accepted+rejected 可生成 accepted 子集 proof；accepted+unknown 在完整 report 下也可生成仅含 accepted 的 proof；unknown 不进入 accepted/rejected。
- [ ] 日志不包含 key、响应正文、完整 CLI 输出或完整本地路径。

### 自动化与发行前验证

执行模型管理相关 Vitest、`gateway_model_probe`/`model_catalog` Rust 测试、`cargo fmt --check`、`git diff --check` 和匹配 TS 改动的类型检查/构建。另有真实随包签名 MSIX 时，在隔离目录执行 `debug models` 和最终 CLI 校验；不可用时明确标记未实测。本任务不做发布构建。验收记录包含源码 revision、构建 commit、运行安装器 SHA-256；打包 Tauri 环境需确认单模型测试无 native confirm、批量应用内确认不产生 ACL error。proof 生命周期另需 barrier 并发测试覆盖 Add vs Reset、Add vs cancel、两个 Add 共用同一 proof。

## 8. 本轮复核记录（2026-10-21）

复核基线提交为 `e09a541a5e25a4ee84bc9547a3230691cf4438ab`；以下修改保留在当前工作树，未提交、未打包：

- Skill Hub 基础清单 SHA 已同步到 `skill-audit.json` 当前字节哈希 `6effd655adbfc6f3be80b1c8348c2da2c428ef9a3d1b77b0c9da05da055e9953`。基础清单、`file-manifest.json`、`checksums.sha256`、`build.rs` 与运行时代码常量一致，54 个 Skill ID 完全相同；嵌入清单回归和完整 Rust 库测试通过。
- 模型管理只对 reqwest 明确标记的连接建立错误自动重试一次，等待 120 ms；超时、HTTP 响应及其他可能已送达的请求不自动重发。前端将 `upstream-connect-failed` 显示为网络连接失败。
- `npm run check` 通过；模型管理 Vitest **69 项通过**。Windows Rust 库 **761 项通过、6 项忽略**；模型目录、网关探测、原生目录/激活相关测试包含在内。相关 Rust 文件格式检查和限定文件差异检查通过。
- 发行目录的 `OpenAI.Codex_x64.msix` Authenticode 状态为 Valid，SHA-256 为 `f2913fdf027a0924ef42198222e325ac8f6331580cbffc51c7a8d18d8ce73c78`。在 D 盘隔离 CODEX_HOME 下，包内 CLI 的 `debug models` 返回 11 个默认模型；合并一个测试模型后 CLI 正常返回 12 个模型。未读取或改动用户 Codex 配置。

发行级 Tauri UI/IPC 整体验收仍需用当前工作树重新构建的安装器验证；本轮没有构建发行包、安装到用户目录或操作真实 U 盘。已发出请求后的超时仍要求用户重新检测，因为自动重发可能重复计费；这是保留的安全边界。





