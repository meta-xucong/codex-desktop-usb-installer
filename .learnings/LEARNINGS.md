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
