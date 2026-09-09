# Codex U 盘一键更新 — 代码闭环开发文档

> CONTRACT_REV: `v2.2-engineering-gate-r1`
>
> 状态：**本轮开发契约**。本轮只解决可以在代码和自动化测试中闭环的问题；签名、实体 USB、真实 arm64 发布资产和外部授权继续保留为实测/外部条件，不因本轮测试通过而视为完成。
>
> 上游契约：`docs/UPDATE-REMEDIATION-DEVELOPMENT.md`（`v2.0-remediation-r1`）和本文件上一版 `v2.1-code-closure-r1`。本文件是增量开发契约，不替换上游 USB 授权、v2 manifest、stale expectation 和事务语义。

## 0. 目标与边界

用户目标：在暂不处理 Authenticode 签名的前提下，把前一轮审计中仍能通过代码解决的更新链路问题补齐，并用不访问真实 GitHub、不依赖实体 U 盘的自动化测试证明这些路径。

本轮目标：

1. 为 Codex payload apply 增加内部可注入的网络/下载/身份校验依赖，生产入口保持现有 Tauri API 和真实网络实现不变。
2. 用确定性 fake transport 覆盖成功更新、下载失败、校验清单缺失/错误、GitHub digest 不匹配、MSIX identity 不匹配、进度回调和失败后回滚路径。
3. 扩充 `resolve_latest` 的远端元数据拒绝矩阵，覆盖缺资产、非 HTTPS、manifest 架构不一致、manifest 字段缺失和 malformed JSON；确保失败发生在下载和破坏性写入之前。
4. 固化失败清理和事务恢复的可验证证据：旧 payload、三个 metadata 文件和授权根不因预提交失败而改变，staged final 文件不会被错误提升。
5. 同步测试文档和审计记录，明确哪些是代码闭环证据，哪些仍需实测或外部帮助。

明确不在本轮：

1. Authenticode 证书、SignPath/CI signing、签名状态门禁和签名资产生成。
2. 实体 USB 的插拔、换盘、真实授权 lease/license 和真实 `payload_apply_codex` E2E。
3. 真实 GitHub 网络、限流、代理服务器、发布仓库写入和正式 release approval。
4. arm64 真实 MSIX、Windows AppX 安装/卸载和 Store/框架依赖安装。
5. 修改公开 Tauri command、renderer payload schema、USB anchor allow-list 或默认发布源。
6. 新增第三方依赖；优先使用现有 callback、临时目录和已有 engine 类型。

本轮追加的工程门禁目标：

1. 消除 workspace 级 `cargo fmt --check` 的既有格式阻断，允许使用 rustfmt 的机械格式化，不改变行为。
2. 修复 `capability_pack` 中无效果的路径替换，统一使用跨平台路径分隔符；保持 Windows `USERPROFILE` 和 readiness 文件语义不变。
3. 让 macOS 预检的“超出 8 GiB”单测使用稀疏文件或等价的逻辑大小构造，避免实际占用临时磁盘；不得降低生产大小上限，也不得把该用例标记 ignored。

## 1. 当前缺口与设计决定

### 1.1 缺口

`resolve_latest` 已经接受 API/asset callback，因此远端解析可以离线测试；但 `apply_transaction` 直接绑定真实下载、checksum 下载、Authenticode 和 MSIX identity 读取，现有测试只能覆盖已经得到 staged 文件之后的提交/恢复，不能证明“下载到提交前”的完整流水线。

这会留下几个代码阶段无法确认的风险：

- 下载失败、checksum 获取失败或 staged 文件大小异常是否一定停在 destructive swap 之前；
- checksum、GitHub digest、identity 任一路径失败时是否会回到旧 bundle；
- 成功流水线是否真的把进度、校验和 `commit_staged_payload` 串在一起；
- 缺资产或恶意 URL 是否在网络和 USB 写入前被拒绝。

### 1.2 设计决定

新增仅限 `codex.rs` 内部的 `ApplyTransport`/等价依赖边界，并保留一个生产包装函数：

```text
apply_transaction(..., network)
    -> 构造真实 transport
    -> apply_transaction_with_transport(..., transport)
```

内部 transport 至少抽象：

1. MSIX 下载（保留目标路径、大小上限和进度回调）；
2. checksum 文本获取；
3. Authenticode 判定（本轮测试用 fake；生产 Windows 实现仍调用现有 engine）；
4. MSIX identity 读取（本轮测试用 fake；生产实现仍读取 staged MSIX）。

生产 `apply`、授权检查、重新解析 release、stale identity 比较、事务目录创建、提交和恢复顺序不得改变。测试只替换上述依赖，不能绕过 `commit_staged_payload` 的路径安全、metadata 一致性、SHA 清单和 rollback 逻辑。

### 1.3 失败处理不变量

对 `apply_transaction_with_transport` 的任何下载/校验错误：

1. 不调用 `commit_staged_payload`；
2. 不移动当前 payload；
3. 不更新 root/USB manifest 或根 `SHA256SUMS.txt`；
4. 不把 staged 文件提升为正式 payload；
5. 由现有 apply 外层调用 `rollback_transaction` 清理事务；若测试直接调用内部流水线，测试必须显式执行同一 rollback 并断言旧状态保持不变。

## 2. 开发任务与允许写入范围

### CODE-001：应用流水线 transport 注入

文件：`src-tauri/src/app/channels/codex.rs`

要求：

- 不改变公开函数签名和 Tauri API；
- 真实网络 wrapper 使用现有 `NetworkConfig`、HTTPS 检查、响应限制和 engine 下载函数；
- 真实身份 wrapper 使用现有 `read_msix_identity`；Authenticode wrapper 在 Windows 保持现有 OpenAI publisher 校验，非 Windows 维持当前编译语义；
- 统一把依赖错误映射为 `AppError::Engine`，保留“下载/解析/checksum/digest/identity”阶段信息；
- `MAX_PACKAGE_BYTES`、下载后实际文件大小检查、SHA256SUMS 按 package moniker 精确匹配、GitHub digest 比较不能移到测试专用路径。

### CODE-002：远端解析拒绝矩阵

文件：`src-tauri/src/app/channels/codex.rs`

要求：

- 继续使用 callback，不访问网络；
- 验证缺少 MSIX/checksum/release-manifest、非 HTTPS URL、manifest malformed、缺少架构字段、架构错配、digest 格式错误；
- 每个失败用例断言不会产生 `ResolvedRelease`，并尽量断言 asset callback 未被调用或调用次数符合预期；
- 不放宽 `owner/repo`、HTTPS 或 v2 manifest 约束。

### CODE-003：完整 apply 流程与回滚测试

文件：`src-tauri/src/app/channels/codex.rs`

用小型临时 bundle 和 fake transport 覆盖：

| 场景 | 必须证明 |
|---|---|
| 成功 | fake 下载写入 staged；checksum、digest、identity 通过；进度到达 total；提交后 payload、metadata、SHA256SUMS 和 backup 一致；事务目录清理。 |
| 下载错误 | 不进入提交；rollback 后旧 payload 和 metadata 完整不变。 |
| checksum 文本错误/找不到 moniker | 不进入提交；旧状态不变。 |
| SHA mismatch | 不进入提交；staged 不被提升，旧状态不变。 |
| GitHub digest mismatch | 不进入提交；旧状态不变。 |
| Authenticode/identity 错误 | 不进入提交；旧状态不变。 |
| identity 与 release manifest 不一致 | 不进入提交；旧状态不变。 |
| callback 报告进度 | 下载进度映射到 `progress(done, fresh.msix.size)`，提交成功后报告最终 payload 大小。 |

测试 fake identity 使用 `codex_win_engine::MsixIdentity`；不伪称 fake 通过了真实签名校验。签名仍由脚本/真实签名资产门禁负责。

### CODE-004：证据同步

文件：

- `docs/UPDATE-TESTING.md`
- `docs/UPDATE-AUDIT.md`
- 本文

要求：记录精确测试命令、通过数量、已知环境失败；将“代码闭环通过”与“真实 USB/签名/arm64 未验证”分开，不写 release approval。

### CODE-005：workspace 工程门禁收口

文件：`src-tauri/src/app/capability_pack.rs`、`src-tauri/src/app/mac_update.rs`，以及 rustfmt 需要触及的既有 Rust 文件。

要求：

- 只做 rustfmt 机械格式化和已确认的 `READINESS_RELATIVE` 无效果替换修正；不借此改变能力包安装、授权或更新语义；
- macOS 测试必须验证真实的逻辑大小上限，但不能为测试分配 8 GiB 实体磁盘；优先使用 Windows sparse file，其他平台使用原生稀疏文件能力；
- 运行完整 `cargo test --lib`、完整 `cargo fmt --check` 和未豁免的 `cargo clippy --lib -- -D warnings`；没有基线错误才能进入最终交接。

## 3. 验收门禁

本轮目标代码必须通过；workspace 级门禁若被冻结范围外的既有文件阻断，必须
保留原始错误、运行带最小豁免的 scoped 命令，并在交接中标为“环境/基线阻断”，
不能伪称全绿：

```text
cargo fmt --check
CARGO_INCREMENTAL=0 cargo test --lib app::channels::codex
CARGO_INCREMENTAL=0 cargo test --lib app::channels
CARGO_INCREMENTAL=0 cargo clippy --lib -- -D warnings
npx tsc --noEmit
npx vitest run src
```

若全量 `cargo test --lib` 仍受机器临时盘空间或 Windows 外部条件影响，必须单独记录失败测试、OS 错误和与本轮改动的因果关系；不能用 targeted test 代替全量测试，也不能把环境错误算作功能通过。

静态审计必须确认：

1. 生产 `apply` 仍先授权、再读取当前 bundle、再重新解析远端，测试注入没有成为 renderer 可控入口；
2. transport 只替换网络/读取依赖，不绕过 payload/manifest/SHA/路径安全和事务提交；
3. 所有失败测试都验证 rollback 或至少验证 destructive swap 未发生；
4. 文档、脚本和代码没有把 `-SkipSignature` 伪装成生产验收；
5. 独立 Agent 未提供只读审计结论时，最终状态必须标为“主控自审，独立审计证据不足”。

## 4. 角色与变更纪律

- 唯一写入者：当前 Codex 主控会话。
- 独立审计者：只读检查本轮 diff、测试证据和契约，不修改文件；当前不把派发超时当作完成证据。
- `COMPLEXITY_GATE`: `ESCALATE_REQUIRED`，原因是更新网络、校验链和事务提交跨越安全边界。
- 禁止提交、暂存、推送或修改外部 release。
- 不得删除用户已有 fixture 或历史产物；测试临时目录必须使用唯一 UUID，并在测试结束清理。

## 5. 完成定义

本轮只有同时满足以下条件才可标记“代码阶段完成”：

1. CODE-001 至 CODE-005 已实现并有针对性测试；
2. 上述 targeted checks、TypeScript 和 Vitest 通过；
3. 文档与审计记录已更新；
4. 静态审计没有发现本轮新增的未覆盖错误路径；
5. 明确列出签名、实体 USB、arm64、真实网络和外部授权仍未完成。

这不等价于“已完全实现一键更新”。完全实现还需要后续实测阶段按 `UPDATE-TESTING.md` 完成真实 USB、真实签名资产、真实 release 和 Windows AppX/依赖验证。

## 6. 本轮实现记录

截至 2026-09-08：

- CODE-001：已完成。生产 `apply_transaction` 使用真实 engine transport；测试通过内部依赖注入复用同一校验/提交路径。
- CODE-002：已完成。`resolve_latest` 已有 valid、缺资产、非 HTTPS、malformed/mismatched manifest、非法 digest 的离线拒绝用例。
- CODE-003：已完成。Codex channel 定向测试 24/24；覆盖 mock apply 成功、下载/checksum 错误、SHA/digest 错误、Authenticode/identity 错误、进度和回滚。
- CODE-004：已完成。`UPDATE-TESTING.md` 与 `UPDATE-AUDIT.md` 已同步，代码闭环和外部实测边界已分开记录。
- CODE-005：已完成。`cargo fmt --check`、未豁免的 `cargo clippy --lib -- -D warnings` 和完整 `cargo test --lib` 均通过；`READINESS_RELATIVE` 已移除无效果替换，macOS 超限用例改为不分配 8 GiB 实体空间的逻辑大小测试。

代码闭环结论：**完成（scoped code closure）**。主控静态自审结论：**通过**；独立审计结论仍不可交付（此前派发超时）。发布结论：**证据不足**，因为真实签名、实体 USB、arm64 资产、真实 GitHub/代理和外部 E2E 仍未完成。
