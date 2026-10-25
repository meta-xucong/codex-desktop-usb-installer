# Codex USB basic-capabilities Dot follow-up

> **当前候选说明：**本文记录当时 follow-up 的实现和历史测试数字。它们早于最新 Node REPL 迁移候选，不能直接作为该候选的全量测试证明；77/11 定向结果若无原始日志和源码/二进制指纹，按未核实报告登记。后续修复顺序和验收状态以[基础能力修复开发规范](CODEX-USB-BASIC-CAPABILITIES-REPAIR-PLAN-20261025.md)及[证据台账](CODEX-USB-BASIC-CAPABILITIES-EVIDENCE-LEDGER-20261025.md)为准。旧模板匹配手工 `node_repl_disabled=true` 也移除，属于用户已知接受的当前策略，不是字段来源识别。

## Scope and decisions

This follow-up independently verifies Dot's 2026-10-25 audit and closes only
code-level issues that are supported by the current source and baseline test
evidence.

Confirmed scope:

1. Use one process-local USB launch-environment builder for ordinary Codex and
   CDP/theme launches. USB launches must receive the managed Python `bin`
   directory (when present), `PYTHONUTF8=1`, `PYTHONIOENCODING=utf-8`, and the
   existing media-MCP launch environment. Non-USB launches receive no USB-only
   environment.
2. Legacy model-catalog normalization repairs only the exact legacy template:
   `description`, `base_instructions`, and `input_modalities=["text"]` must
   match, and `model_messages` must be absent. It removes
   `node_repl_disabled` only when its value is boolean `true`, preserving
   `false` and a missing value. A manually chosen `true` on an exact-template
   entry is indistinguishable from the old manager default and is also removed.
   Newly added entries continue to omit the field and inherit Codex's native
   default. Catalog metadata migration does not prove Node REPL was injected
   into the current session.
3. The uninstall product contract follows the user's explicit instruction to
   remove Codex completely. After the existing two confirmations, the UI asks
   the backend to remove local Codex, CCSwitch, and managed capability data;
   it preserves the USB installation kit. The UI already implements this
   contract. Update stale documentation and tests; do not expand deletion
   targets or perform real user-data deletion.
4. Re-run the failing release-workflow test with a recorded result. The prior
   failure is not classified as an environment restriction until a rerun
   confirms the boundary.

## Boundaries

- Do not alter MCP registration/configuration, model probing, browser or
  cross-session integration, Kimi behavior, or the uninstall deletion
  allowlist.
- Do not remove or skip failing tests to manufacture a green result. Rewrite
  stale uninstall assertions to cover the current two-confirmation/full-local-
  purge contract and retain ancillary failure/retry coverage.
- Use isolated temporary directories for tests. Never invoke uninstall against
  the current user's profile or the USB drive.
- Preserve all pre-existing unrelated working-tree changes and scratch data.

## Baseline evidence

- Branch: `codex/windows-usb-ci-scope-20261016`.
- Baseline commit: `07904e486262e697cef181ea1cabb85ebbaecb4a`.
- Baseline focused run: `vitest run src/app/views/Uninstall.test.tsx scripts/release-workflow.test.mjs --reporter=verbose`.
- Captured output: `.test-tmp/dot-audit-baseline-20261025/uninstall-release-verbose.log`.
- Baseline result: 28 failed, 17 passed across the two files. Twenty-seven
  uninstall failures are tied to removed advanced/keep-data controls, old
  one-confirmation assumptions, or the changed displayed data-path text. The
  release test failed when Git for Windows could not create an MSYS shared
  object (`NtCreateDirectoryObject ... 0xC0000022`); this requires an isolated
  rerun before classifying it.
- Existing unrelated dirty files and scratch directories were present before
  this follow-up and remain out of scope.

## Implementation and acceptance

- Add a shared launch-environment helper and regression coverage for USB with
  and without a managed Python directory, plus non-USB isolation.
- Match the existing migration behavior: clear exact-template `true`, retain
  `false` and missing values, and keep migration idempotent.
- Align uninstall cleanup docs/API comments with the two-confirmation policy
  and update all stale UI tests without reducing meaningful behavior coverage.
- Run focused frontend tests, broader frontend checks, focused Rust tests, and
  formatting/diff checks where the local Windows environment permits.
- Perform a separate read-only audit of the final frozen diff. Record any
  limitation honestly; real USB-machine behavior remains a separate end-to-end
  acceptance boundary.

## Results

### Changes

- `win_update::codex_launch_environment` is now the single process-local USB
  environment builder. Ordinary and CDP/theme launch paths both call it.
  Non-USB calls return an empty override list. For USB calls, it retains the
  prior media environment and adds the managed Python path/UTF-8 variables
  whenever `CODEX_HOME/codex-agent-kit/bin` exists.
- Legacy model normalization removes `node_repl_disabled=true` only for the
  exact legacy template, retaining `false` and missing values. Tests cover the
  migration cases and idempotence; catalog migration still preserves
  `config.toml` and MCP bytes.
- Uninstall UI and backend behavior were already consistent with the user's
  explicit full-local-cleanup decision. Updated stale documentation/comments
  and replaced old UI-switch tests with assertions for the two confirmations,
  complete local Codex/CCSwitch scope, cancellation, USB-kit retention, and
  retry/error behavior. No real user-data deletion was run.
- Hardened the Skill Hub test-only HTTP fixture: it exits if no initial request
  arrives within a bounded period, and accepted sockets are explicitly switched
  to blocking mode before reading. Request-header reads have a five-second
  absolute deadline. Regression tests cover both no initial request and an
  incomplete header. This changes test reliability only, not the Skill Hub
  client.
- Made the Windows process-scope test tolerate a transient Win32 sharing
  violation while removing its unique temporary fixture directory. Retries are
  limited to error 32 and stop after five seconds; process-scope assertions are
  unchanged.
- Updated the OfflineHome tests to check the accessible `status` region because
  the same MCP status wording is also present in expanded installation details.
  Fixed the theme-runtime JSDOM fixture to disconnect its observers before
  closing each window, avoiding teardown-only asynchronous error output.

### Validation

- Focused React uninstall suite: **30 passed**.
- Release workflow suite: **13 passed** when run outside the Windows sandbox.
  The fixture uses a local bare Git repository; no remote repository was
  contacted.
- Final full frontend suite outside the restrictive sandbox, with the bundled
  Python runtime selected for the syntax check: **41 files passed, 478 tests
  passed, 0 skipped, 0 failed**. Captured output:
  `.test-tmp/dot-audit-followup-20261025/vitest-full-final.log`.
- The earlier sandbox-only release fixture failure (`NtCreateDirectoryObject
  ... 0xC0000022`) did not reproduce outside the sandbox; the release workflow
  tests are included in the final passing run. No remote Git repository was
  contacted.
- `npm run check`, `npm run lint`, and `npm run build`: passed. Build emitted
  the existing large-chunk advisory and renderer-entry verification passed.
- Focused Rust `app::win_update::tests`: **55 passed**; focused Rust
  `app::model_catalog::tests`: **74 passed**.
- Final full Rust library suite outside the restrictive sandbox:
  **796 passed, 0 failed, 6 ignored**, finished in 24.48 seconds. Captured
  output: `.test-tmp/dot-audit-followup-20261025/rust-lib-full-final.log`.
  The six ignored cases comprise two compiled-USB-shim tests, one live
  Skill-Hub relay test, one InternalSigned-bundle test, one opt-in release hash
  audit, and one USB revocation E2E test requiring Go; they are not test
  failures.
- The earlier sandbox run showed 25 failed entries and five local HTTP tests
  still running after 60 seconds, but was interrupted before panic summaries.
  A full Windows run and subsequent targeted reruns isolated test-harness
  issues: the Skill Hub retry fixture had no pre-request timeout and did not
  force accepted sockets into blocking mode; a process-scope test could hit a
  transient sharing violation during fixture cleanup. Both are now bounded and
  covered. After these changes, the full Rust suite passes; the earlier log is
  retained as `.test-tmp/dot-audit-followup-20261025/rust-lib-full.log`.
- The Skill Hub module passed **36 tests, 0 failed, 1 ignored** after the fixture
  change. The Windows process-scope test passed three consecutive isolated
  runs. OfflineHome passed **44/44**, and the JSDOM teardown tests passed
  **2/2** without the prior observer error stacks.
- `rustfmt --edition 2021 --check` over both touched Rust files: passed.
- `git diff --check`: passed.

### Audit and remaining verification boundary

- Independent read-only audit first found one stale sentence in the uninstall
  cleanup document suggesting USB-kit deletion could be enabled. That sentence
  was corrected; a second independent read-only audit confirmed consistency
  with the backend rejection and the UI's `removeUsbMedia=false` request. The
  scoped code and document audit is **PASS**. A continuation audit then found
  that the Skill Hub fixture lacked a deadline after accepting a partial
  request; this was fixed with the five-second absolute header deadline and a
  regression test. Final independent re-review: **PASS**. The reviewer
  confirmed the deadline is absolute across fragmented reads, the incomplete-
  header test covers timely worker exit, and the saved test counts match this
  report.
- Audit model/effort routing is `ROUTE_UNVERIFIED`; the agent tool did not
  expose actual model/effort provenance.
- No USB Codex profile or managed Python runtime was available for a real
  desktop launch. The shared CDP environment path is covered by code review
  and unit tests, but must still be exercised on a USB test machine.
- This follow-up does not claim to resolve end-to-end MCP discovery, Browser or
  cross-session tool injection, Kimi behavior, or other test-machine-only
  capability questions.

### Theme restart catalog-preflight correction (2026-10-25)

- Windows CDP/theme relaunch now uses the existing USB model-catalog preflight
  before starting Codex and clears its reload marker only after launch succeeds.
  A preflight or launch error leaves the marker pending. This covers both
  direct CDP theme transaction paths through their shared launch function; it
  does not change Codex configuration or MCP registration.
- Reported command `cargo test --locked --manifest-path src-tauri/Cargo.toml
  --lib usb_theme_launch_gate_tests -- --nocapture`: **5 passed**. The saved
  `theme-focused-final.log` shows the five named tests and result, but does not
  record the invocation line or source fingerprint. This includes
  preflight-before-launch ordering, preflight failure blocking launch, and
  marker retention when launch fails. `rustfmt --edition 2021 --check
  src-tauri/src/app/codex_theme.rs` is reported as passed; the saved
  `rustfmt.log` is empty, so this audit could not independently verify its
  invocation or exit status.
- Related catalog-reload and route-selection tests: **7 passed**.
- The raw `rust-lib-full.log` records 701 passed, 100 failed, and 6 ignored;
  its `running 808 tests` line differs by one from the final count (which sums
  to 807). The raw `rust-lib-full-workspace-temp.log` records 770 passed, 31
  failed, and 6 ignored (807 total), while the earlier narrative reported 771
  passed. Preserve this discrepancy; do not treat the narrative count as the
  raw result. The final `rust-lib-final.log` records 802 passed, 0 failed, and
  6 ignored (808 total). The saved logs do not include a reproducible command
  line or source/test-binary fingerprint, so these results cannot be bound to
  a release candidate. The two earlier failed attempts remain part of the
  record and are not erased by the final successful run.
- No real USB Codex session was available for desktop end-to-end verification.
