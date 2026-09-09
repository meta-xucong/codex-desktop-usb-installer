# Update Workflows — Operational SOPs

Status: **Current maintenance SOP** · The Codex USB payload channel is
implemented; future-source sections are explicitly proposals.

The only authoritative current contracts are
`docs/UPDATE-REMEDIATION-DEVELOPMENT.md` (`v2.0-remediation-r1`),
`docs/UPDATE-CHANNELS.md`, and `docs/UPDATE-TESTING.md`. This SOP is
supplementary and does not supersede them. The implemented Codex Tauri
commands are `payload_check_codex` and `payload_apply_codex`. The older
proposal names `updates_check_all` and `updates_apply` are not implemented
APIs and must not be used for current operations.

Step-by-step operational procedures for shipping updates through each
channel and for responding to the failure modes identified in the
architecture document.

## 1. Mirroring a new upstream codex release

**Trigger**: `wangzhen3691/codex-app` publishes a new release.

**Automation**: `Wangnov/codex-app-mirror` runs
`.github/workflows/sync-upstream.yml` every 6 hours. Manual trigger:

```bash
gh workflow run sync-upstream.yml --repo Wangnov/codex-app-mirror
```

**Workflow definition** (`.github/workflows/sync-upstream.yml` in the
**fork** repo, not this one):

```yaml
name: Sync upstream release
on:
  schedule:
    - cron: "17 */6 * * *"
  workflow_dispatch: {}

permissions:
  contents: write

jobs:
  sync:
    runs-on: ubuntu-latest
    steps:
      - name: Mirror upstream release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          UPSTREAM: wangzhen3691/codex-app
          FORK: ${{ github.repository }}
        run: |
          set -euo pipefail
          UP_TAG=$(gh api "repos/$UPSTREAM/releases/latest" --jq .tag_name)
          FK_TAG=$(gh api "repos/$FORK/releases/latest" --jq .tag_name 2>/dev/null || echo "")
          if [ "$UP_TAG" = "$FK_TAG" ]; then
            echo "Up to date: $UP_TAG"
            exit 0
          fi
          echo "Mirroring $UPSTREAM@$UP_TAG -> $FORK"
          WORK=$(mktemp -d)
          gh release download "$UP_TAG" --repo "$UPSTREAM" --dir "$WORK" --clobber
          # Idempotency: a tag may already exist in the fork even when it is
          # not the fork's latest release. Create-or-upload avoids a failed
          # scheduled run in that case.
          if gh release view "$UP_TAG" --repo "$FORK" >/dev/null 2>&1; then
            gh release upload "$UP_TAG" --repo "$FORK" --clobber "$WORK"/*
          else
            gh release create "$UP_TAG" --repo "$FORK" \
              --title "Mirror of $UPSTREAM@$UP_TAG" \
              --notes "Auto-mirrored by sync-upstream.yml. Verify SHA256SUMS-windows.txt before use." \
              "$WORK"/*
          fi
```

**Verification**: after the workflow completes, confirm the fork's release
contains at least these assets:

* one MSIX for the target architecture:
  `OpenAI.Codex_<packageVersion>_<arch>__<pub>.Msix` (extension match is
  case-insensitive)
* `SHA256SUMS-windows.txt`
* `release-manifest.json`

A release may contain additional assets (for example macOS artifacts,
status images, or both x64 and arm64 Windows MSIX files). The codex
channel only requires the MSIX for the current target architecture plus
  the two metadata files. The existing `tools/update-from-github-fork.ps1`
defaults to `Wangnov/codex-app-mirror`, still accepts an explicit
`-Repository` override, and requires exactly those three downloads for a
selected architecture; the codex update channel follows the same rule.

After the release assets are verified, the desktop flow is
`payload_check_codex` (read-only check of the currently authorized USB bundle)
→ explicit user confirmation → `payload_apply_codex` (fresh remote
re-validation and USB payload/metadata update). This phase does not deploy
Codex to the current Windows installation.

## 2. Shipping a new manager release

The existing release pipeline is unchanged. See `docs/release.md` for the
authoritative procedure. Summary:

1. Bump `version` in `package.json` and `src-tauri/Cargo.toml` to the same
   semver.
2. Run `npm run tauri:build` (Windows) and `npm run tauri:build:mac` (macOS).
3. Sign artifacts per `docs/code-signing-policy.md` and
   `docs/windows-signing.md`.
4. Run `node scripts/gen-updater-manifest.mjs v<version> <artifacts-dir>`
   to produce `latest.json`.
5. Publish the GitHub Release in `Wangnov/Codex-App-Manager` and upload
   `latest.json` + signed installers.
6. Upload the same `latest.json` to the R2 bucket backing
   `codexapp.agentsmirror.com/manager/`. The Cloudflare Worker
   (`cloudflare/manager-download-router`) serves it directly.

Existing manager installs check through the existing `manager_check_update`
command and, after explicit confirmation, apply through
`manager_install_update`. Users are prompted to confirm; nothing is forced.

## 3. Switching the manager distribution domain

Use this SOP when `codexapp.agentsmirror.com` becomes unavailable or
untrusted for **manager self-updates**. This does not, by itself, change
the codex payload source, because the current Codex channel consumes GitHub
Releases directly via `codex_payload_repo`.

**Goal**: move manager updater traffic to a new domain
(e.g. `updates.example.com`) without breaking existing installs.

### Step 1 — stand up the new endpoint

Deploy a Cloudflare Worker equivalent to
`cloudflare/manager-download-router` bound to the new domain. At minimum it
must serve:

* `/manager/latest.json` — the Tauri updater manifest
* `/manager/<version>/<file>` — versioned manager installers

Codex payload proxying is optional and belongs to a future codex source
protocol. Do not mix it into the manager updater protocol unless the codex
channel is explicitly extended to understand that protocol.

### Step 2 — update the manager's updater endpoints

In this repository:

1. Edit `src-tauri/tauri.conf.json::plugins.updater.endpoints`. **Append**
   the new domain's `latest.json` URL. Do **not** remove the existing
   entries — they continue to serve users on older manager versions until
   those users update.
2. Bump the version and ship a new manager release per §2.

### Step 3 — wait for users to migrate

Existing installs pick up the new manager through whichever endpoint is
still alive, especially the GitHub Releases fallback endpoint. The new
manager then knows the new manager-update domain for future releases.

### Step 4 — decommission the old domain

Only after telemetry (or a reasonable wait, e.g. 90 days) shows the
majority of active users are on the new manager. Even then, prefer to keep
a minimal `latest.json` on the old domain that points to the new one, so
stragglers can still migrate.

## 3A. Future proposal: switching codex payload away from GitHub download URLs

The current Codex channel intentionally uses GitHub Releases directly. If GitHub download
availability becomes unacceptable, add a **new codex source type** rather
than overloading `codex_payload_repo`.

Recommended future setting:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexPayloadSourceKind {
    GithubRelease,
    MirrorManifest,
}

pub codex_payload_source_kind: CodexPayloadSourceKind,
pub codex_payload_repo: String,       // used when kind = GithubRelease
pub codex_payload_base_url: String,   // used when kind = MirrorManifest
```

Mirror protocol minimum contract:

```text
<base>/latest/release.json
<base>/<appVersion>/OpenAI.Codex_<packageVersion>_<arch>__<pub>.Msix
<base>/<appVersion>/SHA256SUMS-windows.txt
```

`latest/release.json` should contain the same identity fields currently
read from GitHub (`appVersion`, `packageVersion`, `packageMoniker`,
`architecture`, `sha256`, `sizeBytes`). Once this exists, a manager release
can switch the default from GitHub Releases to the mirror domain while old
users still retain the manual `codex_payload_repo` fallback.



Use this SOP when `wangzhen3691/codex-app` stops publishing releases.

### Step 1 — confirm the abandonment

* The fork's `sync-upstream.yml` has not produced a new release in > 30
  days, AND
* Manual query `gh api repos/wangzhen3691/codex-app/releases/latest`
  returns 404 or shows no activity in the same window.

Do not act on a single missed release — upstream may simply be slow.

### Step 2 — select a replacement source

Candidates, in order of preference:

1. Another active fork of `wangzhen3691/codex-app` that continues the
   release cadence and preserves the asset naming convention
   (`OpenAI.Codex_*_<arch>__*.Msix` / case-insensitive extension,
   `SHA256SUMS-windows.txt`, and `release-manifest.json`).
2. One of the reference repositories listed in `vendor-lock.json`:
   `izmppj740/codex-offline-windows`, `puppnn/codex-installer`. Verify
   they publish MSIX with valid Authenticode signatures before adopting.
3. Self-host extraction from the official Microsoft Store package. This
   requires new tooling and is out of scope for this SOP.

### Step 3 — re-point the fork

In `Wangnov/codex-app-mirror`:

1. Edit `.github/workflows/sync-upstream.yml`'s `UPSTREAM` env var.
2. Manually trigger the workflow once to confirm the new source produces
   the expected asset set.
3. Verify the mirrored release's MSIX passes Authenticode validation by
   running `tools/update-from-github-fork.ps1` against the fork.

### Step 4 — no manager release required

Because the codex channel reads from the fork (which the maintainer
controls), no manager release is needed. Users continue pulling from the
fork's `releases/latest`, which now mirrors the new upstream.

## 5. User-side manual source override

If a user wants to point at a custom codex payload source before the
maintainer ships a new default:

1. Open the manager's Settings.
2. Under **Codex payload source**, enter the GitHub repository in the form
   `owner/repo` (for example `someone/codex-app-mirror-v2`).
3. Save. The value is persisted to `settings.json` under
   `codex_payload_repo` and takes effect on the next `payload_check_codex`
   call (and the subsequent explicitly confirmed `payload_apply_codex`).
4. To revert to the maintainer default, clear the field.

Format validation happens in the channel's source-resolution step. Invalid
values are rejected before any network request is made. Document this in
the user-facing FAQ once the codex channel ships.

## 6. Emergency rollback of a bad codex payload

If a published codex payload turns out to be broken:

1. **Immediately delete the bad release** from `Wangnov/codex-app-mirror`:
   ```bash
   gh release delete <tag> --repo Wangnov/codex-app-mirror --yes
   ```
   `releases/latest` now points to the previous good release. New
   `payload_check_codex` calls will return "no update available" for users
   who haven't yet applied the bad one.
2. **For users who already applied the bad payload**, ship a fixed release
   with a higher version. The codex channel's backup logic
   (`_backup/OpenAI.Codex_<arch>_<old_version>.Msix`) lets advanced users
   manually restore, but the supported path is to publish a fix.
3. **Post-mortem**: add the failure mode to `UPDATE-TESTING.md` so the
   next release catches it earlier.
