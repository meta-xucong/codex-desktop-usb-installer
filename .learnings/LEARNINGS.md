# Learnings

## [LRN-20260906-005] mutable-product-metadata-needs-a-separate-trust-model

**Logged**: 2026-09-06T15:02:00+08:00
**Priority**: high
**Status**: resolved
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
