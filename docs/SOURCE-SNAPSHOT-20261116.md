# USB and 404token source synchronization — 2026-11-16

This snapshot contains the source used by the `usb-update-cleanup-1116` USB release, including the manager's four-stage progress UI, host initialization and settings preservation, offline capability installation/recovery, media MCP, CUA/IAB runtime compatibility, default Terra availability, and update transaction cleanup for the tested small USB volume. The embedded capability pack is version 1.5.12; the Codex runtime compatibility target is 26.1111.21308.

It also includes the deployed 404token USB authorization service fix under `server/usb-license-revocation`: a valid but conflicting registration proof no longer poisons the global store state, and health checks and safe failure diagnostics reflect known storage failures. The engineering audit is in `USB-VPS-STATE-RECOVERY-1116-AUDIT.md`.

`tools/usb-license-signer` is synchronized from the source of the delivered `v0.5.25-preload10-layout-20261106` signer, rather than the older working copy. It retains the default preload amount of 10, adaptive layout, and the existing separation of local signing, VPS activation and account precharge. No signing, registration or charging is performed by this synchronization.

The CCSwitch resource retains its pinned upstream source notice and complete product patch. The existing CUA helper resource is included unchanged from the released package. This publication does not claim a new functional acceptance run; it preserves the already tested implementation and tests.

## Distribution inputs

The following offline archives are supplementary GitHub Release assets for this source snapshot, rather than regular Git blobs:

- `installer-pack/runtime/bundled/chrome-headless-shell-152.0.7977.30-win64.zip`
- `installer-pack/runtime/bundled/ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip`
- `installer-pack/runtime/bundled/hyperframes-0.8.134-win-x64-node-modules.zip`

Their existing release manifests and deployment code are retained. Download and restore them using `docs/OFFLINE-BUILD-INPUTS-20261116.md` before running capability-pack verification or building an offline release. Other full USB assembly inputs and the local formal signing configuration remain separately controlled as described there. This synchronization does not regenerate or weaken release integrity metadata.

Production signer executables with embedded credentials, signing vaults/private keys, account files, production databases, customer-specific diagnostic reports and transient test/release directories are not published.

The independent `codex-starter-kit` repository remains the historical standalone 1.5.4 source line. The current USB-specific 1.5.12 pack and its fixes are maintained in this repository's `installer-pack/`; regenerating it from the historical standalone builder would discard later USB fixes.
