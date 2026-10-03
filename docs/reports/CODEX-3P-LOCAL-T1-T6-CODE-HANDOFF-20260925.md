# Codex U 盘本地 T1–T6：开发交接

项目：`D:\AI\AGENT_U盘\codex-desktop-usb-installer`。
范围：仅本地 T1–T6 的代码、测试源码、资源生成链及独立静态审计。
不包含 T0/Sub2API/VPS、官方 Codex 文件改动、实际构建安装包或真实模型联调。

## 开发结论

本地 T1–T6 的源码修改、配套测试源码、集成资源和独立静态审查已完成，可以进入用户安排的编译及本地测试阶段。
最终独立复审：STATIC_VERDICT=conforming，原文明确确认唯一剩余源码问题关闭。当前没有未关闭的已发现源码阻断项；这不是运行验收通过。
编译、类型检查、运行测试、安装/启动回归、打包和真实模型验证均由用户安排，当前没有运行，退出码 N/A。
不能据本源码交付宣称 GLM、DeepSeek、Claude 已完成最终兼容。

## 已完成的本地实现

T1：模型发现与能力证据分离；sidecar 不可恢复认证；绑定、版本、时效、重复模型 ID 和证据均有保守门禁。
T2：安装后的发现与探测、逐模型状态、取消/重试、失败恢复和显式原生验证入口；原生协议结构、审核者、拒绝响应及随机挑战校验。
T3：删除自动固定 GPT/推理默认值；覆盖普通及 Universal Codex 配置入口；配置保存保护、绑定上下文校验、验证后选择，以及应用未确认状态。
T4：仅元数据诊断、稳定错误码与标识脱敏，不输出完整请求/响应或明文凭据。未知 adapter 版本不编造；真实 T0 对接仍需运行验证。
T5：本地 mock、Rust/TypeScript/PowerShell/Python 测试源码；保留独立存储/激活测试作者证据；RPC 缺口已在现有父模块中补四组反例，没有遗留必须创建的额外 RPC 文件。
T6：源码集成、生成资源一致性入口、52 文件 SHA256 清单与固定审计快照。当前不是新安装包。

## 版本与基线

主仓库 HEAD：e9514d08c343f594b0e452617bdf4a45bc0216b0。
CCSwitch HEAD：1d44b1ba41987cddeb8096fadba8190fab465cbb。
两者未提交、未推送；工作区包含 17 个新增文件和 35 个修改文件。
每个任务的起始/最终逐文件 SHA256 和文件映射在 source-manifest.json 与 task-file-handoff.json。
本轮唯一写入负责人：当前会话；WRITER_STATUS=INACTIVE_SOURCE_FROZEN_CODE_ONLY。
必要的公共约定沿用 DQ-LOCAL-002 至 013；新增 DQ-LOCAL-014 仅收紧有限 RPC 成功结果的一致性，不增加公共接口或上游语义。

## 交给测试的关键事项

必须用这次源码同时重建 Manager 和 CCSwitch；旧发行包未替换，不能用旧二进制判定新链路。
必须包含 17 个未跟踪新增模块/测试；只复制已跟踪文件 git diff 会漏文件。
不要把审计快照整个覆盖回项目，当前工作区才是开发成果；快照用来复核版本。
真实网关测试仅在用户确认 T0 就绪并授权真实验证后进行；mock 通过不是最终兼容证据。

## 本地验证命令（均未运行）

项目根目录：
```powershell
npm run check
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib app::model_
npx --no-install vitest run src/app/views/OfflineHome.test.tsx src/app/views/nativeModelStatus.test.ts src/services/managerApi.test.ts
```
CCSwitch/CCSwitch：
```powershell
pnpm run typecheck
pnpm exec vitest run tests/utils/codexTemplates.compatibility.test.ts
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib usb_preparation_tests
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib usb_model_selection::tests
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib universal_
```
回到项目根目录：
```powershell
python installer-pack/test-adapter-contracts.py ModelCatalogSyncContractTests
powershell -NoProfile -File tools/test-model-catalog-sync.ps1
powershell -NoProfile -File tools/verify-model-provider-resource.ps1
```
这些是定向检查入口，测试团队仍需安装/启动、取消、断网、拔盘、并发配置编辑、失败补偿和真实模型场景。

## 检查与独立审计证据

证据根目录：`.codex-audit\local-t1-t6-20260925\closure-20260925T020144Z\completion-20260925T040053Z`。

- `source-manifest.json`：52 文件起始/最终 SHA256；清单 SHA256：`e596a81e9fe6f7ad05bfd9ab2661d045481c4782d46b79bac530c7b892c45cf2`。
- `task-file-handoff.json`：T1–T6 全部文件分工映射，无遗漏或重复；每任务测试未运行/退出码 N/A。
- `source-final/`：52 个任务文件的最终源码快照，非完整可独立构建项目或安装包。`MANIFEST.json` SHA256：`3c8be3c81037b323710ecd4878253880aae169f30fcaa93123132e86f0fe688a`。
- `independent-review.md`：主审确认原生四项修正及配置补偿正确，仅留下 RPC 正例夹具问题。
- `independent-fixture-review.md`：同一独立审查者复审该夹具修正，结论 conforming，确认唯一剩余问题关闭。
- `review-closure.json`：审查线程、报告哈希、退出码与范围。
- `prior-audit-binding.json`：已有 Universal Codex 五文件独立审计与最终源码哈希绑定。
- `syntax-initial.json` / `syntax-fixture-correction.json`：本轮三文件及最后夹具修正的 Rust 源码解析退出码均 0。
- `reused-syntax-evidence-binding.json`：与最终源码一致的既有解析证据；TypeScript 等未改文件没有重新运行检查。
- `baseline-protection.json`：本轮 1,784 个既有文件（排除两份更新报告）无范围外变化。

独立审查线程：`01a0d6bb-fc34-7b01-9be6-89f0e5f74789`。两次审查进程退出码均 0，仅表示审查任务正常完成，不是产品测试退出码。
本轮生产/测试源码只继续修改 `src-tauri/src/app/model_selection_client.rs`，包括成功响应一致性及四组纯解码反例；最后复审修正仅涉及 cfg(test) 夹具。
原生文件与配置补偿文件未在本轮再修改，只补齐最后独立审查。
没有运行编译、类型检查、测试、程序、安装包或真实网关，也没有更改 Sub2API/VPS、官方 Codex、现有发行包、Git 提交/推送或外部通知。


## T1–T6 文件及验收状态

| 任务 | 文件数 | WRITER_STATUS | DESIGN_QUESTION | 代码阶段 | 运行验收 |
| --- | ---: | --- | --- | --- | --- |
| T1 | 2 | STOPPED_SOURCE_DELIVERED | DQ-LOCAL-002/007 | 已完成，静态审查闭环 | 未运行，N/A |
| T2 | 13 | STOPPED_SOURCE_DELIVERED | DQ-LOCAL-002/003/005/009/010/011/013 | 已完成，静态审查闭环 | 未运行，N/A |
| T3 | 17 | STOPPED_SOURCE_DELIVERED | DQ-LOCAL-004/006/008/009/011/012/014 | 已完成，静态审查闭环 | 未运行，N/A |
| T4 | 1 | STOPPED_SOURCE_DELIVERED | DQ-LOCAL-002 | 已完成，静态审查闭环 | 未运行，N/A |
| T5 | 9 | STOPPED_SOURCE_DELIVERED | 沿用对应产品契约；本轮夹具无新增语义 | 已完成，静态审查闭环 | 未运行，N/A |
| T6 | 10 | STOPPED_SOURCE_DELIVERED | code-only-r1 | 已完成，静态审查闭环 | 未运行，N/A |

所有任务的起始/最终文件基线、完整修改文件清单、测试命令及未验证项分别见上述 JSON 清单与本交接说明。

## 完整修改文件清单

### T1

```text
src-tauri/src/app/model_catalog.rs
src-tauri/src/app/model_capability.rs
```

### T2

```text
src-tauri/src/app/model_probe.rs
src-tauri/src/app/model_native_probe.rs
src-tauri/src/commands.rs
src-tauri/src/lib.rs
src/shared/types.ts
src/services/managerApi.ts
src/app/views/OfflineHome.tsx
src/app/views/NativeModelStatusCard.tsx
src/app/views/nativeModelStatus.ts
src/app/i18n.tsx
src-tauri/Cargo.toml
src-tauri/src/app/mod.rs
src-tauri/src/app/model_process.rs
```

### T3

```text
src-tauri/resources/ccswitch/providers.json
src-tauri/src/app/ccswitch.rs
src-tauri/src/app/model_catalog_activation.rs
src-tauri/src/app/model_selection_client.rs
CCSwitch/CCSwitch/src-tauri/src/usb_model_selection.rs
CCSwitch/CCSwitch/src-tauri/src/database/dao/providers.rs
CCSwitch/CCSwitch/src-tauri/src/services/provider/mod.rs
CCSwitch/CCSwitch/src-tauri/src/provider.rs
CCSwitch/CCSwitch/src/config/codexTemplates.ts
CCSwitch/CCSwitch/src/config/codexProviderPresets.ts
CCSwitch/CCSwitch/src/config/universalProviderPresets.ts
CCSwitch/CCSwitch/src/components/universal/UniversalProviderFormModal.tsx
installer-pack/sync-model-catalog.ps1
CCSwitch/CCSwitch/src-tauri/src/codex_config.rs
CCSwitch/CCSwitch/src-tauri/src/lib.rs
CCSwitch/CCSwitch/src-tauri/src/main.rs
CCSwitch/CCSwitch/src-tauri/src/services/provider/live.rs
```

### T4

```text
src-tauri/src/app/model_diagnostics.rs
```

### T5

```text
src-tauri/src/app/model_probe_fixture.rs
src-tauri/src/app/model_catalog_storage_tests.rs
src-tauri/src/app/model_catalog_activation_tests.rs
src/app/views/OfflineHome.test.tsx
src/app/views/nativeModelStatus.test.ts
src/services/managerApi.test.ts
CCSwitch/CCSwitch/tests/utils/codexTemplates.compatibility.test.ts
tools/test-model-catalog-sync.ps1
installer-pack/test-adapter-contracts.py
```

### T6

```text
tools/build-ccswitch-with-env-patch.ps1
tools/build-oneclick-usb-layout.ps1
tools/build-usb-layout.ps1
tools/verify-model-provider-resource.ps1
src-tauri/resources/ccswitch/verify-product.ps1
installer-pack/audit-codex-compatibility.py
installer-pack/pack.json
installer-pack/file-manifest.json
installer-pack/checksums.sha256
installer-pack/README.md
```


## 本地验收反馈修复（2026-09-25，最新补充）

本轮实际运行指定验收，取代上文这些命令尚未执行的历史状态；没有修改T0或DQ-LOCAL-005。

代码变更共6文件：`src-tauri/src/app/model_native_probe.rs`、`src/app/views/nativeModelStatus.test.ts`、`vite.config.ts`、`tools/test-model-catalog-sync.ps1`、`tools/verify-model-provider-resource.ps1`、`src-tauri/src/app/model_probe_fixture.rs`。原五项修复之外，仅补齐本地test-only accepted socket的阻塞模式，避免Windows10035/WouldBlock。

验收：npm run check退出0；指定Rust命令连续两次63/63退出0；指定Vitest仅3个正式套件83/83退出0；两个PowerShell脚本退出0；显式SourcePath/ArtifactPath兼容验证退出0。保留既有unused run_id警告，新增warning为0。

两轮独立只读复审均为 `STATIC_VERDICT: conforming`，进程退出码均0。原五项修复逐项通过；补充审查确认test-only socket修正、六个最终文件哈希及全部验收日志，无具体剩余缺陷。独立审查线程：`01a0d6f8-ad2a-7aa3-865b-c1700d19edce`。

旧52文件清单不覆盖，新清单保留52条并额外登记vite.config.ts，共覆盖53文件。新source-manifest.json SHA256：`6b6751edfc3cbf6d3efdbdeb39c0769ef1933262e2a8c4716cbf063b3d42fa29`。
证据及详细修复报告：`.codex-audit/local-t1-t6-five-fixes-20260925T050704Z/repair-report.md`。全部文件hash、命令日志、失败历史及独立原文保存在同目录。

WRITER_STATUS=STOPPED_SOURCE_DELIVERED。本地修复已验收，不代表物理U盘和真实模型最终兼容。继续等待T0；上游Responses字段/事件/错误语义变化后重审受影响代码和测试，不沿用旧结论。
