# USB Codex model reload and one-click launch

## Scope

Keep the existing model-add flow and USB authorization behavior. Ensure that both the ordinary launch path and the persisted-theme launch path validate the manager-owned model catalog, restart only the Codex tree bound to the authorized USB when a model reload is pending, and clear the reload marker only after a successful launch. Do not reset user models or alter unrelated Codex configuration.

## Diagnosis

Adding a model writes the catalog and its `config.toml` reference, then creates a reload marker. The ordinary Windows path in `win_update::launch_codex` validates that catalog, closes the managed USB Codex if it is already running, starts Codex, and clears the marker after success.

The themed Windows path is different: `commands::win_launch_codex` may return early after `codex_theme::launch_with_active_theme`. That path starts/restarts Codex with CDP and can hot-apply a theme to an existing process. Before this change it did not run the model-catalog preflight or consume the pending model-reload marker. Thus adding a model while the themed Codex instance is open could leave the process on its old in-memory catalog, and repeated one-click launches could keep taking the themed path without applying the new catalog. This is a source-level lifecycle gap; a particular user's reported launch failure still requires their runtime error/log to confirm the exact failing stage.

## Required behavior

1. Before themed USB launch, resolve and authorize the selected Codex against the signed USB root.
2. Run the same local catalog preflight used by ordinary launch.
3. If the reload marker exists, inspect only processes belonging to that authorized portable root. Close that process before theme startup so the theme route cannot hot-apply onto the stale model catalog.
4. Inspect only `Codex.exe` candidates using the strict process scanner. If a candidate's image path or process state cannot be confirmed, stop with a clear retry instruction. After closing a detected Codex process, verify it exited before continuing.
5. Treat only a confirmed missing marker as “no reload pending”; metadata errors and a non-file marker must stop launch instead of being folded into `false`.
6. Give every marker write a unique generation. After successful launch, clear only the generation observed before that launch; a model change committed during launch must remain pending.
7. If a catalog reload is pending, skip the theme hot-attach route for that launch and use the normal signed-USB launcher. The theme route accepts any ready listener on fixed CDP port 9345 and cannot prove it belongs to the authorized USB Codex process. The saved theme resumes on the next one-click launch.
8. Keep existing USB signature/revocation checks and launch compatibility transaction intact.

## Code map

- `src-tauri/src/commands.rs::win_launch_codex`: invoke the USB model-reload preparation before the theme branch; clear the marker only after a successful themed launch.
- `src-tauri/src/app/win_update.rs::launch_codex`: reuse the shared preparation helper for the ordinary path.
- `crates/codex-win-engine/src/windows_process.rs`: strict, executable-filtered scan so unrelated protected processes do not block launch and inaccessible `Codex.exe` candidates are not silently ignored.
- `src-tauri/src/app/model_catalog.rs`: checked, generation-tagged reload-marker lookup and conditional clear; only `NotFound` means no pending reload.
- `src-tauri/src/app/win_update.rs`: verify process exit after close and test the restart decision, inspection failure, close failure, marker retention, and unresolved process state.
- `src-tauri/src/commands.rs::win_launch_codex`: bypass the fixed-port theme hot-attach only while a model reload is pending, then use the regular signed USB launch path.

## Acceptance

- Targeted Rust tests prove missing/valid/invalid marker handling, generation-safe conditional clearing, no-op without a pending marker, no close when no managed process is running, close and verify a running managed USB tree, and failure propagation when process state cannot be checked, close fails, or the process remains.
- Rust tests and frontend type/lint tests pass.
- Static USB package checks remain unchanged. A physical USB cold launch is a separate runtime check and must not be claimed from unit tests.
