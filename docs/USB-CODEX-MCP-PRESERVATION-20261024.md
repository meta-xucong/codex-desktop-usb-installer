# USB Codex provider handoff 保留 MCP 配置

## 目标

修复 USB 版 Codex 首次 404TOKEN provider handoff 可能覆盖 `CODEX_HOME/config.toml` 中 MCP 配置的问题。安装器已成功合并的 `[mcp_servers.*]` 必须在首次授权切换 provider 后继续存在，Codex 才能发现媒体 MCP。

## 诊断与基线

复核记录 `media-mcp-config-diagnosis.md` 显示：安装合并备份仍含启用的 `media_generation`；之后活动配置被替换为仅含 provider 设置的 286 字节文件；USN 时间线不能识别写入 PID。当前 CCSwitch 源码进一步确认 USB bootstrap 调用 `ProviderService::switch`；普通 provider switch 进入 `write_live_with_common_config` / `write_live_snapshot`，后者将 provider 的整段 Codex TOML 写入活动配置，而 USB 404TOKEN provider 模板没有 MCP 表。这证实普通切换路径存在覆盖机制，但不能证明历史 23:48 的具体写入者。代理接管且代理运行时会走热切换，该分支不写活动配置，因此不是这次正常 handoff 修复的目标路径。

实现前基线：主安装器仓库 HEAD 为 `b9940ae`；嵌套 CCSwitch 仓库 HEAD 为 `1d44b1ba`。主仓库、嵌套仓库均存在大量既有未提交改动。`services/provider/live.rs`、`services/provider/mod.rs`、`codex_config.rs` 已有 USB 404TOKEN 保护与隔离目录实现；这些变更属于基线，不得覆盖或撤销。本次只在 CCSwitch Codex provider live-write 边界增加 MCP 保留逻辑、切换前预检与测试。

## 冻结合同

- 用户目标：落开发文档、审计；按文档落代码、代码审计、测试，并尽可能详尽地在本机测试。
- 非目标：修改 USB 签名/授权、模型添加策略、安装器 MCP 部署、Product Design/Skill Hub、proxy takeover 备份策略、发行版本或打包流程；更改 USB bootstrap 的 provider/model 语义。
- 允许文件：
  - `docs/USB-CODEX-MCP-PRESERVATION-20261024.md`
  - `CCSwitch/CCSwitch/src-tauri/src/services/provider/live.rs`
  - `CCSwitch/CCSwitch/src-tauri/src/services/provider/mod.rs`（provider switch 前预检及 USB bootstrap 状态回归测试）
  - `CCSwitch/CCSwitch/src-tauri/src/config.rs`（Windows 替换现有配置文件时保留旧文件）
  - `CCSwitch/CCSwitch/src-tauri/src/database/dao/providers.rs`（恢复“无当前 provider”状态）
- 写入者：主 Agent 单写入者；独立审计者只读。
- 风险分类：D0（因果路径与修复语义已由源码确认）；I2（provider bootstrap、live config 原子写入和恢复/错误路径跨模块交互）；A1（Codex 配置持久化与 MCP 发现的跨模块不变量）。
- 路由证据：子 Agent 的指定模型/努力值属于请求配置；只有独立运行时元数据可证明实际路由。没有该元数据时标为 `ROUTE_UNVERIFIED`，不把路由声明当验收证据。

## 最小修复设计

在 Codex provider 写入 live 配置前，以结构化 TOML 修改目标 provider 文本。普通 provider switch 必须在回填当前 provider、更新当前 provider 选择或写活动配置/auth 之前完成预构建与预检；预检和后续写入复用同一份已准备的有效 provider 配置，避免校验与实际写入使用不同 TOML。USB bootstrap 会先幂等准备绑定 provider 的数据库记录，再调用 switch；如果 MCP 预检失败，允许保留该 DB-only provider 记录以便重试，但不得改变当前 provider 选择、活动配置或 auth，也不得写成功 handoff ack：

1. 读取当前活动 `config.toml`；当前文件不存在或没有 `mcp_servers` 时，不为 MCP 创建空配置。
2. 若活动配置有合法 `mcp_servers` table，而目标 provider TOML 没有，则复制该 table。
3. 若两侧都有合法 table，按 server ID 合并：目标 provider 显式提供的同名条目优先；活动配置中其余 MCP server 原样补入。
4. 若任一已存在的 `mcp_servers` 不是 table，返回清楚的配置错误并停止切换；不得静默覆盖或丢弃 MCP。此失败必须发生在当前 provider 选择或 live 文件变更之前。
5. 不改成功路径中的 `auth.json` 与 provider/model 选择语义；保留 USB CODEX_HOME 解析和模型选择门禁。bootstrap 只有在 provider/auth/endpoint readback 验证成功后才可写 handoff ack；若激活、后续 MCP 同步、readback 或 ack 失败，恢复切换前的 Codex provider 选择、auth.json 和 config.toml。

切换完成后的 `McpService::sync_all_enabled` 以数据库状态同步所有应用。Codex 路径在同步后，以数据库管理的 ID 为权威：已启用项保留数据库同步后的定义，明确禁用项保持移除；只把数据库未登记的旧活动 MCP 从切换前快照补回。这样保留用户自定义/历史 MCP，同时不反转 CCSwitch 里明确的启停操作。

本次只承诺保留 `mcp_servers` table，不承诺保留 provider 模板未建模的所有其他顶层字段；扩展到所有未知 TOML 键会改变 provider switch 的既有替换语义，超出最小修复范围。

## 必须覆盖的错误与回归场景

- 活动配置含 `media_generation` 与自定义 MCP；目标无 MCP：完整保留两个 server 及其字段。
- 两边有不同 server：合并后都存在。
- 同名 server：目标定义胜出，其他活动 server 保留。
- 目标无 MCP、活动无 MCP：结果不新增 MCP。
- 活动或目标出现非 table 的 `mcp_servers`：明确失败；验证失败后活动配置与 auth 未被改写。
- 通过 USB bootstrap 触发上述失败：允许 DB 中保留绑定 provider 的 DB-only 配置记录；当前 provider 选择保持原值，活动配置与 auth 未变，且不生成成功 handoff ack。
- provider handoff 中，MCP 数据库含一个启用且同 ID 定义不同的项、另一个禁用项、一个启用的数据库新增项；handoff 后启用项采用数据库定义，禁用项不复活，数据库新增项存在，未登记活动 MCP 保留，且成功 ack 只在最终配置核验后存在。
- MCP server 含 `command`、`args`、`enabled`、`env` 等字段，验证解析后的值不变。
- USB bootstrap 后仍有 404TOKEN endpoint/auth，且 bootstrap 不设置 `model` 或推理档位。
- provider TOML 无效或活动配置无法读取时不得写入半成品。

## 验收计划

1. 新增纯函数级 TOML 合并测试覆盖以上合并矩阵和 malformed 输入。
2. 增加/扩展隔离的 provider bootstrap 测试：临时目录同时作为 Codex CLI 的 `CODEX_HOME` 和 CCSwitch 的 `CCSWITCH_USB_CODEX_HOME`，并设置 `CODEX_USB_CCSWITCH_MODE=usb`；bootstrap 前断言 `get_codex_config_dir()` 等于该临时目录。预置 `media_generation` 和自定义 MCP，并在 MCP 数据库中加入同 ID 的冲突启用项和禁用项；bootstrap 404TOKEN 后断言原活动 MCP 定义均完整、404TOKEN endpoint/key 正确且没有自动模型选择。
3. 运行 CCSwitch Rust 定向测试、完整相关 crate 测试、`cargo fmt --check`、Clippy（若本机依赖/目标允许）。
4. 在临时 `CODEX_HOME` 下运行已安装 Codex CLI 的 `codex mcp list`（如果可用），确认发现媒体 MCP；不对真实用户配置做写入。
5. 运行配置安装/模型同步脚本的临时目录回归，确认它们合入的 MCP 能穿过 provider bootstrap；覆盖全局 MCP 同步异常时仍尝试恢复 Codex MCP 的失败分支；失败 bootstrap 还须恢复 provider 选择、auth.json 和 config.toml；检查本次 diff 不触及根仓库其他历史改动。
6. 冻结最终 diff，由独立只读审计者返回 `PASS`、`FAIL` 或 `INSUFFICIENT_EVIDENCE`；修复任何 FAIL 后重新冻结并复审。

## 实施记录

### 发行载荷门禁补强（2026-10-24）

后续复核发现历史 r4 包内 `CodexSwitch.exe` 的 SHA-256 为 `04607C8E8E8122E3D7A7FA3B0C360FA7144556D72772BE4A761AA4989BC36DB2`，时间戳为 2026-10-22；该二进制早于 MCP 保留实现，源码测试通过不能证明 USB 用户拿到的二进制含修复。为防止源码与发行载荷再次错位：

- MCP 合并路径输出固定编译标记 `CODEX_USB_MCP_PRESERVATION_V1`（仅 Debug 日志，不含 MCP ID、密钥或配置值）。
- CCSwitch 构建脚本在源码和最终 EXE 两侧检查标记；旧缓存 EXE 或缺少修复的源码直接失败。
- 产品 verifier、管理器打包入口和最终 U 盘验收同时要求该标记，确保根目录和内嵌副本不能绕过。
- 该门禁只约束 USB 产品版 CCSwitch，不改变 Provider 切换、模型选择、MCP 启停或其他客户端配置语义。

文档首轮审计为 `FAIL`，指出切换状态可能先于 live 写入更新；第二轮 `FAIL` 指出测试目录变量与 CCSwitch 路径解析不一致，且 bootstrap 会先保存 DB-only provider 记录。合同已修订为 switch 内预构建/预检、显式允许保留该幂等 DB-only 记录，并要求当前 provider 选择、live config、auth 和成功 ack 不变；同时锁定两个 home 环境变量到同一临时目录。代理热切换边界和两个仓库的基线均已记录。v3 复核未发现新的合同缺口，但审计结果为 `INSUFFICIENT_EVIDENCE`：当前无法验证审计代理的模型/努力值路由 provenance，因此不作为独立审计 PASS。

代码独立审计首轮为 `FAIL`：发现 `McpService::sync_all_enabled` 在 provider live write 后会覆盖或删除 MCP；已按审计发现增加 Codex-only post-sync MCP merge/write，并扩展 bootstrap 集成测试，注入同 ID 启用冲突和禁用项，核对最终配置。第二轮审计发现全局 MCP 同步在后续客户端失败时会提前返回，跳过 Codex 恢复；当前实现已保存同步结果，始终尝试 Codex 恢复后再返回原同步错误，并在双重失败时同时报告两种错误。新增故障注入测试：Codex MCP 同步先覆盖活动项，随后 Gemini MCP 写入因目录冲突失败；测试确认 Codex 活动项被恢复且没有成功 ack。第二轮还发现验收文案与冲突测试不一致，已修正文档。最终只读复审确认这条错误路径和恢复断言成立，未发现新的代码问题；复审状态为 `INSUFFICIENT_EVIDENCE`，唯一原因是审计代理的模型/努力值路由 provenance 无法独立验证，因此不登记为具备路由证明的审计 PASS。

本机验证（2026-10-24）：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`（早期冻结版本）：1118 passed、0 failed；后续修复以当前定向测试记录为准。
- 定向测试 `services::provider::tests::usb_404token_bootstrap_activates_and_acknowledges_verified_live_route`：1 passed；覆盖同 ID 启用数据库冲突和禁用数据库项。
- 定向测试 `services::provider::tests::usb_404token_bootstrap_restores_codex_mcp_when_later_app_sync_fails`：1 passed；确认 Codex 同步后续 Gemini 同步失败时仍恢复 MCP，且无成功 ack。
- 临时 CODEX_HOME 集成链：运行 `configure-codex.ps1`、`sync-model-catalog.ps1`，再运行本机 `codex mcp list`；列出 `media_generation` 和 `custom`，二者均启用。只读枚举成功，不代表实际启动 MCP 或外部生图端到端通过。
- `cargo test --all-targets`：失败。Windows linker 报 `LNK1104` 打不开 `lnk{GUID}.tmp`，并且并行测试目标构建出现 `cc_switch_lib` crate/`windows_sys` rlib-rmeta 版本冲突；这不是功能断言失败。完整 lib suite 与目标 bootstrap 测试独立成功。
- `rustfmt --check` 报 CCSwitch 原有未提交基线片段的格式差异（`usb_404token_live_readback_matches`、Codex 模型选择错误行及既有 USB provider DAO 代码）；本轮新增代码格式差异已手动修正。`git diff --check` 两仓均通过。

### 本轮发行修复与验证（2026-10-24）

- 为活动 MCP 合并路径增加非敏感编译标记 `CODEX_USB_MCP_PRESERVATION_V1`，仅用于构建产物识别和发行门禁；日志不输出 server ID、配置字段或凭据。
- `build-ccswitch-with-env-patch.ps1` 同时检查源文件与最终 EXE；`verify-product.ps1`、`build-installer.ps1`、`verify-oneclick-usb.ps1` 均拒绝缺少标记的 CCSwitch。这阻止历史 r4 同类二进制在后续重打包时被误当作已修复版本。
- 从 `CCSwitch/CCSwitch` 当前产品源码构建成功，渲染层、`cargo check`、Tauri Windows release 构建和产品静态验证通过。新产物位于 `release/ccswitch-usb-mcp-preservation-20261024/CodexSwitch.exe`，SHA-256：`8CAC3CA860DC07EF6CB6329509BE0C85E82394CCB1214C2E3B5FA6CC63E2AC89`。已同步到 `src-tauri/resources/ccswitch/`，产物目录与规范资源均由 `verify-product.ps1 -SkipNetwork` 验证通过。
- 使用历史 r4 包执行产品 verifier，确认旧二进制因缺少 MCP 标记而失败；新产物与规范资源均通过。
- PowerShell Parser 对本轮修改的四个构建/验证脚本均无语法错误。
- 6 个专项 Rust 回归测试通过：3 个 USB bootstrap handoff/失败恢复测试、3 个 MCP table 合并/冲突/非法输入测试。
- provider 全集 32 项中有 24 项通过、8 项失败；失败均为沙盒环境对 OpenClaw、OpenCode、Claude 临时配置原子替换返回 `PermissionDenied`，包含后续依赖这些写入的断言。本轮 MCP 和 404token 专项用例在并行与串行执行中均通过。
- `cargo fmt --check` 未执行成功：该本机 Rust toolchain 未安装 `cargo-fmt` 子命令。发行构建本身成功。
- 上一轮因多 Agent 并发上限已满，仅完成主控自审；本轮已补做独立只读审计，首次结果为 `FAIL` / `ROUTE_UNVERIFIED`。修复后已发起新一轮独立只读复审。
- 本轮只构建 CCSwitch 产品 EXE 并更新仓库规范 CCSwitch 资源，没有重建完整管理器/一键安装 U 盘包；旧 release 目录保持原样。需将本 EXE 作为 `-CcswitchArtifactDir` 输入进行后续完整打包，新门禁会拒绝旧二进制。

### 独立审计修复轮次（2026-10-24）

- 独立只读审计结论：`FAIL` / `ROUTE_UNVERIFIED`。发现 bootstrap 的后续失败会留下新 provider 状态；MCP 恢复逻辑会复活数据库明确禁用的 ID；Windows 替换配置文件前先删除旧文件；本地 CCSwitch 产品源码被根仓库忽略，裸上游 commit 不能重建该产物；字符串标记只能筛旧二进制，不能单独证明实现正确。
- 已修复前三项：bootstrap 失败时回滚当前 provider 选择、auth.json、config.toml；数据库管理的 MCP ID 以数据库启停与配置为准，未登记的活动 MCP 才保留；Windows 替换文件不再先删除旧文件。
- bootstrap 回归测试覆盖数据库启用项的更新定义、禁用项不复活、数据库新增项、未登记活动项保留，以及 Gemini 下游失败后的 provider/auth/config 回滚和无成功 ack。
- 修复后定向 Rust 测试：USB bootstrap 3/3、MCP TOML 合并 3/3、Windows 现有文件替换 1/1 通过。完整 lib suite 尚未对这轮改动重跑；`rustfmt --check` 仍只报告本地既有未提交片段格式差异。
- 构建脚本现会在发行构建前执行上述功能回归测试，并检查 MCP marker 对应的源函数调用。来源说明与 manifest 已明确：当前 USB 产品源代码修订没有包含在根仓库中，upstream `sourceCommit` 只能代表基础版。
- 修复后完整 `cargo test --lib -- --test-threads=1`：1083 passed、36 failed。失败均发生在沙盒系统临时目录的写入/替换并返回 `PermissionDenied`，涉及 prompts、Hermes、OpenClaw/OpenCode/Claude 与代理配置；针对本次修改的工作区隔离定向测试通过，完整 suite 在当前沙盒内不能记为通过。
- 固定产品源码补丁早期版本已放入 `src-tauri/resources/ccswitch/source-patches/ccswitch-usb-product-1d44b1ba.patch`，当时 SHA-256 为 `4D30EE355A89789501696C3662D529702875E70C4325D275AB69E0B569C60671`；该版本随后被本节所述的最终补丁替代。早期版本从上游基线完整重放了当时的 27 个修改/新增源码文件，干净基线应用后的文件哈希与当时产品工作树逐一相同。
- 补丁当前是本轮新增的未跟踪文件，尚未进入 Git 提交；因此当前工作区和随产品资源包都可复现，其他干净 Git checkout 需先包含本轮文件。构建脚本会校验补丁 SHA、在干净固定基线上自动应用、校验实现调用点，并要求 3+3+1 条回归测试实际通过。
- 此处记录的是当时的中间状态；该轮独立只读复审后来完成，结论与路由 provenance 状态见文末最终审计说明。

### 独立复审补充修复（2026-10-24）

- 复审再次发现 Codex 自身 MCP 同步中途失败时，旧活动表可能复活数据库禁用项；已将失败分支改为从数据库重建全部受管理 ID，仅添加 `apps.codex=true` 的定义，并保留切换前快照中的未管理项。新增 `failed_codex_mcp_sync_recovers_managed_entries_from_database_snapshot` 回归用例。
- 复审发现失败分支在同步后再次读取数据库，第二次读取失败仍会留下部分切换状态。现已在 provider 状态变更前读取一次 MCP 快照，并把同一快照交给同步和恢复逻辑；恢复路径不再二次查询。同步服务增加 `sync_all_enabled_from_snapshot`，原有无参快照 API 保持兼容。
- 构建脚本预打补丁路径已收紧：固定 HEAD 必须精确匹配；补丁需能完整反向应用；dirty path 集合必须等于补丁文件集合；再从固定 HEAD 归档应用补丁，对每个变更文件逐一比 SHA-256。临时核对目录位于仓库内以适配本机沙盒限制。
- 预打补丁检查也拒绝 Git index 中的 `assume-unchanged` 和 `skip-worktree` 隐藏标记；本机检查结果两者均为 0。`git status --ignored` 当前只列出标准 `dist`、`node_modules`、Tauri `gen` 和 `target` 构建目录。
- 构建脚本现将 `git status --ignored` 纳入检查；只允许 `node_modules`、`dist`、`release`、Tauri `gen`/`target` 等标准构建目录及明确的非构建文档/日志，其余 ignored path 一律拒绝。修改后再次执行 `-SkipBuild` 构建全门禁成功。
- 构建/产品 verifier 新增 `CODEX_USB_MCP_SYNC_FAILURE_RECOVERY_V1` 源码及二进制门禁，旧 EXE 不得仅凭原 MCP 保留标记通过；来源说明和 manifest 均指向当前产品补丁。
- 最新产品补丁 SHA-256：`E4F8DB42FD646FFBAA6A63DA294C58CBE1727BC197609CB3719E7D5DFFF3423E`。本轮定向测试 8/8 通过，`cargo check --locked` 通过。
- 已完成真正 Tauri Windows Release 构建，产物目录 `release/ccswitch-usb-mcp-preservation-20261024-r5/` 和规范 `src-tauri/resources/ccswitch/` 的 EXE 哈希均为 `BFB08F381EF476D6E6907F9B74D7A637E736CEC76EF3AF48F224D312A2AF7E08`；两份 verifier 均通过，EXE 含新恢复标记。旧 `r2`/`r3` 产物保留，不作为本轮修复版。
- 修复后再执行构建脚本 `-SkipBuild` 全门禁：源码重放、renderer、8 条定向测试、`cargo check`、EXE 复制和产品 verifier 全通过。完整一键安装包没有重建，因此安装介质仍需下一步用 `r5` 目录构建。
- 中间构建记录：`release/ccswitch-usb-mcp-preservation-20261024-r2/CodexSwitch.exe`，SHA-256：`25BC711526F6196606DE77F3E95586A48F6985DEE88BA33701E9C534D2394773`。该产物之后被 `r5` 替代，不是当前规范资源或本轮最终产物；请以文末 `r5` SHA-256 和 verifier 结果为准。
- 固定产品源码补丁已移入 `src-tauri/resources/ccswitch/source-patches/`，并随产物目录输出；manifest 与产品 verifier 校验其 SHA-256。构建脚本在干净的固定基线上自动应用补丁，且构建前要求 3 个 bootstrap、3 个 TOML 合并、1 个 Windows 替换用例通过。
- 完整一键安装包仍未重建；本轮只验收单独 CCSwitch 产物与源码修复。全量 Rust 测试仍有 36 条受当前沙盒临时目录 `PermissionDenied` 限制，不据此宣称整个 CCSwitch 测试套件通过。
- 最终独立只读复审：就本次 ignored-path 构建门禁和单独 CCSwitch 产物范围返回 `PASS / ROUTE_UNVERIFIED`，未发现合理的 P1/P2 阻断。审计者确认完整安装包未重建不属于本次代码/单独产物验收范围；完整包交付时仍须另行重建并核对 r5。

### 最终状态核对（2026-10-24）

- 再次读取工作区后，当前 `r5/CodexSwitch.exe` 与 `src-tauri/resources/ccswitch/CodexSwitch.exe` SHA-256 相同：`BFB08F381EF476D6E6907F9B74D7A637E736CEC76EF3AF48F224D312A2AF7E08`；产品补丁 SHA-256 为 `E4F8DB42FD646FFBAA6A63DA294C58CBE1727BC197609CB3719E7D5DFFF3423E`。
- 此处为整包重建前的状态记录；随后已完成完整 Codex USB 一键安装包和匹配签名器目录构建，具体产物与验收见文末“完整 U 盘包与签名器发行（2026-10-24）”。在该构建完成前，旧一键安装包不包含本修复。
- 独立复审对上述范围为 `PASS / ROUTE_UNVERIFIED`：未发现合理的 P1/P2 阻断；审计模型/路由 provenance 无法独立核验，因此不宣称路由身份已证明。完整包后续交付前仍须用 `r5` 重建并再次验收。
- 本机定向回归 8/8 及 `cargo check --locked` 通过。完整 Rust lib suite 有 36 项因沙盒临时文件 `PermissionDenied` 失败，不能记为全量测试通过。

### 完整 U 盘包与签名器发行（2026-10-24）

- 完整目录：`release/usb-oneclick-v0.5.25-mcp-preservation-20261024-r5/`；入口为 `CodexDesktopSetup.exe`。静态清单共 582 个文件，约 1.78 GB，含官方 Codex Desktop `26.1016.62342` x64 MSIX（SHA-256 `9085cf2e2eb5081f894a2cf334e1f36dbb9438ecb409d20195aa2fe1ecfcb4c8`）、管理器、CCSwitch r5、运行时、默认能力包、资源与自验证脚本。
- 完整包通过 `verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork`；包内 CCSwitch 和嵌入式副本都通过产品 verifier，EXE SHA-256 与 r5 一致。Microsoft VC++/WebView2 组件签名与哈希通过；入口 EXE 的 `requireAdministrator`、Codex entry shim 的 `asInvoker` 与管理器的签名公钥绑定通过。
- 匹配签名器目录：`release/usb-license-signer-gui-v0.5.25-mcp-preservation-20261024-r5/`。使用现有 0.5.25 签名器发行二进制（SHA-256 `56C963E453409337C9977027EAB702CD4FA41AEA5B04B5080D00980808568283`）及其配套加密 portable vault、公共绑定描述、profile、README 和 manifest；签名器源码不受本次 MCP 修复影响。由于当前构建环境没有签名器编译所需的 404token 管理员密钥，没有重编签名器 EXE；复制后的 manifest 与 `SHA256SUMS.txt` 全部匹配。
- `verify-usb-signing-pair.ps1` 通过：包和签名器均绑定 releaseVersion `0.5.25`、releaseId `codex-26.1016.62342-model-add-20261023`、同一 keyId 和公钥摘要。管理器与 U 盘包使用 `SignedRequired` / 内部 USB 授权策略。
- 这是静态发行目录，不含运行期 `usb-license.json`。真实 U 盘授权签名需要把包复制到目标 U 盘后由签名器绑定该介质身份生成；本机未伪造该文件，也未声称完成真实 U 盘签名或 Authenticode 代码签名。
- 本次构建前 `npm run check`、`npm run lint` 通过。Vitest 在当前沙盒临时目录上发生 `EPERM rename`，37 个套件未能加载；改用工作区临时目录后无可用进程输出，手动停止。不得把前端测试记为通过。MCP 修复专项测试仍为 8/8，`cargo check --locked` 通过；完整 Rust 套件受沙盒临时文件权限限制。
