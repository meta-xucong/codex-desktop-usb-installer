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

## [ERR-20260916-G29] update-anchor-must-bind-the-authorized-slot-not-its-current-occupancy

**Logged**: 2026-09-16
**Priority**: critical
**Status**: fixed in source; local release verification passed; target-machine verification pending
**Area**: codex-payload-update / usb-signing / release-packaging

### Summary

Codex payload update changed the payload, release metadata, SHA256SUMS and may
create the first backup file. If the USB license anchor hashes those changing
bytes, a legitimate update invalidates its own authorization. A first draft of
the stable representation also treated an absent backup row differently from
an occupied backup row, which would reproduce the same failure after the first
update.

### Root Cause

The top-level manifest schema was overloaded as the cryptographic algorithm
selector, and the checksum canonicalization omitted dynamic rows only when
they were present. That made the update lifecycle depend on whether the
initial package already had a backup file. Separately, a stale Relay result
could terminate Auto source selection before a newer later source was tried.

### Fix

- New packages keep root/USB top-level `schemaVersion=2`, but explicitly declare
  `updateCapability.schemaVersion=3` and `anchorSchema=3` on both manifests.
- `stable-update-v1` canonicalizes payload, both manifests and the single
  backup slot to fixed `MUTABLE` markers; all other checksum rows remain
  `IMMUTABLE hash path`. The backup slot marker is emitted even when empty.
- The updater computes the projected anchor before the first destructive rename
  and verifies the same anchor, license digest, installation id and key id
  after commit; failures go through rollback.
- Auto source selection continues after unreachable/invalid/stale/equal sources
  and only accepts a newer candidate; explicit source selection never silently
  falls back.
- Build, verifier and portable signer require the same explicit capability
  schema and versioned public profile; old v1/v2 anchors remain launch/read
  compatible but are refused by the Codex update channel.

### Prevention

Every release update must test both “no backup before first update” and
“one backup already present” against the same anchor. Never infer the anchor
algorithm from the document top-level schema, never copy a profile by filename,
and never stop Auto selection merely because one source is healthy but stale.

### Verification

Stable-anchor crate tests: 12/12; manager full lib tests: 415 passed, 2
ignored; signer tests: 32/32; frontend: 419 passed, 1 skipped; compatibility
audit: 50 Skills/7 Agents; adapter contracts: 42/42; PowerShell AST: 8/8.
The fresh G29 release package and portable signer passed local self-verification
and profile-pair verification. The physical USB was not modified. Target-machine
sign/real apply remain release-boundary checks.

---

## [ERR-20260915-G24] codex-payload-apply-repeated-local-hash

**Logged**: 2026-09-15
**Priority**: high
**Status**: resolved in source; physical USB verification pending
**Area**: codex-payload-update

### Summary

G24 reported that Codex payload update spent minutes on a 774 MB removable-drive payload and then
showed “更新信息已过期” without downloading. The report attributed this to a destructive token TTL,
but the current source's Codex payload command does not arm or validate that token; the tested g28
binary and source therefore need to be treated as potentially mismatched.

### Root Cause

The source performed repeated full payload reads in one check/apply path: capability validation,
SHA256SUMS verification, local snapshot construction, and commit preflight. On a USB volume this
created several GB of redundant I/O before source resolution/download, amplifying any confirmation
window or stale-state problem and leaving early failures poorly observable.

### Fix

- Added metadata-only capability preflight for update check/apply, then resolve the remote identity
  before the expensive full local integrity pass.
- Added a per-call verified digest table from `SHA256SUMS`; snapshots reuse only that immediately
  preceding full verification result. No digest is cached across check/apply sessions, and size/mtime
  are never treated as a security substitute.
- Kept full capability/integrity checks and an independent payload hash at the commit boundary.
- Added capability, source-resolve, integrity, snapshot, commit, and command error stage/code logs;
  added a preparing progress event.
- Documented the binary/source mismatch and the rule that a future destructive-token integration
  must use a long-operation lease/heartbeat rather than silently weakening validation.

### Prevention

Before packaging a new manager, run the Codex channel tests and inspect an apply log for ordered
`stage=source-resolve`, `stage=integrity`, `stage=snapshot`, `stage=download`, and `stage=commit`
records. Never diagnose a token-expiry report from strings alone: first confirm that the tested EXE
and source use the same command path. Do not add cross-session hash caching to hide slow removable
media.

### Verification

`cargo fmt --all -- --check`, `cargo clippy --lib -- -D warnings`, `cargo test --lib`: passed;
401 tests passed, 2 ignored. A new regression test verifies digest-table snapshot equivalence and
fail-closed behavior when a required digest is missing. Real signed USB/g28 binary reproduction is
not claimed by source-level tests.

---

## [ERR-20260915-G28] interactive-signing-desktop-differs-from-build-host

**Logged**: 2026-09-15
**Priority**: critical
**Status**: in_progress
**Area**: usb-signing / release-configuration

### Summary

G27 Bundle 使用了 Codex 构建宿主诊断出的 `4d1407…`，但用户实际双击签名器的 Windows
桌面读取到 `216ae3…`。G27 Bundle 深度校验通过、License 缺失，拒签原因仍是公钥绑定不一致。

### Root Cause

Windows DPAPI 结果被错误地按“当前 Codex 执行环境”代表“用户实际签名桌面”。构建宿主、服务
会话、管理员进程和交互式桌面可能不是同一个安全上下文；相同 keyId 不能证明是同一把私钥。

### Fix

- 正式 profile 恢复到用户现场真实交互式签名桌面的 `216ae358…`；
- G28 重新生成管理器、Bundle 和签名器发布目录；
- 发布说明明确区分“公共绑定构建证明”和“交互式签名机可用证明”；
- 真实桌面必须从 G28 新目录运行 `--diagnose-key`，并完成 sign/readback/verify；
- 不再用 Codex 宿主的本地 DPAPI 结果自动旋转生产 profile。

### Prevention

构建机没有签名机私钥时只能使用受控 profile 构建公共绑定，manifest 必须注明尚未完成目标
桌面验收。交付前必须收集实际签名桌面的诊断证据；旧 Bundle、旧日志和旧窗口一律不算证据。

### Related Files

- `tools/usb-signing-release-profile.json`
- `docs/USB-SIGNER-INTERACTIVE-KEY-BINDING-G28-DEVELOPMENT.md`
- `tools/build-usb-license-signer-gui.ps1`
- `tools/verify-usb-signing-pair.ps1`

---

## [ERR-20260915-G27] published-gui-key-binding-was-not-directly-verified

**Logged**: 2026-09-15
**Priority**: critical
**Status**: resolved
**Area**: usb-signing / release-gate

### Summary

用户使用最新签名工具时仍看到“签名环境不满足”。现场日志中的 GUI 公钥摘要为
`216ae358…`，而待签 Bundle 与当前命令行密钥诊断为 `4d140722…`。此前发布校验只跑
了 CLI `init-key`，因此“CLI 可用”被错误地当成了“用户双击的 GUI 可用”。

### Root Cause

历史 GUI、旧进程状态、CLI 和新源码可能来自不同发布目录或运行时上下文。静态 Bundle
校验、Cargo 构建成功、独立 CLI `init-key` 成功都不能证明即将交付的 GUI 会读取同一把
密钥。相同 `keyId` 也不能替代公钥摘要比较。

### Fix

- 增加 GUI 隐藏诊断入口 `--diagnose-key <临时 JSON>`，复用桌面 GUI 的实际密钥解析管线，
  只输出 `keyId` 和公钥摘要，不输出私钥；
- `build-usb-license-signer-gui.ps1` 默认对已复制的发布 EXE 执行诊断，与 canonical profile
  不一致时 fail-closed；
- `verify-usb-signing-pair.ps1` 支持 `-SignerGuiPath`，正式配对校验优先检查发布 GUI；
- 以当前新 GUI 实时诊断得到的 `4d140722…` 恢复 canonical profile；旧 `216ae358…` 包和
  日志只保留为历史不兼容证据；
- 发布说明和回归清单明确要求关闭旧窗口、从最新目录只启动一个 GUI。

### Prevention

每次换包或换签名工具都必须完成 GUI、Bundle、管理器三方 `keyId + publicKeySha256`
比较；不得只看 CLI、旧日志或静态清单。未通过 GUI 诊断的目录不能作为正式签名工具交付。

### Related Files

- `tools/usb-license-signer/src/bin/usb-license-signer-gui.rs`
- `tools/usb-license-signer/src/gui.rs`
- `tools/build-usb-license-signer-gui.ps1`
- `tools/verify-usb-signing-pair.ps1`
- `docs/USB-SIGNER-KEY-BINDING-G27-DEVELOPMENT.md`

### Additional G27 finding

The first implementation invoked the Windows-subsystem GUI with PowerShell `&` and read the
diagnostic file immediately. The process exit could be observed before the file was flushed,
causing a false “diagnostic failed” result. The build and pair-preflight scripts now use
`Start-Process -Wait -PassThru` and require both exit code zero and a present JSON file.

### Resolution

Resolved 2026-09-15: the new signer package passed the GUI diagnostic gate, pair preflight,
static Bundle verification, signer tests, main Rust tests, capability-pack audit, and the
historical-key negative case. The release script also removes only a newly created partial
output directory when a gate fails, preventing a failed artifact from being mistaken for a
complete release.

---

## [ERR-20260913-G22] stage2-finalizer-self-reference

**Logged**: 2026-09-13T04:40:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: windows-uninstall

### Symptom

二阶段卸载清理后弹出 Windows“位置不可用”，报告写入失败并显示 `E_FAIL`；清理目标本身可能已删除。

### Root Cause

finalizer 把最终报告写回已被清理的 `%LOCALAPPDATA%\io.github.wangnov.codexappmanager\logs`，形成
报告根目录自引用。

### Prevention

独立临时报告根目录、目标交叉检查、明确阶段/退出码/失败路径/重试建议，以及日志目录缺失时的 fallback；
新增 G22 回归测试并在发行包中拒绝运行期授权残留。

---

## [ERR-20260913-G23-SIGNING-PAIR] usb-package-has-no-matching-local-signer

**Logged**: 2026-09-13T21:30:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: infra

### Summary

r11 U 盘包完整性检查通过，但当前签名电脑无法满足签名条件。

### Error

```text
Bundle 要求：prod-2026-01 / 216ae358…
当前 DPAPI：prod-2026-01 / 4d140722…
结果：签名器在写入 usb-license.json 前拒绝
```

### Context

- 只读 `inspect`：Bundle 深度校验通过、签名绑定为 `216ae358…`、License 状态为 `missing`。
- 当前机器 `init-key`：DPAPI 公钥摘要为 `4d140722…`。
- U 盘没有被写入错误许可证；该拒绝是预期的 fail-closed 行为。

### Suggested Fix

在持有 `216ae358…` 对应 DPAPI 私钥的签名电脑上签名；交付前运行 `tools/verify-usb-signing-pair.ps1`，禁止只凭“构建成功”交付。不得改清单摘要或删除 DPAPI 文件绕过检查。

### Metadata

- Reproducible: yes
- Related Files: tools/verify-usb-signing-pair.ps1, tools/usb-signing-release-profile.json, docs/USB-SIGNING-KEY-BINDING-G22.md
- See Also: ERR-20260913-G22

---

## [ERR-20260913-G22] usb-package-bound-to-wrong-dpapi-key

**Logged**: 2026-09-13T00:00:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: infra

### Summary
新打包的 G21 U 盘包在测试机无法签名，界面显示本机公钥摘要为 `216ae3…`，而包绑定了开发机的 `4d1407…`。

### Error
```text
签名环境与当前安装包不匹配；本机 keyId=prod-2026-01 / 公钥摘要=216ae358…
```

### Context
- 打包命令从当前 PowerShell 的 USB 公钥环境变量构建。
- 两个环境使用相同 keyId，但 DPAPI Blob 实际不是同一把密钥。
- 签名器拒绝写入许可证是正确的 fail-closed 行为。

### Suggested Fix
使用项目级发行公钥 profile 固定生产公钥；构建和验包在环境变量为空时自动加载 profile，在摘要不一致时立即失败。

### Metadata
- Reproducible: yes
- Related Files: tools/usb-signing-release-profile.json, tools/usb-signing-profile.ps1
- See Also: ERR-20260906-003

### Resolution
- **Resolved**: 2026-09-13
- **Notes**: Added canonical profile and build/verify fail-closed checks; next package is rebuilt against the target signer key.

---

## [ERR-20260906-013] package-lock-powershell-empty-key

**Logged**: 2026-09-06T15:10:00+08:00
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
- **Resolved**: 2026-09-06T15:11:00+08:00
- **Notes**: Re-ran the inspection with Node.js and confirmed `browserslist` was updated from 4.28.6 to 4.28.9.

---

## [ERR-20260906-014] rg-windows-wildcard-path

**Logged**: 2026-09-06T15:16:00+08:00
**Priority**: low
**Status**: resolved
**Area**: diagnostics

### Summary
A read-only `rg` invocation passed a Windows wildcard path as a literal search path, producing an invalid filename error while the remaining searches still completed.

### Error
```
rg: src-tauri/tauri.*.conf.json: 文件名、目录名或卷标语法不正确。 (os error 123)
```

### Context
- Ripgrep accepts glob filters through `-g`; Windows does not expand this wildcard path for it.
- The command was diagnostic only and made no filesystem changes.

### Resolution
- **Resolved**: 2026-09-06T15:16:00+08:00
- **Notes**: Used explicit paths plus `-g` filters for subsequent searches.

---

## [ERR-20260906-015] mismatched-async-command-handle

**Logged**: 2026-09-06T15:19:00+08:00
**Priority**: low
**Status**: resolved
**Area**: infra

### Summary
An asynchronous test launched through an orchestration cell was first polled as a direct command session, so the polling tool rejected the identifier.

### Error
```
write_stdin failed: Unknown process id 41568
exec cell 41568 not found
```

### Context
- Orchestration cell IDs belong to `functions.wait`; direct command session IDs belong to `write_stdin`.
- The test process was not cancelled and no product or repository state was changed by the failed polls.

### Resolution
- **Resolved**: 2026-09-06T15:20:00+08:00
- **Notes**: Re-ran the long Windows engine suite as a direct command, polled its returned session ID correctly, and obtained 173 passed / 1 ignored.

---

## [ERR-20260906-016] malformed-log-filter-regex

**Logged**: 2026-09-06T15:28:00+08:00
**Priority**: low
**Status**: resolved
**Area**: diagnostics

### Summary
A JavaScript regex used only to filter downloaded CI log lines over-escaped a slash, so the wrapper failed before calling GitHub.

### Error
```
SyntaxError: Invalid regular expression flags
```

### Context
- No command ran and no local or remote state changed.
- The GitHub job itself continued unaffected.

### Resolution
- **Resolved**: 2026-09-06T15:29:00+08:00
- **Notes**: Replaced the regex with explicit lowercase substring checks, then retrieved the complete failing job tail.

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
## [ERR-20260906-017] cross-platform-fixture-unused-import

**Logged**: 2026-09-06T15:53:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
Converting Windows-only path fixtures to platform-native `PathBuf` values left an obsolete `Path` import, and the local `-D warnings` gate rejected it.

### Error
```
error: unused import: `Path`
  --> src/app/ccswitch.rs:977:21
```

### Context
- The test conversion itself passed all 305 active manager tests.
- The strict Clippy command correctly caught the stale import before the change was pushed.

### Suggested Fix
After changing test fixture types, run strict Clippy before committing and remove imports that were only needed by the old literals.

### Metadata
- Reproducible: yes
- Related Files: src-tauri/src/app/ccswitch.rs

### Resolution
- **Resolved**: 2026-09-06T15:53:00+08:00
- **Notes**: Removed the obsolete import and reran the strict validation gate.

---
## [ERR-20260906-018] powershell-rg-regex-quoting

**Logged**: 2026-09-06T15:56:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tooling

### Summary
A PowerShell `rg` scan used a double-quoted regular expression containing literal double quotes, so PowerShell parsed part of the pattern as syntax.

### Error
```
ParserError: Array index expression is missing or not valid.
```

### Context
- The read-only scan was checking standalone engine tests for Windows path literals.
- No source command ran and no files were changed by the failed invocation.

### Suggested Fix
Use a single-quoted PowerShell string for regular expressions that contain literal double quotes and backslashes.

### Metadata
- Reproducible: yes
- Related Files: crates/codex-win-engine/src

### Resolution
- **Resolved**: 2026-09-06T15:56:00+08:00
- **Notes**: Reissued the scan with a single-quoted pattern.

---
## [ERR-20260906-019] windows-verbatim-temp-parent-component

**Logged**: 2026-09-06T16:01:00+08:00
**Priority**: low
**Status**: resolved
**Area**: tests

### Summary
Canonicalizing the test temp root on Windows produced a verbatim `\\?\` path, which changed how the parent-traversal fixture exposed `..` to `Path::components`.

### Error
```
assertion failed: validate_cleanup_path(&root.join("..").join("escape")).is_err()
```

### Context
- Temp-root canonicalization is needed on macOS because `/var` is a system symlink to `/private/var`.
- It is unnecessary on Windows and made the traversal assertion less representative of ordinary Win32 paths.

### Suggested Fix
Canonicalize the temp root only on non-Windows hosts; retain `std::env::temp_dir()` on Windows.

### Metadata
- Reproducible: yes
- Related Files: crates/codex-win-engine/src/portable.rs

### Resolution
- **Resolved**: 2026-09-06T16:01:00+08:00
- **Notes**: Added target-specific temp-root handling and reran the standalone engine tests.

---

## [ERR-20260913-G23] stale-startup-guard-disabled-actions

**Logged**: 2026-09-13T02:30:00+08:00
**Priority**: high
**Status**: resolved
**Area**: windows-usb-ui

### Summary

The signed U-disk manager initially disabled both “one-click install” and “check and install update”, then enabled them only after the user waited. The backend was still performing its intentional startup integrity check; the UI had not yet consumed the new authorized status.

### Root Cause

`OfflineHome` made the final action decision from the first `usbGuardStatus()` result. Its only follow-up read was a 1-second polling timer, and the final payload probe did not trigger a fresh Guard read. `About` did not treat the explicit `starting` state as retryable.

### Fix

- Refresh the Guard after `offlinePayloadProbe()` completes.
- Poll `starting` at 250ms for a bounded window, then fall back to 1 second.
- Show a non-error startup-readiness message.
- Retry `starting` in About with a bounded 20-attempt window.
- Keep all existing signed authorization and backend fail-closed checks unchanged.

### Verification

- `npm exec vitest run src/app/views/OfflineHome.test.tsx src/app/views/About.test.tsx`
- Result: 2 test files, 31 tests passed.

### Prevention

Every asynchronous USB startup consumer must test the initial `starting` state, a later authorized transition, and the failure/timeout paths. See `docs/USB-STARTUP-READINESS-G23.md`.

---

## [ERR-20260913-G24] machine-bound-signer-cannot-be-copied

**Logged**: 2026-09-13T23:55:00+08:00
**Priority**: critical
**Status**: resolved
**Area**: usb-signing

### Summary

The previous signer depended on the current Windows user's DPAPI key. Copying only the EXE to
another computer could not sign, while copying a raw private key would have destroyed the security
boundary.

### Root Cause

The signing key had machine/user protection but no portable encrypted transfer format. A package
could be buildable and still be signable only on a different hidden machine, producing repeated
`keyId`-same-but-public-digest-different failures.

### Fix

- Rotate the canonical profile to the current T14S public key `4d140722…` and rebuild manager/
  Bundle artifacts;
- add AES-256-GCM + Argon2id `codex-usb-signing-key.vault` with create-only writes;
- add GUI native password prompt and shared in-memory password pipeline;
- add release manifest/hash support for optional vault inclusion;
- keep DPAPI only as the first-use migration source on the matching signing computer.

### Prevention

Run `tools/verify-usb-signing-pair.ps1` before delivery. Copy the complete signer folder including
the vault; never place a raw seed in EXE, Bundle, command line, environment variables, or reports.
Historical `216ae3…` and current `4d1407…` packages must remain explicitly separated.

### Verification

Rust 26 tests, GUI 3 tests, clippy with `-D warnings`, static Bundle verification, public-key pair
preflight, installer-pack audit (50 Skills/7 Agents), and cross-directory portable-vault positive/
wrong-password tests all passed.

---

## [ERR-20260914-G25] stale-signer-window-shows-old-key-binding

**Logged**: 2026-09-14T10:45:00+08:00
**Priority**: high
**Status**: resolved
**Area**: usb-signing-gui

### Summary

The same signer EXE could be opened more than once. An older window could retain a
previous inspection result (for example the historical `216ae3…` binding) while a
new window had already validated the current `4d1407…` Bundle. The user saw the
old window's disabled signing state and reported that the environment was invalid.

### Root Cause

The GUI had a transaction mutex for signing/key access, but it did not have a
process-wide GUI instance guard. The inspection state was intentionally held in
each window and was not shared or invalidated when another window started.

### Fix

- Added a named Windows GUI instance mutex.
- Added a window-class fallback check for older signer builds that do not yet
  hold the new mutex; the second launch explains that the existing window must be
  closed before opening the current release.
- Kept the fail-closed key comparison unchanged: `keyId + publicKeySha256`
  must still match before signing.

### Prevention

When a signing check appears stale, close every signer window and start exactly
one EXE from the newest release directory. Verify the displayed binding against
`tools/verify-usb-signing-pair.ps1`; do not rotate or delete the DPAPI key to
make an old Bundle appear valid.

### Verification

Current F: Bundle: deep inspection passed, `SignedRequired` binding is
`4d140722…`, local DPAPI key is `4d140722…`, and the fresh GUI startup log
reported `canSign=true`. The U 盘 was not modified during diagnosis.

---

## [ERR-20260914-G26] uninstall-preflight-rejected-missing-optional-parent

**Logged**: 2026-09-14
**Priority**: high
**Status**: resolved
**Area**: uninstall-cleanup / skill-hub-routing

### Summary

二阶段卸载器在全新用户目录上仍可能把“可选的历史 `wangnov` 目录尚未创建”误判为路径不安全，因而没有启动 finalizer。同期 Skill Hub 资源曾误带 GitCode/Gitee 派生镜像设计，给默认安装链增加了不必要的仓库、账号和发布前置条件。

### Root Cause

`canonical_or_parent` 只规范化目标的直接父目录；当多个可选父级同时不存在时，安全预检直接失败。Skill Hub 的构建期和运行期来源门禁也曾按国内镜像数量设计，和实际约定的“自有仓库 → Relay → GitHub fallback”不一致。

### Fix

- 路径安全检查向上寻找最近存在的普通目录，再按原缺失组件补回逻辑路径；不创建缺失目录、不放宽重解析点/越界检查，并新增 local/roaming `wangnov` 双缺失回归。
- 默认来源资源只保留固定 Relay 和 GitHub fallback；构建期与运行期都允许零国内源，GitCode/Gitee 派生仓库工具不再进入主链。
- 保留来源尝试、阶段、结果和脱敏诊断，Relay 失败后才进入 GitHub，下载前后继续校验同一版本与 SHA-256。

### Prevention

新机卸载回归必须覆盖可选目录不存在、目录已存在、重解析祖先和报告目录独立性；来源链变更必须同时更新资源 JSON、`build.rs`、运行时解析和契约测试。禁止把测试占位域名或新托管仓库当作默认发布前置条件。

### Verification

`cargo test --manifest-path src-tauri/Cargo.toml --locked`：400 passed，2 ignored；Skill Hub 定向测试 18 passed；前端 416 passed、1 skipped；`npm run check`、`npm run lint`、`npm run build`、Rust 格式检查和 Relay Python/契约测试均通过。公网现场检查确认 Relay metadata/archive 路由可达，网络偶发超时仍由超时与 GitHub fallback 处理。

---

## [ERR-20260915-G25] skill-hub-source-invalid-was-not-actionable

**Logged**: 2026-09-15
**Priority**: high
**Status**: fixed in source; existing packaged USB audit passed; fresh EXE rebuild pending
**Area**: skill-hub-routing / release-packaging

### Summary

Skill Hub displayed `SKILL_REPO_SOURCE_INVALID` and only “后端没有返回可公开的进一步详情”, so a user could not tell whether the local release resource, remote metadata, or the network was at fault.

### Root Cause

The runtime loader depended on one Tauri `resource_dir()` path and collapsed missing/read failures into a detail-less source error. The same stable code was also reused for remote metadata and stale apply identities. The release scripts copied the resource but did not make its presence, fixed Relay/GitHub identity, and SHA256SUMS coverage an explicit U 盘 acceptance gate.

### Fix

- Added deterministic portable/Tauri source-resource candidates and a compile-time canonical fallback used only when all external candidates are absent.
- Existing but malformed/reparse resources still fail closed with a stage/candidate/reason detail; no malformed file is silently replaced.
- Tightened `expected` source ID/kind pairing and exposed a specific confirmation-identity detail.
- The UI now displays safe backend details for all Skill Hub failures instead of unconditionally hiding every `SKILL_REPO_*` detail.
- Added build/USB verifier gates for `resources/skill_hub_sources.json`, fixed Relay/GitHub identity, and SHA256SUMS coverage; added cleanup allowlist entries for both static Skill Hub resources.

### Prevention

Every new source or relay change must be tested at three boundaries: compiled fallback, packaged EXE resource directory, and generated USB bundle. A generic source-invalid code is not enough; logs and the disclosure UI must identify the phase without exposing credentials or user paths.

### Verification

Rust 403 passed with 2 ignored (405 collected), Skill Hub 20/20, frontend 417/417 with one skipped, TypeScript,
lint, production renderer build, PowerShell parser checks, and the new verifier against the existing
g28 USB bundle passed. A live Relay probe returned metadata/archive HTTP 200 with matching size and
SHA-256, and GitHub API returned HTTP 200. A fresh signed EXE/USB physical smoke test remains a
separate release step.

---

## [ERR-20260916-G30] usb-lease-readiness-misreported-as-signature-failure

**Logged**: 2026-09-16
**Priority**: critical
**Status**: fixed in source and G30 package; target-machine smoke test pending
**Area**: USB Guard / offline Codex install

### Summary

A signed USB bundle reached `offline-payload-stage` while the persisted status
said `authorized/running`, but the live lease or `CodexUsbGuard.exe` lock was
not ready. The backend correctly rejected the protected operation; the UI
incorrectly showed “更新包签名校验失败”.

### Root Cause

`spawn()` was treated as helper readiness, `status.json` was trusted as a live
authorization snapshot, and the UI staged before doing an action-boundary
guard refresh. The error classifier also mapped lease/watchdog markers into
the cryptographic signature bucket.

### Resolution

The manager now stops stale helpers even when their bytes are unchanged, waits
up to five seconds for the helper lock, masks stale authorized snapshots,
retries a transient authorization state for a bounded three seconds, and
rechecks authorization before MSIX staging. A stable `usb_authorization` code
separates lease/watchdog readiness from real license or Authenticode failures.

### Prevention

Never use `status.json` or `spawn()` alone as proof of USB authorization. Keep
the backend gate fail-closed, test the first-click startup race, and preserve
the distinction between signature, bundle-integrity, and live-lease errors.

### Verification

Added Rust classification/status-state tests and frontend tests proving that
preflight failure does not create a staged payload. Full local regression,
G30 package verification, and signing-pair preflight pass; the physical USB
and target Windows test remain external evidence.

---

## [ERR-20260916-G33] skill-hub-watchdog-masked-rustls-provider-panic

**Logged**: 2026-09-16
**Priority**: critical
**Status**: fixed in source and G33 package; target-machine smoke test pending
**Area**: skill-hub-network / update-source-compatibility

### Summary

Skill Hub 的“检查更新”可能一直显示检查中，最后只给 renderer watchdog 超时；同时 Codex 更新会把已确认的镜像“无更新”错误覆盖成备用源不可用。修复网络客户端后，开发机真实探针又发现远端归档使用合法的多行 YAML description，旧 frontmatter 解析器仍会把有效 Skill 判为格式错误。

### Root Cause

依赖树因 Tauri updater 启用 `reqwest` 的 `rustls-no-provider`，Skill Hub 独立 client 构建前没有安装 crypto provider，`Client::build()` 直接 panic，native invoke 不一定向 renderer settle。另一个问题是 Codex Relay 生产 metadata 已升级到 protocol 2，而客户端只接受 protocol 1；Auto 聚合也把后续 Relay/GitHub 失败当成整体失败。网络恢复后，内容层的严格 frontmatter 解析又与生产 Skill Hub 的 `description: >-` 多行标量不兼容。

### Resolution

- 直接依赖并安装 Rustls `ring` provider，provider 竞争初始化可安全收敛，异常改为有阶段信息的 `SKILL_REPO_UNAVAILABLE`，禁止 panic。
- Skill Hub check/apply 读取持久化 system/direct/custom proxy，并保留有界请求/renderer 超时。
- Relay 接受 protocol 1/2；protocol 2 必须校验嵌套 release/tag/完整 assets 身份一致。
- Auto 只要有合法源证明 no-newer，就不再被备用源失败覆盖。
- frontmatter 解析器增加受限 YAML 多行 description 支持，只放宽已验证的块标量，不执行通用 YAML 或放宽归档安全边界。

### Prevention

任何升级 reqwest、Tauri updater 或 Rustls 的变更，必须先跑 client-construction 测试；任何 Relay metadata schema 变化必须用真实生产 shape 的 resolver 测试；“无更新”必须和“所有源不可用”分别验收；Skill Hub 内容契约变更必须用当前远端归档做一次 metadata、archive、复核和解析的原生探针，不能只测 HTTP 200/哈希。

### Verification

Rust 全量 `421 passed, 3 ignored`，Skill Hub 定向 21 passed，Codex 定向 42 passed；签名器 32 passed；前端全量 `421 passed, 1 skipped`，TypeScript、lint、renderer build、cargo fmt 通过。当前 Rust client 的 Relay metadata/archive/复核/解析探针通过，公网 Relay protocol 2、Skill Hub metadata/archive HTTP 200 且 archive size/SHA-256 匹配；新 EXE 与签名器配对自检通过。真实大陆网络、目标机和物理 U 盘仍待现场验收。

---

## [ERR-20260917-G34] skill-hub-windows-directory-flush-misclassified

**Logged**: 2026-09-17
**Priority**: high
**Status**: fixed in source and G34 package; physical target-machine validation pending
**Area**: skill-hub-local-persistence

### Summary

Skill Hub 远端 metadata、archive 和 verify 已成功后，Windows 本地 staging 初始化仍可能失败，界面却显示“更新包无效或已过期”。现场详情中的 `[local path]` 是前端脱敏占位符，不是网络地址。

### Root Cause

目录持久化代码使用 `CreateFileW` 的 `GENERIC_READ` 打开 NTFS 目录，再调用 `FlushFileBuffers`。Windows 对该组合返回 `ERROR_ACCESS_DENIED (5)`；同一目录使用 `GENERIC_WRITE` 可正常刷新。旧代码还把这个 OS I/O 错误包装为 `SKILL_REPO_LEDGER_INVALID`，造成错误分类和错误文案。

### Resolution

目录刷新改用 `GENERIC_WRITE`，捕获并保留 Win32 错误码；本地目录打开/flush 错误使用 `SKILL_REPO_LOCAL_IO`，前端映射为磁盘权限/空间提示并允许重试。check staging 初始化失败时做尽力清理，并增加真实 Windows 临时目录回归测试。

### Prevention

涉及目录 durability 的 Windows API 修改必须在 NTFS 临时目录上走真实 `sync_directory` 测试；不能以网络请求成功推断本地 apply 成功。`SKILL_REPO_LEDGER_INVALID` 仅用于清单语义错误，权限、句柄和磁盘错误必须使用独立本地 I/O 错误码。发布前必须从同一当前源码生成 manager、U 盘包和匹配签名器，禁止复用历史 manager/profile。

本次重打包还发现签名器构建脚本曾以 BOM-less UTF-8 保存；Windows PowerShell 5.1 按 ANSI 解析中文字符串时会把脚本判为语法错误。脚本已修为结构明确并保存为 UTF-8 BOM；以后所有发行脚本都必须在 Windows PowerShell 5.1 下执行一次 `-File` 解析冒烟。

---

## [ERR-20260917-G35] skill-hub-base-resource-uses-wrong-runtime-root

**Logged**: 2026-09-17
**Priority**: high
**Status**: fixed in source; G35 package and signer pair validation passed
**Area**: skill-hub-portable-resource-resolution

### Summary

U 盘版 Skill Hub 安装/检查仍报“基础 Skill ID 资源缺失”，并显示
`SKILL_REPO_LEDGER_INVALID`。G34 包内实际已有基础清单，说明失败发生在运行时定位而不是文件内容或远端下载。

### Root Cause

Skill Hub 来源目录已经按便携 EXE 同级资源优先解析，但 `base_manifest()` 仍只拼接
Tauri `resource_dir()`。便携入口在部分机器上得到的是本机安装/缓存资源目录，该目录没有
`skill_repo_base_manifest.json`；不存在路径又被通用 `io_error()` 误标为 ledger 无效。

### Resolution

基础清单现在按“EXE 同级 resources → EXE 同级文件 → Tauri resource_dir → 编译内置副本”解析。
外部文件存在但身份无效时 fail closed，并返回独立的
`SKILL_REPO_BASE_RESOURCE_INVALID`；所有外部候选缺失时使用 build.rs 已校验的内置副本。
安装包构建同时检查基础清单的 source/hash/ID 顺序，阻止只打包管理器而漏掉资源。

### Prevention

便携功能不能只依赖 Tauri `resource_dir()`，每个发行资源都必须有同级 EXE 候选和编译内置/明确缺失策略。
发布门禁必须检查最终输出目录，而不是只检查仓库或中间构建目录。`SKILL_REPO_LEDGER_INVALID`
只用于热更新账本语义错误；发行资源错误必须使用独立稳定错误码。
