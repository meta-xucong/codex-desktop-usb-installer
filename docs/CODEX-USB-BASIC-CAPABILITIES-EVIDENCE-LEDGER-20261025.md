# U 盘版 Codex 基础能力差异与证据台账（2026-10-25）

**用途：**登记证据来源、版本绑定、判断强度和后续状态。此台账不把历史报告升级为当前测试结论。状态规则与章节顺序见[主开发规范](CODEX-USB-BASIC-CAPABILITIES-REPAIR-PLAN-20261025.md)；目标逐项验收见[回归矩阵](CODEX-USB-BASIC-CAPABILITIES-REGRESSION-MATRIX-20261025.md)。

**首个 GitHub checkpoint 范围：**为避免混入当前大量既有 dirty source、安装包和临时文件，首个 checkpoint 只提交本次四份开发计划及紧邻的 follow-up 证据文档。它只保存计划/证据状态，不是完整源码或工作区备份；当前源码候选仍由第0章的 tracked diff 与 untracked 指纹单独界定。

## 证据分类

| 标记 | 含义 | 可支持 | 不可支持 |
|---|---|---|---|
| `STATIC` | 当前可定位源码或文件头 | 代码控制流、静态字段/文件差异 | 测试机运行时成功、上游服务能力 |
| `PACKAGE` | 明确版本的原包/产物检查 | 该包条目内容和字段 | 测试 payload 同版本，除非哈希绑定 |
| `LOG` | 原始日志、命令、环境、结果可复核 | 被记录的那次运行 | 后续不同源码/包的结果 |
| `USER_REPORT` | 用户转述的现象或结果 | 作为待核对线索 | 未读原始证据的通过/根因结论 |
| `HYPOTHESIS` | 合理但未证实解释 | 设计对照测试 | 宣布根因或实施大范围回退 |

缺项明确写 `UNKNOWN` / `NOT_VERIFIED`；不要补猜版本、哈希、签名者、会话目标或请求状态。

## 当前已有事实与未知项

| ID | 项目 | 已见证据 | 判断 | 未知/下一步 | 状态 |
|---|---|---|---|---|---|
| E-01 | 模型基础指令 | `model_catalog.rs` 定义长 `MODEL_BASE_INSTRUCTIONS` 并在新条目写入 `base_instructions`；旧精确模板迁移可替换整段指令。 | `STATIC`：自定义指令确实注入；其中委派措辞可能影响模型行为。 | 同版本省略字段真实 fallback、最终模型提示词、工具表及委派收件回执；不能证明抑制原生多代理是根因。 | 待执行 |
| E-02 | Node 旧目录迁移 | 当前用户策略：仅精确匹配旧模板的布尔 `true` 删除；`false`、缺失、其他类型和不匹配不删；新模型省略字段。 | `USER_REPORT` + 源码待实施者重核：匹配模板手工 `true` 也删，是用户知情接受的不可区分边界。 | 当前候选源码/测试需绑定；不可沿用更早“保留 true”判断。 | 待执行 |
| E-03 | App Tools `.mcp.json` | 上一轮只读检查找到仓库内 `release/upstream/codex-app-26.1002.52244/OpenAI.Codex_26.1002.7124.0_x64__2p2nqsd0c76g0.Msix`；其中 `codex_app` 使用 `cmd.exe` 调脚本，cwd 为 `.`；脚本含多候选 Node 回退。当前源码 patch 直接选相对 bundled Node。 | `PACKAGE`：样本配置/脚本与源码改写有差异；改写保留 cwd 和其他配置字段。此包未证明与测试 USB payload 同版。 | 对照正式候选内 launcher、环境/argv、真实 initialize、工具表、调用和跨会话回执。 | 待执行 |
| E-04 | PE 子系统 | 远端候选源码已将 Code Mode host 移出子系统改写清单；仍改写若干 Node、Browser 和其他 sidecar。 | `STATIC`：只说明新构建遵守源码清单，不证明 stdio 根因或其他 sidecar 无副作用。旧 USB payload 不在本轮范围。 | 用户生成的新候选版本/hash、实际父进程/std handles 和 host 工具调用。不能推断 CUI/GUI 是工具故障根因。 | 新候选待验 |
| E-05 | Windows 环境 | 历史审计记录普通 USB/CDP 共享环境 builder、受管 Python PATH 优先和 UTF-8，曾报告定向测试 55/55。 | `LOG` 尚需绑定具体源码；不是当前测试机 E2E。 | 按四种入口核实实际调用与子进程环境。 | 待执行 |
| E-06 | Browser/focus | 历史报告包含 `reason=focus`；用户补充 issue 48522 作者把 focus 因果写为 hypothesis。 | `USER_REPORT`/`HYPOTHESIS`：只作线索；主窗口一次性激活不等于 Browser 插件 reconcile gate。 | 目标版本日志、工具表、插件缓存、焦点/PID 时间线。 | 待执行 |
| E-07 | Search/tool_search | 历史源码审计中模板 `supports_search_tool=false`；用户补充目标 Codex 版本区分 DIRECT 与 DEFERRED，tool_search 仅按支持与可搜索 deferred 工具条件出现。issue 44984 是 Bedrock delayed dynamic tools 线索。 | 版本/provider 相关，不可外推至所有 MCP/404token。 | 候选安装版本、provider/model、请求/响应与该回合工具表。 | 待执行 |
| E-08 | USB profile | 源码路径把 USB `CODEX_HOME` 放在本机 LocalAppData 的独立目录，并核对与 USB 卷分离。 | `STATIC`：隔离是产品边界；不证明跨会话成功。 | 测试机实际环境变量和 A→B→A 的 canonical ID、目标收件/回复回执。 | 待执行 |
| E-09 | Skill Hub / 自动标题 / Kimi | 用户提供的范围说明：Hub 曾调整测试服务器 fixture；自动标题未修；Kimi 和跨模型视觉工具未整体解决。 | `USER_REPORT`：不能把 fixture 变化当生产修复，也不能把历史任务状态未知当失败/成功。 | 生产代码入口、原始首次请求阶段、任务 ID 状态、当前会话标题请求与结果。 | 待执行 |
| E-10 | 卸载 | 已有产品决定：两步确认后清理本机 Codex/CCSwitch/受管数据，保留 USB 安装介质。 | 产品契约已确定，不重提“改为可选清理”。 | 只做静态路径和专用恢复测试环境验收；真实用户数据删除另行授权。 | 待执行 |
| E-11 | CDP 主题模型目录预检 | 当前 `codex_theme.rs` 的 `launch_codex_with_cdp` 调用 `launch_with_catalog_reload_preflight`；该 helper 先 prepare，再 launch，二者均成功后才调用 complete。`.test-tmp/dot-theme-preflight-20261025/theme-focused-final.log` 记录 5/5；`catalog-reload-focused.log` 记录 7/7；`rust-lib-final.log` 记录 802 passed、0 failed、6 ignored。 | `STATIC` + 本机日志：代码行为和相关定向用例可核对；全量日志存在较早失败尝试，见下表。所有日志均缺可复核的命令行及源码/二进制指纹，属于当前工作区证据，不能绑定发行候选。 | 四类启动路径全矩阵及 USB 测试机验证；检查最终源码是否与这些日志编译时相同。 | 代码已实现；USB 实机待验 |
| E-12 | 委派指令最小收敛 | `model_catalog.rs::MODEL_BASE_INSTRUCTIONS` 只删除 `prefer local sequential work;`，其余安全/不确定 handoff 措辞保留；旧精确签名 `LEGACY_MODEL_BASE_INSTRUCTIONS`、迁移条件、用户自定义条目写入逻辑未改。关联目录测试 1/1 通过。 | `STATIC` + 定向测试：变更确实只影响管理器提供的模板；没有实际 host delegation 回执，不能确认原故障根因或“恢复原生委派”。 | 同版本/provider/model 的实际工具表、最终回合委派调用/回执；真实 USB 候选待绑定。 | 代码已改；运行时待验 |
| E-13 | Code Mode host PE 子系统 | 远端源码清单移除 `codex-code-mode-host.exe`；CUI fixture 和未知 GUI fixture 定向测试通过。AppTools launcher/PE Node 耦合保持不变。 | `STATIC` + 定向测试：约束新构建；不证明 host stdio 工具已恢复。旧 USB payload 恢复/迁移已由用户取消。 | 用户生成的新候选及实际 host stdout/stdin/文件和命令工具调用。 | 新候选待验 |

## 历史测试结果：不得套用当前候选

| 报告数字/记录 | 证据状态 | 正确用法 |
|---|---|---|
| Rust 796 通过、0 失败、6 忽略；前端 478 通过、0 失败；历史 check/lint/build/fmt/diff 结果 | 历史 follow-up 文档记录；用户明确指出早于最新 Node 迁移。原始日志/源码指纹未在本轮重新绑定。 | 仅保留为历史记录，不写入当前候选全量 PASS。逐项复核原日志、命令、构建目标、忽略理由、源码/测试二进制指纹。 |
| 最新 Node 迁移定向 Rust 77/77、UI 11/11 | `USER_REPORT`；本轮没有核验 77/11 原始日志、命令或哈希。 | 标为“用户报告/原始记录未核实”，不是已失败，也不是当前已证明 PASS。可作为最小取证清单。 |
| 更早全量 248/0/3 或其他旧数字 | 历史记录，候选与目标不明。 | 不用于当前验收。 |

### 2026-10-25 CDP 主题修复相关日志复核

| 日志 | 原始结尾/测试范围 | 复核结论 |
|---|---|---|
| `.test-tmp/dot-theme-preflight-20261025/theme-focused-final.log` | 目标二进制运行 5 项 `usb_theme_launch_gate_tests`；5 passed、0 failed、0 ignored。包含 preflight-before-launch、preflight failure blocks launch、launch failure keeps marker pending、signed revocation/target mismatch。 | 定向测试通过。日志未带完整命令行、SHA 或 commit/diff 指纹。 |
| `.test-tmp/dot-theme-preflight-20261025/catalog-reload-focused.log` | 运行 7 项目录 reload/主题相关测试；7 passed、0 failed、0 ignored，含旧进程退出确认、marker 保留及正常 USB launch 路径。 | 定向测试通过。不是 USB 真实启动。 |
| `.test-tmp/dot-theme-preflight-20261025/rust-lib-full.log` | 日志先显示 `running 808 tests`，结尾为 701 passed、100 failed、6 ignored（三项合计 807）。 | 计数有 1 项差异；失败记录保留，不能以稍后的成功日志抹除。 |
| `.test-tmp/dot-theme-preflight-20261025/rust-lib-full-workspace-temp.log` | 807 tests；770 passed、31 failed、6 ignored。 | 另一轮尝试仍有失败，具体归因需看该日志的失败用例；不并入最终成功计数。 |
| `.test-tmp/dot-theme-preflight-20261025/rust-lib-final.log` | 808 tests；802 passed、0 failed、6 ignored。 | 成功终态日志支持一次 Tauri library 全量通过；缺命令行和源/二进制指纹，因此是未绑定当前候选的本机记录。 |
| `.test-tmp/dot-theme-preflight-20261025/rustfmt.log` | 文件为空（0 行）。 | 单凭空日志不能证明 rustfmt 命令或成功状态；该结果仍是用户报告。 |

该轮日志没有记录外部模型/推理来源，也没有 USB CODEX_HOME 桌面启动或介质验收。不要寻找通知密钥或据缺少 ServerChan 通知将修复判为阻断；这类通知不在本修复验收范围。

## 2026-10-25 最小实现与最终回归记录

以下为当前工作区命令输出；测试时 HEAD 为 docs-only checkpoint `00521fd65706b699942615b4ea87bcb12fbce4de`，Rust/应用源码仍包含已有未提交修改。没有完整 dirty source diff hash 与测试二进制 hash，因此结果不能绑定最终远端源码候选。

| 命令 | 结果 | 证据解释 |
|---|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml --lib tested_catalog_entry_preserves_exact_id_and_inherits_native_input_modalities -- --nocapture` | 1 passed | 覆盖模板变更的关联目录断言。 |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib code_mode_host -- --nocapture` | 1 passed | 未知 GUI host fixture 保持不改。 |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib portable_console_subsystem_patch_is_fixed_width_and_idempotent -- --nocapture` | 1 passed | sidecar 修改宽度和幂等；fixture 中 host 保持 CUI。 |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib codex_app_tools_mcp -- --nocapture` | 沙箱内 1 passed、1 failed；获批沙箱外仅重跑失败项后 1 passed | 失败是唯一临时 fixture 的 `MoveFileExW` 返回 `os error 5`；同一单测沙箱外通过。 |
| `cargo test --manifest-path src-tauri/Cargo.toml --workspace --quiet` | 702 passed、100 failed、6 ignored | Windows 沙箱运行；可见失败跨多个无关模块，含临时文件 `os error 5`、reparse/AppX/WpSystem fixture 路径、网络 fixture 等。输出截断，未逐项核实全部失败归因；不将其全部归为同一种原因或归因到本轮字符串变更。 |
| `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --all-targets --quiet` | 211 passed、39 failed、3 ignored | Windows 沙箱运行；失败集中在临时目录原子替换 `os error 5`、junction/reparse 和 WpSystem/AppX 测试路径拒绝。目标 host/PE 定向测试单独通过；不对整个 crate 标 PASS。 |
| `npm run check` / `npm run lint` | 均通过 | 当前工作区前端类型与 lint。 |
| `npm test` | 沙箱内 41 suite 在收集测试前 EPERM；获批沙箱外完整运行 41 文件、477 passed、1 skipped | 所有 41 个 suite 在沙箱内无法创建/rename Vitest 临时缓存；沙箱外前端和 `scripts/release-workflow.test.mjs`、`scripts/mirror-release.test.mjs` 随全套运行通过。 |
| `npm run build` | 通过 | Vite/renderer-entry 检查通过；有 App bundle chunk 大于 500 KiB 的非阻断提醒。 |

首次全量运行记录后，按失败域复核了三个沙箱权限敏感的独立 fixture：原子文件硬链接、临时 HKCU 注册表键、loopback HTTP 重试均在获批环境各 1/1 通过；结果支持这些代表失败与受限 runner 相符，不足以把先前全部失败归结为同一原因。另运行 `npm run test:release -- --reporter=verbose`，mirror-release 与 release-workflow 两文件合计 56/56 通过，其中 release-workflow 为 13 项。完整前端 verbose 日志确定唯一跳过项是 `tools/skill-hub-relay-sync.test.mjs` 的 Python 语法执行；日志明确记录当前 runner 无 `python`，静态断言仍运行，非 release 测试。

## 2026-10-25 获批环境复核

以下命令在本地提交 `c0a04c9bfc75ade9735f30727e9e1e318a28e804` 后运行，保存了完整 Rust 和 Vitest 输出。运行时工作区仍有本任务之外的未提交源码改动；没有 dirty-tree 完整指纹或独立干净 worktree，所以这是当前混合工作区回归，不声称等同于只含该提交的干净构建，也不绑定最终发行包。

| 命令 | 结果 | 日志与解释 |
|---|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml --workspace --quiet` | 808 项：802 passed、0 failed、6 ignored | `.test-tmp/minimal-repair-final-20261025/rust-workspace-final-elevated.log`。获批环境下全量通过；先前受限运行的失败记录保留作 runner 对照，不抹除。 |
| `npm test -- --reporter=verbose` | 41 文件通过；477 passed、1 skipped | `.test-tmp/minimal-repair-final-20261025/vitest-final-elevated.log`。唯一跳过为 Skill Hub 测试中需本机 `python` 的语法验证；运行环境未安装该命令，静态断言通过。 |
| `npm run test:release -- --reporter=verbose` | 56 passed、0 failed、0 skipped | 独立覆盖 `scripts/mirror-release.test.mjs` 与 `scripts/release-workflow.test.mjs`；后者 13 项均通过。全量 Vitest 同时也收录这两个文件。 |
| 权限敏感代表 fixture（3 条独立 cargo test 命令） | 原子硬链接 1/1；临时 HKCU 注册表键 1/1；loopback HTTP 重试 1/1 | 获批环境下均通过，与先前受限运行中 `os error 5`、注册表拒绝和 loopback `Connect`/`10013` 失败相对照。只证明这三项环境差异，不能据此替其余失败逐项断因。 |

未运行 `cargo clippy`、未单独运行 Rust 格式检查、未执行 GitHub release workflow；未打包、签名或发布产品，未做真实 USB/安装/卸载操作。GitHub 最终修复提交推送被自动审批拒绝，故远端仍只包含上面的 docs-only 检查点。

补充版本边界：`CODEX-USB-CODE-MODE-HOST-AUDIT-HANDOFF-20261025.md` 和整改文档另记录本机已安装包 `26.1016.8622.0` 的 host 签名检查；上表仓库内参考 MSIX 文件名是 `26.1002.7124.0`。两者是不同包证据，不能交叉绑定；本台账未独立复核前述签名验证过程，也没有证明任一包等于测试机 USB payload。

## 每个证据项的追加模板

```text
ID / 日期时间与时区：
目标机器/USB 身份与 profile：
Codex / provider / model / package 版本：
源码基线与 dirty diff 标识：
实际命令或用户动作：
退出码/阶段/结果：
原始日志路径与 SHA-256（未采集写 UNKNOWN）：
状态：PASS / FAIL / NOT_RUN / ENV_BLOCKED
解释与边界：
```

日志脱敏后保存；不要放入 access token、密码、完整密钥、无关会话内容或用户文件正文。

