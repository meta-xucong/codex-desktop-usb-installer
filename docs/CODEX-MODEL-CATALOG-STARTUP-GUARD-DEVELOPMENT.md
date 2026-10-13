# Codex 模型目录写入与启动前完整性保护

## 目标与边界

避免安装器生成或接管的 `codex-usb-native-model-catalog.json` 因 UTF-8 BOM、Codex 版本 schema 不匹配、文件被外部编辑或写入中断而再次阻断 Codex 启动。

- 只保护本安装器管理的模型目录；不改写用户自行配置的外部 `model_catalog_json`。
- 不动测试机或用户当前 Codex 配置，不清理其他备份，不改变许可证、CCSwitch、模型探测和启动兼容流程。
- 所有修复均限于已验证的 USB Codex 数据目录及固定的管理器目录文件。
- “永远避免”按工程可保证的范围定义：安装器每次启动前都对当前管理器目录做本地结构和所有权校验；管理器提交候选前做相同的本地校验；无法安全修复时 fail closed 并说明原因。USB 启动预检不启动 Codex CLI，避免每次启动带来的高资源占用。显式添加模型会在提交前以隔离 `CODEX_HOME` 调用随包 Codex 的 `debug models`，验证运行时确实接受候选目录；这个 gate 必须保留，但必须按 Codex 的实际规范化输出比较，不能把有效候选误判为拒绝。外部程序可在校验之后继续改文件，故不承诺抵御任意进程在检查与启动之间的恶意并发写入。

## 基线证据与原因

审计报告 `catalog-fix-result.md` 记录的直接故障是文件以 `EF BB BF` 开头；将 BOM 去掉后，同一 Codex CLI 的 `debug models` 返回成功并恢复启动。报告也记录更早出现过 catalog schema 字段缺失/类型不符。

当前仓库与报告的归因不同：`model_catalog.rs` 使用 `serde_json::to_vec_pretty` 和原子文件写入，不会生成 UTF-8 BOM。启动预检会在本地验证受管文件的 identity、JSON 结构、模型 ID 安全性和唯一性，并在可能时从有效受管备份恢复；显式 Add 通过 `model_native_probe.rs::validate_final_catalog` 调用隔离的随包 Codex CLI。2026-10-13 追加的根因报告记录了 `native-catalog-unsupported`，但本地复现已进一步证明：同一候选 JSON 被当前随包 Codex CLI 的 `debug models` 成功解析，拒绝发生在管理器对规范化输出的过严比较，而不是 CLI 的 schema parser。

## 固定行为

### 模型目录写入

1. 保留 Rust JSON 序列化与 `atomic_file` 原子替换，不引入 PowerShell 写文件。
2. 写入前对当前目录和候选目录做本地边界校验（大小、管理器 identity、模型键/slug、重复项），随后在独立临时目录运行随包 CLI 的 `debug models`。仅当 CLI 成功且其规范化输出与候选一致时才提交。
3. 只有测试凭证仍有效、USB 账户绑定未变化、配置快照一致且本地结构校验通过，才越过现有 commit 边界并写入 catalog/config；失败时不写，不启用模型。
4. 修正 manager 生成的保守模型条目，使其提供已知必需字段，并省略具有 serde 默认值的非 Option 字段；运行时 CLI 是最终 schema 判定者，不靠手写 schema 取代它。

### 启动前目录预检与修复

只在签名 USB 安装、启动目标已通过现有授权/根目录绑定之后，并在关闭已有 Codex、改写配置、运行兼容补丁或启动进程之前执行：

1. 解析 Codex config.toml，只有精确匹配管理器标记块和预期固定 catalog 路径时才进入自动修复；用户配置中的外部目录保持原样。
2. 对管理器 catalog 做普通文件/大小/UTF-8/JSON/identity/模型项检查。仅对该固定的管理器文件识别并剥离 UTF-8 BOM；规范化后通过本地结构和唯一性校验，再用原子写入保存。若邻接 `.bak` 不是管理器文件，使用保留备份的原子替换方式，绝不删除或覆盖它。
3. 若当前文件本地校验失败，读取 `atomic_file::backup_path(catalog)` 中的上一版本；仅当备份属于管理器且通过同一套本地结构校验，才原子恢复备份。
4. 修复前再次核对 config、catalog 和备份快照；如果管理器配置引用被撤销或任一文件变化，拒绝启动且不写文件。当前目录及备份都不可用时，启动仍 fail closed。用户可在管理器复位：只有配置中的管理器块精确且完整时，复位才移除该引用。复位不会删除或改写 catalog 家族，因为 config 备份可能仍引用相同路径，且外部写入者不受进程内锁约束；解除活动配置引用即可恢复 Codex 原生模型列表。未引用的管理器文件不参与模型状态，用户再次显式添加时从空目录开始，避免恢复旧模型。配置结构不安全或检测到外部目录时保持 fail closed。
5. config 指向非管理器路径、管理器标记损坏/重复、路径类型异常等情况均不自动修改用户文件；按已有 fail-closed 策略明确报错。

### 竞争写入与诊断

- 在管理器进程内使用现有 `model_catalog::FILE_LOCK` 将预检、备份读取、验证结果和必要原子修复串行化。
- CLI 在隔离临时 CODEX_HOME 运行，不加载真实用户配置、MCP、密钥或工作区；仅校验候选 catalog 解析/schema。
- 记录可诊断阶段和 CLI 错误码，不记录 API Key、模型目录全文或用户配置内容。
- 启动前做短时复读/哈希一致性检查；若验证后目标文件发生变化则拒绝本次启动，不使用过时的通过结果。

## 复用代码与执行清单

| 目标 | 复用点 | 位置 |
|---|---|---|
| BOM-free 序列化 | `serde_json::to_vec_pretty` | `src-tauri/src/app/model_catalog.rs::add_tested_model_files` |
| 原子写入及可恢复 `.bak` | `atomic_file::write_atomic`、`atomic_file::backup_path` | `src-tauri/src/app/atomic_file.rs` |
| 管理器 identity、模型 ID 边界 | `catalog_model_ids`、`owned_json`、`regular_or_absent`、`read_optional` | `src-tauri/src/app/model_catalog.rs` |
| 显式 Add 的 CLI gate | `validate_final_catalog`；写临时 catalog/config 并运行 `debug models`，不属于 USB 启动预检 | `src-tauri/src/app/model_native_probe.rs` |
| USB 安装根授权 | `authorize_usb_codex_launch` 和 `usb_guard::authorized_usb_codex_root` | `src-tauri/src/app/win_update.rs`, `usb_guard.rs` |
| 启动位置 | `launch_codex` 在关闭/补丁/启动前的授权段 | `src-tauri/src/app/win_update.rs` |
| 存储边界测试 | 临时目录和路径参数化 catalog 测试 | `src-tauri/src/app/model_catalog.rs` 的测试及 `model_catalog_storage_tests.rs` |

## 允许修改文件

- `docs/CODEX-MODEL-CATALOG-STARTUP-GUARD-DEVELOPMENT.md`
- `src-tauri/src/app/model_catalog.rs`
- `src-tauri/src/app/model_catalog_storage_tests.rs`（仅当测试需独立拆分）
- `src-tauri/src/app/win_update.rs`
- `src-tauri/src/app/model_native_probe.rs`（仅复用校验函数所需的小修）

不得覆盖 `src-tauri/src/commands.rs` 的既有未提交用户改动，不动其他未提交文件，不修改 USB 内容或真实用户 profile。

## 测试与验收

1. 写入候选：通过序列化生成的候选首字节不是 BOM；错误 identity、预存/新增重复 slug、非法 slug、损坏 JSON 或注入的 validator 拒绝时不得提交；确认 catalog 与 config 均未变化且 commit callback 未触发。
2. 启动预检：管理器 catalog 有 BOM、其余结构合法时，只去 BOM、保留模型和已有备份；非管理器 `.bak` 必须字节不变。损坏当前目录时，只恢复本地验证通过的 manager `.bak`；当前和备份都无效时拒绝启动，复位路径仅脱离精确 manager config block 并保留不确定文件；恶意/外部 catalog 和未由 manager 精确标记的 config 不被改写。
3. 竞争变更：模拟预检/复位期间 config 所有权撤销、目标 catalog 内容变化，均拒绝使用过期快照；复位不得删除身份不明的文件。
4. Rust 格式检查、Tauri crate 定向测试和 Windows 目标编译通过。若当前环境没有可运行的随包 `resources/codex.exe`，明确区分单测通过与真实 Codex CLI 集成验证未执行；不得据此声称 USB 发行版已验收。
5. 独立只读审计确认写入边界、恢复来源、启动前调用顺序、无外部配置改写、无敏感日志及已有工作区变更保留。

## D/I/A 与完成状态

- D0：故障和安全边界明确；不新增用户可见配置项。
- I2：写入与启动跨模块一致性、恢复状态、隔离 CLI 验证和并发外部编辑都要覆盖。
- A1：跨模块完整性/恢复逻辑需独立只读审计。
- 开发状态：本次新增了重复模型 ID 写入拒绝、损坏/身份不明文件保留并解除精确 manager config block 的复位逻辑及回归测试。历史测试结果不代表本次变更；当前验收和发行产物状态以最新验收记录为准。
