# USB326 Audit Remediation and Verification

Audit target cited by the independent report: `326bdee7174ca23abc3832b250e44068728d2d62`.
The audited branch now includes `544e147cefb0872640712213bed46ee579f46d74` and `7ed42cd315f33e8a430df34d63a3cbd23291f46f`; remaining installer/MCP remediation is in the current working tree. This is not a claim that the original audit target commit itself contains the fixes.

## Scope and safety

Address findings F1–F11 from the pasted independent audit. No production API credentials were used, no paid generation request was sent, no deployment was performed, and no physical USB device was modified. The ZIP reproduction bundle was unavailable and was not needed for the source-level fixes or local contract tests.

## Findings and implementation

| ID | Implementation | Evidence / remaining boundary |
| --- | --- | --- |
| F1 | `install-all.ps1` accepts and forwards the media catalog fields passed by Rust to `install-to-codex.ps1`. | Python/PowerShell entry-contract test passes. |
| F2 | Model activation verifies the signed MSIX payload while allowing only the known PE console-to-GUI subsystem normalization; native probing runs a private executable copy reconstructed from the verified signed payload. | Synthetic PE positive and mutation-negative tests are in Rust suite. Exact final release MSIX/CLI PE subsystem was not available for verification. |
| F3 | Media authorization state is stored in a separate validated sidecar instead of mutating PowerShell readiness data. Status refresh re-verifies the DPAPI credential and degrades stale/failed authorization instead of reviving `ready`. | Rust capability-pack tests pass. Full Windows release install/refresh cycle remains a release test. |
| F4–F5 | Component maintenance preserves the user's current model routes. Catalog refresh restores only a configured Seedance model with its route ID; it leaves un-routed Seedance disabled and disables media when there is no compatible selected route. | React component tests pass; no live provider generation request was made. |
| F6 | Rust explicitly passes all manager-owned media model slots, including empty values, to the child install process. Removed the hard-coded Image-2 fallback. | Installer contract suite passes with stale-value fixtures. |
| F7 | MCP wrapper clears inherited endpoint/profile/key first. It accepts a complete launcher-marked signed USB tuple only when valid and, when a saved connection exists, exactly matching it; valid DPAPI credentials override that tuple. Invalid tuples fail closed. CredentialCheck rejects launcher environment credentials, and Rust strips all four values before calling it. | PowerShell runtime tests cover stale inherited data, signed tuple use, invalid URL, and DPAPI-only readiness. Independent read-only audit found no remaining credential mixing path. |
| F8 | Capability metadata uses canonical UTF-8 byte lengths, so LF and CRLF checkouts produce the same manifest metadata. | Metadata refresh and `--check` pass: 308 manifest entries and 309 checksum entries. |
| F9 | Process matching recognizes exact supported Codex image aliases, including `Codex.exe`, `CodexCore.dat`, and legacy `ChatGPT.exe`, while requiring the verified managed install root. | Focused Windows process-matcher test passed in the prior validation pass. Full engine suite has environment-specific failures described below. |
| F10 | Reset-pending intent survives marker write failure and is retried before clearing. | Fault-injection test is included in the Rust suite. |
| F11 | POSIX locking uses the two-argument `fcntl.flock(fd, operation)` API. | Python test validates the call contract; full adapter suite passes. |

## Local verification receipts

- `python installer-pack/test-adapter-contracts.py`: **59 passed, 0 failed** (latest handoff run; includes the added regression cases).
- `npm run test -- --run src/app/views/OfflineComponentsSheet.test.tsx`: **14 passed**.
- `npm run check`: passed.
- Earlier `cargo test --manifest-path src-tauri/Cargo.toml --lib`: **713 passed, 0 failed, 6 ignored**; a later local run recorded 724 passed. In the current restricted execution environment the full rerun yielded **609 passed, 115 failed, 6 ignored**, with failures concentrated in denied Windows filesystem/registry operations and blocked loopback/network fixtures. Treat the full suite as requiring an unrestricted Windows rerun before release acceptance.
- `python tools/refresh-capability-pack-metadata.py --check`: passed, 308 manifest entries / 309 checksum entries.
- `git diff --check`: passed; Git reports expected line-ending normalization warnings for the PowerShell scripts.
- Separate engine-suite validation previously reported **215 passed, 21 failed, 2 ignored**. The failures were Windows fixture/environment limitations (junction creation, restricted temp paths, distinct-volume fixture assumptions, and path encoding); the targeted process-alias regression passed. This is not represented as a fully passing engine suite.
- Independent read-only review of the F7 delta confirmed the mixed endpoint/key path is closed at source level. The reviewer did not run tests; the receipts above are from this local verification run.

## Follow-up: Codex startup failure after a custom MCP install script

The user-provided incident report identified a UTF-8 BOM as the direct `config.toml` parse failure. A source search found the same Windows PowerShell 5.1 hazard in this product's own `configure-codex.ps1` and `render-codex-config.ps1`: both wrote TOML with `Set-Content -Encoding UTF8`, while their Python checks used `utf-8-sig` and therefore accepted the BOM.

Both writers now emit UTF-8 without BOM, require a strict UTF-8/TOML parse before publishing output, and atomically replace the target only after validation. The config merger stops without modifying the existing file if no TOML validator is available or validation fails. Added regression coverage confirms both output files have no BOM, overwrite works, and invalid merged TOML leaves the previous `config.toml` byte-for-byte intact. The full installer adapter suite passes with this follow-up: **59 passed, 0 failed**.

The external script also used a USB path for a wrapper whose data root is derived from its local kit directory, and omitted the packaged Python interpreter. The product's renderer already installs the wrapper under the user-local `.codex/codex-agent-kit/bin` and defaults the interpreter from that kit's runtime; retain that route and avoid user-authored config paths that point into a removable drive.

## Release status

The source-level remediation and listed local suites pass, but this is **not release acceptance**. Still required against the exact final artifact:

1. Build the Windows release installer from a clean checkout and retain its receipt bound to the final source SHA.
2. Inspect the exact release MSIX's `resources/codex.exe` PE subsystem and hash inputs used by F2.
3. Unpack the final USB package to an isolated location and test first-launch and repeat-launch through the normal launcher and direct USB entry.
4. Verify media MCP `initialize`, `tools/list`, and `media_capabilities` using the packaged wrapper and ordinary Codex launch environment; keep live billable generation as an explicitly authorized separate acceptance step.

Until these artifact-level checks pass, do not mark the USB release as fully accepted or ready to publish.
