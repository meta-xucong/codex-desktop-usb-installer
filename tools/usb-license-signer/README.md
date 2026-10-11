# usb-license-signer

Standalone Windows signer for the frozen Codex USB license contract. The
binary signs only a single explicitly selected removable NTFS volume and
bundle-relative path after a deep bundle preflight. It never accepts private
key material on the command line or through environment variables.

## Build

From the repository root:

```powershell
cargo fmt --manifest-path crates/codex-usb-license/Cargo.toml
cargo fmt --manifest-path tools/usb-license-signer/Cargo.toml
cargo build --release --manifest-path tools/usb-license-signer/Cargo.toml
```

## Commands

```powershell
.\tools\usb-license-signer\target\release\usb-license-signer.exe scan
.\tools\usb-license-signer\target\release\usb-license-signer.exe inspect --drive E: --bundle-relative-path CodexUsb [--report C:\Reports\inspect.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe inspect --drive E: --bundle-relative-path CodexUsb --skip-authenticode [--report C:\Reports\inspect.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe sign --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "SIGN <volumeSerial>-<anchorSha256-first12>" [--key-blob C:\Keys\prod-2026-01.dpapi] [--report C:\Reports\sign.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe sign --drive E: --bundle-relative-path CodexUsb --key-id prod-2026-01 --confirm "SIGN <volumeSerial>-<anchorSha256-first12>" --key-blob C:\Keys\prod-2026-01.dpapi --skip-authenticode [--report C:\Reports\sign.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe verify --drive E: --bundle-relative-path CodexUsb [--key-id prod-2026-01] [--key-blob C:\Keys\prod-2026-01.dpapi | --public-key-b64 BASE64] [--report C:\Reports\verify.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe verify --drive E: --bundle-relative-path CodexUsb [--key-id prod-2026-01] [--key-blob C:\Keys\prod-2026-01.dpapi | --public-key-b64 BASE64] --skip-authenticode [--report C:\Reports\verify.json]
.\tools\usb-license-signer\target\release\usb-license-signer.exe init-key
```

The CLI also supports `export-portable` and `inspect-portable` with
`--password-stdin` for a controlled automation harness. Do not put a real
password in a script, repository, environment variable, or command history;
the normal workflow is the GUI password prompt.

`sign` requires an exact confirmation string printed in its review line. After
the atomic USB write and local readback checks succeed, the signer automatically
uploads only the signed registration envelope to `https://license.404token.xyz`
to activate the license. Registration v2 carries only the public `USB-####`
asset number from the signed account binding; it never carries the rest of the
account binding or any account credential. The GUI's “同步/补发并激活” flow
upgrades an older v1 registration sidecar without changing the signed license.
VPS registration retries are idempotent; a revoked
record is never reactivated by this operation. Network failure leaves a valid
local signature with VPS activation marked pending; use the GUI's
“同步/补发并激活” action after rescanning to retry. A previous unsigned license
keeps its `installationId`; a signed, malformed, or fingerprint-mismatched
license is not overwritten. A post-write readback, physical re-observation,
signature check, and deep bundle check are required before the command can
return `status: success`; the separate `activationStatus` field must be `active`
before claiming VPS activation. Failure or an unknown post-write state returns
a non-zero exit code (unknown state uses exit code 2).

## Key storage

The compatibility CLI path can read a local Windows DPAPI-protected blob for
the current user via `CryptUnprotectData`. The decrypted envelope is versioned
and binds the requested `keyId` to a 32-byte Ed25519 seed. `init-key` can create
or reuse the one local DPAPI blob and prints only the public key, key id and
public-key digest; it never prints the seed or plaintext blob.
`decode_key_blob_plaintext` is exposed only for a controlled provisioning/test
boundary. The formal one-click GUI release does not use this path; it requires
the portable vault beside the EXE.

For machine-independent signing, the formal release signer carries the encrypted portable vault beside the executable.
The vault contains an AES-256-GCM encrypted Ed25519 seed, with an Argon2id-derived
key; it does not contain a plaintext private key. Copy the complete signer
directory, including the vault and its public descriptor, to another Windows x64
computer and enter the same password in the GUI. The other computer does not
need the original user's DPAPI store or public-key binding. If the vault is
missing, the GUI fails closed and asks you to copy the complete signer directory;
it never falls back to a machine-local DPAPI key. Vault provisioning remains a
controlled release step, and the vault is create-only. The password is never
accepted from an environment variable or written to logs.

Reports and old-license audit copies are written only to caller-selected or
local-app-data paths. They contain metadata and digests, not private keys or
complete license signatures.

The CLI's default deep verifier requires the release manifest contract, payload
hash/size, `SHA256SUMS.txt`, required entries/directories, the allowlisted
root `usb-license.json` mutability rule, and valid Authenticode for the MSIX
and required PE inputs. For the USB-internal-license workflow, pass the
explicit `--skip-authenticode` flag; that mode keeps all structure, manifest,
hash, payload, and path checks while omitting only PE/MSIX Authenticode. The
GUI uses this explicit internal-license mode and labels it in the review.

## One-click Windows GUI

Build `usb-license-signer-gui` and publish it as `CodexUsbLicenseSigner.exe`:

```powershell
$guiOutput = Join-Path .\release ("usb-license-signer-gui-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
pwsh .\tools\build-usb-license-signer-gui.ps1 -OutputPath $guiOutput `
  -SigningProfilePath .\release-profiles\usb-v<version>-<release>-key<digest>.json `
  -PortableVaultPath .\path\codex-usb-signing-key.vault `
  -PortableVaultDescriptorPath .\path\codex-usb-signing-key.vault.public.json
```

Use a new, empty output directory for each build; the script refuses to overwrite
an existing non-empty release directory.

Double-click the published EXE. It scans removable NTFS volumes, performs the
same structural inspection as the CLI with Authenticode skipped, automatically
discovers the encrypted portable vault beside the EXE and enables one-click
signing only when the Bundle's formal public-key binding is valid. The GUI asks
only for the vault password; it never checks the current computer's DPAPI key.
It automatically submits the signed registration proof to the VPS after local
readback verification, then runs internal-license `verify`. If VPS networking
fails, the window reports activation as pending and allows a later retry after
rescan. An existing revoked VPS record remains revoked. If the vault is absent,
the GUI fails closed and asks for the complete signer directory.
The GUI calls the shared `run_with_args` pipeline in this crate; it is not a
second signing implementation. It never asks the user to select a key file or
enter a KeyId, and never generates or stores plaintext private keys. See
`../../docs/USB-LICENSE-SIGNER-GUI-DEVELOPMENT.md` for the installer integration and
production prerequisites.

Hardware removable-media, DPAPI, and production Authenticode tests remain
Windows-environment tests and are not represented by the deterministic unit
tests in this repository.

### Optional 404token/Sub2API provisioning

The GUI also has a “register account and sign” flow. It uses the fixed
`https://404token.xyz` Sub2API endpoint and accepts the short email domain and
a non-negative preload amount; the administrator credential is compiled into
the trusted production executable. The address is read-only, the production
client rejects other destinations, and HTTP redirects are disabled for
requests carrying the administrator credential.
The target CCSwitch group is fixed to `unified-api-internal`; there is no
group-ID field and no per-model key selection. New accounts use the fixed
initial password `12345678`. The signer allocates the next
`USB-xxxx` account, creates the user API key through the user login flow, injects
the binding into the signed `usb-license.json`, verifies the writeback, and only
then performs the first balance addition with a stable idempotency key.

The administrator key is loaded automatically: managed environment variables
take precedence and production builds contain the credential embedded at
compile time. The GUI field is read-only and is not a manual input; published
signer directories must not contain a sidecar administrator-key file. Only
non-secret endpoint preferences are saved in the normal signer-local
configuration.
The administrator key is never put in the runtime USB bundle, license, reports,
logs, or CCSwitch. Existing bindings are
reused and do not trigger an implicit second preload. Before sending the
account-creation request, the signer records the intended USB number and email
in a DPAPI-encrypted recovery record. It then records the remote user ID and
generated key as each step succeeds, allowing the next attempt to reconcile or
resume the same account after an interrupted request. The user-key request uses
a stable `Idempotency-Key` across retries; duplicate prevention after an
ambiguous response depends on the Sub2API server honoring that header. The
record is deleted after successful license readback. Production registration
and preload still require a separately authorized disposable test
account/amount;
the repository tests use a local HTTP fixture only.

In this one-click flow, initial balance addition runs only after the signed USB
license is locally verified and the VPS confirms a matching active registration.
Pending, conflicting, or revoked VPS states skip the preload; syncing the same
proof later does not create another account or repeat a completed preload.

If signing and readback succeed but the preload request fails, the GUI keeps a
non-secret pending record under
`%LOCALAPPDATA%\CodexUsbLicenseSigner\pending\USB-xxxx.json` and enables
“retry preload”. The retry replays only the balance request with the original
idempotency key; it never creates another account or API key. The backend must
honor `Idempotency-Key`; otherwise confirm the balance in the admin console
before retrying.

## Release binding gate

Each release supplies its own versioned public-only profile; formal build scripts
require that profile explicitly and never fall back to a fixed repository key.
Before publishing, the release script verifies the public descriptor of the
bundled portable vault and compares the release `keyId` and `publicKeySha256`
with the Bundle, manager, signer manifest, and vault descriptor. A CLI
`init-key` result or an old GUI log is not part of the formal release gate and
cannot block or authorize a portable signer on another computer.
