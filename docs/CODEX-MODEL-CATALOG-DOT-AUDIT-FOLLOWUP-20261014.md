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
- 验证收据：`rustfmt --edition 2021 --check` 对三个修改的 Rust 源文件通过；`git diff --check` 通过；模型目录定向测试 96 passed、0 failed；完整 `cargo test --lib` 708 passed、0 failed、6 ignored。第一次源码构建发现 `remove_file` 闭包生命周期编译错误，两项测试随后暴露断言/注入点问题，均已修正并在完整回归前重跑定向测试。
- 独立只读审计第一轮发现 Add/Reset 跨进程竞态和 USB 可变路径校验后启动的 TOCTOU 窗口；已修复：Add/Reset 全程共用配置命名 mutex，实际运行验签 CLI 的临时副本并再次核对包内 SHA-256。第二轮独立审计进行中；只有第二轮通过后，代码才可视为审计闭环。未构建发行安装包或操作实体 U 盘。
