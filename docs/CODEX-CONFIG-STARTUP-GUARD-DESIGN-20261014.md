# Codex config startup guard — design and implementation scope

## Frozen task contract

**Goal:** prevent the installer’s Windows Codex launch path from silently failing when an MCP/config script has left `~/.codex/config.toml` with a UTF-8 BOM or malformed TOML. Repair only the BOM case when the remainder parses; otherwise preserve the file and return an actionable error before starting or altering Codex.

**Baseline:** `176b03def5097b94196c21e1e926d1af08f25e59` at task start. The existing dirty worktree from the earlier USB326 remediation is pre-existing task state and must be preserved.

**D/I/A:** D0 (the user accepted a startup preflight with a specific safe BOM-only repair rule); I2 (the guard crosses startup routes, file backup/atomic recovery, and failure handling); A1 (persistent user config and launch integrity).

**Allowed implementation boundary:** add one config guard module, register it in `app/mod.rs`, invoke it from the Windows manager launch command and the shared theme-restart command boundary before theme/update/startup mutations, add focused unit tests, and update this design with exact test receipts. Do not modify unrelated launch policy, providers, models, licensing, MCP behavior, or existing user config except the narrowly proven UTF-8 BOM repair.

## Behavior contract

1. Missing `config.toml`: allow startup unchanged.
2. Valid UTF-8 TOML without BOM: allow startup without writing the file.
3. UTF-8 BOM + valid TOML after removing exactly the three BOM bytes: create a timestamped byte-for-byte backup, atomically write the BOM-free bytes, re-read and parse the result, then allow startup.
4. Malformed TOML, invalid UTF-8, non-regular input, a BOM-bearing symlink, or a file changed during repair: do not replace it and do not launch Codex. Return a concise error with the config path and reason; preserve the original bytes. A valid symlink without BOM remains read-only and is allowed. If a backup was made before a race is detected, retain and identify it.
5. If post-write validation fails, restore the exact preimage atomically only after confirming the file still contains the exact bytes written by this repair. If the file is unreadable or has since changed, do not overwrite it; retain the backup and report the recovery path. This narrows the race but cannot serialize arbitrary external writers that do not participate in the installer's lock.
6. Run before any manager launch path can close an existing process, update settings, patch the USB payload, or spawn Codex. The USB direct-entry IPC already routes through `commands::win_launch_codex`. Theme try-on with restart, apply, and full-off also restart Codex through a separate command path; they share `begin_theme_restart_guard`, which now runs the same preflight before theme operations. Keep both boundaries covered.

## Reuse and code map

- `src-tauri/src/commands.rs::win_launch_codex`: shared Windows manager/direct-entry boundary.
- `src-tauri/src/commands.rs::begin_theme_restart_guard`: shared boundary for theme try-on restart, apply, and full-off before process/config mutations.
- `src-tauri/src/app/mod.rs`: module registration.
- `src-tauri/src/app/paths.rs::codex_home_dir`: resolve the existing user Codex config location.
- `src-tauri/src/app/atomic_file.rs::write_atomic_preserving_backup`: existing atomic replacement primitive; use it for the normalized bytes and rollback. Do not invent another replace protocol.
- Existing `toml = 0.8.2` dependency: strict TOML parser; no new dependency.
- Existing installer adapter tests already cover no-BOM PowerShell writes. This change adds Rust startup-recovery tests rather than duplicating that coverage.

## Acceptance tests

- absent config and valid config are read-only success cases;
- BOM + valid TOML is backed up exactly, normalized, reparsed, and succeeds;
- BOM + invalid TOML, invalid UTF-8, and malformed TOML without BOM fail without changing original bytes;
- conditional rollback refuses to overwrite a later external write;
- post-write/restore behavior stays fail-closed;
- `git diff --check` and focused Tauri Rust tests pass.

## Implementation and verification receipt

- The manager launch command and shared theme restart boundary both invoke the guard after acquiring the launch operation lock and before any Codex/theme mutation.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib app::codex_config_guard::tests`: **6 passed, 0 failed** on Windows (re-run in this handoff).
- A prior local run recorded the full library suite as **724 passed, 0 failed, 6 ignored**. A fresh rerun in the current restricted execution environment produced **609 passed, 115 failed, 6 ignored**; failures include Windows access-denied filesystem/registry fixtures and loopback/network fixtures. This rerun does not establish a product regression, but the full suite is not currently verified in this environment.
- Current handoff checks: adapter contracts **59 passed**; `OfflineComponentsSheet` **14 passed**; `npm run check` passed; capability metadata `--check` passed (308 manifest entries / 309 checksum entries).
- `git diff --check`: passed (Git emitted only existing LF-to-CRLF working-copy warnings for installer PowerShell files).
- Independent read-only audit: **PASS** after fixing theme restart coverage and conditional rollback. Auditor confirmed the documented test receipts against source conditions; the auditor did not rerun tests.
- A re-entrant external writer can still race in the narrow interval between the final byte comparison and atomic replacement; no arbitrary external editor/script can be serialized unless it cooperates with a shared lock. On detecting a later write during post-write validation, rollback is now conditional and preserves the newer content.
- At the initial implementation handoff, no installer/USB package had been built. A later validation pass is recorded below; it supersedes that statement only for the local signed-policy candidate package, not for a physically licensed USB device.

## Release boundary

This guard protects manager, theme-restart, and USB direct-entry launch flows. A bare external Codex executable started outside the manager cannot run this preflight. It is still protected from installer-generated BOMs by the no-BOM, validate-before-atomic-replace writers in `configure-codex.ps1` and `render-codex-config.ps1`. Final Windows installer/USB artifact and physical-device launch acceptance remain separate release gates.

## Follow-up validation receipt — 2026-10-14

This section records a later local validation pass against the current source tree and a `SignedRequired` build candidate. It does not change the implementation contract above.

- Fixed a Windows PowerShell 5.1 build-script parsing issue: `tools/build-installer.ps1` contained Chinese UTF-8 text but lacked a UTF-8 BOM, so Windows PowerShell 5.1 could parse it using the active legacy code page. The script now has a UTF-8 BOM. A standard `powershell.exe -File ...` invocation reached normal parameter binding (with an intentionally invalid parameter) instead of failing at parse time.
- Built local candidates for installer `0.5.24` and USB one-click layout using the production `SignedRequired` policy profile `g42-media-ui-20261007` / key id `prod-2026-10-03`. Candidate paths: `release/setup-v0.5.24-signed-validation-20261014/` and `release/usb-oneclick-signed-validation-20261014/`.
- `tools/verify-oneclick-usb.ps1` passed for the signed-policy candidate with `-SkipNetwork`; payload hash, VC++/WebView2/CCSwitch assets, manager policy and execution manifests, and capability-pack manifest/checksums verified. This was package verification, not a production license issuance or physical USB signing operation.
- `tools/test-offline-bundle.ps1` passed after redirecting Windows temp storage into the repository's writable test directory: 339 extracted files and 338 SHA entries. The test bundle used a synthetic MSIX and did not prove Authenticode verification or a real Codex GUI launch.
- The packaged `media-generation-mcp.py --healthcheck` returned `ready: true`, runtime/protocol passed, Python 3.12.10, UTF-8, and six tools. It explicitly reported `gatewayCatalogStatus: not_checked` and `generationCapability: not_tested`; no live gateway request or media generation was performed.
- Fresh focused regression runs passed: Rust modules `model_catalog` (94), `model_native_probe` (21), `media_generation` (3), `capability_pack` (10), and `codex_config_guard` (6); adapter contracts (59); `OfflineComponentsSheet` (14); `npm run check`; and capability metadata check (308 manifest entries / 309 checksum entries). The focused model-catalog tests cover append-without-dropping-existing-models, first-add native model seeding, and the verified-snapshot / single-observation hash paths.
- A fresh unrestricted library-suite attempt in the current sandbox ended with 609 passed, 115 failed, and 6 ignored. Failures were concentrated in Windows filesystem/registry access and loopback/network fixture restrictions. This environment run is not a full-suite PASS; the focused affected modules above passed.
- `git diff --check` passed. The candidate packages were built locally and have not been tested on a physical USB drive. No real license activation, Codex GUI cold start, online model addition, API-key injection into a live profile, or real image/video generation was performed in this validation pass.

### Validation conclusion

The earlier source-level startup/config guard and model-catalog fixes have passing focused regression coverage, and a `SignedRequired` local candidate package passes static package verification. The evidence is sufficient to proceed to manual release acceptance, but not to claim final production release acceptance: the full Rust suite remains sandbox-limited, and real-device startup plus online Codex/MCP operation still need an environment that can exercise them.
