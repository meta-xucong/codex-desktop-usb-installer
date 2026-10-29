# CC Switch 4.0.20 USB packaging boundary

This is a source-upgrade candidate, not a rebuilt Windows release.

## Pinned input

- Upstream version: 4.0.20.
- Commit: `85ad4969b3399500baaea9d46b1e08f87aba80e1`.
- USB overlay: `source-patches/ccswitch-usb-product-85ad4969.patch`.
- Current full product patch SHA256: `69B135C95A04F06CDF8ED2002E0D3224E43219AF5A101299A786A0A7966210CA`.
- The source patch SHA256 is recorded in `manifest.json` after the backend and frontend ports are exported together; any further source edit requires re-export and hash refresh.
- The existing checked-in EXE remains the previous 4.0.20 product build. Its SHA256 and source version/commit provenance remain unchanged; `pending-rebuild` records that it does not contain the current source patch.

## Build requirements

`tools/build-ccswitch-with-env-patch.ps1` requires the exact upstream commit plus the complete source patch, without additional modified/hidden input. There is no regex-based source modification after this check. `-SkipBuild` is rejected for this upgrade.

The build requires a Windows toolchain with the repository's Node/pnpm dependencies, Rust/MSVC, and Tauri prerequisites. It builds the production renderer, executes the enumerated USB regression filters, runs locked Cargo checks, then performs the production Tauri build. Filter names/counts must match the final exported source tests; a zero-test success is not acceptable.

The executable must contain both the existing nonce-bound handoff protocol and the new `CODEX_USB_V4_FIELD_WRITE_V1` marker. `CODEX_USB_FULL_IMPORT_V1` identifies the required full-import feature. Windows build gates run exactly 10 bootstrap, 17 model-selection, and 23 `usb_upgrade_` tests, all with `--test-threads=1`. The current attempt passed model-selection 17/17; bootstrap had 3/10 pass and upgrade had 17/23 pass before sandbox `PermissionDenied` blocked the remaining cases. These are required build steps, not a claim that Windows execution has already passed. These markers are stale-artifact guards, not substitutes for semantic tests. Old 3.x MCP merge markers are not required: the upgrade must retain the official 4.x key-field writer rather than porting old full-snapshot restoration.

Only a successful build stamps `artifact.buildStatus=built`, matching source version/commit and the real EXE hash. The output includes the source patch, manifest, verifier, license, provider catalog and a generated final source notice.

## Staging and whole-bundle checks

1. Rebuild the manager using the existing installer build with the complete new artifact directory. The manager-side `model_selection_client.rs` now supplies USB mode/home to the CLI selection helper; updating only CodexSwitch.exe is insufficient. Do not copy only the EXE or edit its hash manually.
2. The installer verifies the input product, stages the exact source patch with the EXE/manifest, and verifies the staged product again.
3. The one-click layout refuses to stamp new metadata over an EXE that does not match the verified canonical product. Both `ccswitch/` and `resources/ccswitch/` receive the same audited patch, manifest, source notice and verifier.
4. Whole-bundle verification checks the new marker in both executables, their equality and joint manifest hashes, then verifies each product copy independently.
5. Existing SignedRequired, signing-profile and Authenticode requirements remain unchanged. No signing keys or policies are generated or relaxed by this source upgrade.
6. Database migration and isolated Windows launch acceptance are separate requirements. Do not point the candidate at a real profile until the migration/restore test has passed; preserve a copy for rollback.

## Evidence available in the cloud

The static test is an optional developer check only, not invoked by the build script or required for packaging. Existing build and verification flows remain the acceptance path.

`node --test tools/check-ccswitch-upgrade-contract.mjs` verifies static packaging contracts against the actual source metadata and old EXE bytes. These are not PowerShell execution tests, a Windows build, signature verification, or successful launch evidence. The current cloud environment has not executed those Windows-only stages.
