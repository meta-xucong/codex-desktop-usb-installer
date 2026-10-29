# Diagnostic build source map

This handoff contains the diagnostic-only release executable, collector source,
mock tests, exact MSIX static evidence, and build/authorization notes. The source
repository has pre-existing unrelated local edits, so this ZIP intentionally
does not include a broad `git diff` that would misattribute those edits to the
diagnostic task. The following implementation points are isolated by the
`usb-cdp-diagnostic` feature and `codex-usb-cdp-diagnostic` binary:

- `src-tauri/src/diagnostic_main.rs`: dedicated Cargo entry point.
- `src-tauri/tauri.diagnostic.conf.json`: separate product name, application
  identifier, and main binary identity. `lib.rs` selects this context only for
  the diagnostic feature.
- `src-tauri/src/main.rs`: compile-time refusal to build the ordinary binary
  with the diagnostic feature, including `--all-features` builds.
- `src-tauri/src/app/paths.rs`: diagnostic settings, provenance, cache, and
  theme-store roots derived from a distinct qualifier.
- `src-tauri/src/app/usb_guard.rs`: diagnostic helper argument, executable,
  lease/status directory, and explicit existing-CODEX_HOME validation.
- `src-tauri/src/app/win_elevation.rs`: separate elevated manager mutex.
- `src-tauri/src/app/win_update.rs`: loopback CDP port 9222 is selected only
  when the user initiated the Codex launch through the diagnostic manager.
- `tools/build-installer.ps1` and `tools/build-oneclick-usb-layout.ps1`: unique
  diagnostic output directories, required `SignedRequired` policy, marker and
  manifest profile. The inner manager filename remains `CodexAppManager.exe`
  because that is required by the existing USB layout contract.
- `tools/verify-oneclick-usb.ps1`: validate the diagnostic manifest marker,
  helper/state namespace, and selected existing Codex profile.

Focused Rust test results are in `TEST_RESULTS.md`. No test used a live Codex
process, credentials, a real license, or a signer vault. Signing and the
USB-bound license remain separate local release steps.
