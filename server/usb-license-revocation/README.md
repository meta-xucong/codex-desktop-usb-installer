# USB license revocation service

Standalone Go service, authenticated HTTPS admin console, and local administration CLI for `codex-usb-oneclick`. It implements the VPS contract in `docs/USB-LICENSE-REMOTE-REVOCATION-AUTO-CLEANUP-DEVELOPMENT.md` §§3, 5, 7, 9.1, 10, 12, 13. The listener accepts only loopback IP addresses; production TLS terminates at the reverse proxy.

## Build and local verification

From this directory:

```powershell
gofmt -w ./cmd ./internal
go test ./...
go build -o .\out\revocationd.exe .\cmd\revocationd
go build -o .\out\codex-license-admin.exe .\cmd\codex-license-admin
```

On Linux, omit `.exe`. No command above deploys or writes to a VPS. Tests create ephemeral SQLite databases and fixture-only Ed25519 keys under Go's temporary test directories.

## Files and secrets

- `revocationd` reads the private status key and the license public-key keyring. It stores no registration envelope, license signature, full USB fingerprint, bearer secret, email, model key, full account binding, or project data. It stores only the signed public 404token asset number (for example, `USB-0010`) when present.
- `codex-license-admin` reads the public license keyring, database and journals. It does not load the status signing private key.
- The status private key file contains standard Base64 of the raw 64-byte Ed25519 private key plus an optional final newline. Keep it `root:codex-license-revocation` mode `0640` or service-owner mode `0600`; never put it in source control, the USB bundle, logs, or an image.
- The license keyring is JSON: `{"keys":{"<licenseKeyId>":"<Base64 Ed25519 public key>"}}`. It contains only public license-signing keys. Keep old IDs until no supported license uses them.
- `codex-license-admin keygen --out /etc/codex-usb-license-revocation/status-signing-key --key-id revocation-YYYY-MM` generates a new status key without overwriting an existing file. It prints only the public key. Publish that public key and ID to a client release before changing the service's active `USB_REVOCATION_SIGNING_KEY_ID`.

The initial keyring file is intentionally not populated. Add the existing license signer public key for each accepted `licenseKeyId`; never copy the license signing private key to this service.

## Admin console

The console is served at `/admin/` by the same loopback-only service. There is no default account or password-reset route. To enable it, set all of the following in the protected service environment file:

```ini
USB_REVOCATION_ADMIN_USERNAME=admin
USB_REVOCATION_ADMIN_PASSWORD=replace-with-a-unique-password-of-at-least-12-characters
USB_REVOCATION_ADMIN_AUTH_STATE=/var/lib/codex-usb-license-revocation/admin-auth.json
# Optional: leave empty to enroll during the first admin login.
USB_REVOCATION_ADMIN_TOTP_SECRET=
USB_REVOCATION_ADMIN_ORIGIN=https://license.404token.xyz
USB_REVOCATION_ADMIN_COOKIE_SECURE=true
```

The configured password seeds a PBKDF2-HMAC-SHA256 verifier in the protected auth-state file on first start. On the first successful login, the console requires the operator to set a new password and enroll Google Authenticator before opening the license list. The page displays a private Base32 setup key; if no key was preconfigured, the server generates one. In Google Authenticator choose **Add account → Enter a setup key**, paste the key, choose a time-based code, then enter the current 6-digit code to finish. The service stores the password verifier and TOTP key in the owner-only auth-state file; it does not store the new password in clear text. Keep both the environment file and auth-state file readable only by root/the service owner. Each 6-digit code is accepted once, with a 30-second period and a one-step clock-skew window. Later password changes require the current password and a valid TOTP code, then end all active sessions.

If the auth-state file is lost, the environment password becomes the bootstrap password again and first-login setup is required again. Keep the file on the service's protected data volume and include it in secure backups. If the authenticator is lost, use the protected VPS environment/SSH recovery procedure to restore or reset the auth-state; there is no public reset endpoint. If username and password are both empty, `/admin/` is disabled; configuring only one is a startup error.

For loopback-only testing, use `USB_REVOCATION_ADMIN_ORIGIN=http://127.0.0.1:8787` and `USB_REVOCATION_ADMIN_COOKIE_SECURE=false`. Never use HTTP or disable Secure cookies on a public interface. Mutating admin requests require an authenticated session, the exact configured Origin, and a CSRF token. Login attempts are rate-limited.

After local signing and readback verification, the signer automatically posts only its signed `usb-license-revocation-registration.json` envelope to `POST /v1/licenses/register`. New v2 proofs may include the signed public 404token asset number, such as `USB-0010`; the service never receives the rest of the account binding, email, Sub2API user ID, or API credential. A matching active v1 record can be enriched once by the signer's “登记/同步” flow or by importing its updated signed registration file, without changing the license or its revocation state version. Conflicting numbers and revoked records are not overwritten. Repeating the same registration is idempotent. If the VPS is unavailable, the signer reports activation as pending and its “同步/补发并激活” button can safely retry. The console's “登记新 U 盘” file upload remains a manual recovery path. The console displays the installation ID, 404token number, signer key ID, status/version/timestamps, and masked license/fingerprint digests. It does not issue or display an API bearer key; `licenseKeyId` identifies the key that signed the license.

The “作废” action requires a reason, checking the confirmation box, and retyping the installation ID before submission. The authenticated username is recorded as the actor in the existing audit journal. A successful revoke changes the record to `revoked` and increments its state version. The USB learns this on its next online launch through the manager; offline startup follows the configured fail-open policy, and directly launching the original Codex entry bypasses the manager gate.

The public service endpoints are `POST /v1/licenses/check`, `POST /v1/licenses/register`, and `GET /healthz`. Admin routes are separate and authenticated. The example Caddy configuration forwards only those endpoints and `/admin` paths; all other paths return 404.

## Local run

Set these environment variables to temporary paths and a fixture keyring before running the binary:

| Variable | Default | Purpose |
|---|---|---|
| `USB_REVOCATION_LISTEN_ADDR` | `127.0.0.1:8787` | Must be a literal loopback IP and port. |
| `USB_REVOCATION_DATABASE` | `/var/lib/codex-usb-license-revocation/state.db` | SQLite database, WAL enabled. |
| `USB_REVOCATION_AUDIT` | `/var/lib/codex-usb-license-revocation/audit.jsonl` | Primary append-only journal. |
| `USB_REVOCATION_AUDIT_BACKUP` | `/var/backups/codex-usb-license-revocation/audit.jsonl` | Independent append-only mirror; put this on separately protected backup storage. |
| `USB_REVOCATION_RESTORE_MARKER` | `/var/lib/codex-usb-license-revocation/RESTORE-IN-PROGRESS` | If present, API signing stays disabled. |
| `USB_REVOCATION_LICENSE_KEYRING` | `/etc/codex-usb-license-revocation/license-keyring.json` | Public keyring for registration proofs. |
| `USB_REVOCATION_SIGNING_KEY` | `/etc/codex-usb-license-revocation/status-signing-key` | Private status key file. |
| `USB_REVOCATION_SIGNING_KEY_ID` | empty | ID included in each signed response. |
| `USB_REVOCATION_MAX_RATE_PER_MINUTE` | `60` | Process-wide check limit. |
| `USB_REVOCATION_RATE_BURST` | `10` | Initial check burst. |

The service is stateless across repeated request IDs: it does not cache signed responses and signs the current database state with the exact request ID in each successful response. Clients must supply a fresh 128-bit CSPRNG request ID per check.

For a disposable local demonstration, use two PowerShell windows from this directory. These fixture keys are public test material and must never be deployed. In window A, build and launch the server with fixture paths and a temporary state directory:

```powershell
go build -o .\revocationd.exe .\cmd\revocationd
$state = Join-Path $env:TEMP 'usb-revocation-local-demo'
New-Item -ItemType Directory -Force $state | Out-Null
$env:USB_REVOCATION_DATABASE = Join-Path $state 'state.db'
$env:USB_REVOCATION_AUDIT = Join-Path $state 'audit.jsonl'
$env:USB_REVOCATION_AUDIT_BACKUP = Join-Path $state 'independent-audit.jsonl'
$env:USB_REVOCATION_RESTORE_MARKER = Join-Path $state 'RESTORE-IN-PROGRESS'
$env:USB_REVOCATION_LICENSE_KEYRING = (Resolve-Path .\fixtures\local\license-keyring.json).Path
$env:USB_REVOCATION_SIGNING_KEY = (Resolve-Path .\fixtures\local\status-signing-key).Path
$env:USB_REVOCATION_SIGNING_KEY_ID = 'revocation-test-2026'
$env:USB_REVOCATION_LISTEN_ADDR = '127.0.0.1:8787'
.\revocationd.exe
```

In window B, set the same environment variables (or copy the block above without the final server launch), build the admin CLI, import the fixture, then inspect both active and revoked responses:

```powershell
go build -o .\codex-license-admin.exe .\cmd\codex-license-admin
.\codex-license-admin.exe import --registration-file .\fixtures\local\registration.json --initial-status active --actor local-test
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8787/v1/licenses/check -ContentType 'application/json' -InFile .\fixtures\local\check-request.json
.\codex-license-admin.exe revoke --installation-id usb-install-001 --reason local-test-revoke --actor local-test
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8787/v1/licenses/check -ContentType 'application/json' -InFile .\fixtures\local\check-request.json
```

The fixture's request ID is fixed for reproducible protocol testing; production clients must generate a fresh request ID for every check. To exercise recovery, create a backup while active, revoke, stop the server, restore only the backup `state.db` while leaving both current audit journals in place, create the restore marker, restart the server (checks stay 503), then run `codex-license-admin recover` and `codex-license-admin verify`. Remove the restore marker only after verification, then restart the service; the current journal replays the revoked version into the restored database.

## Admin CLI

Run the CLI on the VPS through a local shell or SSH, as the service account (or an administrator with equivalent access to the DB and both journal paths). The CLI remains available for recovery and scripted maintenance; routine USB registration and revoke operations can also be performed in the authenticated admin console.

```sh
codex-license-admin import --registration-file /secure/path/usb-license-revocation-registration.json --initial-status active --actor operator-id
codex-license-admin inspect --installation-id usb-install-001
codex-license-admin revoke --installation-id usb-install-001 --reason "support case 123" --actor operator-id
codex-license-admin reactivate --installation-id usb-install-001 --reason "appeal approved" --actor operator-id
codex-license-admin backup --out-dir /mnt/offline-backup/revocation-2026-10-09T120000Z
codex-license-admin recover
codex-license-admin verify
```

Import accepts only the signed registration envelope and verifies the raw decoded `payloadB64` bytes with the configured license public key. New records start `active`, version 1. A matching active v1 record may receive its missing 404token number once from the signer's exact v2 re-registration; any other duplicate or conflicting number is refused. The registration `issuedAt` must remain the original unsigned decimal Unix-millisecond string from `UsbLicense.issuedAt`; it is never converted to a date or reserialized before signature verification. Repeating revoke/reactivate at the current target status is idempotent. Each actual transition writes both journals before SQLite commits; a failed journal or transaction cannot produce a signed state. Reasons are stored only in the private audit trail; do not include personal information.

## Backup and recovery

Use `codex-license-admin backup` to create a new snapshot directory containing `state.db`, an audit snapshot, and a SHA-256 manifest. Copy that directory to separately protected storage, and back up the status signing key separately from both database and audit files. Periodically verify an isolated restore.

The service compares the primary journal with its independent mirror and checks both against SQLite records and SQL audit rows. A stale SQLite snapshot that exactly matches a complete journal prefix is replayed forward before serving. Divergence, corrupt/truncated journal, a restore marker, a database that is ahead of the journals, or a mismatched audit table makes the service return 503 and sign no state.

For a planned database restore:

1. Stop the service. Preserve the current two audit journals; do not replace them with an older snapshot's journal copies.
2. Place the configured restore marker, restore `state.db`, and start the CLI with the same environment.
3. Run `codex-license-admin recover` to replay the verified current journals into the restored database. The marker remains present, so the public service still cannot sign.
4. Run `codex-license-admin verify`. Remove the marker only after it reports a complete state and audit match, then start the service.
5. If the journals disagree or their history cannot be proven complete, keep the marker and service stopped; reconcile from an independently retained journal or repair state manually before service resumes.

Keep the mirror on a separately protected volume or remote append-only backup. Two files that are both restored from the same stale snapshot cannot prove that a later event once existed; no local software can infer history that every copy has lost. The recovery process therefore never restores old audit copies over newer live journals. Do not restore or rotate the status private key as a way to make old state appear valid. Key loss requires publishing a new key ID and client trust update before switching keys.

## Deploy

See `deploy/` for the systemd unit, environment example, and Caddy TLS reverse-proxy template. The service binds `127.0.0.1:8787`; the example proxy targets `license.404token.xyz` and forwards only `POST /v1/licenses/check`, `POST /v1/licenses/register`, `GET /healthz`, and the authenticated `/admin/` console. Do not expose the service port directly. Install the binary under `/opt/codex-usb-license-revocation` read-only, create the dedicated service account, set database/journal/keyring/key permissions, verify HTTPS and DNS, and validate backups before production. Nothing in this repository deploys or configures a live VPS.

Production release remains blocked until a real HTTPS domain/certificate and the matching client trust key are configured, and the negative offline/recovery behavior is tested end to end.
