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
| E-04 | PE 子系统 | 上一轮在该本地 MSIX 条目中只读解析 Code Mode host、`node.exe`、`node_repl.exe`、Chrome extension-host：PE32+、Subsystem 3；四个 security directory 均非零。源码清单不再包含 host，仍改写若干其他 sidecar。 | `PACKAGE` + `STATIC`：参考包字段和源码规则已观察。证书表非零不是签名链验证，也不是测试 payload 对照。 | 同版本可信包、签名/哈希核验、实际父进程/std handles、USB 文件哈希与 overlay 记录。不能推断 CUI/GUI 是工具故障根因。 | 待执行 |
| E-05 | Windows 环境 | 历史审计记录普通 USB/CDP 共享环境 builder、受管 Python PATH 优先和 UTF-8，曾报告定向测试 55/55。 | `LOG` 尚需绑定具体源码；不是当前测试机 E2E。 | 按四种入口核实实际调用与子进程环境。 | 待执行 |
| E-06 | Browser/focus | 历史报告包含 `reason=focus`；用户补充 issue 48522 作者把 focus 因果写为 hypothesis。 | `USER_REPORT`/`HYPOTHESIS`：只作线索；主窗口一次性激活不等于 Browser 插件 reconcile gate。 | 目标版本日志、工具表、插件缓存、焦点/PID 时间线。 | 待执行 |
| E-07 | Search/tool_search | 历史源码审计中模板 `supports_search_tool=false`；用户补充目标 Codex 版本区分 DIRECT 与 DEFERRED，tool_search 仅按支持与可搜索 deferred 工具条件出现。issue 44984 是 Bedrock delayed dynamic tools 线索。 | 版本/provider 相关，不可外推至所有 MCP/404token。 | 候选安装版本、provider/model、请求/响应与该回合工具表。 | 待执行 |
| E-08 | USB profile | 源码路径把 USB `CODEX_HOME` 放在本机 LocalAppData 的独立目录，并核对与 USB 卷分离。 | `STATIC`：隔离是产品边界；不证明跨会话成功。 | 测试机实际环境变量和 A→B→A 的 canonical ID、目标收件/回复回执。 | 待执行 |
| E-09 | Skill Hub / 自动标题 / Kimi | 用户提供的范围说明：Hub 曾调整测试服务器 fixture；自动标题未修；Kimi 和跨模型视觉工具未整体解决。 | `USER_REPORT`：不能把 fixture 变化当生产修复，也不能把历史任务状态未知当失败/成功。 | 生产代码入口、原始首次请求阶段、任务 ID 状态、当前会话标题请求与结果。 | 待执行 |
| E-10 | 卸载 | 已有产品决定：两步确认后清理本机 Codex/CCSwitch/受管数据，保留 USB 安装介质。 | 产品契约已确定，不重提“改为可选清理”。 | 只做静态路径和专用恢复测试环境验收；真实用户数据删除另行授权。 | 待执行 |
| E-11 | CDP 主题模型目录预检 | 当前 `codex_theme.rs` 的 `launch_codex_with_cdp` 调用 `launch_with_catalog_reload_preflight`；该 helper 先 prepare，再 launch，二者均成功后才调用 complete。`.test-tmp/dot-theme-preflight-20261025/theme-focused-final.log` 记录 5/5；`catalog-reload-focused.log` 记录 7/7；`rust-lib-final.log` 记录 802 passed、0 failed、6 ignored。 | `STATIC` + 本机日志：代码行为和相关定向用例可核对；全量日志存在较早失败尝试，见下表。所有日志均缺可复核的命令行及源码/二进制指纹，属于当前工作区证据，不能绑定发行候选。 | 四类启动路径全矩阵及 USB 测试机验证；检查最终源码是否与这些日志编译时相同。 | 代码已实现；USB 实机待验 |

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

