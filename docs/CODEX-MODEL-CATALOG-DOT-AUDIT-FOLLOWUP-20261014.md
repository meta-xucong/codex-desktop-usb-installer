# Codex 模型目录 Dot 审计问题闭环设计

日期：2026-10-14
基线：`65176acbd96f8d1256a12e002d64af7833823280`

## 目标与边界

修复 Dot 针对首次添加/恢复默认模型流程报告的 1 项 P1、2 项 P2，并验证其列出的两项兼容性疑问。添加行为必须保留 Codex 原生模型和现有上游模型；只有用户明确选择“恢复默认”才清除上游条目。

本轮只允许修改：

- `src-tauri/src/app/model_catalog.rs`
- `src-tauri/src/app/model_catalog_activation.rs`
- `src-tauri/src/app/model_native_probe.rs`
- 本开发文档及相应模块内的回归测试

保留工作区中与本任务无关的已有改动，不做提交、推送、打包或 USB 写入。

风险分类：D0（触发条件与预期行为明确）；I2（Codex 可执行文件信任、跨文件恢复顺序和故障恢复相互影响）；A2（会执行可变介质中的程序，并改写用户模型配置）。

## Dot 发现及设计判断

| 编号 | Dot 发现 | 当前判断 | 修复方向 |
|---|---|---|---|
| P1 | 添加前直接启动可变目录的 `resources/codex.exe`，未做签名 MSIX 到 CLI 内容哈希核验 | 成立。首次添加的 `native_default_catalog` 直接调用 `win_process::capture`。 | 复用 `model_catalog_activation::verified_runtime` 已有的 Authenticode/MSIX/CLI SHA-256 链路；原生目录读取和候选校验都必须先取得绑定当前已签名 USB Bundle 的可信 CLI 路径。校验失败必须在创建进程前终止。 |
| P2 | sidecar 损坏或归属不明会让恢复默认保留目录，后续添加重新启用旧上游模型 | 成立。当前把 catalog 与能力 sidecar 混成一个“不确定文件族”；sidecar 不参与活动模型清单，却会阻止清空 catalog。 | catalog、catalog 备份和能力 sidecar 分开判断。sidecar 只作能力缓存，不能阻止清理已确认归管理器所有的活动目录；不明归属的 catalog 本身仍须保留且不能自动覆盖/重新启用。 |
| P2 | 先清空 catalog/删除备份、再改配置，配置写入失败会留下受管配置引用空目录且无备份 | 成立。 | 改成“写入复位意图 → 原子移除配置引用并回读确认 → 清理确认归管理器所有的目录和备份 → 清除意图”。配置写入失败时不清 catalog、不删备份；任何后续清理失败都保留意图，阻止 Add 重新启用旧目录，并允许用户重试恢复默认。 |
| 兼容性 | `debug models` 是否受机器级配置污染 | 代码已 `env_clear`，并设置临时 `CODEX_HOME`、`HOME`、`USERPROFILE`、`APPDATA`、`LOCALAPPDATA`、`TEMP`、`TMP` 和受限 `PATH`；将以确切随包 CLI 的隔离执行记录复核。 | 保持所有相关 Codex 调用共用同一隔离环境；确认日志只保存无凭据的模型 JSON 和 stderr。 |
| 兼容性 | Codex 原生 `truncation_policy.mode=tokens` 会否被本地校验拒绝 | 已确认会。随包 CLI 的原生输出包含 `tokens/10000`；`normalize_legacy_model_entries` 当前只接受 `bytes`。 | 仅允许 `bytes` 和 `tokens` 两种已验证模式，且两者都要求正整数 limit；其他模式或字段类型仍 fail closed。对真实输出形态增加回归夹具。 |

## 修改与恢复约束

### 添加时的可执行文件信任

1. 不以 `authorized_usb_codex_root()/resources/codex.exe` 的路径存在作为可信证明。
2. 将既有 `verified_runtime` 拆出可复用的运行时核验入口：重新确认当前签名 Bundle/授权、安装根目录与当前 USB 一致，核验随包离线 MSIX 哈希及 Authenticode，然后比较 MSIX 内 `app/resources/codex.exe` 与本地 CLI 的 SHA-256。
3. `native_default_catalog` 和 `validate_final_catalog` 不再接收任意可执行文件路径；两者都先取得可信路径，再把二进制复制到本次唯一临时目录并在复制时重新核验 SHA-256，实际运行临时副本，不再从 USB 可变路径启动。
4. 验证失败必须发生在 `Command::new`/`win_process::capture` 之前；不执行不匹配的 EXE，也不写用户 catalog/config。
5. 添加流程的取消/时限仍有效；包校验增加的耗时必须包含在管理操作预算内，不能无限等待。

### 恢复默认事务

1. 复位意图文件 `codex-usb-native-model-catalog.reset-pending` 放在当前 `CODEX_HOME`，内容固定、可验证身份，不包含模型/凭据。文件或其 `.bak` 若损坏或归属不明，则 Add fail closed，Reset 显示需人工处理，不能删除/覆盖未知文件。
2. 写意图后，先原子移除确切管理器配置块并确认回读；失败时保留 catalog 与备份，Add 看到意图后拒绝执行。
3. 只清理身份已确认的 manager catalog 和 manager backup。catalog 主文件或备份不属于管理器时保留它们并保留复位意图；配置已回到原生模型，后续 Add 必须阻断，不能把旧清单重新挂回去。重试 Reset 可以在未知文件被妥善处理后完成。
4. sidecar 损坏或未知归属不得阻止可信 catalog 的清空。sidecar 不是模型权威来源，本轮不以它恢复模型列表。
5. 全部确认清理完成后，最后删除复位意图；若删除失败，Add 仍 fail closed，用户可再次执行恢复默认以收尾。
6. Add 与 Reset 从配置快照到提交全程共同持有 `Local\\CodexUsbModelSelection` 命名互斥锁。锁有界等待；未取得锁时不改文件，提示稍后重试。保留模块内锁作为同进程保护。
7. 在写入配置前再次比对 config/catalog/backup 快照。不要在任一操作失败后误报复位成功。

## 回归与验收

- **P1 负例：**候选 EXE 与签名 MSIX 内 CLI 哈希不一致时，记录验证失败；断言 CLI 启动回调未执行，catalog/config 字节不变。
- **P1 正例：**确切随包 MSIX 的 CLI 哈希相同，可信运行时核验成功后才执行 `debug models` 和候选目录校验。
- **P2 sidecar：**构造有效 manager catalog + managed config + 损坏/foreign sidecar；执行恢复默认后 config 不再引用 catalog、manager catalog 为空、sidecar 原字节保留；随后 Add 从 Codex 默认目录重新播种，旧 upstream 行不复活。
- **P2 配置失败：**通过注入的配置 writer 让 config 提交失败；断言 config、catalog、backup 保持提交前字节，复位意图存在，后续 Add fail closed。确认再次 Reset 成功可完成清理并删除意图。
- **P2 清理失败：**配置 detach 成功而 catalog/backup 清理失败；断言配置已回到原生模型、意图保留、Add 不能重新启用旧目录；重试恢复默认可闭环。
- **tokens 兼容：**原生 entries 的 `tokens/10000` 在添加其他模型后仍保留；`bytes/正整数` 与 `tokens/正整数` 接受；未知 mode、零/负/非整数 limit 拒绝且不写文件。
- **机器配置隔离：**使用确切随包 CLI，在临时 profile 环境运行 `debug models`；确认输出仅反映临时 home 中的目录，不读取调用用户的 `.codex/config.toml`，并记录完整 CLI 哈希等于签名 MSIX 对应 entry 哈希。
- 完成格式检查、模型目录定向 Rust 测试及现有全量 Rust 测试；独立审计针对固定最终版本和测试收据做只读复核。实体 U 盘/发行包未实际执行前，不宣称 USB 实测验收。

## 本地实施记录（2026-10-14）

- 已在 `model_catalog_activation.rs` 抽出共用运行时核验：模型目录使用当前授权 USB 上的 Codex 根目录，并校验签名离线 MSIX、发布者 Authenticode 与包内 CLI 哈希；原生目录采集和候选验证不能再传入可变路径。
- 已加入“核验失败则运行闭包不执行”的单元测试；原生采集与候选验证都走同一门控。
- 按独立审计发现补强 TOCTOU：实际执行的是复制到本次唯一临时目录的 CLI 副本，复制过程必须匹配已验签 MSIX 内 CLI SHA-256；新增源文件被替换时复制失败且 runner 不运行的测试。
- 按独立审计发现补强 Add/Reset 并发：二者现在从快照读取到最终提交/复位完成都共同持有已有的 `Local\\CodexUsbModelSelection` 命名互斥锁。长校验不再可能越过复位完成后重新提交旧快照；互斥锁 10 秒超时则无文件改动并要求重试。
- 已将恢复默认改为持久化恢复意图、先原子解除配置、再清理已确认归属的 catalog/备份、最后清除意图。Add 遇到有效、损坏或归属不明的恢复标记均 fail closed。Sidecar 及其备份保持原字节，不再阻断已确认 catalog 清理。
- 已补两类故障注入测试：配置替换失败时目录与备份不变；配置解除后目录清理失败时保留恢复标记并阻止 Add；两种场景都可由再次 Reset 收尾。
- 已允许原生 Codex 使用的 `truncation_policy.mode` 为 `bytes` 或 `tokens`，并保留正整数 limit 校验；新增原生 `tokens` 首次 Add 保真测试及未知策略反例。
- 机器配置隔离复核：`model_native_probe::configure_command` 先 `env_clear()`，再只提供系统启动必需环境与临时 `CODEX_HOME/HOME/USERPROFILE/APPDATA/LOCALAPPDATA/TEMP/TMP`；本地 API Key 仅在显式原生能力验证流程传入，不用于 `debug models`。因此未发现由机器级 Codex 配置污染首次目录读取的代码路径。
- 独立只读审计第二轮确认 Add/Reset 命名 mutex 及首次目录采集/候选校验的临时副本路径已关闭，但指出能力探测 `probe_windows` 仍直接启动原始 USB CLI，尚未绑定签名 MSIX 中的哈希。
- 已将验签 CLI 的预期 SHA-256 从 `collect` 传入 `NativeProbeInput`。能力探测先将 USB CLI 复制到唯一临时目录，复制时逐字节核验 MSIX 中的签名哈希；哈希不符即删除副本并终止。`--version`、`debug models` 和 app-server Session 均只使用该临时副本，结束时再复核副本哈希。新增能力探测 runner 必须收到临时副本的正例和源文件哈希不符时不启动的反例。
- 新验证收据：仅对本轮修改的两个 Rust 文件运行 `rustfmt --edition 2021 --check` 通过；`git diff --check` 通过；`model_native_probe` 定向测试 19 passed、0 failed；完整 `cargo test --lib` 715 tests，709 passed、0 failed、6 ignored。项目全量 `cargo fmt --check` 仍会报告其他既有文件 `capability_pack.rs`、`media_generation.rs` 的格式差异；本轮未触碰这些文件，避免扩展范围。
- 第三轮独立只读复审结论 PASS：能力探测所有 `--version`、`debug models` 和 app-server Session 启动都使用同一份按签名 MSIX CLI 哈希核验的临时副本；当前代码路径未发现 USB 原始 CLI 的进程启动绕过。Add/Reset 命名 mutex、sidecar 清理、恢复标记事务和 tokens 兼容均未回退。
- 残余边界：临时副本使用随机临时目录和独占新建，但没有显式专用 ACL；本修复抵御 USB 文件在验签后被替换，不覆盖同一 Windows 用户权限下的恶意进程在副本哈希后、进程启动前再次改写文件。若该威胁进入产品范围，需另行设计受保护临时目录/ACL。尚未通过正式发行包或实体 U 盘冷启动验证随包 Codex CLI；单测和源码审计不代表该项已验收。
- 本轮未构建发行安装包或操作实体 U 盘；完成源码审计不等于正式 USB 包验收。

## Dot 对 544e147 复审后的补充修复（2026-10-14）

- P1（非 Windows 编译）：给引用 Windows-only `TemporaryArea` 的 `verified_probe_executable` 增加 `#[cfg(windows)]`；MSIX entry 提取和纯路径校验 helper 保持跨平台可测。
- P1（PE subsystem 兼容）：USB portable CLI 仅作身份核验；仍要求其完整字节在恢复 `Subsystem` 后与已验签 MSIX 成员哈希一致。能力探测不再复制/执行 USB 上经过 PE 调整的文件，而从本次已验签 payload 中读取精确 `app/.../resources/codex.exe` 成员，写入独占临时文件并核对成员 SHA-256 后执行。MSIX 路径、成员名和摘要都来自 `verified_runtime`；目录穿越、空文件、超大文件、截断或摘要不符均在启动前失败并清理副本。`NativeProbeResult` 继续绑定经核验的 USB 文件摘要，以便提交前重新确认安装身份。
- P2（复位重试）：确认恢复标记仍存在时，后续 Reset 即使配置已解除，也会再次写入 reload marker；只有 marker 成功后才清除恢复标记。测试断言重试仍返回 `applied_restart_pending`。
- P2（启动重载进程检测）：启动前除 `Codex.exe` 外还识别安装根目录内的 `CodexCore.dat` 与旧名 `ChatGPT.exe`；进程快照名只作筛选，并检查真实进程路径及叶名称。Windows 子进程回归覆盖两个别名。
- 验证收据：`model_native_probe` 定向测试 **20 passed**；`model_catalog` 定向测试 **98 passed**；Windows 进程别名回归 **1 passed**；完整 `cargo test --lib` **713 passed、0 failed、6 ignored**；`codex-win-engine` 全量测试 **236 passed、0 failed、2 ignored**。修改 Rust 文件的 rustfmt 和 `git diff --check` 通过。
- 本机工具链为 `x86_64-pc-windows-msvc`；实际 Windows 编译和测试已运行。未运行非 Windows 目标交叉编译；`TemporaryArea` 引用函数已明确限定为 Windows。未取得随包 MSIX/最终发行包，未做正式 USB 冷启动，因此发行包验收仍待做。
- 同用户权限下恶意进程可能改写哈希后临时副本的威胁不纳入本次修复范围；此限制已披露。
- 第四轮独立只读复审结论 PASS：确认上述四项均闭环，并复核 Add/Reset 锁、sidecar 与 tokens 兼容没有回退。审计方未编辑文件、未重跑测试；其源码结论与本地测试收据相符。

## 7ed42cd 后续复审补充（2026-10-14）

- 新发现 P1：能力验证不再在 Authenticode 通过后从可变 U 盘路径派生 CLI 摘要。暂存整包哈希拒绝预检后被替换的 U 盘源；随后仍需固定本地暂存文件，避免本地暂存路径在哈希、Authenticode 和 ZIP 成员读取之间变化（见下方 439098c 复审补强）。
- 新发现 P2：安装目录 CLI 的 raw SHA-256 和 PE Subsystem 3→2 归一化 SHA-256 现在由同一打开文件的一次顺序字节流产生。实现先缓存并解析有界 PE 头前缀，然后对同一前缀与后续流同时计算 raw/normalized 摘要；验收只返回这次被归一化摘要认证的 raw 摘要，不再混用两轮路径读取。
- 新增可控回归：源包在预检 hash 后替换会导致暂存摘要校验失败；已暂存快照不受之后 USB 源包替换影响；MSIX 成员提取仍只接受快照摘要；分段变化输入验证 raw/normalized 两个结果都来自同一实际读取流。所有用例使用合成字节/ZIP fixture，不运行测试二进制。
- 验证收据：4 条新增替换/摘要回归分别通过；完整 `src-tauri --lib` **717 passed、0 failed、6 ignored**；`codex-win-engine --lib` **236 passed、0 failed、2 ignored**；三个变更 Rust 文件的 `rustfmt --check` 与 `git diff --check` 通过。测试使用合成 ZIP/PE fixture，不执行 Codex 或其他不可信二进制。
- 本段测试收据对应提交 `439098c`；其后独立 A2 复审发现暂存 MSIX 文件可在按路径验签与成员读取之间被替换，已在下文补强。当前只安装 `x86_64-pc-windows-msvc` target，非 Windows 交叉编译未运行；最终发行包、随包 MSIX 和实体 U 盘冷启动仍是独立验收项。

## 439098c 独立复审后的暂存快照补强（2026-10-14）

- A2 复审确认 U 盘源替换已被 staging SHA-256 拒绝，但指出本地 TEMP 中的暂存 MSIX 仍会被按路径多次重新打开；哈希、Authenticode、Appx 元数据、成员摘要和提取尚未锁定到同一文件对象。该结论成立，不能以“U 盘源已复制”替代对暂存对象的固定。
- 修复后，模型能力验证先完成本地暂存，再以 Windows `FILE_SHARE_READ` 打开暂存 MSIX：允许 Authenticode/ZIP 只读打开，拒绝后续写入、删除或替换句柄；从固定文件句柄重新计算整包 SHA-256，执行路径型 Authenticode 检查，再从同一句柄复算 SHA-256。任一阶段无法取得锁、摘要不符或验签失败都 fail closed。
- 固定句柄由 `VerifiedRuntime` 持有，并克隆传入每个 `NativeProbeInput`，直到该次 CLI 成员提取结束；Catalog 探测的路径也在 `verified_catalog_runtime` 返回对象存活期间完成提取。因此入口元数据、CLI 成员摘要和最终提取路径在文件句柄存活期间只能读取被锁定的 staged 文件；不会从 U 盘原始路径重新派生摘要。
- 新增 Windows 合成文件回归：Authenticode 回调期间的写入和替换失败；将固定句柄克隆给探测任务后释放原句柄，替换仍失败；释放最后一个句柄后替换可成功。测试不调用 Authenticode、Codex CLI 或真实 Provider。
- 验证收据：新增暂存句柄锁回归 **1 passed**；完整 Tauri `cargo test --lib` **718 passed、0 failed、6 ignored**（当前最终测试代码重跑后更新收据）；本轮修改 Rust 文件的 rustfmt 与 `git diff --check` 通过。既有 `codex-win-engine` **236 passed、0 failed、2 ignored** 代码未变。
- 限制：未构建发行安装器、未取得正式 USB MSIX 或实体 U 盘，未进行冷启动验收；本轮验证的是 Windows 文件共享锁行为与合成包路径。当前工具链仍只安装 `x86_64-pc-windows-msvc`，非 Windows 交叉编译未运行。
- 固定提交 `176b03def5097b94196c21e1e926d1af08f25e59` 的独立 A2 代码复审结论：**PASS**（P1/P2 闭环，既有四项/Add-Reset/sidecar/tokens 未见回退）。审计员未编辑代码、未独立重跑测试；审计证据与本地测试收据分开记录。新增 pin 测试没有覆盖 pin 前已有 writer 或父目录替换，也未实测真实 Authenticode/Codex；源码审查未发现绕过路径。正式包、MSIX 与实体 U 盘冷启动仍为独立发布验收项。
