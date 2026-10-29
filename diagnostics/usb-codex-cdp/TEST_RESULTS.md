# Test results

Date: 2026-10-28. Node.js v24.14.1 on Windows.

Commands:

```powershell
node --check diagnostics\usb-codex-cdp\collector.mjs
node --check diagnostics\usb-codex-cdp\verify-asar-renderer.mjs
node --test diagnostics\usb-codex-cdp\collector.test.mjs
.\diagnostics\usb-codex-cdp\verify-msix-renderer.ps1 -MsixPath <exact-msix-path>
```

All syntax checks passed. The collector suite has 8 passing tests: exact marker positioning in the mock CDP fixture; Codex target and loopback filtering; whitelist normalization; foreign pause not resumed; real WebSocket mock attach/pause/resume flow; disconnect handling; timeout reported as `not_observed`; and shifted `actualLocation` rejection.

The PowerShell verifier was run against the exact MSIX recorded in `MSIX_RENDERER_EVIDENCE.md`. It confirmed package SHA/version, internal app version, renderer SHA, unique guard and dispatch-return markers, and static source positions. This is exact-package static evidence, not a live V8 breakpoint-resolution test.

Windows diagnostic target build:

```powershell
$profile = Get-Content release\usb-oneclick-v0.5.25-uac-profile-alignment-20261027-r2\verification\usb-signing-release-profile.json -Raw | ConvertFrom-Json
$env:CODEX_USB_MANAGER = '1'
$env:CODEX_USB_GUARD_POLICY = 'SignedRequired'
$env:CODEX_USB_GUARD_PUBLIC_KEY_ID = $profile.keyId
$env:CODEX_USB_GUARD_PUBLIC_KEY_B64 = $profile.publicKeyB64
$env:CODEX_USB_REVOCATION_URL = 'https://license.404token.xyz/'
$env:CODEX_USB_REVOCATION_KEYRING_JSON = '{"keys":{"revocation-20261010":"9BOdJLfX3HDjXnw+stH+TB44YNwV/RDsmyPiGbmJl8c="}}'
cargo build --release --manifest-path src-tauri\Cargo.toml --bin codex-usb-cdp-diagnostic --features usb-cdp-diagnostic --target x86_64-pc-windows-msvc
```

The release build completed after diagnostic path isolation. The EXE was copied to `diagnostics\usb-codex-cdp\build\CodexUsbCdpDiagnostic.exe` (27,859,456 bytes; SHA-256 `7DBEE4A5D6B4CD5D8426542F6385FC7F15575C970ECEA0DF206CD0FE4B6DEE89`). Authenticode status is `NotSigned`; this EXE is not a runnable USB package.

Focused Rust checks after the isolation fixes:

```powershell
cargo check --manifest-path src-tauri\Cargo.toml --bin codex-app-manager
cargo check --manifest-path src-tauri\Cargo.toml --bin codex-usb-cdp-diagnostic --features usb-cdp-diagnostic
cargo test --manifest-path src-tauri\Cargo.toml --lib --features usb-cdp-diagnostic diagnostic
cargo test --manifest-path src-tauri\Cargo.toml --lib --features usb-cdp-diagnostic signed_required_unsigned_license_clears_lease_without_stopping_manager
cargo test --manifest-path src-tauri\Cargo.toml --lib guard_artifacts_and_state_directory_use_the_diagnostic_namespace_only_in_diagnostic_builds
cargo test --manifest-path src-tauri\Cargo.toml --lib remote_debugging_is_only_enabled_for_an_explicit_diagnostic_codex_launch
cargo check --manifest-path src-tauri\Cargo.toml --all-features --bin codex-app-manager
```

The normal target and diagnostic target checks passed. Diagnostic-filtered library tests passed 10/10; SignedRequired's unsigned-license rejection fixture passed 1/1; ordinary guard namespace test passed 1/1; normal-build port default test passed 1/1 (`None` unless the diagnostic launch branch is selected). The `--all-features` normal binary check failed at the intentional compile-time guard in `src-tauri/src/main.rs`, preventing the ordinary entry point from inheriting diagnostic behavior. PowerShell parser checks passed for `build-installer.ps1`, `build-oneclick-usb-layout.ps1`, and `verify-oneclick-usb.ps1`.

Collector behavior remains unchanged from its 8/8 approved local loopback mock run. `cargo fmt` ran after the final Rust edits; the focused Rust tests and PowerShell parser checks passed.

Not performed: launching Codex, opening a real CDP port, attaching to MSI-15, observing runtime values, verifying live V8 `actualLocation`, confirming main-process acceptance, constructing/signing a diagnostic USB bundle, or accessing a signer vault/private key.

## Lint repair verification (2026-10-28)

The initial repository `npm run lint` produced 53 errors in the three diagnostic Node scripts. The following checks passed after the minimal lint repair:

```powershell
npm run check
npm run lint
node --check diagnostics\usb-codex-cdp\collector.mjs
node --check diagnostics\usb-codex-cdp\collector.test.mjs
node --check diagnostics\usb-codex-cdp\verify-asar-renderer.mjs
node --test diagnostics\usb-codex-cdp\collector.test.mjs
.\diagnostics\usb-codex-cdp\verify-msix-renderer.ps1 -MsixPath 'D:\AI\AGENT_U盘\.codex-browser-availability-diagnostic-20261025\release\browser-diagnostic-20261025-1554\payload\OpenAI.Codex_x64.msix'
git diff --check
```

Collector mock results: 8 passed, 0 failed. Exact MSIX static verification passed for package SHA/version, internal app version, renderer SHA, unique guard and dispatch-return markers, and source positions. `git diff --check` passed.

The lint fix only adds Node globals for the three standalone diagnostic `.mjs` files, removes an unused regex constant and test import, converts a no-op ternary into an explicit branch, removes unnecessary escapes from a template literal, and documents intentionally empty error/cleanup handlers. No collector protocol, target selection, whitelist, breakpoint, or cleanup behavior was changed. The EXE was not rebuilt; its SHA-256 remains `7DBEE4A5D6B4CD5D8426542F6385FC7F15575C970ECEA0DF206CD0FE4B6DEE89`.
