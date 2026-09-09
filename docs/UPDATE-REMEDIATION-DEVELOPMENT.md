# Codex Payload 更新通道 — 修复版开发文档

> CONTRACT_REV: `v2.0-remediation-r1`
>
> 状态：**实现已落地，主控静态复核完成；独立审计证据不足**。本文是针对 `UPDATE-IMPLEMENTATION-SUMMARY.md` 的问题修复开发契约；代码、测试和文档已按此契约实施，但不表示真实 USB E2E 或 release approval 已完成。本文创建于 2026-09-08，最终结论记录在 `UPDATE-AUDIT.md`。

## 0. 任务记录

### 用户目标

> [$multi-agent-dev](C:\Users\T14S\\.codex\skills\multi-agent-dev\SKILL.md) 按你的思路，做一版修改的开发文档，然后审计确认是否正确

本次交付目标：把上一轮审计发现的问题转成一份可执行、可验收、不会误伤 USB 授权机制的修复开发文档，并按该契约完成代码、自动化测试和主控静态复核；独立只读审计与真实 Windows E2E 作为发布前置门禁保留。

### 目标与非目标

目标：

1. 修复 Codex MSIX 更新通道使用错误 USB 路径的问题。
2. 允许更新 root `manifest.json` 中的 Codex payload 元数据，同时保持 USB 授权锚点稳定且仍能检测不可授权的篡改。
3. 让成功更新后的 USB 包重新通过现有完整性校验、清理和启动链路。
4. 补齐 stale expectation、请求头、失败恢复、重试、配置入口和真实集成验收。

非目标：

1. 本次不实现本机已部署 Codex 的自动更新；USB payload 刷新与 Windows AppX 部署仍是两个独立的用户确认动作。
2. 本次不新增统一 `updates_check_all` / `updates_apply` 产品 API；继续使用现有 `payload_check_codex` / `payload_apply_codex`，避免为只做 Codex 通道而扩大公共 API。
3. 不新增第三方 Cargo/npm 依赖，不改变既有 `UpdateSource::Custom` 语义。
4. 不自动迁移或重签旧版 USB 授权文件；旧版 bundle 必须按兼容策略处理。

### 基线与限制

- 项目根：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`
- 起始仓库没有初始 Git commit，当前大量文件处于基线 `A`/未跟踪状态；本任务不能把普通 `git diff` 当作可靠变更边界。
- 真实 Windows 可移动盘、签名许可证和完整构建产物是最终 E2E 的外部条件；没有它们时只能报告“证据不足”，不能报告通过。
- 本文冻结时的基线测试为 Codex channel 14/14；后续增量契约 `v2.1-code-closure-r1` 已将 Codex channel 扩展到 23/23，并增加 mock transport 的完整 apply 前置校验/失败回滚证据。真实签名 MSIX、实体 USB apply、GitHub 网络和 arm64 资产仍不在代码阶段闭环范围内，详见 `docs/UPDATE-CODE-CLOSURE-DEVELOPMENT.md` 与 `docs/UPDATE-AUDIT.md`。

### 分工与门禁

- 主控/文档唯一写入者：当前 Codex 会话。
- 按需思考：由主控完成当前契约决策；如发现必须改变授权模型或外部发布策略，提交 `DESIGN_QUESTION`，不得由执行者自行放宽。
- 独立审计：文档固定后交给独立只读 Agent；审计者不得修改本文、代码、测试或验收标准。
- `COMPLEXITY_GATE`: `ESCALATE_REQUIRED`。原因：跨 `commands`、更新通道、USB guard、发布校验脚本、持久化元数据和安全授权边界。
- `WRITER_STATUS`: `frozen`（代码与文档版本冻结后不再由写入者修改）。
- `AUDIT_OWNER`: 记录于最终交接消息与 `UPDATE-AUDIT.md`。

## 1. 现状证据与问题编号

以下问题是本修复的范围，不是“顺手优化”：

| 编号 | 现状证据 | 影响 | 优先级 |
|---|---|---|---|
| FIX-001 | `commands.rs` 把 `authorized_usb_codex_root()`（`<bundle>\\apps\\Codex`）作为更新根；`codex.rs` 却按 `<root>\\manifest.json` 和 `<root>\\payload` 工作。真实 release 根目录才有这些文件。 | check 读错路径；apply 可能写入错误目录。 | P0 |
| FIX-002 | apply 修改 root `manifest.json` 和 payload，但不更新 `usb-manifest.json`、根 `SHA256SUMS.txt`，且 `usb_guard` 的 anchor 当前哈希两个 manifest。 | 校验失败；已有 lease/license 的 anchor 改变，后续授权可能失效。 | P0 |
| FIX-003 | GitHub API 使用的 curl 请求没有报告承诺的 `Accept` / `User-Agent`。 | 请求契约不完整，受 GitHub API 策略、限流或代理影响。 | P1 |
| FIX-004 | `codex_payload_repo` 有存储字段但 Settings 没有配置入口。 | 所谓“用户可切换备用源”无法通过产品完成。 | P1 |
| FIX-005 | stale 校验未覆盖 release tag、全部相关 asset identity，也没有在 destructive rename 前重新读取本地当前版本。 | check 与 apply 之间的远端或本地变化可能被错误接受。 | P1 |
| FIX-006 | `payload.size` 使用 GitHub API 的声明值；成功写 root manifest 时 `atomic_file` 还会留下 `.bak`，payload `_backup` 也会成为未列入根 SHA 清单的新文件。 | manifest/全包校验不再反映真实文件树。 | P1 |
| FIX-007 | 多个 metadata 文件和 payload 的替换不是一个事务；失败后重试可能与已有 backup 冲突，部分失败还会遗留 staging。 | 断电、权限错误或网络失败后状态不可预测。 | P1 |
| FIX-008 | `UPDATE-CHANNELS.md`、`UPDATE-TESTING.md` 仍以 `updates_*` 和旧 `UpdateContext` 描述实现，`UPDATE-AUDIT.md` 仍写实现尚不存在。 | 开发、审计和验收依据互相矛盾。 | P2 |
| FIX-009 | `verify-payload.ps1`/`verify-oneclick-usb.ps1` 对当前 x64 产物存在硬编码，且没有明确事务残留文件和 backup 的架构/清单策略。 | arm64 或更新后带 backup 的 bundle 可能无法按契约验收。 | P1 |
| FIX-010 | 新通道默认源仍是历史 `meta-xucong/codex-app-mirror`，但现有安装/下载链路、站点和当前发布源使用 `Wangnov/codex-app-mirror`。 | 用户清空设置后会落到落后或不一致的默认源。 | P1 |

## 2. 修复后的正式契约

### 2.1 USB 路径语义

统一约定 `UpdateContext.bundle_root` 是**被当前 manager 运行实例授权的 USB bundle 根目录**，不是 Codex 提取目录。真实布局：

```text
<bundle_root>/manifest.json
<bundle_root>/usb-manifest.json
<bundle_root>/SHA256SUMS.txt
<bundle_root>/payload/OpenAI.Codex_<arch>.msix
<bundle_root>/apps/Codex/<extracted runtime>
```

要求：

1. `payload_check_codex` 和 `payload_apply_codex` 都从 `usb_guard::authorized_bundle_identity().0` 获得 bundle root。
2. `authorized_usb_codex_root()` 只用于未来的提取运行目录/部署相关动作，不得作为本通道的 manifest/payload 根。
3. `apply` 的第一条可执行语句必须仍是 `usb_guard::require_authorized()?`；随后必须重新观察授权 bundle，并确认它与 context 中的 bundle root 相同。
4. renderer 不得提供路径；不得用 settings 中保存的旧盘符替代当前授权 bundle。
5. 所有 payload、manifest、staging、backup 路径必须通过 bundle root 派生，并拒绝 reparse point / junction / symlink 逃逸。

### 2.2 Manifest 与完整性清单

root `manifest.json` 仍是离线 payload 的权威版本来源，保留并更新：

- `codexDesktopVersion`
- `packageVersion`
- `releaseTag`
- `payload.sha256`
- `payload.size`

更新后必须同步：

1. `usb-manifest.json.codexInstaller.payloadSha256`
2. `usb-manifest.json.codexInstaller.payloadSize`
3. 根 `SHA256SUMS.txt` 中受影响文件（至少 root manifest、usb-manifest、当前 payload；成功状态下还要覆盖实际存在的 backup 和其他非排除文件）。

所有写入都要保留原有无关字段。`payload.size` 必须来自最终本地 MSIX 的 `metadata().len()`；GitHub asset size 只能用于进度和上限预检查，不能作为最终清单事实。

成功状态不得在 bundle 根或 payload 中留下未处理的 `*.bak`、临时文件或未列入 `SHA256SUMS.txt` 的 backup。若保留恢复文件，必须明确加入清单并纳入下一次恢复流程；推荐把事务临时文件放在 `.staging`，成功后完整清除。

### 2.3 USB guard anchor v2：只允许明确的可变字段变化

不能简单地继续哈希两个 manifest 的原始字节，也不能把整个 manifest 从 anchor 中删除。修复采用版本化、allow-list 的 canonical anchor：

1. `usb-manifest.json` 增加/使用明确的 update capability，且必须是已实现的 `schemaVersion == 2`；未知未来版本一律拒绝更新，不得用 `>= 2` 猜测兼容。
2. `usb_guard` 为 v2 bundle 计算 canonical anchor：
   - root `manifest.json` 解析为 JSON，移除且只移除 `/codexDesktopVersion`、`/packageVersion`、`/releaseTag`、`/payload/sha256`、`/payload/size`；
   - `usb-manifest.json` 解析为 JSON，移除且只移除 `/codexInstaller/payloadSha256`、`/codexInstaller/payloadSize`；
   - 其余字段、字段类型、payload 路径、架构、程序布局和产品身份都必须参与哈希；
   - 用确定性的递归 key 排序 JSON 序列化，并加入文件名/长度分隔，避免拼接歧义。
3. `UsbFingerprint`/license/lease 必须带 `anchor_schema`（旧序列化数据缺失时按 v1 解释）。v1 继续按旧 raw anchor 校验；v2 使用上述 canonical anchor。
4. v1 bundle 不得被静默转换成 v2，也不得在没有重新生成/重签 v2 license 的情况下允许 payload apply；`payload_check_codex`/`payload_apply_codex` 在任何远端请求前都必须检查 update capability，v1/未知版本直接返回“需重建/重新授权 USB 包”的明确错误。现有 `usb_guard` 为旧功能保留的 unsigned auto-enroll 不得成为 Codex update 的回退路径；update command 不能因 license 缺失而自动 enroll 或改写 license。
5. 新构建脚本必须生成 v2 bundle 和 v2 license/fingerprint；不能把旧 signed license 的 fingerprint 直接改写。
6. 修改可变字段不能改变 v2 anchor；修改 allow-list 之外的字段必须使授权失败。必须分别有“合法 payload 更新后授权仍在”和“篡改不可变字段后 fail closed”的测试。

这是本任务对原 C4/C8 之外新增的安全契约；实现不得用“更新后重新自动 enroll unsigned license”绕过它。

### 2.4 网络与远端可信边界

check 必须无文件写入、无 payload 下载、无本机部署。apply 必须：

1. 读取当前持久化 settings，空 repo 使用与现有安装链路一致的 `Wangnov/codex-app-mirror` 默认源，非空值严格验证为 `owner/repo`。
2. 对 GitHub API 请求显式发送 `Accept: application/vnd.github+json`、稳定的 `User-Agent`，并保持 HTTPS-only、现有代理/超时和响应大小限制。
3. 重新获取 release metadata，不信任前端 URL、repo、asset URL 或版本字符串。
4. 完整比较 checked identity 与 fresh identity：repo、release id、release tag、MSIX asset id/name、checksum asset id/name、release-manifest asset id/name、package moniker、架构、MSIX package version、app version、GitHub digest（存在时）。任一不一致返回 `StaleExpectation`。
5. API 返回的 asset id 相同不代表全部 identity 相同；第二次获得 download URL 时仍必须使用已比较过的 fresh asset 对象，不得只比较 MSIX id。
6. destructive rename 前重新读取 root manifest 的当前 `codexDesktopVersion`、payload path、现有 payload 状态；当前版本必须仍等于 checked `current_version`，否则返回 stale，不得移动文件。

### 2.5 校验链

对 staged MSIX 按以下顺序校验：

1. 目标文件大小不超过 `MAX_PACKAGE_BYTES`，下载完成后读取实际大小。
2. 用 fresh `SHA256SUMS-windows.txt` 按 package moniker/目标文件名解析 SHA256，并计算本地 SHA256。
3. 若存在 GitHub `assets[].digest`，规范化大小写、空白和 `sha256:` 前缀后比较；不允许格式异常时静默跳过，格式异常应作为远端元数据错误或 stale 处理。
4. Windows 上验证 OpenAI Authenticode。
5. 读取 MSIX identity，严格校验 package name、publisher、主包 resource id、架构和 package version。
6. 所有校验失败都删除 staged final file；`.part` 的保留/清理策略必须与文档一致，不能把“可续传”与“失败后清理”写成相反要求。

### 2.6 多文件提交与恢复

payload、root manifest、usb-manifest、SHA256SUMS 必须使用一个可恢复的本地事务。不得依赖“多个 `write_atomic` 连续成功”来宣称整体原子。

推荐实现：

1. 在 `<bundle_root>/.staging/codex-update-<transaction-id>/` 保存 old metadata 快照、next metadata、事务状态和 staged MSIX；transaction id 必须不能来自 renderer。
2. 下载和校验全部成功后，先生成 next root manifest、next usb-manifest，并基于最终计划文件树生成 next `SHA256SUMS.txt`。
3. 在 destructive swap 前验证 next metadata 互相一致，且根校验脚本可以在临时 fixture 上通过。
4. 旧 MSIX 备份名固定为 `OpenAI.Codex_<arch>_<old_package_or_app_version>.msix`，只保留一份；不得依赖一个已存在的同名 backup 让 retry 失败。
5. 记录事务状态后替换 payload，再按固定顺序提交 metadata；每一步失败都能用 old snapshot + backup 恢复到“旧 payload + 旧全部 metadata”，或完成到“新 payload + 新全部 metadata”，不能留下未记录的第三种状态。
6. manager 启动、check 和 apply 入口都要先检测并处理未完成事务：自动恢复只能是确定性的本地恢复，不得借恢复触发远程更新。
7. 成功提交后清除事务目录、atomic writer 产生的根目录 `.bak` 和多余临时文件，再运行全包校验；全包校验失败必须进入恢复流程。
8. 使用进程内 Codex update operation lock，防止两个 apply 并发交叉移动 payload；现有 download lock 不能替代整个 swap/metadata lock。

### 2.7 设置与 UI

1. 保留已有 `codexPayloadRepo` 序列化和 normalize 逻辑；在 Settings 增加 Codex payload repo 输入框，显示格式约束、空值含义和保存结果。
2. 设置 UI 不得把 `UpdateSource::Custom` 的 manager URL 语义改成 GitHub repo；两者仍是不同字段。
3. About 的 Codex 更新入口仅在 Windows + 当前受保护 USB bundle 可用时显示或明确禁用；非 Windows/普通安装环境不能呈现一个必然失败的可用更新按钮。
4. apply 返回 `StaleExpectation` 后清除旧 pending update，要求重新 check；不能让用户重复确认同一个已过期对象。
5. 保留现有手动确认语义。Tauri command 仍是后端最终边界；UI busy 状态不能作为并发或安全控制。
6. 继续发送 `payload-update-progress`；`done` 只在完整提交、清理和最终校验成功后发送。失败应发送明确的失败状态或由前端从 command error 显示失败，不得把半完成状态显示为成功。

## 3. 允许修改的文件与职责边界

以下是实现阶段的最小写入范围；未列出的文件不能因“顺手整理”而修改。

| 文件/模块 | 允许工作 |
|---|---|
| `src-tauri/src/commands.rs` | 传递 authorized bundle root、命令边界校验、Codex operation lock 接入、进度事件保持兼容。 |
| `src-tauri/src/app/update_channel.rs` | `UpdateContext` 的 bundle-root 语义、事务/错误类型所需的内部类型；不新增统一 channel API。 |
| `src-tauri/src/app/channels/codex.rs` | 远端解析、完整 stale 比较、校验链、事务 swap、manifest/usb-manifest/SHA256SUMS 更新和单元/集成测试。 |
| `src-tauri/src/app/usb_guard.rs` | v2 canonical anchor、anchor schema、旧版 fail-closed 兼容策略和对应测试。 |
| `src-tauri/src/app/usb_media_cleanup.rs` | 仅在新事务目录、v2 metadata 或 backup 清理/恢复需要时同步安全清单；不得放宽未知文件保护。 |
| `tools/build-oneclick-usb-layout.ps1`, `tools/build-usb-layout.ps1` | 生成 v2 update capability、v2 anchor 所需的 bundle 元数据和完整 SHA256 清单。 |
| `tools/verify-payload.ps1`, `tools/verify-oneclick-usb.ps1` | 从 manifest 的 `architecture`/payload path 派生目标，不硬编码 x64；支持 v2 动态 payload metadata，明确允许的 backup/事务目录和失败状态策略；成功 bundle 必须严格无遗漏。 |
| `src/app/views/Settings.tsx`, `src/app/views/About.tsx`, `src/services/managerApi.ts` | repo 设置入口、平台/授权状态 UI、stale 清理和进度/错误呈现。 |
| `docs/UPDATE-CHANNELS.md`, `docs/UPDATE-TESTING.md`, `docs/UPDATE-AUDIT.md` | 仅在实现完成后同步为实际 API 和 v2 契约；删除已不存在的统一命令或明确标记为未来设计。 |

唯一写入者在执行阶段仍只能是一个 worker；若拆分代码任务，必须按上述文件集合切成互不重叠的写入集，并先确认旧 worker 已停止且工作区状态可见。

## 4. 验收矩阵

### 必须通过的功能/安全验收

| ID | 场景 | 预期 |
|---|---|---|
| R1 | 使用真实 v2 fixture，bundle 根有 manifest/payload，`apps/Codex` 没有它们 | check 从根目录读版本并发现更新；不访问错误子目录。 |
| R2 | 有效用户确认后 apply | 只替换 USB payload；不调用 Windows AppX 部署；最终 manifest、usb-manifest、SHA256SUMS 与文件一致。 |
| R3 | apply 成功后运行 `verify-oneclick-usb.ps1 -SkipSignature -SkipNetwork` | 通过；无未列入清单的 `.bak`、临时或 backup 文件。 |
| R4 | apply 成功后重新观察 USB guard | v2 anchor 不变，license/lease 仍 authorized。 |
| R5 | 修改 root manifest 的不可变字段、usb-manifest 的不可变字段、payload 路径或架构 | guard 或 preflight fail closed；不发生 payload swap。 |
| R6 | v1 旧 bundle | 明确返回需重建/重新授权；不得修改 payload 或自动重签 license。 |
| R7 | check 与 apply 间修改 repo、release id/tag、任一 asset identity、package moniker、版本或本地当前 manifest | `StaleExpectation`；不发生 rename。 |
| R8 | API response 缺 Accept/UA、digest 格式异常、缺 asset、repo 不存在 | 清晰失败；不下载/不写 USB。 |
| R9 | 断网、取消、超限、SHA mismatch、digest mismatch、Authenticode/identity 失败 | staged final file 清理策略符合契约；旧 bundle 不变；可安全 retry。 |
| R10 | metadata 写入、SHA 清单写入、payload rename 任一步失败，随后 retry | 自动恢复或明确恢复旧完整状态；不会因旧 backup 同名而永久卡死。 |
| R11 | 两个 apply 并发、拔盘/换盘、reparse point | 只有一个操作；授权或路径变化时 fail closed，不写其他卷。 |
| R12 | Settings 修改 repo、清空 repo、非法 repo；非 Windows/普通安装环境打开 About | 设置持久化语义正确；UI 不误导用户执行不适用操作。 |
| R13 | x64 与 arm64 v2 fixture，各自含稳定目标文件和一份 backup | verifier 按 manifest 架构工作，完整 SHA 清单覆盖规定文件；不因 x64 硬编码误判。 |

### 必须执行的命令/证据

实现阶段至少执行并记录：

```text
CARGO_INCREMENTAL=0 cargo test --lib app::channels
CARGO_INCREMENTAL=0 cargo test --lib app::update_channel
CARGO_INCREMENTAL=0 cargo test --lib app::settings_store
CARGO_INCREMENTAL=0 cargo test --lib app::usb_guard
npx tsc --noEmit
npx vitest run src
```

Windows 机器上必须另外执行（当前已在 Windows PowerShell 5.1 的 x64 fixture
上以 `-SkipSignature` 执行通过；正式发布必须去掉该参数）：

```text
powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-oneclick-usb.ps1 -BundlePath <v2-fixture> -SkipSignature -SkipNetwork
```

正式签名包的发布门禁命令为：

```text
powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-oneclick-usb.ps1 -BundlePath <signed-v2-fixture> -SkipNetwork
```

不能把没有物理 USB 的情况包装成 R1/R4/R11 通过；只能记录为证据不足，并保留可复现 fixture 结果。

当前可复现 fixture：
`release/usb-oneclick-v2-fixture-r3`。它证明 x64 构建与 verifier 链路，不能
替代 arm64、物理可移动卷、授权 lease 或真实 apply/故障注入证据。
严格构建与 verifier 还要求 Launcher/Manager 为有效 Authenticode PE；未签名
开发产物只能走显式 `-SkipSignature` 夹具路径。

### 测试覆盖要求

不得只增加解析单测。必须有：

- 完整 apply happy path 的 mock network/download 测试；当前 v2.1 增量契约已覆盖下载、checksum、digest、fake Authenticode、MSIX identity、metadata 提交和 rollback，但不替代真实网络、真实签名与实体 USB；
- 每个 stale 字段至少一条反例；
- v1/v2 anchor、合法 mutable metadata 变化、不可变字段篡改测试；
- metadata/payload/SHA 清单事务失败注入和 retry 测试；
- `About` / `Settings` 的平台、确认、stale 清除和 repo 设置 UI 测试；
- 最终 Windows fixture 的 verifier 通过证据。

## 5. 实施顺序与交接门禁

1. 先实现并测试 v2 bundle/anchor 契约，确认旧 v1 bundle 的 fail-closed 行为。
2. 修正命令到 bundle root 的路径传递；加入 apply operation lock 和 destructive boundary re-observation。
3. 实现完整 fresh identity/stale 校验和 GitHub headers。
4. 实现事务快照、payload backup、四类 metadata 生成、恢复和成功后清理。
5. 更新构建/校验脚本，先在 fixture 上通过 R1-R6，再接 UI。
6. 增加 repo 设置、平台 gating、stale UI 行为和前端测试。
7. 冻结版本，停止唯一写入者，记录文件哈希和所有命令结果，交独立审计。
8. 审计若结论为“不符合”，按 `问题编号 → 修正/补证 → 新版本 → 受影响测试 → 独立复审` 回流；审计者不直接修改。

## 6. 审计记录

本文不把自身 SHA-256 嵌入自身内容，避免形成无法满足的自引用哈希。版本冻结、独立审计 Agent ID、最终哈希和审计结论必须在本次交付交接记录中绑定；未收到独立审计结论前，不得把本文或实现标为 release 通过。

| 字段 | 值 |
|---|---|
| 文档版本 | `v2.0-remediation-r1` |
| 文档 SHA-256 | 由版本冻结交接记录提供，不在本文自嵌 |
| AUDIT_OWNER | 由独立只读 Agent 在版本冻结后提供，记录于交接消息 |
| 审计结论 | 由独立审计消息提供：`符合` / `不符合` / `证据不足` |
| 未决 DESIGN_QUESTION | 当前无；若 v2 anchor 或旧 license 策略改变，必须回流本节和第 2.3 节 |

当前文档只能作为实现输入，不能作为代码已修复、真实 USB 已通过或 release 已批准的证明。
