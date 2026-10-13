# Codex 模型添加失败：根因与修复设计

日期：2026-10-13
状态：设计冻结，按本文实施
适用范围：模型目录 Add 提交前的本地安全校验和随包 Codex CLI 兼容校验

## 1. 目标与边界

修复最新版管理器中“模型测试通过，点击添加后报 `native-catalog-unsupported`，配置文件没有写入”的问题，同时保留候选目录提交前的 Codex 原生解析校验。不能通过取消 gate、忽略解析失败或先写入再观察启动结果来绕过问题。

本轮只修复安装器代码与测试。不得改真实用户 `CODEX_HOME`、Codex 二进制、物理 U 盘或发行包；不请求网关，不做付费生成。Add 必须保持追加语义，Reset 才允许删除由管理器管理的模型。

## 2. 证据与根因

### 2.1 用户报告

`add-model-fails-20261013.md` 记录：测试模型 `claude-haiku-4-5` 后点击“添加到 Codex”，界面返回 `当前 Codex 不接受此模型目录（native-catalog-unsupported）；未写入配置`。用户检查确认目录文件、备份、reload 标记和 config 托管块均未产生，说明失败发生在提交之前。页面恢复为“还没测试”是 Add 请求失败后的票据失效状态，不代表曾经提交成功。

报告进一步推测候选 JSON 仍不符合 schema 或 PowerShell BOM 写入，但这些是推测，不能据此改写管道。

### 2.2 本地复现

以当前发行包中签名 MSIX 的 Codex CLI 为准，将 `claude-haiku-4-5` 生产路径生成的候选 JSON 放入隔离目录，使用临时 `CODEX_HOME`、`HOME`、`USERPROFILE`、APPDATA 和 TEMP 执行：

```text
codex.exe -c "model_catalog_json=<isolated catalog.json>" debug models
```

CLI 返回码为 0，stderr 为空，stdout 返回一个 slug 和候选对应的模型对象。候选可被随包 Codex 接受。CLI 将以下两个输入字段（值为 JSON null）从输出中省略：

- `comp_hash: null`
- `default_reasoning_level: null`

CLI 还会按运行时语义添加默认字段，或由 `base_instructions` 派生 `model_messages`。当前 `catalog_model_entry_matches` 只允许少数默认值，并要求候选的每个字段都必须在 CLI 输出中出现。因此候选虽然通过 CLI schema 解析，管理器仍因上述两个 null 字段未回显而把比较结果判为 false，再折叠成 `CatalogUnsupported` / `native-catalog-unsupported`。

### 2.3 根因结论

**确定根因：最终目录验证器把“候选被 CLI 拒绝”和“CLI 成功规范化候选、但省略了无语义的 null 字段”混为一类。** 模型生成、UTF-8/BOM、账户绑定、测试凭据和网关探测都不是本次最新版本失败的原因。

旧版“能够写入但启动失败”属于同一模型目录提交管道中缺少有效提交前保护的另一类故障表现；没有旧包实际写入的目录样本，本轮不将它断言为同一个具体 schema 缺陷。旧包的风险说明了为什么不能删除当前 CLI gate。

### 2.4 旁证与不纳入范围项

- `add_tested_model_to_catalog` 在原子提交前调用 `validate_final_catalog`；失败发生在 `mark_commit` 和磁盘提交之前，符合用户观察到的“没有写入”。
- 生产候选由 Rust `serde_json` 序列化并经原子写入；当前 Rust 源码没有 PowerShell 写候选目录的通道，故报告里的 PowerShell/BOM 解释不成立于此代码路径。
- 输入目录上限为 1 MiB，而 native probe 当前目录/CLI 输出限制只有 256 KiB。这会让较大的合法目录出现另一类误拒，是相关可靠性缺口；本次把 native probe 的目录限制与主目录上限统一为 1 MiB，并将 CLI stdout 限制设为 2 MiB，保留严格上限。
- 报告提到的能力包安装、WebView 重影、失效 MCP 路径和上游额度风险，不属于模型 Add 提交根因，本轮不修改。

## 3. 修复规则

1. **保留原子提交前 CLI gate。** CLI 非零退出、超时、取消、输出无效、身份/slug 不符或额外未知语义字段时必须拒绝，且不得写入。
2. **仅允许证据确认的两个 null 字段省略。** 当前随包 Codex 输出证实会省略 `comp_hash` 和 `default_reasoning_level`，因此仅这两个键在候选值为 JSON `null` 时可于运行时输出中缺失。其他 null 字段必须被 CLI 回显，未知键即使为 null 也必须拒绝缺失；若未来 Codex 行为改变，先增加同版本真实 CLI 证据与负例测试，再扩展此白名单。
3. **保留现有已知规范化白名单。** `default_reasoning_summary=null → "auto"`、`web_search_tool_type=null → "text"`、`effective_context_window_percent=null → 95`、`base_instructions → model_messages` 和 false-only legacy flags 继续逐项精确校验。
4. **保持对象闭合。** root 只接受 `models`，模型对象只接受候选字段、已知规范化默认字段和明确白名单中的两个可省略 null 字段。slug 集合、条数、唯一性必须完全一致；不能容忍额外或遗漏模型。
5. **统一有限大小边界。** 输入 catalog 最大 1 MiB；`debug models` stdout 上限为 2 MiB，因为格式化 JSON 与派生字段可能使输出大于输入；stderr 继续有限排空，不能无限缓存。任何超限均 fail closed。
6. **错误表述忠实。** CLI 执行失败、CLI 输出不匹配和目录大小超限不得混成“模型 API 不可用”。现有 UI 继续展示稳定错误码；诊断细节只允许写脱敏字段差异和阶段信息，不记录配置全文、密钥或完整目录。
7. **不改变失败后的 UI 测试票据失效策略。** 失败代表当前 Add 未提交，旧测试票据作废可防止重放过期证明；成功状态以已持久化目录读取结果为准。

## 4. 代码执行清单

| 文件 | 现状 | 修改 |
| --- | --- | --- |
| `src-tauri/src/app/model_native_probe.rs` | `catalog_model_entry_matches` 对所有候选字段要求 CLI 回显；null `comp_hash` 和 `default_reasoning_level` 缺失即 false。native catalog/CLI 输出上限均为 256 KiB。 | 仅对实测确认省略的 `comp_hash`、`default_reasoning_level` 两个 null 键允许缺失；其他字段仍严格匹配。增加生产形态 fixture 与 unknown-null 负例。目录 cap 统一到 `model_probe::BODY_LIMIT`（1 MiB），CLI stdout 上限调到 2 MiB。 |
| `src-tauri/src/app/model_catalog.rs` | Add 路径在原子写入前执行该 CLI 验证；用户报告的无写入行为由此 gate 导致。候选生成、owner/binding/proof、CAS、并发锁、原子快照均已存在。 | 保持现有行为与提交保护；只对调用顺序/限额做测试确认，不降低保护、不重建文件通道。若测试发现限制与 1 MiB 不一致，仅修正 probe 限额引用。 |
| `docs/CODEX-MODEL-CATALOG-STARTUP-GUARD-DEVELOPMENT.md` | 曾写“普通 Add 不运行 Codex CLI”，与当前实现不符。 | 校正为“Add 前有隔离 CLI gate；USB 启动预检不启动 CLI”。 |
| `docs/CODEX-MODEL-ADD-FAILURE-ROOT-CAUSE-20261013.md` | 新设计与事件记录。 | 本文为开发执行和验收清单。 |

### 可复用且不得重造的逻辑

- 候选条目生成：`model_catalog.rs::conservative_tested_model_entry`
- Catalog schema/identity/slug/大小/原子提交：`model_catalog.rs::add_tested_model_files_with_validator`
- 所有权、账户绑定、proof、CAS 与 commit fence：同一 Add 调用链
- 隔离目录与子进程约束：`model_native_probe.rs::TemporaryArea`、`win_process::capture`
- Codex 输出等价检查：`catalog_output_matches` / `catalog_model_entry_matches`
- 启动期本地修复与备份恢复：保持既有 `preflight_managed_catalog_at_home`

禁止另写 PowerShell 生成器、第二套候选 schema、UI 假成功状态或绕过 CLI gate 的快捷写入路径。

## 5. 测试与审计验收

### 单元回归

- 使用与 `conservative_tested_model_entry` 一致的生产字段；构造当前 Codex `debug models` 的规范化输出，包含派生 `model_messages`、已知默认值、以及 `comp_hash` 和 `default_reasoning_level` 缺省。必须通过。
- 对 `comp_hash` 和 `default_reasoning_level` 分别验证：CLI 省略或回显 null 通过，回显其他值拒绝；其他 null 字段被省略、未知 null 字段缺失必须拒绝。
- 删除候选中非 null 的关键字段、改变 slug、加入未知字段、增加/删除/重复模型、改变 identity/schema/client_version 均拒绝。
- 验证 false-only legacy flags 和现有归一化规则仍正确；未经白名单的额外运行时字段拒绝。
- 验证输入 1 MiB 和 CLI 输出 2 MiB 上限各自在 limit-1、limit、limit+1 三个边界值的判定，超限失败关闭。
- 复核 Add 是 append-only，Reset 才清 managed models；validate 失败时 marker、catalog、config 和备份都不变化，账户绑定与 commit fence 检查继续生效。

### 集成与构建

- 使用当前随包发行 MSIX 中的 `codex.exe` 和隔离临时 `CODEX_HOME` 重跑生产候选：退出码 0，输出 matcher 接受。
- 执行定向 Rust tests、`cargo fmt --check` 和 Windows Tauri crate 编译。
- 冻结实现差异后由独立只读审计者审查：安全 gate、null 等价范围、错误边界、提交前保护、测试证据及变更范围。审计有缺口则修复并重新审计。
- 本轮不做物理 U 盘、真实网关/API 调用和重新打包；因此完成后可确认代码路径与测试修复，不得声称新发行包已在 USB 真机验收。

## 6. 本轮完成定义

只有当生产候选由随包 CLI 成功解析且规范化比较通过；危险变体全部被拒绝；Add 校验失败不写入、Add 成功保持旧模型并追加新模型；目标测试与 Windows 编译通过；独立审计 PASS 时，代码修复才算完成。发行包与 U 盘实测应作为后续单独验收。
