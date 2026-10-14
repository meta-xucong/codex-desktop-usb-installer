# Uninstall finalization handshake repair — 2026-10-14

## Frozen task contract

- Goal: prevent the confirmed primary Codex/CCSwitch uninstall from being reported as an opaque partial failure when the detached cleanup helper does not reach its ready handshake; retain a useful report and avoid orphan helper processes.
- Baseline: `2d5b1d2` (`codex/usb-fixes-20261004`). The working tree already contains unrelated PowerShell BOM, MCP, packaging, and documentation edits; preserve them.
- Evidence: the test-machine report remains `awaitingManagerExit` / `prepare` / `helperFinalization=pending`; the manager log says the primary Portable Codex uninstall and CCSwitch uninstall completed, then the helper timed out after 15 seconds. No helper-specific error was captured.
- Most likely handoff failure mechanism: before this repair, any helper error called `show_summary()`, which could keep the detached helper blocked in a modal notification for up to 60 seconds while its stdout/stderr were null. The manager watched only for `ready.json` for 15 seconds. This matches the observed alive-but-not-ready timeout and explains why the JSON report stayed at its initial state. The exact validation error on that test machine is no longer recoverable from its report.
- Causal boundary: `src-tauri/src/app/uninstall_cleanup.rs` is unchanged between `7afbad8` and the current baseline. The BOM changes affect PowerShell source files/build validation; the report fails before the detached PowerShell finalizer is started. They are not a demonstrated cause. The modal/short-handshake interaction is a source-backed likely mechanism, but it does not reveal the first validation error that occurred on the test machine.
- Non-goals: do not rerun uninstall, touch a test machine, alter model/MCP behavior, roll back the existing PowerShell encoding work, or remove any existing user data.
- Allowed scope: `src-tauri/src/app/uninstall_cleanup.rs`, focused tests in that module, and this design/verification note.
- Safety invariant: only the exact spawned helper process may be terminated; helper cleanup/deletion is never committed unless the manager validates the nonce/PIDs and writes `commit.json`. On any pre-commit failure, preserve the main uninstall result and record a terminal cleanup failure.
- Acceptance: source audit of every spawned-helper exit path, targeted tests for terminal failure reporting and no-commit behavior, Rust formatting/compile or focused Windows-target verification available in this environment, and a separate read-only audit bound to the frozen diff.
- Risk: I2 (cross-process handoff, timeout and recovery states); A2 (destructive cleanup protocol and report correctness). Design ambiguity D0: the safe response to missing readiness is to terminate only the exact child process, never commit cleanup, and record the failure.

## Implementation design

1. Replace the 15-second magic wait with a named, bounded 60-second readiness deadline suitable for slower Windows startup/security scanning.
2. Add a child-process RAII guard. It terminates only the exact helper child on pre-commit returns and waits no longer than three seconds for exit confirmation. If termination is not confirmed, preserve the helper directory rather than deleting files that a child may still have open. Successful validated handoff disarms both guards.
3. Have pre-ready helper failures publish a small nonce/PID-bound diagnostic marker in the generated helper directory and exit without a detached modal dialog. The parent consumes that marker and updates the durable uninstall report. The marker has an atomic staging name and is included in all exact helper cleanup allow-lists.
4. Publish an atomic, nonce/PID-bound helper phase marker (`discoveringRoots` through `publishingReady`). Include the last validated phase in startup errors, early-exit reports, and timeout reports. It is diagnostic only and grants no cleanup authority.
5. Record spawn failure, helper error, early child exit, invalid handshake, commit failure, and timeout as terminal report states. Do not leave `awaitingManagerExit` after a failed handoff.
6. Keep the existing explicit commit gate unchanged: no `commit.json`, no second-stage target deletion.

## Verification record

- Implemented in `src-tauri/src/app/uninstall_cleanup.rs`; no PowerShell BOM files, model/MCP sources, installation data, or test-machine files were changed.
- Focused Rust tests passed for terminal handoff failure recording, nonce/PID-bound startup-error and phase parsing, exact-child termination, helper RAII cleanup, and the Windows PowerShell finalizer parser.
- Full `app::uninstall_cleanup::tests::` run after the no-commit gate test: 41 passed / 10 failed. Failures in this sandbox included Windows access-denied errors in pinned-file tests, atomic report replacement, and synthetic finalizer runs exiting before readiness. These failures were not all independently reproduced on a less restricted Windows environment, so they remain an open verification limit rather than being labeled baseline failures.
- `rustfmt --check` on the changed Rust module and `git diff --check` passed. The crate's test target compiled successfully.
- No physical-device uninstall was run. The supplied BOM fix is not on the failing pre-PowerShell startup path; this task does not claim it caused the regression.
- A separate static audit of the prior implementation identified that the initial report alone could not reveal which helper startup stage stalled. This revision adds live stage evidence. The audit must be repeated against the final source hash; real Windows/device handoff acceptance remains a separate release check.
- Independent audit of final revision: pending.
