# Update Architecture

Status: **Current maintenance reference** · The Codex USB payload channel is
implemented; the unified cross-channel command surface described below is
historical/proposed context only.

The only authoritative current contracts are
`docs/UPDATE-REMEDIATION-DEVELOPMENT.md` (`v2.0-remediation-r1`),
`docs/UPDATE-CHANNELS.md`, and `docs/UPDATE-TESTING.md`. This document is
supplementary and does not supersede them. The implemented Codex Tauri
commands are `payload_check_codex` (read-only check) and `payload_apply_codex`
(explicit phase-1 USB payload apply). The older proposal names
`updates_check_all` and `updates_apply` are not implemented APIs and must not
be used for current integrations.

## Goals

1. **Manual confirm, never auto-apply.** The manager may check for updates in
   the background but MUST NOT download or install any update without an
   explicit user action. This applies to every channel, including the manager
   itself.
2. **Source is configurable.** Every update channel resolves its download
   source from `AppSettings`, falling back to a per-channel default baked
   into the binary. No source is hard-coded at the call site.
3. **Domain is replaceable within the correct protocol boundary.** The
   manager self-update domain is replaceable through Tauri updater
   endpoints. The current codex payload source is a GitHub repository
   (`owner/repo`), so its immediate fallback is changing repo, not changing
   a generic download domain. A future mirror-domain protocol can be added
   as a second codex source type if GitHub access becomes the bottleneck.
4. **Survive upstream abandonment.** If `wangzhen3691/codex-app` (the
   non-official upstream that re-publishes signed Codex MSIX packages) stops
   shipping, users must be able to switch to a replacement source without
   reinstalling.
5. **Reuse existing infrastructure.** Cloudflare Worker
   (`cloudflare/manager-download-router`), R2 bucket, IHEP S3 CN failover,
   Tauri updater, minisign key, `usb_guard`, and the existing
   `UpdateSource::{Auto, Mirror, Custom}` settings enum are reused as-is.
   No VPS, no new signing key, no new endpoint schema.

## Update channels

The manager maintains four conceptually distinct channels. Each channel owns
exactly one artifact family and its own source resolution, version
comparison, and apply strategy.

| Channel       | Artifact                                 | Source of truth                              | Signature scheme            | Ships in |
| ------------- | ---------------------------------------- | -------------------------------------------- | --------------------------- | -------- |
| `manager`     | `codex-app-manager` installer (NSIS/DMG) | Tauri updater endpoints (`tauri.conf.json`)  | minisign (Tauri updater)    | current  |
| `codex`       | `OpenAI.Codex_*.Msix` on the USB drive (extension case-insensitive) | `Wangnov/codex-app-mirror` GitHub Release | Microsoft Authenticode (OpenAI) | implemented |
| `ccswitch`    | `CodexSwitch.exe` on the USB drive       | TBD (rebuilt by maintainer from `farion1231/cc-switch`) | minisign (maintainer-signed) | future   |
| `skill-repo`  | skills/` folder snapshot on the USB drive | TBD                                          | TBD                         | future   |

The `manager` channel is **already implemented** and is referenced here only
so the unified abstraction stays honest. See
`src-tauri/src/commands.rs::manager_check_update` and
`manager_install_update` for the existing contract.

### Why codex is a separate channel from manager

The Tauri updater (`tauri_plugin_updater`) is purpose-built for replacing the
running executable. It enforces a minisign signature over a Tauri-specific
manifest (`latest.json`). The codex payload, in contrast:

* is an MSIX already signed by OpenAI's Authenticode certificate — adding a
  second minisign layer is redundant and creates a key-management burden;
* lives on the USB drive, not in the manager's installation directory;
* has its own version scheme (`26.602.40724`-style appVersion, not semver);
* is delivered through a different distribution channel (GitHub Releases on
  a mirror fork, not the manager's own release pipeline).

Trying to push codex through the Tauri updater would corrupt both trust
models. Keep them separate.

## Update lifecycle

The shared lifecycle remains the architecture for current and future
channels, but only the manager and Codex paths are implemented today. The
shipped Codex flow is:

```
payload_check_codex ──► notify ──► user confirms ──► payload_apply_codex ──► report
   │                                       │
   └─ never applies by itself              └─ re-validates version before apply
```

For the `codex` channel, `apply` has **two explicit user-confirmed phases**:

1. **Refresh USB payload** — download, verify, and atomically replace the
   MSIX plus the root `manifest.json`, `usb-manifest.json`, and
   `SHA256SUMS.txt` metadata.
2. **Deploy the refreshed payload** — only if Codex is already installed on
   the current Windows machine and the user explicitly chooses this second
   action, invoke the existing Windows AppX update flow.

Replacing a `.Msix` file on the USB drive does **not** update an AppX/MSIX
package already deployed on Windows. The UI must label phase 1 as “更新 U
盘安装包” and phase 2 as “更新此电脑上的 Codex”; it must never claim phase
1 updated the running app.

### Shared types and dispatch

`src-tauri/src/app/update_channel.rs` contains the current shared channel
types. The enum-dispatch example below is an architectural sketch for the
closed channel set, not a shipped command registration surface.

**Important Rust detail**: native `async fn` in traits is not object-safe,
so we cannot use `Box<dyn UpdateChannel>` with `async fn` directly. The
project does not currently depend on the `async-trait` crate. The cleanest
options, in order of preference:

1. **Enum dispatch** (recommended for this codebase). Four channels is a
   small, closed set. Use an enum and a `match` on the channel name; no
   trait objects needed. This avoids adding a dependency.
2. Add `async-trait = "0.1"` to `src-tauri/Cargo.toml` and use
   `#[async_trait]` on the trait and each impl. Familiar pattern, small
   dep cost.

The examples below use **enum dispatch** to match the codebase's existing
preference for minimal dependencies:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelKind {
    Manager,
    Codex,
    Ccswitch,    // future
    SkillRepo,   // future
}

impl ChannelKind {
    pub async fn check(
        &self,
        ctx: &UpdateContext,
    ) -> Result<Option<UpdateInfo>, AppError> {
        match self {
            Self::Manager => channels::manager::check(ctx).await,
            Self::Codex => channels::codex::check(ctx).await,
            Self::Ccswitch => channels::ccswitch::check(ctx).await,
            Self::SkillRepo => channels::skill_repo::check(ctx).await,
        }
    }

    pub async fn apply(
        &self,
        ctx: &UpdateContext,
        expected: &UpdateInfo,
    ) -> Result<(), AppError> {
        match self {
            Self::Manager => channels::manager::apply(ctx, expected).await,
            Self::Codex => channels::codex::apply(ctx, expected).await,
            Self::Ccswitch => channels::ccswitch::apply(ctx, expected).await,
            Self::SkillRepo => channels::skill_repo::apply(ctx, expected).await,
        }
    }
}
```

Each channel lives in its own module under
`src-tauri/src/app/channels/<name>.rs` and exposes two free functions
`check` and `apply`. Adding a channel means: new module, new enum variant,
two new `match` arms. No trait objects, no `async-trait` dependency.

### UpdateInfo payload

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub channel: ChannelKind,        // "manager" | "codex" | "ccswitch" | "skill-repo"
    pub current_version: String,
    pub latest_version: String,
    pub release_notes: Option<String>,
    pub size_bytes: Option<u64>,
    pub requires_restart: bool,      // true only for `manager`
    pub payload: serde_json::Value,  // channel-specific, opaque to UI
}
```

The `payload` field carries channel-specific data needed to re-identify the
remote artifact (e.g. codex carries the repo, release id/tag, asset names,
package moniker, and expected app/package versions). It MUST NOT be treated
as trusted input during `apply`. The backend re-fetches remote metadata and
re-resolves download URLs before use; URLs round-tripped through the frontend
are display-only at most.

### Command surface

The manager self-update uses the existing `manager_check_update` and
`manager_install_update` commands. The implemented Codex payload channel uses
the following two commands; `payload_apply_codex` is phase 1 only and does not
deploy Codex to the current Windows installation:

```rust
#[tauri::command]
pub async fn payload_check_codex(
    state: State<'_, ManagerState>,
) -> Result<Option<UpdateInfo>, CommandError>;

#[tauri::command]
pub async fn payload_apply_codex(
    app: AppHandle,
    state: State<'_, ManagerState>,
    info: UpdateInfo,
) -> Result<(), CommandError>;
```

`payload_check_codex` is read-only and returns no update when the authorized
USB bundle is current. After an explicit user confirmation,
`payload_apply_codex` re-fetches and re-validates the remote identity, then
updates the USB payload and its three root metadata files. There is no shipped
unified all-channel check/apply command.

## Three-layer replaceability

This is the core risk-mitigation property. Each layer is independently
replaceable, and each replacement can be triggered by a different actor.

### Layer 1 — upstream codex source (`wangzhen3691/codex-app`)

* **Failure mode**: author deletes or abandons the repository.
* **Detection**: `sync-upstream.yml` GitHub Actions workflow in
  `Wangnov/codex-app-mirror` fails to find a new release for > 30 days,
  or upstream API returns 404.
* **Response**: maintainer forks a replacement upstream, updates
  `sync-upstream.yml`'s `UPSTREAM` variable, waits for the next sync.
* **User impact**: zero. Users still pull from `Wangnov/codex-app-mirror`,
  which the maintainer controls.

### Layer 2 — manager distribution domain (`codexapp.agentsmirror.com`)

* **Failure mode**: domain lapses, is revoked, or its Cloudflare zone is
  transferred.
* **Detection**: `manager` channel update checks fail against
  `https://codexapp.agentsmirror.com/manager/latest.json`, while the
  GitHub Releases updater endpoint still works.
* **Response**: maintainer ships a new manager release with updated
  `tauri.conf.json::plugins.updater.endpoints` (line 90-93). Users update
  the manager through the **GitHub Releases Tauri updater endpoint**
  (`tauri.conf.json` line 92), which does not depend on the dead domain.
* **User impact**: one extra click (`Update manager → restart`). Manager
  self-updates resume through the new endpoint.

**Important boundary**: this layer does not automatically move codex
payload traffic if the codex channel is using GitHub Releases directly.
For the current GitHub-release Codex channel, changing the payload source means changing
`codex_payload_repo` (repo-level source). If we later add a domain-backed
codex mirror protocol, document it as a new source type rather than
pretending the manager updater endpoint controls payload downloads.

### Layer 3 — manager distribution (worst case)

* **Failure mode**: GitHub account suspended, all custom domains lost.
* **Response**: publish the manager through a new GitHub org and a new
  domain. Existing users would need to manually download the new manager
  once. This is the only scenario that breaks the "no reinstall" guarantee.
* **Mitigation**: keep the GitHub Releases endpoint in
  `tauri.conf.json::plugins.updater.endpoints` permanently, so as long as
  GitHub is reachable the manager can still self-update.

## Source resolution order

Every channel resolves its source by walking this chain, in order:

1. **User override** — a per-channel field in `AppSettings` (e.g.
   `codex_payload_repo` for the codex channel). Validated by a
   channel-specific validator before use. Always wins. Never overwritten
   by the manager.
2. **Baked-in default** — a `&'static str` constant in the channel's source
   file (for Codex, `DEFAULT_CODEX_PAYLOAD_REPO` is
   `Wangnov/codex-app-mirror`). Updated by shipping a new manager release.
3. **Hard failure** — if neither is reachable, surface the error to the UI.
   Never silently fall back to a stale cached copy without telling the user.

**Why per-channel fields instead of reusing `UpdateSource::Custom` +
`custom_url`**: the existing settings drive the mirror-manifest protocol
used by `windows_endpoints_for_settings`
(`src-tauri/src/commands.rs:100-128`). The codex channel uses GitHub
Releases as its protocol, which has a different URL shape and a different
notion of "source" (a repo identifier, not a base URL). Forcing them to
share a setting would couple two protocols and produce confusing UX. Each
channel gets its own optional override field; absent override → default.

## Future codex mirror protocol

The v1 codex channel uses GitHub Releases because it is the smallest
correct implementation: the fork already publishes the three required
assets (`.Msix`, `release-manifest.json`, `SHA256SUMS-windows.txt`) and
GitHub exposes stable asset metadata through the Releases API.

If GitHub download availability or domain control becomes a real problem,
add a second codex source protocol (`MirrorManifest`) with its own setting
(e.g. `codex_payload_base_url`). Do not overload `codex_payload_repo` with
URLs. A repo identifier and a mirror base URL are different protocols and
must stay distinct.

Minimum future mirror contract:

```text
<base>/latest/release.json
<base>/<appVersion>/OpenAI.Codex_<packageVersion>_<arch>__<pub>.Msix
<base>/<appVersion>/SHA256SUMS-windows.txt
```

Until that protocol exists, "changing the codex download domain" means
shipping a new manager that changes the codex channel's source protocol,
not merely changing `tauri.conf.json` updater endpoints.

## Security invariants

These MUST hold for every channel, regardless of future refactors:

1. **Codex MSIX**: Authenticode signature MUST be `Valid` and the package
   identity MUST be `OpenAI.Codex`. Validated by porting the logic in
   `tools/collect-payload.ps1:47-71` into Rust.
2. **SHA256**: every downloaded artifact's SHA256 MUST match the value
   published in the source's checksum file (`SHA256SUMS-windows.txt` for
   codex, minisign `.sig` + Tauri manifest for the manager).
3. **No unsigned installs**: `AppSettings::signed_only` is hard-coded `true`
   and is surfaced read-only in the UI. Do not change this.
4. **Stale expectation**: `apply` re-fetches the version immediately before
   any destructive operation and rejects with `StaleExpectation` on mismatch.
   The existing `manager_update_matches_confirmation`
   (`src-tauri/src/commands.rs:452-459`) is the reference pattern.
5. **USB authorization**: every channel's `apply` calls
   `usb_guard::require_authorized()` before touching the USB drive.

## Non-goals

* Auto-update in the background. Explicitly rejected by the product owner.
* Delta/patch updates. Nice-to-have; deferred until per-channel sizes
  justify the complexity.
* WebSocket push. Polling on app focus + a periodic timer is sufficient.
* Cross-channel atomic transactions. Channels update independently.

## Open questions

* Do we need a per-channel `disabled` flag in settings, beyond the global
  `disable_codex_self_updates` that already exists? Probably yes for
  `skill-repo`, but defer until that channel is designed.
* The implemented Codex apply emits `payload-update-progress`; any future
  shared progress envelope for additional channels remains a proposal.
