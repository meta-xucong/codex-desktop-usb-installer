# Learnings

## [LRN-20260906-005] mutable-product-metadata-needs-a-separate-trust-model

**Logged**: 2026-09-06T15:02:00+08:00
**Priority**: high
**Status**: superseded by G28-R3 target-desktop binding correction
**Category**: best_practice
**Area**: uninstall-transaction
**Source**: user-report
**Pattern-Key**: harden.mutable_release_metadata

### Summary
Runtime metadata created beside an immutable offline distribution must not be classified either as an immutable checksum entry or as an unconditional filename exception.

### Evidence
- All 28 shipped files matched `SHA256SUMS.txt`, yet the manager-created `usb-license.json` was the sole unknown file and blocked source-media finalization.
- Moving the license to one machine would break the physical-USB identity contract.
- A filename-only allowlist would let malformed or unrelated same-name data enter a destructive cleanup scope.

### Practice
Keep immutable release files under exact digest control. Model necessary mutable files as typed product metadata with bounded size, strict schema/product checks, stable device identity and configured signature validation. Carry that identity and the mutable bytes across detached cleanup handoff, then delete the metadata last. Continue to fail closed and enumerate every genuinely unknown path.

### Resolution
Applied in v0.5.24 with cleanup plan schema 3, active-lease fingerprint authorization, license/backup snapshots and final-commit deletion.

### Metadata
- Related Files: src-tauri/src/app/usb_guard.rs, src-tauri/src/app/usb_media_cleanup.rs, src-tauri/src/app/uninstall_cleanup.rs
- Tags: usb, license, uninstall, immutable-manifest, transaction
- See Also: ERR-20260906-009, LRN-20260906-003

---

## [LRN-20260916-G29] stable-update-anchor-must-model-the-update-lifecycle

**Logged**: 2026-09-16
**Priority**: critical
**Status**: resolved
**Category**: security / reliability / release
**Area**: Codex payload update
**Pattern-Key**: codex-update-explicit-anchor-schema-and-slot-model

### Learning

更新不是“替换一个 MSIX 文件”这么简单：它会同步改变版本元数据、两份
清单、SHA256SUMS，并在第一次更新时创建 backup。授权锚点必须绑定“允许变化
的槽位”，而不是绑定这些槽位当前是否为空。否则第一次合法升级就会把自己的
许可证变成无效许可证。

### Practice

- 顶层清单格式版本与密码锚点算法版本分离；新包两侧显式写
  `updateCapability.schemaVersion=3`、`anchorSchema=3`。
- 稳定锚点对 payload、两份 manifest、SHA256SUMS 中的单一 backup 槽位始终
  写固定 `MUTABLE` 标记；未知文件、第二个 backup、重复/穿越路径仍参与拒绝。
- commit 前先对虚拟下一状态计算锚点，commit 后从磁盘重新完整验证；任一
  授权快照变化都回滚，不能靠自动重新 enroll 或重签绕过。
- Auto 源必须区分不可达、非法、过旧和无更新；过旧源不能阻断后续镜像/Relay/
  GitHub 候选。check 与 apply 使用同一 source/release identity。
- 每次发布生成新的版本化 profile，并让管理器、Bundle、签名器和 portable
  vault descriptor 在打包阶段互相校验；不使用机器公钥或 profile 文件名猜测。

### Verification

G29 code gates passed; no physical USB was written. The remaining evidence
boundary is the fresh generated package on a real signed USB and a mainland
network target machine.

### Metadata

- Related Files: crates/codex-usb-license/src/lib.rs,
  src-tauri/src/app/channels/codex.rs, src-tauri/src/app/usb_bundle.rs,
  tools/build-oneclick-usb-layout.ps1, tools/verify-oneclick-usb.ps1,
  docs/UPDATE-ANCHOR-STABLE-CODEX-G29-DEVELOPMENT.md
- Tags: codex-update, usb-anchor, stable-schema, source-fallback, release-gate

---

## [LRN-20260915-PKG-COUNT] PowerShell 发布门禁不能假设管道结果永远是数组

**Logged**: 2026-09-15
**Priority**: high
**Status**: resolved
**Category**: reliability
**Area**: release-packaging
**Source**: packaging run failure
**Pattern-Key**: release.powershell-scalar-pipeline-result

### Summary

PowerShell 在赋值时会把只有一个元素的管道结果自动解包为标量。发布脚本对这类结果直接读取 `.Count`，会让资源已经正确时仍在最后门禁失败。

### Practice

对“是否存在某个属性”的检查使用 `PSObject.Properties["name"]` 和 `$null` 判断，不依赖管道结果的数组形态；修复后必须实际跑过完整发布脚本，而不是只做脚本解析。

### Resolution

已修复 `tools/build-installer.ps1` 的 Skill Hub 来源门禁，并用当前版本完整构建验证通过。该问题不再影响安装器打包。

### Metadata

- Related Files: tools/build-installer.ps1
- Tags: powershell, packaging, skill-hub, regression

---

## [LRN-20260915-G24] update-preflight-must-not-hash-the-same-removable-payload-repeatedly

**Logged**: 2026-09-15
**Category**: performance / reliability / security
**Area**: codex-payload-update
**Pattern-Key**: update.local-integrity-digest-scope

### Practice

Treat update confirmation state and local integrity state separately. Resolve the remote identity
after cheap capability metadata, then perform one complete SHA256SUMS pass and construct the local
snapshot from its verified digest table. That table is valid only inside the current call chain; it
must not survive from check to user confirmation. Keep a fresh final commit-boundary payload hash
and authorization re-observation so the optimization does not weaken TOCTOU protection.

### Resolution

G24 was fixed in `channels/codex.rs` with metadata-only preflight, same-pass digest reuse, final
independent verification, and stage/code logging. The G24 report's token-expiry inference remains
explicitly unproven for current source because `payload_apply_codex` has no destructive-token path.

### Metadata

- Related Files: src-tauri/src/app/channels/codex.rs, src-tauri/src/commands.rs,
  src-tauri/src/errors.rs, docs/UPDATE-CONFIRM-TOKEN-EXPIRY-G24-DEVELOPMENT.md
- Tags: codex, update, removable-media, sha256, snapshot, toctou, observability
- See Also: ERR-20260915-G24

---

## [LRN-20260915-G29] release-profile-is-version-input-not-machine-constant

**Logged**: 2026-09-15
**Priority**: critical
**Status**: in_progress
**Category**: correction / security
**Area**: usb-signing and release
**Source**: user_feedback and repeated G27/G28 signing mismatch
**Pattern-Key**: usb-signing.version-profile-not-machine-key
**Recurrence-Count**: 1
**First-Seen**: 2026-09-15
**Last-Seen**: 2026-09-15

### Summary

生产 USB signing profile 不是固定的机器配置。每个发行版本必须显式携带自己的 profile，管理器、
Bundle 和 signer 必须由同一份 profile 生成；可迁移能力由匹配的加密 portable vault 提供。

### Details

此前把某次构建宿主或交互式桌面的 `keyId + publicKeySha256` 写成仓库默认“canonical production
profile”，导致构建宿主的 DPAPI 密钥与用户实际签名桌面不同步时，反复出现“构建成功但 signer
不能签名”。`keyId` 相同也不能证明是同一把钥匙。profile 只声明当前版本的公共绑定，DPAPI 只
负责在受控环境导出 portable vault；其他电脑不应依赖原电脑的 DPAPI。

### Prevention

- SignedRequired 的 manager、Bundle、signer 和 pair preflight 都必须显式收到当前版本 profile；
  正式流程不能回退到仓库固定文件或当前机器自动生成的 key。
- profile 记录 `releaseVersion/releaseId`，输出目录按版本隔离；所有三方同时比较 keyId、摘要和版本。
- signer 若要跨电脑工作，必须带匹配 vault 及公共描述；没有匹配 vault 时不得把 `-SkipLocalKeyCheck`
  当作“可交付签名器”证据。
- 最终仍需实际 Windows x64 环境完成 sign/read-back/verify；静态构建通过不等于签名环境通过。

### Metadata

- Related Files: docs/USB-SIGNING-VERSIONED-PROFILE-DEVELOPMENT.md, tools/usb-signing-profile.ps1,
  tools/build-installer.ps1, tools/build-oneclick-usb-layout.ps1,
  tools/build-usb-license-signer-gui.ps1, tools/verify-usb-signing-pair.ps1
- Tags: usb, signer, release-profile, portable-vault, dpapi, key-rotation
- See Also: LRN-20260913-G23, LRN-20260914-G25, ERR-20260914-G25

---

## [LRN-20260915-G28] dpapi-context-is-not-release-key-evidence

**Logged**: 2026-09-15
**Priority**: critical
**Status**: in_progress
**Category**: release-engineering / security
**Area**: usb-signing
**Source**: user现场报告
**Pattern-Key**: separate.build-host-from.interactive-signer

### Summary

Windows DPAPI 公钥只能代表执行该诊断的安全上下文，不能自动代表用户将要双击的交互式签名桌面。
本条曾错误地把“受控”实现成永久固定 profile；正确规则是每个版本使用受控、显式传入的版本化
profile。构建宿主只负责把该版本公共绑定编进 Bundle，不能据此临时改变 profile。

### Practice

- 生产公钥只从当前版本显式传入的 release profile 读取；
- 构建宿主诊断结果若与 profile 不同，说明宿主不是签名机，不得改 profile 迎合宿主；
- 在实际签名桌面运行发布 GUI 的 `--diagnose-key`，确认 GUI、Bundle、profile 三方一致；
- 真实 U 盘签名完成 readback/verify 后，才把交付标记为完成；
- 记录执行上下文和 EXE 路径，避免旧目录/旧窗口造成误判。

### Resolution

G28 先按用户真实交互式桌面 `216ae3…` 重新绑定；本次 G29 又把固定 profile 规则改为版本化
profile 输入，并将“构建证明”和“签名机证明”分开记录。

### Metadata

- Related Files: `tools/usb-signing-release-profile.json`,
  `docs/USB-SIGNER-INTERACTIVE-KEY-BINDING-G28-DEVELOPMENT.md`
- Tags: usb, signer, dpapi, interactive-session, release-profile, regression-prevention
- See Also: `ERR-20260915-G28`, `ERR-20260915-G27`

---

## [LRN-20260915-G27] published-gui-must-be-the-key-binding-authority

**Logged**: 2026-09-15
**Priority**: critical
**Status**: resolved
**Category**: reliability / release-engineering
**Area**: usb-signing
**Source**: user-report, GUI startup log, fresh binary diagnostic
**Pattern-Key**: prevent.cli-only-signer-preflight

### Summary

签名发行的权威对象是用户实际双击的 GUI 二进制，不是独立 CLI、旧窗口或历史日志。只有把
“新发布 GUI 诊断、Bundle 声明、管理器编译绑定”三方的 `keyId + publicKeySha256` 固定比较，
才能阻止“构建成功但仍然不能签名”的重复回归。

### Practice

- 发布 GUI 提供无窗口 `--diagnose-key <temporary-json>`，只输出非敏感公钥元数据；
- 发布脚本在复制 EXE 后立即执行该诊断，profile 不匹配就失败；
- 配对预检优先传入 `-SignerGuiPath`，CLI `init-key` 只作辅助；
- 每次交付前关闭旧签名器窗口并从新目录启动唯一实例；
- 真实 U 盘签名前，先完成本地静态 pair preflight，签名后再 readback/verify。

### Resolution

G27 将上述门禁落入 GUI、构建脚本、配对脚本和发布文档；最终状态必须以 release GUI
诊断和完整构建测试记录为准。

另外，Windows GUI 子系统的诊断调用必须使用 `Start-Process -Wait -PassThru`，并同时检查
退出码和 JSON 文件存在；普通 PowerShell `&` 可能在文件落盘前返回，制造假失败。发布脚本
失败时只清理自己新建的空发布目录，避免留下可被误用的半成品。

### Metadata

- Related Files: `tools/build-usb-license-signer-gui.ps1`, `tools/verify-usb-signing-pair.ps1`,
  `tools/usb-license-signer/src/bin/usb-license-signer-gui.rs`,
  `docs/USB-SIGNER-KEY-BINDING-G27-DEVELOPMENT.md`
- Tags: usb, signer, gui, key-binding, release-gate, regression-prevention
- See Also: `ERR-20260915-G27`, `LRN-20260914-G25`

---

## [LRN-20260913-G22] uninstall-finalizer-must-not-self-delete-report-root

**Logged**: 2026-09-13T04:40:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: best_practice
**Area**: windows-uninstall
**Source**: user-report
**Pattern-Key**: harden.uninstall_report_root

### Summary

卸载二阶段不能把最终报告写入自己刚刚删除的 App Manager 数据目录。清理动作可能已经成功，
但报告回写失败会被用户看到为卸载失败，并触发 Windows“位置不可用”。

### Practice

在生成清理计划时就固定独立临时报告根目录，并与所有删除目标做规范化路径交叉检查；二阶段只写
独立根目录，报告必须记录 phase、exitCode、具体失败路径和可重试建议。打开日志只能在产品目录存在
时使用产品目录，否则回退到临时报告目录。静态 U 盘发行包不携带运行期 usb-license.json。

### Resolution

G22 已在 `uninstall_cleanup` 中落地，并加入报告字段、finalizer 状态门禁、路径安全测试和发行包检查。
归档说明见 `docs/reports/codex-v0524-uninstall-stage2-selfref-g22.md`。

---

## [LRN-20260913-MODEL-SKILLHUB] snapshots-need-provenance-and-diagnostics

**Logged**: 2026-09-13T04:40:00+08:00
**Priority**: high
**Status**: resolved
**Category**: correction
**Area**: skill-hub-model-sync
**Source**: user-report
**Pattern-Key**: harden.offline_snapshot_contract

### Summary

能力包或模型快照不能把“默认策略已更新”冒充为上游实时目录已更新；在线失败也不能只显示前端超时。

### Practice

能力包发布必须带可复现的文件清单、LF 规范化哈希、来源树摘要和只读验证入口。Skill Hub 每次 relay/GitHub
尝试记录阶段、来源、重试序号和结果类别，错误细节脱敏后通过 UI 详情和诊断复制入口提供。模型同步只发布
脱敏元数据，保留活动第三方模型，明确迁移停用官方模型，并记录最终 MSIX 取证边界。

### Resolution

G23/G22 主线已加入跨 Python/PowerShell 包门禁、`gpt-6-astra` 默认迁移、`gpt-5.2` 历史停用记录及
Skill Hub 结构化诊断。上游签名载荷仍含旧条目时，只报告“默认已迁移、上游待刷新”。

---

## [LRN-20260913-G22] recurring-usb-release-key-must-be-canonical

**Logged**: 2026-09-13T00:00:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: best_practice
**Area**: infra
**Source**: user-report
**Pattern-Key**: harden.canonical_usb_release_key
**Recurrence-Count**: 3+
**First-Seen**: 2026-09-10
**Last-Seen**: 2026-09-13

### Summary
正式 U 盘包不能从“当前执行打包的 Windows 用户”隐式选择签名公钥；必须从受控发行 profile 固定公钥，避免与实际 DPAPI 签名机不一致。

### Details
`keyId=prod-2026-01` 在不同环境中可能对应不同的 Ed25519 公钥。开发机曾把 `4d1407…` 编译进 Bundle，而实际签名机保存的是 `216ae3…`，因此签名器正确拒绝写入授权。构建成功不代表目标签名机可以签名。

### Suggested Action
构建、验包和发布报告统一读取 `tools/usb-signing-release-profile.json`；环境变量为空时加载 profile，已设置但摘要不一致时 fail-closed。每次构建后必须比较 manager 内嵌公钥、Bundle 摘要和签名机公钥摘要。

### Metadata
- Related Files: tools/usb-signing-release-profile.json, tools/usb-signing-profile.ps1, tools/build-installer.ps1, tools/build-oneclick-usb-layout.ps1, tools/verify-oneclick-usb.ps1
- Tags: usb, signing, dpapi, release, key-binding
- See Also: LRN-20260906-003

### Resolution
- **Resolved**: 2026-09-13
- **Notes**: Added canonical public-only release profile and fail-closed build/verify preflight. Rebuilt package must use the target signer public key; the signer continues to reject packages bound to another key.

---

## [LRN-20260906-004] gui-process-start-is-not-product-readiness

**Logged**: 2026-09-06T13:45:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: best_practice
**Area**: windows-launch
**Source**: user-report
**Pattern-Key**: harden.gui_readiness

### Summary
A successful `CreateProcess` call or short process-liveness interval is not a valid success condition for an Electron/WebView2 desktop product.

### Evidence
- Codex created processes and transient window state before Chromium terminated with `GPU process isn't usable`.
- CCSwitch had a normal 914x638 main window but it remained behind the manager, while unrelated 14x14 Tao/WebView bookkeeping windows confused visual diagnosis.
- The user could complete both workflows only after software-rendering parameters and explicit window presentation were applied.

### Practice
Scope process discovery to the exact product root, ignore tiny helper HWNDs, require a sizeable top-level window to remain present across a stability interval, restore/clamp/present it, and clean the failed process tree before a compatibility retry. Keep risky compatibility flags child-scoped and use them only after a safer attempt demonstrably fails.

### Resolution
Applied in v0.5.23 to USB Codex and CCSwitch launches, with exact-version compatibility memory and release-manifest gates.

### Metadata
- Related Files: crates/codex-win-engine/src/windows_process.rs, crates/codex-win-engine/src/sys.rs, src-tauri/src/app/ccswitch.rs
- Tags: windows, electron, webview2, gpu, postcondition
- See Also: ERR-20260906-006

---

## [LRN-20260906-003] filesystem-path-text-is-not-device-identity

**Logged**: 2026-09-06T12:18:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: best_practice
**Area**: windows-usb
**Source**: user-report
**Pattern-Key**: harden.path_identity

### Summary
Equivalent Windows path strings must not be used as the authorization identity for a removable device or bundle.

### Evidence
- The watchdog rediscovery path serialized `D:\\bundle`, while the manager lease serialized `D:\bundle` for the same directory.
- Volume serial, relative bundle path and anchor digest all matched, yet a raw full-path comparison blocked installation before mutation.
- Windows can additionally expose the same object using verbatim prefixes, case differences and 8.3 names.

### Practice
Canonicalize an existing path for safe I/O and diagnostics, but authorize the bundle using stable independently observed identity fields. Construct paths from validated components, keep full paths diagnostic-only, and test equivalent representations alongside genuine device/folder/content mismatches.

### Resolution
Applied in v0.5.21 to USB scanning and the portable install authorization boundary. Targeted, full backend, release-media and real-MSIX transaction tests passed.

### Metadata
- Related Files: src-tauri/src/app/usb_guard.rs
- Tags: windows, usb, lease, canonicalization, identity
- See Also: ERR-20260906-004, LRN-20260906-002

---

## [LRN-20260906-001] async-deployment-requires-independent-postcondition

**Logged**: 2026-09-06T04:26:00+08:00
**Priority**: high
**Status**: resolved
**Category**: best_practice
**Area**: windows-appx
**Source**: user-report
**Pattern-Key**: harden.async_postcondition

### Summary
An asynchronous Windows deployment result can complete before a separately projected state becomes visible. `DeploymentResult.IsRegistered` is therefore a diagnostic hint, not an authoritative installation contract on supported Windows 10 builds.

### Evidence
- The target machine recorded a successful OnDemandRegister operation and exposed an exact healthy current-user package while `IsRegistered` remained false.
- Treating the hint as authoritative produced a false installation failure after Windows had already committed PackageUser.

### Practice
Journal native API completion separately from a bounded, independent postcondition. For AppX registration, verify exact identity, current-user visibility, health, partial-stage state, intended physical placement and ownership. On retry, observe an already committed state before replaying a mutating operation.

### Resolution
Applied to both controlled AppX recovery and the outer native/PowerShell USB installation paths in v0.5.19, with delayed-projection, timeout/resume and journal-tamper regression tests.

---

## [LRN-20260906-002] appx-data-placement-must-be-observed

**Logged**: 2026-09-06T06:10:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: correction
**Area**: windows-appx
**Source**: user-report
**Pattern-Key**: harden.appx_data_placement

### Summary
Writing `RestrictAppDataToSystemVolume=1` and assigning `RegisterPackageOptions.AppDataVolume` are requests, not proof that Windows placed packaged-app data on the system volume.

### Evidence
- On Windows 10 Home 19045 the exact Codex program package was staged and registered on the USB volume, while AppX event 643 recorded the USB `AppDataVolume` and `%LOCALAPPDATA%\Packages\OpenAI.Codex_*` became junctions into `<USB>\WpSystem\<SID>\...`.
- The installer accepted `not-created` immediately after registration, so the first launch could materialize a wrong-volume data layout after installation had already been reported successful.

### Practice
Use the oldest explicit AppX registration API that accepts an `appDataVolume`, log the requested native volume identity, and require an independently observed ordinary system-volume package-data layout before declaring the data-local contract complete. `not-created`, policy readback, setter readback, and deployment success are diagnostic states only.

### Resolution
v0.5.20 removed AppX registration from the USB new-install route entirely. The verified MSIX is now transactionally extracted to the authorized bundle's `apps/Codex`; `CODEX_HOME`, APPDATA and LOCALAPPDATA are checked by both declared and canonicalized volume before installation, and Codex launches with an explicit local `CODEX_HOME`. AppXVolume/WpSystem code remains only for legacy cleanup, so first-launch AppX data materialization can no longer violate the new-install contract.

### Metadata
- Related Files: crates/codex-win-engine/src/appx_data_recovery.ps1, crates/codex-win-engine/src/sys.rs
- Tags: appx, usb, app-data, windows-10
- See Also: LRN-20260906-001

---

## [LRN-20260913-G23-SIGN-PAIR] buildability-is-not-signability

**Logged**: 2026-09-13T21:30:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: best_practice
**Area**: infra
**Source**: user-report
**Pattern-Key**: harden.usb_signing_pair_preflight
**Recurrence-Count**: 4
**First-Seen**: 2026-09-10
**Last-Seen**: 2026-09-13

### Summary

“Bundle 构建成功”不等于“当前签名电脑可以签名”。正式交付前必须验证 Bundle 声明的发行公钥和实际 DPAPI 签名机是同一把密钥。

### Details

本次 r11 Bundle 正确绑定 `prod-2026-01 / 216ae358…`，但当前 `T14S` 签名电脑的自动 DPAPI key 是 `4d140722…`。签名器在写入 `usb-license.json` 前拒绝是正确的 fail-closed 行为；没有匹配的私钥时，不能通过改清单或重新生成同名 keyId 解决。

### Suggested Action

发布前运行 `tools/verify-usb-signing-pair.ps1`，同时检查当前版本显式 profile、Bundle `usb-manifest`、signer manifest 和匹配签名机/portable vault 的 `keyId + publicKeySha256`。如果摘要不一致，停止发布；密钥轮换必须显式更新 profile、管理器、Bundle、签名器交付说明并重新签名全部新介质。

### Metadata

- Related Files: tools/verify-usb-signing-pair.ps1, tools/build-usb-license-signer-gui.ps1, docs/USB-SIGNING-KEY-BINDING-G22.md
- Tags: usb, signing, dpapi, release, key-binding, preflight
- See Also: LRN-20260913-G22

### Resolution

- **Resolved**: 2026-09-13
- **Notes**: Added release-profile metadata to the signer package, added a pair preflight script, and recorded that the current machine must not be used to sign the 216ae3-bound r11 package.

---

## [LRN-20260913-G23] usb-actions-require-live-startup-readiness

**Logged**: 2026-09-13T02:30:00+08:00
**Priority**: high
**Status**: resolved
**Category**: best_practice
**Area**: windows-usb-ui
**Source**: user-report
**Pattern-Key**: harden.startup_readiness_race

### Summary

When the native USB guard deliberately exposes a `starting` state while a full bundle/AuthentiCode scan runs in the background, the UI must refresh the live Guard result immediately after any other expensive startup probe. A one-time initial snapshot plus a slow timer creates a false-negative window where safe install/update actions look permanently unavailable.

### Evidence

- The signed USB bundle eventually became installable without changing files or restarting the manager.
- `usb_guard::current_status()` intentionally returned `starting/not-running` until the background worker finished.
- `OfflineHome` used the initial Guard snapshot for the final payload/installed decision and only observed the later authorization through a 1-second polling interval.

### Practice

Keep the backend fail-closed. After the payload probe, refresh the Guard once; while the explicit `starting` state remains, use a bounded short poll and then return to the normal cadence. Add the same transient-state retry to every screen that consumes the authorization gate. Never enable actions based on a stale snapshot, a missing query result, or a visible button alone.

### Resolution

Implemented in G23: `OfflineHome` refreshes after the payload probe, accelerates bounded `starting` polling, presents an explicit waiting status, and `About` retries `starting` for a bounded window. Added regression coverage for both install and update consumers.

### Metadata

- Related Files: src/app/views/OfflineHome.tsx, src/app/views/About.tsx, docs/USB-STARTUP-READINESS-G23.md
- Tags: windows, usb, startup, readiness, polling, fail-closed
- See Also: ERR-20260913-G23

---

## [LRN-20260913-G24] portable-signer-must-use-an-encrypted-vault

**Logged**: 2026-09-13T23:55:00+08:00
**Priority**: critical
**Status**: resolved
**Category**: security
**Area**: usb-signing
**Source**: implementation
**Pattern-Key**: harden.portable_signer_without_machine_binding

### Summary

让签名器跨电脑工作，不能把 Ed25519 私钥裸放进 EXE，也不能把“当前电脑 DPAPI 可解密”
误当成可迁移能力。正确边界是：旁置加密 vault + 运行时密码；首次在发行签名电脑上
由匹配的 DPAPI 密钥生成 vault。

### Practice

- Bundle、管理器、签名器交付前必须同时核对 `keyId + publicKeySha256`；
- 当前发行 profile 轮换后，旧公钥包只作为历史版本，不得混用；
- vault 使用 AES-GCM 加密和 Argon2id 派生密钥，create-only，密码不进入参数/环境/日志；
- GUI 首次签名自动生成 vault，其他电脑复制完整签名目录后只输入密码；
- 自动化测试必须覆盖正确密码跨目录解密、错误密码、截断 vault 和重复创建拒绝。

### Resolution

2026-09-13 完成 `key_blob` portable vault、GUI 原生密码对话框、共享密码管线、发布脚本
`-PortableVaultPath`、发行绑定轮换和配对预检；Rust 26 项测试、GUI 3 项测试、clippy
和跨目录 CLI 正/负例均通过。

### Metadata

- Related Files: tools/usb-license-signer/src/key_blob.rs, tools/usb-license-signer/src/gui.rs,
  tools/build-usb-license-signer-gui.ps1, docs/PORTABLE-USB-SIGNER-DEVELOPMENT.md
- Tags: usb, signer, portable, vault, argon2, aes-gcm, key-rotation
- See Also: ERR-20260913-G23-SIGN-PAIR

---

## [LRN-20260914-G25] signer-gui-must-be-single-instance

**Logged**: 2026-09-14
**Priority**: high
**Status**: resolved
**Category**: reliability
**Area**: usb-signing
**Source**: user-report and startup-log evidence
**Pattern-Key**: prevent.stale-gui-inspection

### Summary

签名器的“环境不满足”不能只看某一个窗口上的旧状态。签名器必须在进程层面只允许一个
GUI 实例，避免旧窗口继续展示历史公钥绑定而误导用户。

### Practice

- 启动时获取命名 GUI mutex；已存在实例时提示用户使用已有窗口；
- 对没有 mutex 的历史版本，再按窗口类名兜底检测并把旧窗口置前；
- 仍以 Bundle 中声明的 `keyId + publicKeySha256` 与本机/便携签名库做严格比较；
- 发布前同时跑 Bundle pair preflight，并确认 fresh GUI log 最终为 `canSign=true`。

### Resolution

2026-09-14 在 `windows.rs` 增加 GUI instance mutex，在 `gui.rs` 增加旧版本窗口兜底检测；
复核 F: 当前 r12 Bundle、签名器和本机 4d 公钥一致，未写入 U 盘。

### Metadata

- Related Files: tools/usb-license-signer/src/windows.rs, tools/usb-license-signer/src/gui.rs,
  tools/verify-usb-signing-pair.ps1
- Tags: usb, signer, gui, singleton, stale-state, key-binding
- See Also: ERR-20260914-G25

---

## [LRN-20260914-G26] optional-cleanup-roots-must-not-be-created-to-pass-safety-check

**Logged**: 2026-09-14
**Priority**: high
**Status**: resolved
**Category**: reliability / security
**Area**: uninstall-cleanup and skill-hub-update
**Source**: user-report, G22/G23 diagnosis, multi-agent audit
**Pattern-Key**: safe.canonicalize-absent-deep-path

### Summary

可选清理目标可能连同多个父目录一起不存在。安全校验的正确做法是规范化最近存在的普通祖先，再补回缺失后缀；不能为了校验而创建目录，也不能简单跳过祖先检查。

### Practice

- 先运行现有祖先重解析点、路径遍历和范围检查；
- 从目标向上回溯到最近存在目录并规范化，按原始组件顺序补回缺失尾部；
- 删除仍使用原有 no-follow、树复核和最终 postcondition；
- 用 local/roaming 可选父级同时缺失的独立临时目录测试，确认目标仍为 `absent` 且没有副作用。

### Resolution

G26 已在 `uninstall_cleanup.rs` 落地，并把默认 Skill Hub 来源收口为自有内容的 Relay → GitHub fallback。国内源只能是显式、经过严格校验的扩展，不能成为默认构建或运行门槛。

### Metadata

- Related Files: src-tauri/src/app/uninstall_cleanup.rs, src-tauri/src/app/skill_repo_update.rs, src-tauri/build.rs, src-tauri/resources/skill_hub_sources.json, docs/SKILL-HUB-MODEL-SYNC-AND-UNINSTALL-G22-DEVELOPMENT.md
- Tags: windows, uninstall, path-safety, reparse-point, skill-hub, relay, fallback
- See Also: ERR-20260914-G26

---

## [LRN-20260915-G25] release-owned-source-resources-need-runtime-and-package-fallbacks

**Logged**: 2026-09-15
**Priority**: high
**Status**: in_progress
**Category**: reliability / observability
**Area**: skill-hub-routing and release-packaging
**Source**: user screenshot, source audit, live Relay/GitHub probe
**Pattern-Key**: release.source-resource-contract

### Summary

A source directory can be correct in the checkout and still fail in a portable EXE when the runtime resource path is stale, missing, or different from the developer layout. If the backend maps that condition to a generic `SKILL_REPO_SOURCE_INVALID`, the user cannot distinguish packaging failure from mainland network failure.

### Practice

- Treat release-owned source JSON as a three-part contract: compile-time canonical bytes, deterministic runtime candidate paths, and explicit package/SHA256SUMS verification.
- Use the embedded canonical source only when all external copies are absent; reject an existing malformed or reparse copy rather than silently masking tampering.
- Keep source identity errors distinct in detail from transport failures and show the sanitized detail in the disclosure UI.
- Preserve the fixed own-repository Relay → GitHub fallback; do not add derived mirrors merely because a source error is opaque.

### Resolution

G25 added the source-resource candidates/fallback, full safe details, stricter expected source identity, UI detail rendering, build/USB source gates, and cleanup allowlist entries. G24 Codex payload changes remain separate and unchanged.

### Metadata

- Related Files: src-tauri/src/app/skill_repo_update.rs, tools/verify-oneclick-usb.ps1,
  tools/build-oneclick-usb-layout.ps1, tools/build-installer.ps1,
  src-tauri/src/app/usb_media_cleanup.rs, docs/SKILL-HUB-SOURCE-INVALID-G25-DEVELOPMENT.md
- Tags: skill-hub, relay, github-fallback, portable, resources, diagnostics, packaging
- See Also: ERR-20260915-G25, LRN-20260914-G26

---

## [LRN-20260915-PKG-KEY-ROTATION] 用户指定签名机后必须整套重新绑定

**Logged**: 2026-09-15
**Priority**: critical
**Status**: resolved
**Category**: security / release
**Area**: usb-signing
**Source**: user clarification and release preflight
**Pattern-Key**: release.profile-bundle-signer-single-binding

### Summary

签名工具不应偷偷继承构建宿主的 profile。每次发行必须用同一版本 profile 重编管理器、安装器、U 盘 Bundle 和签名器；正式签名器必须携带匹配的加密 portable vault，运行电脑不参与密钥选择。

### Practice

- 旧公钥只作为历史 profile 保存，不能与新包混用；
- 只比较 `keyId + publicKeySha256`，不把机器指纹写进 Bundle；
- 交付前同时做 profile、管理器、Bundle、签名器 manifest、portable vault 公共描述的配对预检；
- 正式 GUI 只使用同目录 portable vault + 用户密码；不能把 DPAPI Blob 放进包，也不能在 vault 缺失时退回本机 DPAPI。

### Resolution

此前错误地把构建宿主的 `4d1407…` 当成了用户实际双击签名器桌面的密钥；该结论已被用户现场报告推翻，`g28-r2` 不能作为可签名交付物。

G28-R3 改为使用实际发行密钥的 `216ae3…` 公共绑定，releaseId 更新为
`g28-r3-final-20260915`，并重新构建管理器、Bundle 和携带 portable vault 的签名器。之后
正式构建脚本强制要求 vault；GUI 缺少 vault 时只提示复制完整目录，不检查运行电脑公钥。
当前构建宿主只能验证公共 profile 和 vault 描述的一致性；最终仍须以任意 Windows x64 电脑
输入正确 vault 密码后的 sign/readback/verify 为准。

### Metadata

- Related Files: tools/usb-signing-release-profile.json, release-profiles/, tools/build-installer.ps1, tools/build-oneclick-usb-layout.ps1, tools/build-usb-license-signer-gui.ps1
- Tags: usb, signer, profile, key-rotation, release-preflight

---

## [LRN-20260916-G30] persisted-authorized-is-not-live-usb-authorization

**Logged**: 2026-09-16
**Priority**: critical
**Status**: resolved in source and G30 package; target-machine verification pending
**Category**: reliability / security UX
**Area**: USB Guard readiness

### Learning

`status.json` 中的 `authorized/running` 只是渲染快照，不能单独作为 U 盘
授权凭证；Windows `spawn()` 返回成功也不等于 helper 已经取得活动锁。首次
点击如果早于 helper 就绪，安全门会正确拒绝，但若把所有 USB 门禁都归类为
签名错误，用户会得到完全错误的诊断方向。

### Practice

- 启动时回收旧 helper，即使二进制未变；等待正确的 helper 锁后再发布可操作状态。
- UI 动作前必须重新读取实时 guard，后端仍必须在受保护边界再次 fail-closed 校验。
- 对租约/看门狗未就绪、许可证签名无效和 Bundle 完整性失败使用不同稳定错误码。
- 对启动过渡只做有界重试，终态 mismatch/invalid 立即拒绝，不能无限等待或绕过签名。
- 回归测试必须覆盖“status 已授权但 helper 未就绪”的首次点击场景。

### Resolution

G30 增加 `usb_authorization` 错误分类、helper 就绪等待、过期状态遮蔽、后端
有界授权重试和安装前实时复核，并同步 Rust/前端测试及开发文档。G30 安装器、
便携签名器已用同一版本化发行 profile 构建并通过本机配对预检。以后看到
`status=authorized` 与 `lease/helper` 同时出现时，先判断 live readiness，再判断
密码学签名。

### Metadata

- Related Files: src-tauri/src/app/usb_guard.rs, src-tauri/src/errors.rs,
  src/app/views/OfflineHome.tsx, src/app/errorCopy.ts,
  docs/USB-AUTHORIZATION-READINESS-RACE-G30-DEVELOPMENT.md
- Tags: usb, lease, watchdog, readiness, signature-diagnostics

---

## [LRN-20260916-G30-RELEASE-AUDIT] stale-local-runtime-must-not-stand-in-for-a-release-test

**Logged**: 2026-09-16
**Priority**: high
**Status**: resolved by release audit
**Category**: release verification / runtime provisioning

### Learning

开发机现有能力包是旧版 1.2.0，虽记录 runtime ready，但其 `bin` 下的
`python.cmd`、`pip.cmd`、`node.cmd`、`npm.cmd` 已退化为 14 字节的 `\\` 壳；
不能把当前用户目录的旧状态当成新发行包的验收证据。

### Practice

- 每次发布必须同时做源码审计、包内审计和隔离用户目录安装。
- 隔离安装必须检查生成的命令壳是可执行的 ASCII wrapper，并检查真实 runtime health。
- 发布包的 `pack.json` sourceCommit/version 与现场 readiness 版本必须可追溯；旧状态只能作为迁移/修复场景单独测试。

### Verification

G30 源码包和 USB 包的兼容性审计、253 文件包校验、隔离安装（50 Skills、7 Agents、
Python health passed、命令壳 63/70 字节）均通过。以后若只看到旧 `readiness.json`
或旧 wrapper，不得据此判定新包失败，必须重跑隔离安装或现场安装验证。

### Metadata

- Tags: runtime, wrapper, capability-pack, release-audit, regression-prevention

---

## [LRN-20260916-G33] network-client-provider-and-source-contracts-are-release-contracts

**Logged**: 2026-09-16
**Priority**: critical
**Status**: resolved in source; release smoke test pending
**Category**: update reliability / regression prevention

### Learning

前端看到的 watchdog 超时不等于网络慢：当 reqwest 使用 `rustls-no-provider` 时，未显式安装 crypto provider 会在 client 构建阶段 panic，renderer 可能永远收不到 native 结果。与此同时，更新源的 metadata 协议和“无更新/源不可用”是独立契约，不能靠旧 schema 或单一错误标志推断。

### Practice

- 每个独立 reqwest client 在测试中必须覆盖“构建即成功”，并在依赖特性改变后复核 provider 初始化。
- 应用代理配置必须在 client 边界读取；错误只记录 source/phase/reason，不记录凭据或完整 URL。
- Relay schema 测试必须包含当前生产协议和身份回显；未知协议或身份不一致 fail closed。
- Auto 更新结果只在没有任何合法 source 时才返回 source-unavailable；合法 no-newer 证据优先保留。
- renderer watchdog 只能是最后兜底，不能代替 native deadline、错误返回和可观测性。
- 网络探针必须继续走归档解析；HTTP 200、size 和 SHA-256 通过后，还要用当前远端 `SKILL.md` frontmatter 复核，生产格式变化只能以受限解析扩展并配回归测试。

### Metadata

- Related Files: src-tauri/src/app/skill_repo_update.rs, src-tauri/src/app/channels/codex.rs, src/app/views/SkillRepoUpdateCard.tsx, docs/UPDATE-SOURCE-AND-SKILL-HUB-RESILIENCE-G33-DEVELOPMENT.md
- Tags: rustls, reqwest, skill-hub, relay-protocol, update-source, watchdog, frontmatter

---

## [LRN-20260917-G34] directory-flush-needs-write-access

**Logged**: 2026-09-17
**Priority**: high
**Status**: resolved in source and G34 package; physical target-machine validation pending
**Category**: Windows durability / error classification

### Learning

`FlushFileBuffers` 不是只读目录检查。Windows 普通 NTFS 目录必须用带 `GENERIC_WRITE` 的目录句柄刷新；用 `GENERIC_READ` 打开后再 flush 会稳定返回错误 5。这个本地 I/O 失败不能伪装成远端包或 ledger 无效。

### Practice

- 所有 Windows 目录 durability 代码都要保留 `GENERIC_WRITE + FILE_FLAG_BACKUP_SEMANTICS` 的真实 API 回归测试。
- source resolve、archive verify、local staging、apply 和 commit 必须分阶段记录，任何一阶段失败都要保留自己的稳定错误码。
- 构建 U 盘包和签名器时，必须同时校验当前 manager 的编译期公钥绑定、版本化 profile 和 portable vault，避免历史产物重新组合。

---

## [LRN-20260917-G35] portable-resource-resolution-must-follow-the-running-exe

**Status**: applied in source; G35 package and signer pair validation passed
**Category**: portable packaging / Skill Hub resource loading

便携 U 盘程序的 Tauri `resource_dir()` 不一定等于运行中 EXE 旁边的 resources 目录。
来源清单与基础 Skill 清单必须使用一致的“运行中 EXE 优先”策略；对于编译期已校验的只读清单，
可使用内置副本避免资源复制遗漏阻断正常功能，但外部文件存在且篡改/损坏时必须拒绝。

发布前要在最终包根目录检查 `resources/skill_repo_base_manifest.json`，并比较其 source/hash/ID 顺序
与 `content/codex-starter-kit/manifest/skill-audit.json`；不能只看 `installer-pack` 或 Cargo
资源目录。

### Metadata

- Related Files: src-tauri/src/app/skill_repo_update.rs, src/app/errorCopy.ts, src/shared/types.ts, docs/SKILL-HUB-BASE-MANIFEST-PORTABLE-RESOURCE-G35-DEVELOPMENT.md
- Tags: windows, ntfs, FlushFileBuffers, skill-hub, local-io, release-pairing, powershell-encoding

---

## [LRN-20261023-MODEL-TOOLS] do not rule out model-catalog tool filtering without a runtime probe

**Logged**: 2026-10-23T04:59:08+08:00
**Priority**: high
**Status**: pending
**Category**: correction
**Area**: model catalog / Codex tool registration
**Source**: user correction and local runtime investigation

### Summary

Earlier analysis concluded too strongly that adding a third-party model was unrelated to missing MCP and host tools. Treat model selection as a live hypothesis until the same Codex Desktop build is compared with native and added models.

### Details

The new test machine still lacked MCP after adding a model and repairing the capability pack. Source inspection also found generated model entries explicitly disabled Skills, plugin, App instructions, and Node REPL. Isolated CLI checks do not reproduce Desktop's host bridge, so they cannot disprove the user's Desktop report. A separate loopback CLI comparison found that the Node REPL flag alone did not change the captured base tool list, and Code Mode tests were confounded by a missing host executable.

### Suggested Action

Keep instruction-inclusion switches separate from actual capability switches. Restore relevant host usage instructions where applicable, but retain conservative tool-disable flags until that specific capability is tested. Validate the actual Desktop app-server tool list, config path, and MCP invocation on a test machine before closing the root-cause investigation.

### Metadata

- Related Files: src-tauri/src/app/model_catalog.rs, docs/MODEL-CATALOG-HOST-TOOLS-AND-MCP-20261023.md
- Tags: model-catalog, mcp, tools, desktop-runtime

---

## [LRN-20261025-CAPABILITY-MANIFEST-CANONICAL-BYTES] Validate pack metadata with the repository canonicalizer

**Logged**: 2026-10-25
**Priority**: medium
**Status**: resolved
**Category**: best_practice
**Area**: packaging / integrity metadata
**Source**: error

### Summary

The capability-pack manifest records canonical text byte lengths, not physical Windows file sizes.

### Details

- A manual audit initially compared `entry.bytes` to the on-disk byte count and incorrectly reported two text files as mismatched because their line endings were CRLF.
- The repository refresh tool normalizes CRLF and CR to LF before hashing and counting text files.
- After changing a packaged file, update metadata with `tools/refresh-capability-pack-metadata.py` and validate with its `--check` mode instead of comparing raw file sizes.

### Suggested Action

Use the repository canonicalizer for capability-pack hashes and lengths; report only mismatches from the official `--check` result.

### Metadata

- Related Files: `tools/refresh-capability-pack-metadata.py`, `installer-pack/file-manifest.json`, `installer-pack/checksums.sha256`
- Tags: package-integrity, line-endings, canonical-hash
