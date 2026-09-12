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

`sign` requires an exact confirmation string printed in its review line. A
previous unsigned license keeps its `installationId`; a signed, malformed,
or fingerprint-mismatched license is not overwritten. A post-write readback,
physical re-observation, signature check, and deep bundle check are required
before the command can return `status: success`. Failure or an unknown
post-write state returns a non-zero exit code (unknown state uses exit code 2).

## Key storage

Production signing reads a local Windows DPAPI-protected blob for the current
user via `CryptUnprotectData`. The decrypted envelope is versioned and binds
the requested `keyId` to a 32-byte Ed25519 seed. `init-key` can create or reuse
the one local DPAPI blob and prints only the public key, key id and public-key
digest; it never prints the seed or plaintext blob. `decode_key_blob_plaintext`
is exposed only for a controlled provisioning/test boundary.

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
pwsh .\tools\build-usb-license-signer-gui.ps1 -OutputPath $guiOutput
```

Use a new, empty output directory for each build; the script refuses to overwrite
an existing non-empty release directory.

Double-click the published EXE. It scans removable NTFS volumes, performs the
same structural inspection as the CLI with Authenticode skipped, automatically
discovers or initializes the one local DPAPI key, and enables one-click signing
only when the Bundle's formal public-key binding matches. It automatically runs
internal-license `verify` after signing.
The GUI calls the shared `run_with_args` pipeline in this crate; it is not a
second signing implementation. It never asks the user to select a key file or
enter a KeyId, and never generates or stores plaintext private keys. See
`../../docs/USB-LICENSE-SIGNER-GUI-DEVELOPMENT.md` for the installer integration and
production prerequisites.

Hardware removable-media, DPAPI, and production Authenticode tests remain
Windows-environment tests and are not represented by the deterministic unit
tests in this repository.
