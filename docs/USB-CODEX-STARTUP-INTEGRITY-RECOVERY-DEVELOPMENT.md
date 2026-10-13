# USB Codex 启动完整性恢复与互斥开发文档

## 状态与基线

- 审计基线：`e02c97981de201aca0432f54914770c961683731`
- 本文目标：修复 USB 版 Codex 启动兼容补丁在中断时留下的 EXE/overlay 不一致；让会执行启动补丁的管理器命令与安装、更新互斥；收紧 Electron PE 资源键匹配。
- 影响范围：仅限 Windows USB Codex 路径及其 Tauri 管理器启动命令。普通非 USB portable 安装不新增恢复记录。
- 本文是实现边界；超出此处列出的行为、文件和测试范围的改动须先更新本文并复审。

## 审计结论复核

三项发现均能在基线代码中复现。

1. `crates/codex-win-engine/src/portable.rs` 的 `refresh_embedded_asar_integrity` 先原子替换 `CodexCore.dat`，再原子更新 `codex-usb-entry-overlay.json`。函数级错误会尝试回滚核心，但进程退出、断电、拔盘或回滚失败可能跳过回滚。`crates/codex-win-engine/src/sys.rs` 在执行启动兼容修复前先调用 `installed_usb_codex_launch_exe`，后者严格拒绝 overlay 与核心哈希不一致。因此中断态会在修复代码运行前阻断启动。
2. `commands.rs::win_launch_codex` 没有取得 `OperationManager` 锁。USB direct-entry IPC 复用该命令，所以 UI 和该 IPC 入口都没有与安装/更新形成后端互斥。只读 `snapshot()` 不能取代 `begin_guard` 的进程内和跨进程锁。
3. `electron_asar_integrity_value_spans` 将 `/` 转换为 `\` 后再匹配 `resources\app.asar`。Electron Windows 运行时对资源键做 ASCII 小写化，但不做等价的斜杠归一化，因此补丁器可能接受运行时不会读取的键。

## 设计约束

1. overlay 校验必须继续 fail closed；不得跳过入口 shim、manifest、别名或核心哈希校验。
2. USB 完整性事务资格通过独立的 `LaunchOptions::usb_integrity_transaction` 表达。管理器调用方必须先取得签名授权，并确认目标为 portable 且其规范化 Windows 路径等于 `SignedUsbAuthorization::codex_root`；不匹配时拒绝启动，不回退启动其他已登记的 portable。该要求同样适用于所有主题热应用、撤销、重启和恢复入口，而且要先于关闭 Codex、写入 config 或向 CDP 写值。事务资格与 `require_usb_entry_overlay` 分开：旧 schema 2 可以恢复已存在的 overlay 事务，但不强制 direct-entry overlay；普通 portable 不读取 USB overlay 内容、不处理或修改 overlay/journal/备份，也不会仅凭文件名进入事务（可能检查 overlay 路径的文件类型/存在性）。
3. 事务恢复在 `installed_usb_codex_launch_exe` 严格哈希门之前执行。无恢复记录的任意哈希不匹配仍然拒绝启动。
4. 恢复记录只含 schema、固定文件名所对应的状态哈希和 PE 补丁使用的 ASAR JSON 头哈希，不含盘符或任意路径。恢复始终操作安装根目录下的固定文件名。
5. 恢复记录本身不是信任根。必须验证普通文件/非 reparse 属性、旧核心备份哈希、旧 overlay 的核心绑定、记录的哈希格式、从旧核心按现有 PE 算法推导出的新核心，以及从旧 overlay 仅改写 `originalSha256` 推导出的新 overlay。最终仍运行现有严格 USB overlay 校验。
6. 写入失败或无法识别的状态不得启动 Codex；必须保留 journal/备份供下次尝试，且错误清楚说明恢复未完成。
7. 操作锁范围是当前管理器 `OperationManager` 的共享 per-user 锁，覆盖同一用户数据目录中的窗口及 IPC 管理器实例；不承诺跨 Windows 用户或不同 user-data/smoke profile 的锁互斥。

## USB PE/overlay 事务协议

### 固定文件

在 USB Codex 安装根目录（`apps/Codex`）使用三个固定的 product-owned 文件：

- `codex-usb-integrity-recovery.json`：事务 journal。
- `CodexCore.dat.codex-usb-recovery.bak`：本次更新前核心的原始字节。
- `codex-usb-entry-overlay.json.codex-usb-recovery.bak`：本次更新前 overlay 的原始字节。

这些文件位于 USB bundle 已定义的 `apps/Codex` 可变运行目录，不改变 bundle 静态文件校验 allowlist。安装/修复开始新事务前，如果 journal 不存在但存在上述孤立备份，只能在它们是普通非 reparse 文件时删除；异常类型必须拒绝修改。journal 存在时绝不清理证据。

Journal schema v1 字段：

- `schemaVersion`
- `oldCoreSha256`、`newCoreSha256`
- `oldOverlaySha256`、`newOverlaySha256`
- `asarHeaderSha256`

所有哈希为 64 位十六进制。schema 不接受额外路径或文件名字段。

### 正常更新顺序

1. 读取并验证当前 USB overlay/入口状态；确认 `originalSha256` 与当前 `CodexCore.dat` 相符。读取 ASAR JSON 头并计算哈希。
2. 在内存中从当前核心推导新核心，并从原始 overlay 字节仅替换唯一的 `originalSha256` 值，得到完整的新文件字节。overlay 解析必须拒绝重复的顶层 `originalSha256` 键；重新解析生成结果并确认该值等于新核心哈希。计算四个文件哈希。
3. 以 `create_new` 创建并写入旧核心备份、旧 overlay 备份；对文件执行 `sync_all` 并回读核验哈希。
4. 通过现有原子替换 helper 写入 journal；helper 的临时文件必须 `sync_all`，Windows 替换继续使用 `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`。journal 落盘并核验后才允许改核心。
5. 原子替换 `CodexCore.dat`，再原子替换 overlay。
6. 回读两文件并确认分别等于 `newCoreSha256`、`newOverlaySha256`。
7. 先删除 journal，再删除两个备份。若只完成 journal 删除就中断，剩余的孤立备份不会触发恢复；下次正常事务按“孤立备份”规则清理。

任何发生在 journal 落盘后的错误都调用同一恢复函数。恢复成功后只允许落在已验证的旧/旧或新/新状态：部分状态回到旧/旧；已经完整提交的新/新状态保留并完成清理。本次兼容补丁仍返回原错误，避免把失败 I/O 当作成功；下一次启动重新检查状态并继续。若恢复失败，保留证据并拒绝启动。

### 启动前恢复状态机

在管理器拿到 `Launch` 操作锁后，且仅当 `options.usb_integrity_transaction == true` 时，`launch_codex_with_options` 才恢复 USB 事务。随后，`require_usb_entry_overlay == true` 的新 schema 调用严格入口校验；旧 schema 2 在有效 USB 上下文中允许 overlay 缺省，但如 overlay 存在仍执行哈希校验。普通 portable 的可执行文件解析忽略 overlay，启动兼容补丁遇到同名 overlay 时跳过 USB 完整性刷新，不读、删或回滚 USB journal/备份。恢复状态只由磁盘实际文件哈希决定：

| 当前核心 | 当前 overlay | 动作 |
| --- | --- | --- |
| old | old | 未开始修改或已经回滚；清理 journal/备份，继续严格 overlay 校验 |
| new | old | 部分提交；用已验证备份原子恢复 old core，确认 old/old，再清理并继续严格校验 |
| old | new | 部分回滚/异常态；用已验证备份原子恢复 old overlay，确认 old/old，再清理并继续严格校验 |
| new | new | 两个文件均已完成且与推导结果一致；按提交完成清理，再继续严格校验 |
| 其他/哈希不符/文件类型异常 | 任意 | 保留证据并拒绝启动，不尝试猜测或自动改写未知文件 |

`newCoreSha256` 不由 journal 单独背书：读取旧核心备份，验证其旧哈希和旧 overlay 绑定，再使用记录中的 ASAR 头哈希调用现有 `patch_pe_electron_asar_integrity` 推导完整新核心，要求推导结果哈希等于 `newCoreSha256`。`newOverlaySha256` 通过验证旧 overlay 后，仅修改 `originalSha256` 为推导出的新核心哈希计算。恢复后无论 old/old 还是 new/new，都必须经过原有入口 shim、manifest、别名与核心哈希校验；不可绕过。

ASAR 在后续启动可能需要重做兼容修改；恢复到 old/old 只证明 PE 和 overlay 状态成对，不证明 ASAR 当前已和 PE 对齐。正常启动流程随后运行兼容补丁，根据当前 ASAR 头再次更新 PE/overlay。

## 启动操作互斥

1. 在 `OperationKind` 增加 `Launch`，序列化为 `launch`；TS `OperationKind` 联合类型同步增加。它只是互斥租约，不具备下载/暂停/取消语义。
2. `win_launch_codex` 在 USB 授权以外的任何安装根解析、Codex 文件读取或兼容补丁之前取得 `begin_guard(..., Launch)`，并持有到启动命令完成/失败。
3. USB direct-entry IPC 已经调用 `win_launch_codex`，继续复用该同一锁。
4. `codex_theme_try_on_restart`、`codex_theme_apply`、`codex_theme_off(full = true)` 也会重启 Codex并可能运行同一个 Windows portable 启动补丁，因此其后端命令入口改为取得同一 `Launch` 锁并持有至完成。`codex_theme_try_on`、`codex_theme_off(full = false)` 不重启，不在本次扩展范围。
5. 安装、更新、修复、卸载继续使用现有 OperationManager 锁。禁止为启动另建互斥实现或在持有 `Launch` 时再次嵌套获取。
6. `win_launch_codex` 取得锁后不得继续依赖 `snapshot().is_none()` 判断是否可以走主题路径；成功取得锁已经证明没有同一 OperationManager 锁内的竞争操作，应保留原来的主题启动能力。`ensure_theme_may_restart_codex` 需要拆分授权/卸载检查与并发判定：三个主题重启入口在 `begin_guard(Launch)` 前执行授权检查、拿锁后不得再用原 `snapshot().is_some()` 拒绝自己。当前已有的用户侧无关未提交改动不得被覆盖。
7. Launch 操作不允许通过现有取消命令取消。前端不把它误识别为安装/更新下载恢复任务；`RootCrashBoundary` 不将其当作破坏性提交阶段，通用关闭确认语义保持现状。

## PE 资源键匹配

移除路径斜杠替换，只按 ASCII 忽略大小写精确匹配 `resources\\app.asar`。测试接受大小写变化，拒绝正斜杠和混合斜杠。无匹配时沿用明确失败，不把 no-op 当作可启动证明。

## 复用代码清单与允许修改范围

### 复用

- `crates/codex-win-engine/src/portable.rs::replace_file_atomically`：临时文件、`sync_all`、Windows write-through 原子替换；补充专用于创建固定备份的同步写辅助函数。
- `patch_pe_electron_asar_integrity`、`app_asar_header_sha256`、`crate::download::sha256_file`：推导和核验核心/ASAR 哈希。
- `UsbEntryOverlay` 与 `find_json_object_member_value`/`parse_json_string_at`：解析 overlay，并在原始字节中精准改写 `originalSha256` 值以保持其他字节不变。
- `installed_codex_launch_exe_with_usb_gate`：恢复完成后继续使用既有严格验证。
- `OperationManager::begin` / `begin_guard`：复用已有同进程和跨进程文件锁。

### 预计改动文件

- `docs/USB-CODEX-STARTUP-INTEGRITY-RECOVERY-DEVELOPMENT.md`：本设计和验收契约。
- `crates/codex-win-engine/src/portable.rs`：固定 journal/备份名、事务 helper、恢复状态机、路径匹配及纯/故障注入测试。
- `crates/codex-win-engine/src/sys.rs`：`LaunchOptions` 分开表达 USB 事务资格和 overlay 强制策略；恢复先于可执行文件解析，兼容补丁和 locale 补丁沿用明确的 USB 模式。
- `src-tauri/src/app/win_update.rs`、`src-tauri/src/app/codex_theme.rs`：仅从已验证 USB 上下文传入事务资格；schema 策略继续单独控制是否强制 direct-entry overlay。
- `src-tauri/src/app/oplock.rs`：`OperationKind::Launch` 和共享锁回归测试。
- `src-tauri/src/commands.rs`：Win launch 与会重启 Codex 的主题命令持锁；移除自持锁后的过时 snapshot 条件。保留该文件已存在的非本任务用户改动。
- `src/shared/types.ts`：前端 union 增加 `launch`。
- 必要时 `src/app/RootCrashBoundary.tsx` / `src/app/ErrorBoundary.tsx`：仅在新增联合项触发穷尽性或错误语义问题时调整；Launch 继续走通用文案。

不得改动 USB 签名格式、CodexEntryShim、许可证/联网校验、安装包清单、更新源、普通非 USB portable 行为或其他既有未提交 UI 功能。

## 测试与验收

### `codex-win-engine`

- PE 键：大小写变体成功；`resources/app.asar`、混合分隔符拒绝。
- 事务正常提交：old/old -> new/new，文件内容、overlay 哈希和 journal/备份清理正确。
- 中断模拟：journal 已持久化后模拟进程中止并构造 new/old；下一次恢复回到 old/old，再跑兼容修复能够达到 new/new。
- old/new 反向混合态：使用已验证备份恢复 old/old；journal 发布前备份准备失败不得修改核心或 overlay。
- overlay 替换失败 + core 回滚成功：恢复 old/old，清理证据，错误返回；重试成功。
- overlay 替换失败 + core 回滚失败：journal/备份保留；下次恢复 old/old 后可继续，不启动未校验核心。
- new/new：恢复只清理证据，内容不重写。
- journal 删除后、备份清理中断：剩余孤立备份不触发恢复，下一次事务安全清理。
- 恶意/损坏 journal、错误哈希、缺少/被篡改备份、非普通/reparse 文件、未知核心或 overlay 状态：失败关闭并保留证据。
- 无 journal 的 overlay/core 哈希不匹配仍由严格入口验证拒绝。
- 普通 portable 即使出现格式损坏的同名 USB overlay、journal 和备份，完整启动兼容路径也不得解析 overlay、恢复/覆盖核心或清理这些文件。
- USB Codex 根缺失但 provenance 中存在已登记本机 portable 时，普通检测虽可返回该 portable 供状态/规划使用，启动授权必须因其路径不等于签名授权的 `codex_root` 而拒绝；其核心、overlay、journal 和两个备份内容保持不变。主题重启路径使用相同目标绑定。
- USB Codex 根缺失但检测到已登记本机 portable 时，主题热应用、试穿撤销、主题关闭/完整恢复、调试重启和配置文件回退都必须在首个 Codex/CDP/config 副作用前拒绝；注入失败的目标授权 gate 不得执行 side-effect sentinel。主题路径再次执行实际重启时还要复核路径，防止预检后安装位置变化。
- 旧 schema 2 USB bundle 在 `require_usb_entry_overlay=false` 时仍能恢复有效 journal；overlay 可缺省，但存在时仍做哈希验证。
- 正常第二次兼容启动无不必要 PE/overlay 改写。

测试优先使用临时目录和合成 PE/ASAR fixture，故障点通过内部可替换 I/O 操作或显式阶段 helper 注入；禁止真实 USB 或真实安装被测试写入。

### 管理器与互斥

- 锁单测：Launch 持有时另一 `OperationManager` 实例对同一 lock path 无法开始 Install/Update；Install/Update 持有时 Launch 也被拒绝；guard drop 后可以重试；现有取消命令继续拒绝 Launch。
- Windows `commands` crate 编译/测试；确认 USB direct-entry IPC 仍进入同一 `win_launch_codex`。
- theme restart 三个入口持锁，live-only 两个入口不新增嵌套锁。
- `win_launch_codex` 持锁后主题启动分支仍可执行。

### 交付门槛

1. 本文开发文档独立审查通过并冻结后才开始代码修改。
2. 引擎相关测试、oplock 测试、格式检查和 Tauri Rust 编译全部通过。
3. 独立只读代码审查确认状态机 fail-closed、锁覆盖正确、无嵌套死锁且无越界文件/USB 写入。
4. 任何未覆盖或失败的测试必须明确列为未验收；不以既有提交前报告代替本轮重跑。
5. 本任务不构成新的 USB EXE 发行包验收，也不宣称最终 U 盘冷启动/真实盘符验证已经完成。该项需在代码验收后按最终构建另做发布验证。
