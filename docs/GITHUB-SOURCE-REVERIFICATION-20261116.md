# GitHub source re-verification — 2026-11-16

## Finding and correction

The first publication (`774a6e0`) included the delivered signer's updated `gui.rs` but omitted its newly added `src/gui_layout.rs`. The GUI declares that module explicitly, so a fresh checkout was incomplete even though the delivered signer executable was unaffected. This follow-up adds the exact layout source from the tested `v0.5.25-preload10-layout-20261106` signer. It changes no signing, account, precharge, activation or product runtime behavior.

The recheck inventories the complete signer source directory, including newly added files, rather than relying only on the files previously tracked in the older working copy.

## Comparison scope

GitHub `main` was fetched and its actual Git blob contents were compared against the development source and the complete `usb-update-cleanup-1116` distribution. Text uses the repository's CRLF/LF normalization; executable resources are compared byte for byte. No new SHA/checksum mechanism was used.

- 937 tracked development product files match; no differences or missing files.
- 36 independently added source/test files match; no differences or missing files.
- All 20 delivered signer source/manifest/documentation files match after adding the layout module. Its three shared license-crate files also match the delivered signer's source inputs.
- 545 capability-pack files match the released pack. The remaining three files are the intentionally excluded offline distribution archives listed below, not unidentified omissions.
- The published CUA v3 helper and CCSwitch executable match their released resources byte for byte.

| Area | Checked implementation/evidence |
| --- | --- |
| Signer preload and layout | `gui.rs` initializes the preload field with `10`; the delivered signing/readback/precharge sequence is retained; `gui_layout.rs` is present and identical to the tested source. |
| Offline installation | The released `provision-runtime.ps1`, dependency metadata and offline deployment scripts match, including Python/venv/ensurepip, Node, browser, FFmpeg and HyperFrames paths. |
| Interrupted installation/update recovery | Runtime recovery scripts, `portable.rs`, `install_tx.rs`, update call sites and their regression files match the tested development source. |
| CUA / IAB | Old-version and 26.1111 runtime compatibility code is retained; the actual CUA helper matches the distribution; the IAB owner/tab/visibility changes are present. |
| Media MCP | The published implementation matches the distribution, including `phase-trace-v2-catalog-v1`, safe catalog diagnostics and runtime discovery guidance. |
| VPS authorization | All service source matches the deployed-fix development tree, including request-local registration conflicts, known-state health responses, safe error categories and `state_health_test.go`. |

## Checks run for this re-verification

- Signer GUI entry and referenced modules parse successfully with the existing Rust formatter in stdout-only mode; no source formatting changes were applied.
- The existing layout regression passes its five-resolution by four-scaling matrix.
- Nine 26.1111 runtime patch tests pass. One opt-in real-bundle syntax test is skipped because no real-bundle fixture was supplied to this source-only run; it is not reported as passed.
- The runtime-guidance check passes all 13 example parses and its simulated discovery/result-handling cases.
- Existing VPS Windows and Linux acceptance logs each contain 98 passing test/subtest entries and zero failing entries. These are retained results from the prior fix, not new production requests or a new deployment.

This is source/resource consistency verification plus targeted regression. It does not claim a new desktop CUA/IAB end-to-end run, clean-machine installation or signing/recharge operation.

## Distribution boundary

The source checkout deliberately excludes these release inputs:

- `installer-pack/runtime/bundled/chrome-headless-shell-152.0.7977.30-win64.zip`
- `installer-pack/runtime/bundled/ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip`
- `installer-pack/runtime/bundled/hyperframes-0.8.134-win-x64-node-modules.zip`

They remain in the full offline USB distribution. As with the existing signed MSIX/WebView2 inputs, restore the exact distribution artifacts before a full offline rebuild. Their existing manifests are unchanged. A Git clone by itself is not a complete offline release.

The standalone `codex-starter-kit` repository is the historical 1.5.4 line; the current USB-specific 1.5.12 code is this repository's `installer-pack/`. The inherited pack source-provenance fields are not a claim that the standalone builder contains these later USB fixes. Do not regenerate the current USB pack from that older builder.

Private credentials, signing vaults, embedded-credential signer executables, production databases and customer records remain outside publication.
