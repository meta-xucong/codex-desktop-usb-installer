# Errors

## [ERR-20260906-012] malformed-functions-exec-wrapper

**Logged**: 2026-09-06T15:12:00+08:00
**Priority**: low
**Status**: resolved
**Area**: infra

### Summary
The first repository-inspection tool call contained placeholder JavaScript and failed before invoking any shell or Git command.

### Error
```
SyntaxError: Unexpected token '?'
```

### Context
- The wrapper failed during JavaScript parsing; no command was executed and no repository or GitHub state changed.
- The intended operation was a read-only Git/GitHub inventory.

### Resolution
- **Resolved**: 2026-09-06T15:13:00+08:00
- **Notes**: Reissued a schema-valid `exec_command` wrapper and confirmed repository, authentication, branches and remotes successfully.

---

## [ERR-20260906-013] package-lock-powershell-empty-key

**Logged**: 2026-09-06T10:15:00+08:00
**Priority**: low
**Status**: resolved
**Area**: diagnostics

### Summary
A read-only PowerShell inspection of `package-lock.json` failed because `ConvertFrom-Json` tried to materialize an object containing an empty property name.

### Error
```
ConvertFrom-Json: The provided JSON includes a property whose name is an empty string.
```

### Context
- npm lockfiles can contain package-map keys that PowerShell's default object conversion cannot represent safely.
- The command was diagnostic only and made no filesystem changes.

### Suggested Fix
Inspect npm lockfiles with Node.js, or use `ConvertFrom-Json -AsHashTable` on supported PowerShell versions.

### Resolution
- **Resolved**: 2026-09-06T10:16:00+08:00
- **Notes**: Re-ran the inspection with Node.js and confirmed `browserslist` was updated from 4.28.6 to 4.28.9.

---

## [ERR-20260906-011] validation-command-used-wrong-cargo-manifest

**Logged**: 2026-09-06T14:55:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
Two validation commands initially targeted the repository root/src-tauri manifest even though the Rust workspace and the ignored real-MSIX test live in different crate manifests.

### Error
```
cargo metadata: could not find Cargo.toml at repository root
real_usb_payload_extracts_transactionally: running 0 tests
```

### Context
- The repository has no root Cargo workspace manifest.
- The real MSIX extraction test belongs to `crates/codex-win-engine`, not `src-tauri`.
- Neither invocation mutated product state; the zero-test result was explicitly rejected as evidence.

### Resolution
- **Resolved**: 2026-09-06T14:57:00+08:00
- **Notes**: Re-ran formatting with `src-tauri/Cargo.toml`, then ran the ignored real payload test against `crates/codex-win-engine/Cargo.toml`; exactly one real test executed and passed.

---

## [ERR-20260906-010] apply-patch-after-rustfmt-context-drift

**Logged**: 2026-09-06T14:48:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
A small wording patch used the pre-rustfmt one-line context and failed after rustfmt had wrapped the string expression.

### Error
```
apply_patch verification failed: Failed to find expected lines
```

### Context
- The intended target text was unchanged; only its surrounding formatting differed.
- No file was partially edited by the failed patch.

### Suggested Fix
Re-read the exact local context after formatting, then apply the smallest text-only patch.

### Metadata
- Reproducible: yes
- Related Files: src-tauri/src/app/usb_media_cleanup.rs

### Resolution
- **Resolved**: 2026-09-06T14:48:00+08:00
- **Notes**: Re-read the formatted lines and applied the typo correction against current context.

---

## [ERR-20260906-009] usb-license-blocks-source-media-finalizer

**Logged**: 2026-09-06T14:35:00+08:00
**Priority**: high
**Status**: resolved
**Area**: backend

### Summary
v0.5.21 completed the primary portable uninstall but its detached source-media finalizer preserved the USB kit because the manager-created `usb-license.json` was classified as an unknown distribution file.

### Error
```
二阶未完成
unknown files (not in manifest): ['usb-license.json']
```

### Context
- All 28 immutable `SHA256SUMS.txt` entries matched; `usb-license.json` was the only extra file.
- The manager auto-enrolls that mutable identity file at startup, while strict media cleanup originally accepted only immutable release-manifest entries.
- Unknown user content must still fail closed, so a blanket ignore rule would be unsafe.

### Suggested Fix
Model the license and its atomic backup as typed manager-owned mutable metadata, validate their complete schema and fingerprint against the exact pinned bundle identity, report every blocking path, and delete the validated metadata only in the final commit after the guard has stopped.

### Metadata
- Reproducible: yes
- Related Files: src-tauri/src/app/usb_media_cleanup.rs, src-tauri/src/app/uninstall_cleanup.rs, src-tauri/src/app/usb_guard.rs
- See Also: ERR-20260906-004, LRN-20260906-003

### Resolution
- **Resolved**: 2026-09-06T15:02:00+08:00
- **Notes**: v0.5.24 now captures the active lease fingerprint before watchdog shutdown, validates the typed primary/backup license in both uninstall phases, snapshots its bytes across helper handoff, deletes it only at final commit, and enumerates genuine blockers. Targeted 17/17, manager 305/305, frontend 364/364, real-MSIX extraction and three full-kit verifier runs passed.

---

## [ERR-20260906-008] release-output-must-be-empty

**Logged**: 2026-09-06T13:50:00+08:00
**Priority**: low
**Status**: resolved
**Area**: release

### Summary
The final rebuild attempted to reuse the already-populated v0.5.22 output directory, and the release script correctly failed closed to prevent mixed-version residue.

### Error
```
发布输出目录必须为空；请使用新的版本目录，避免残留旧 EXE、资源或许可证
```

### Context
- The directory was the disposable v0.5.22 candidate generated earlier in this same task.
- The previous v0.5.21 source kit and every unrelated release directory remain out of scope.

### Suggested Fix
Validate the exact generated path is inside the repository release directory, remove only that disposable candidate, and rebuild into the now-absent version directory.

### Metadata
- Reproducible: yes
- Related Files: tools/build-usb-layout.ps1

### Resolution
- **Resolved**: 2026-09-06T13:50:00+08:00
- **Notes**: Followed the release script's clean-directory contract; no old or user-supplied release directory was changed.

---

## [ERR-20260906-007] vitest-jest-runinband-option

**Logged**: 2026-09-06T14:20:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
The first full frontend-test invocation passed Jest's `--runInBand` option to Vitest, which rejects that option before collecting tests.

### Error
```
CACError: Unknown option `--runInBand`
```

### Context
- `npm test` already maps to `vitest run`; this project does not use Jest's CLI.
- TypeScript and ESLint completed successfully in the same validation group.

### Suggested Fix
Run `npm test` without Jest-only arguments. Keep the release/offline suites serial when resource contention matters.

### Metadata
- Reproducible: yes
- Related Files: package.json

### Resolution
- **Resolved**: 2026-09-06T14:20:00+08:00
- **Notes**: Corrected the invocation to the repository's declared `npm test` command.

---

## [ERR-20260906-006] post-install-launch-reported-before-usable-window

**Logged**: 2026-09-06T14:10:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: windows-launch

### Summary
v0.5.21 completed the USB portable install but treated process creation/short liveness as delivery success even when Codex's Chromium GPU process later died and CCSwitch never presented a foreground, usable window.

### Error
```
GPU process exited unexpectedly: exit_code=1
FATAL:gpu_data_manager_impl_private.cc:417] GPU process isn't usable. Goodbye.
```

### Context
- Target: Windows 10 19045 with Intel integrated graphics and 125% DPI.
- Codex became usable only with `--disable-gpu --no-sandbox`.
- CCSwitch became usable with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--disable-gpu` plus foreground activation.
- The 14x14 `Tao Thread Event Target`/WebView helper is not a trustworthy product-window postcondition; the normal CCSwitch main window was already about 914x638 but remained behind the manager.
- The source already invokes CCSwitch after the portable install, so G11 is primarily an observability/foreground failure, not a missing frontend call.

### Suggested Fix
Give USB launches a bounded visible-window contract, use sandboxed software rendering first, retry Codex with the empirically proven no-sandbox fallback only if the first attempt exits or never exposes a usable window, launch CCSwitch with WebView2 software rendering, foreground only a sizeable top-level window under the exact managed executable root, and refresh stale locally copied watchdog binaries across manager versions.

### Metadata
- Reproducible: yes
- Related Files: crates/codex-win-engine/src/sys.rs, crates/codex-win-engine/src/windows_process.rs, src-tauri/src/app/ccswitch.rs, src-tauri/src/app/win_update.rs, src-tauri/src/app/usb_guard.rs

### Resolution
- **Resolved**: 2026-09-06T13:45:00+08:00
- **Notes**: v0.5.23 requires a stable sizeable window, clamps it to a monitor work area, uses child-scoped software rendering for both products, retries Codex with no-sandbox only after the safe attempt fails, persists that fallback per exact Codex version, and refreshes stale local guard bytes before issuing a new lease. Engine, manager, frontend, release, real-MSIX extraction and complete-kit verification passed.

---

## [ERR-20260906-004] usb-lease-path-representation-mismatch

**Logged**: 2026-09-06T12:15:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: backend

### Summary
v0.5.20 authorized the USB successfully but rejected installation because the watchdog and manager serialized the same bundle with different separator representations.

### Error
```
USB 看门狗：当前安装器目录与已授权 U 盘租约不一致
status.bundlePath = D:\\usb-oneclick-v0.5.20-final
lease.bundlePath  = D:\usb-oneclick-v0.5.20-final
```

### Context
- `scan_removable_for` concatenated a drive root already ending in `\` with a relative suffix beginning in `\`, producing a doubled separator.
- `authorized_usb_codex_root` trusted the renderer-friendly status snapshot and compared its full path to `current_exe`'s parent as raw text.
- The three stable fingerprint fields matched, and no payload mutation had begun.

### Suggested Fix
Build scanned paths from validated components, canonicalize observed existing bundle paths, and authorize the install root by a fresh observation matched to the current process-owned lease fingerprint. Keep full paths diagnostic-only.

### Metadata
- Reproducible: yes
- Related Files: src-tauri/src/app/usb_guard.rs
- See Also: LRN-20260906-002

### Resolution
- **Resolved**: 2026-09-06T12:18:00+08:00
- **Notes**: v0.5.21 constructs scanned paths from validated components, canonicalizes observed bundle paths, and authorizes the mutating install destination by the manager-owned lease fingerprint instead of status-path text. Equivalent path-representation and genuine mismatch tests pass; the rebuilt real release passed strict media verification and real 802 MB MSIX extraction.

---

## [ERR-20260906-005] rustfmt-relative-file-argument

**Logged**: 2026-09-06T12:08:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
A combined validation command passed a package-relative source path to `cargo fmt` from the repository root, so only the formatting subcommand failed before tests continued.

### Error
```
Error: file `src\app\usb_guard.rs` does not exist
```

### Context
- Command ran from the repository root with `--manifest-path src-tauri\Cargo.toml`.
- The subsequent targeted Cargo tests were a separate native process and completed successfully.

### Suggested Fix
Run `rustfmt --edition 2021 src-tauri\src\app\usb_guard.rs` for a single file, or run package-wide `cargo fmt` without passing a source path after `--`.

### Metadata
- Reproducible: yes
- Related Files: src-tauri/src/app/usb_guard.rs

### Resolution
- **Resolved**: 2026-09-06T12:09:00+08:00
- **Notes**: Re-ran `rustfmt` with the repository-relative full source path, then passed `git diff --check` and the targeted tests.

---

## [ERR-20260906-001] web-search-extraction

**Logged**: 2026-09-06T06:02:00+08:00
**Priority**: medium
**Status**: pending
**Area**: config

### Summary
The required web-search skill could not run because its OpenClaw configuration file was absent.

### Error
```
{"error": "C:\\Users\\T14S\\.openclaw\\sosoclaw.json not found"}
```

### Context
- Attempted the skill's `scripts/web_search.py --json` workflow for Microsoft AppX documentation.
- All four independent queries failed before network access.

### Suggested Fix
Configure the search skill's documented credential/config source or make its script support the active Codex environment; until then, use a clearly disclosed authoritative-source fallback.

### Metadata
- Reproducible: yes
- Related Files: C:/Users/T14S/.codex/skills/web-search-extraction/scripts/web_search.py

---

## [ERR-20260906-003] real-msix-portable-staging-residue

**Logged**: 2026-09-06T06:09:00+08:00
**Priority**: medium
**Status**: resolved
**Area**: backend

### Summary
The opt-in real 802 MB MSIX test completed the USB portable install but left the empty `apps/.codex-app-manager-staging` parent behind.

### Error
```
assertion failed: !root.join("apps/.codex-app-manager-staging").exists()
```

### Context
- The installer removed the per-operation `portable-<UUID>` child with `remove_dir_all` but never removed its now-empty fixed parent.
- Strict source-media cleanup correctly treats an unexpected runtime directory as a blocker, so this harmless-looking residue could prevent a later complete USB uninstall.
- Review also found preparation and two observer-error exits that did not consistently clean or roll back their work tree.

### Suggested Fix
Centralize operation-directory cleanup, remove the fixed parent only with non-recursive `remove_dir` when empty, call it on ordinary preparation/rollback/success exits, and preserve intentional crash-recovery windows.

### Metadata
- Reproducible: yes
- Related Files: crates/codex-win-engine/src/portable.rs, src-tauri/src/app/usb_media_cleanup.rs

### Resolution
- **Resolved**: 2026-09-06T06:12:00+08:00
- **Notes**: Added one centralized cleanup routine that removes the exact per-operation tree and then uses non-recursive `remove_dir` on the fixed parent only when empty. Preparation and observer failures now clean/rollback consistently. The full engine suite passed 171/171, and the same real 802 MB MSIX extraction test passed with no staging parent left behind.

---

## [ERR-20260906-002] npm-test-parallel-timeout

**Logged**: 2026-09-06T06:05:29+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
The full Vitest suite hit one five-second timeout while it ran concurrently with the release suite and PowerShell bundle simulation.

### Error
```
scripts/mirror-release.test.mjs:798
Test timed out in 5000ms; cleanup then observed ENOTEMPTY in the still-active fixture.
```

### Context
- `npm test`, `npm run test:release`, and `npm run test:offline-bundle` were intentionally started in parallel.
- The same two-file release suite completed 56/56 in that same run, so the first hypothesis is host resource contention rather than a product assertion failure.

### Suggested Fix
Run the full Vitest suite once in isolation. If it reproduces, make the fixture completion/cleanup deterministic or set a justified per-test timeout; do not accept only the isolated sub-suite as the final regression result.

### Metadata
- Reproducible: unknown
- Related Files: scripts/mirror-release.test.mjs

### Resolution
- **Resolved**: 2026-09-06T06:06:15+08:00
- **Notes**: `npm test` completed 32/32 files and 364/364 tests when run once in isolation. The failure was caused by running the full suite and its release subset concurrently; final validation schedules them serially.

---
