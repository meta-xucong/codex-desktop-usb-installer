# 2026-10-23 模型添加流程简化

> 该版本描述的是早期简化方案；其“两档探测、探测后再确认、最终 Codex CLI 校验”流程已由 [2026-10-24 直接添加方案](MODEL-ADD-DIRECT-COMMIT-20261024.md)取代。本文保留为决策历史，不作为当前实现规范。

## 目标与范围

缩短第三方模型从检测到写入 Codex 目录的耗时，减少上游抖动造成的重复检测，同时维持账号隔离、单次授权、Codex 目录兼容性、原子写入和 MCP 配置保留。

本次只调整模型管理检测/添加链路和相应诊断状态。不得改动签名校验、U 盘授权合同、能力包安装/MCP 注册流程、模型目录备份恢复语义或发行打包流程。

工作区基线已有未提交改动。尤其 `gateway_model_probe.rs`、`model_catalog.rs`、能力包安装文件已有 MCP 注册和模型能力修复；实现必须保留这些既有改动，只在本文件列出的模型添加边界内继续修改。

## 方案

### 检测

1. `/models` 只在检测阶段请求。复用现有最多三次、仅对可安全重试的只读 GET 错误进行重试。推理请求保留底层现有的单次 pre-request `Connect` 重试（该类错误发生在请求体发送前）；不得重试可能已经到达上游的超时、网络中断、认证、响应格式或其他生成错误，避免重复请求/费用。
2. 选择模型后默认只检测 `medium`。若上游明确返回不支持该档位，最多再检测一次 `low`。首个成功档位即停止；网络/超时/认证等不确定错误立即失败，不继续逐档请求。服务端验证最多两个档位，避免调用方绕过 UI 扩大请求数。
3. 费用确认仍然明确。写入的默认档位和支持档位只包含实际通过的档位，避免把未经验证的能力写入模型目录。

### 提交

1. 提交不再重复请求 `/models`，也不再比较重新拉取的上游目录摘要。已签发的服务端一次性凭据已绑定精确模型 ID、供应商、服务地址、U 盘账号绑定、凭据摘要、复位 epoch 和有效期；模型在检测阶段先从目录解析，再以该模型真实请求通过后签发凭据。
   检测时同时把该模型已解析的 `owned_by` 元数据放进短时服务端票据，Add 使用票据中的值，不信任前端回传，也不为恢复显示名称而额外请求上游。
2. 提交时保留本地绑定和 CODEX_HOME 复核、并发/取消门禁、文件快照一致性检查、随包 Codex 对候选目录的最终兼容性验证、凭据在首次原子写入前单次消费、原子写入及失败恢复。
3. 一次性凭据必须至少剩余 610 秒，覆盖最多 600 秒的本地验证预算和 10 秒提交余量；推理检测票据有效期为 1200 秒。

### 失败状态

- 可重试的提交前本地故障或目录操作忙碌保留有效凭据，按钮显示重试；不得要求重新进行有费用的模型检测。
- 凭据过期、账号/服务绑定改变、reset/cancel、提交结果不确定时撤销提交资格并刷新实际目录状态。
- 已通过的检测结果与可提交凭据分开呈现。传输异常或提交结果不确定时不能把“已检测通过”伪装成“从未检测”；也不能在提交结果不明时盲目重复写入。

## MCP 与配置文件安全边界

新增模型只更新管理器自己的模型目录及 `config.toml` 内的模型目录托管块。不得重序列化或重建整个 TOML。保留配置快照竞争检查、原子替换和恢复逻辑。包含 `[mcp_servers.media_generation]` 的回归夹具必须在添加前后保留原段落及值。

本地测试只能证明配置文本没有被模型添加代码删除；不能代替测试机上的 Codex Desktop 工具注册及 MCP 实际调用验收。当前媒体 MCP 注册修复继续作为独立验收门禁。

## 允许修改

- `src-tauri/src/app/gateway_model_probe.rs`：检测请求预算、推理档位尝试、一次性凭据校验和 Add 阶段。
- `src-tauri/src/app/model_catalog.rs`：仅在一次性凭据消费接口变更必须时调整调用；保留已有 MCP/TOML 合并实现。
- `src/app/views/UpstreamModelManagerSheet.tsx` 及其测试：默认档位、成功停止、失败状态区分。
- `src/app/views/UpstreamModelManagerSheet.test.tsx`：检测档位、重试和“不确定写入仍显示检测通过”回归。
- `docs/MODEL-ADD-FLOW-SIMPLIFICATION-20261023.md`：本开发合同和验收记录。

不得修改安装器/能力包/MCP 注册/签名/发布代码，除非测试发现本范围内无法修复的直接依赖；如遇此情况先停止并提交证据，不扩大范围。

## 验收

1. 成功默认档位只产生一次 reasoning 生成请求。
2. 客户端只提交 `medium`、`low` 两档；仅 `UnsupportedEffort` 允许从 `medium` 回退一次到 `low`。其他错误不尝试第二档；两档都不支持时不签发凭据。底层 pre-request `Connect` retry 不得扩展为超时/网络响应后的重发。
3. Add 调用不执行 `/models` 网络请求；只消费检测时签发的、精确绑定且未过期的一次性凭据。
4. 账号、API 凭据、服务地址、模型 ID、CODEX_HOME、取消/reset、过期或并发配置变化仍被拒绝；提交前故障不写目录。
5. Codex 最终目录验证、目录/配置快照检查、原子写入、恢复和幂等结果检查保持有效。
6. 重试类失败保留通过状态及有效凭据；失效/过期凭据不允许提交；未知结果显示“待确认”并刷新目录，不显示成“未检测”。
7. 添加模型时 MCP TOML 段保持不变；模型条目不关闭宿主 Skills/插件/App 使用说明。
8. 定向 Rust 和前端测试、格式/类型检查通过。独立只读审计结论为 `PASS`；测试机 MCP 端到端未执行时明确保持开放，不宣称已修复。

## 实施与审计记录

### 2026-10-24 Codex CLI 回显兼容修复

- 失败证据：`codex-app-manager(20).log` 中 Sonnet 与 Kimi 的上游探测通过；三次 Add 均到达 `native-final-process-validate`。随包 CLI 退出码为 0，但管理器将其 `debug models` 输出判为不匹配，未进入模型目录提交。原日志只记录 stdout 长度，没有字段差异。
- 复现：从已验证的 Codex `26.1016.8622.0` MSIX 提取 `app/resources/codex.exe`（`codex-cli 0.163.0-alpha.11.2`），在隔离 CODEX_HOME 对 12 模型候选运行 `debug models`。CLI 退出码为 0，模型数量和 slug 集合一致；回显省略多个 `model_messages.instructions_variables: null`。旧版逐对象比较将合法 null 缺省误判为不匹配。
- 修复：保留 Codex 原生解析门禁、精确模型数量/slug 集合、所有候选字段及原子提交检查；仅允许 `model_messages.instructions_variables` 预期为 null 时在 CLI 回显中缺省。该对象其他字段仍逐项比较。CLI 额外回显只允许已观测到的运行时字段及其固定默认值（包括由 `base_instructions` 生成的 `model_messages`）；未知模型字段、未知 root 字段、候选字段变化、非 null 嵌套值缺失、重复/额外模型均拒绝。回显字段不会写入管理器目录。
- 回归测试覆盖嵌套 null 缺省、非 null 缺失、指令字段变化、提交字段变化、模型集合及 root 变化。
- 本机已用包内精确新版 Codex CLI 复现回显差异；真实 USB CODEX_HOME 未修改，未调用 404token 或媒体 MCP。
- 本次验收：新版 CLI 在隔离 `CODEX_HOME` 退出码 0，候选/回显均为 12 个模型且 slug 集合相同，观察到 11 个预期 null 的 `instructions_variables` 缺省。匹配器测试 22/22、目录流程测试 64/64 通过；格式检查、独立审计和最终 USB 包验证以本次最终记录为准。

### 实施记录

- 已完成：Add 阶段移除 `/models` 重拉取与目录摘要复核；保留检测时解析的模型元数据、服务端一次性票据和 Add 本地兼容性/原子写入校验。
- 已完成：UI 以 `medium`、`low` 顺序请求；服务端限制为该顺序且最多两档，只在明确 `UnsupportedEffort` 时回退，并在首档成功时停止。
- 已完成：检测通过状态单独持久化；可重试本地失败保留 ticket，未知/失败提交刷新实际目录且保留已检测结果。
- 已完成：保留底层仅对 pre-request `Connect` 的一次安全重试；不重试可能已送达上游的生成超时/中断。
- 验证：`npm test -- --run src/app/views/UpstreamModelManagerSheet.test.tsx --pool=threads --maxWorkers=1`，38/38 通过；`npm run check` 通过；`git diff --check` 通过。
- 环境限制：本机未找到 `cargo`、`rustfmt` 或 `rustc`，因此 Rust 定向测试和格式检查未能执行；不据此宣称 Rust 编译通过。
- 独立只读审计：首轮发现 low-only 绕过 medium、一个旧 proof 状态断言和 UI 过期时间偏差；三项均已修复。复审确认修复存在，未发现新的源码阻塞项；审计最终为 `INSUFFICIENT_EVIDENCE`，因为审计代理路由 provenance 标记 `ROUTE_UNVERIFIED`，且本机未能运行 Rust 编译/测试。
- 测试机 Codex Desktop/MCP 端到端验收：未执行，仍需单独实机确认。

### 本次最终验收记录（2026-10-24）

- 新版 Codex CLI `0.163.0-alpha.11.2` 在隔离 `CODEX_HOME` 重放成功：退出码 0；候选和回显均为 12 个模型、slug 集合完全相同；11 个 native 模型的 `model_messages.instructions_variables: null` 在回显中缺省。
- Rust 定向回归：`app::model_native_probe::tests` 22/22、`app::model_catalog::tests` 64/64 通过；覆盖固定额外字段白名单、未知模型/嵌套字段拒绝、nested null 兼容，以及模型目录提交边界。`rustfmt --edition 2021 --check` 和 `git diff --check` 通过。
- 独立代码审计：PASS。审计确认 output-only `model_messages` 必须从候选 `base_instructions` 精确派生，字段和值固定；未知模型字段、未知嵌套字段和未知 root 字段 fail-closed。
- USB 正式构建成功，前端 production build 与 Rust release build 均通过。完整 USB 包：`release/usb-oneclick-v0.5.25-20261024-model-add-fix-r4`；包内 Codex `26.1016.8622.0`，MSIX SHA-256 `9085cf2e2eb5081f894a2cf334e1f36dbb9438ecb409d20195aa2fe1ecfcb4c8`。
- 包内 `verify-usb.ps1 -SkipNetwork -InternalSigned` 通过；签名工具配对检查通过，portable vault 与 `keyId=prod-2026-10-03`、公钥摘要及发布 profile 一致。签名 GUI 协议未变化，复用原签名工具。
- 未在真实 Codex Desktop 会话中执行上游模型添加或 MCP 端到端调用；本次使用真实随包 CLI 和隔离目录验证，不触碰用户真实 `CODEX_HOME`，也未调用 404token。
