# Log 12 repair plan (2026-10-22)

## Scope

Fix only the four reported paths: remove the empty per-model information card; make the capability-pack USB full-access switch reach the final installer; keep model Add within its short-lived proof lifetime while allowing slow removable-media verification; and make the signed USB 404token provider become the active Codex provider only after its key has been persisted and its live configuration switch succeeds.

## Evidence and root causes

- `codex-app-manager(12).log` reports `NamedParameterNotFound,install-all.ps1` for `EnableFullAccess`. Rust invokes `install-all.ps1` with that switch, but the wrapper neither declares nor forwards it to `install-to-codex.ps1`.
- Model Add fails in `release-cli-member-hash`. This stage hashes the CLI member from the already staged, Authenticode-verified MSIX; verification must remain intact. The Add budget was 180 seconds, and the budget is shared by gateway revalidation and local native verification. Raise the maximum Add budget to 270 seconds, keep the existing 190-second minimum proof-age check, then cap the operation deadline to `min(270s, current_proof_remaining - 10s)`. This reserves 10 seconds for commit without forcing the user to click within 20 seconds after a successful test. If proof life is insufficient, fail as retryable/expired before starting any catalog write. Do not skip or weaken any hash, signature, or local catalog check.
- CCSwitch's USB bootstrap previously prepared the device key in its private DB without switching the active provider; CCSwitch also defaulted to `~/.codex` while the USB manager uses an isolated local `CODEX_HOME`. Scope CCSwitch to the same USB-specific home and activate the bound provider. The live-write guard now permits that activation only inside a thread-local one-shot bootstrap scope, after the provider has been checked against the exact managed home, bound endpoint, injected key, and a model-free config. The generic Apply/startup path remains denied, and `model`, `model_reasoning_effort`, or `model_reasoning_summary` make even the bootstrap write fail closed. Acknowledge only after active provider, live endpoint, and key readback succeed.
- The model row always renders an empty `.upstream-model-manager-add-panel`; its CSS makes an otherwise empty node look like a card. Render the reasoning disclosure only when results exist, without the panel wrapper.

## Hard constraints

- Preserve all unrelated existing work and release artifacts.
- Do not execute an unverified CLI or weaken MSIX staging, Authenticode, member hash, or catalog checks.
- The USB key remains environment-injected, absent from logs and CLI arguments, and conflicts with a customized 404token endpoint fail closed.
- Keep all existing add/reset concurrency and proof-consumption semantics.
- Rebuild the USB package only after focused tests and independent read-only audit pass. Use a fresh versioned output folder.

## Acceptance

1. UI test proves no per-model info-card wrapper is rendered, while available reasoning results remain accessible.
2. Wrapper contract test proves `EnableFullAccess` is accepted by `install-all.ps1` and forwarded only when set.
3. Rust tests prove the Add deadline is capped by actual proof life with a 10-second commit reserve, reaches 270 seconds when at least 280 seconds remain, and preserves the retryable timeout message; run the native Windows crate tests.
4. CCSwitch tests prove USB mode targets the same isolated Codex home; generic writes remain denied even when USB environment variables are present; only the model-free, exact-bound provider can be activated inside the one-shot bootstrap scope; model/reasoning-bearing configs remain denied; the scope is restored after use; and the bootstrap writes a nonce-bound acknowledgement only after active provider/live endpoint/key readback. No plaintext key is written to the acknowledgement.
5. Run frontend and Rust test suites, build the CCSwitch artifact and installer, then verify package manifest/checksums and inspect the final bundle. Independent audit must PASS before calling the package ready.

## Execution and release evidence (2026-10-22)

- Focused UI test: `UpstreamModelManagerSheet.test.tsx` passed (39 tests). Installer wrapper static contract passed (7 tests). Model Add proof-lifetime test passed. CCSwitch USB handoff tests passed (2 tests), including the one-shot authorization scope and ACK/readback path.
- An isolated Windows PowerShell invocation test ran the packaged `install-all.ps1` against a mock downstream script and confirmed `EnableFullAccess`, `TargetCodexHome`, and the existing installer flags were received; it did not touch a real Codex home.
- `build-installer.ps1` completed its full configured gates: TypeScript check, ESLint, 37 source test files / 438 tests, the Rust test gate, and optimized Windows USB-manager build. It also verified the launcher and administrator manifests and the 54-skill resource identity.
- CCSwitch product renderer, `cargo check`, optimized Windows release build, and product manifest/hash verifier passed. The new `CodexSwitch.exe` SHA-256 is `04607C8E8E8122E3D7A7FA3B0C360FA7144556D72772BE4A761AA4989BC36DB2`.
- The working `installer-pack` had stale integrity metadata from unrelated in-progress edits, so the release used a fresh isolated copy of the previously packaged 1.5.4 capability pack with only this task's `install-all.ps1` forwarding fix overlaid. Its 54 skill IDs match the manager's base manifest in the same order; regenerated metadata passed the pack checker. This avoids sweeping unrelated working-tree skill changes into the release.
- Independent read-only audit: PASS; no blocker found in the four LOG12 fixes. The auditor did not rerun tests or build.
- Fresh internal-signed USB layout: `release/usb-oneclick-v0.5.24-20261022-log12-r1`. `verify-oneclick-usb.ps1 -InternalSigned -SkipNetwork` passed; the final manifest and all sampled executable/wrapper hashes are present in `SHA256SUMS.txt`. No API key or license is embedded. This is package verification, not a physical USB cold-start test or a newly signed USB license.
- No signer rebuild was needed: the changes are in the manager, CCSwitch activation, and capability installer wrapper; the signing protocol and signer were unchanged.

## Additional local runtime acceptance (2026-10-22)

- Installed the capability pack from the **built USB layout** using its packaged `install-all.ps1`, targeting a newly created `.tmp-log12-runtime-install/profile` and separate `.codex` home. Exit code 0; readiness reported `overallStatus=ready`, `core.status=ready`, 54/54 Skills, 7/7 Agents, config `merged`, and `secretsStored=false`. All writes were confined to this temporary profile; the installer reported `environmentApplied=false` and did not change the real user's environment variables.
- The same run provisioned the bundled offline runtimes and passed Python 3.12.10, Manim 0.21.0, Node 22.22.0, FFmpeg 9, HyperFrames 0.8.134 CLI and browser checks. The installed media wrapper's own `-HealthCheck` returned JSON with `ready=true`, local runtime/protocol passed, MCP 1.3.11, UTF-8, and six tools. Gateway model discovery and paid generation were intentionally not invoked because they require a real API credential and may incur charges.
- Re-ran the same installation against the same isolated profile: exit code 0, again `overallStatus=ready`, 54/54 Skills, 7/7 Agents, media runtime/protocol passed, and config merged. This checks repeat-install/ownership behavior without involving a user's existing Codex files.
- Re-ran the full USB manager Rust suite directly with the pinned Rust 1.95 toolchain: **763 passed, 0 failed, 6 ignored**. The Add proof/deadline suite passed **19/19**. Frontend model-manager test passed **39/39**; installer forwarding/static checks passed **7/7**.
- Re-ran CCSwitch USB-specific tests: `usb_404token` **2/2 passed**; bootstrap authorization boundary **1/1 passed**. A full CCSwitch Rust suite is **not green**: five `tests/skill_sync.rs` tests failed (first reports imported count 0 instead of 1; the following four fail acquiring the now-poisoned shared test mutex). These tests cover unrelated Skill import/synchronization paths, and the test file was not modified for LOG12. To establish baseline, a detached clean worktree at nested CCSwitch `HEAD 1d44b1ba` reran that integration target: it independently failed 4/7 tests (`restore_skill_backup...` first, followed by mutex-poison failures). This confirms the failure exists on the clean baseline, independent of LOG12 changes; the first failing test differs due concurrent test ordering. The LOG12 working-tree full suite remains red and is not represented as a full-suite PASS.
- Re-ran the built bundle's own `verify-usb.ps1 -SkipNetwork -InternalSigned`: passed manifest/checksum checks, embedded CCSwitch static verifier, installer/launcher resources, and Codex MSIX identity/hash (`26.1002.7124.0`). This remains a static/release-layout test, not a physical USB cold boot or real provider test.
- The integration install detected that another Codex process was already running on this workstation, so the temporary install's `restartRequired=true` metadata is expected. No real Codex process was restarted or pointed at the temporary profile.
- Independent read-only follow-up audit: **PASS for the LOG12 scope and local package evidence**. The auditor independently checked the CCSwitch EXE and MSIX hashes against the release manifest/checksum list, confirmed the isolated readiness paths and secret-free status, and classified the `skill_sync` failures as unrelated to changed files. The auditor did not rerun tests. It also noted the CCSwitch bundle's recorded upstream source commit does not include all dirty local patches; the built EXE hash is verifiable, but source-to-binary provenance is not fully reproducible until those patches are committed or separately fingerprinted.
