# VPS protocol contract

## Public endpoints

The USB client-facing API is `POST /v1/licenses/check`, `POST /v1/licenses/register`, and `GET /healthz`. `healthz` reports process liveness only; it reveals no database/key details. An optional authenticated operator console is served at `/admin/`; its paths are separate from the USB client protocol. All other paths are 404/405. The app listener must be a literal loopback address and is intended to sit behind a TLS reverse proxy.

The check accepts `application/json`, at most 64 KiB, strict known fields only, rejects duplicate JSON keys, and has 3 second application, 5 second connection read/write, and 30 second idle limits. A process-wide token bucket defaults to 60 checks/minute with burst 10. Error bodies are generic and unsigned. Unknown database state and unknown installation records return 503; malformed request/proof returns 400; no error path returns a signed status.

## Registration proof verification

The outer registration envelope is JSON with exactly `payloadB64` and `signatureB64`. Both use strict standard Base64. The service verifies Ed25519 over the exact bytes decoded from `payloadB64`; it does not decode and reserialize those bytes before verification. Legacy registrations use strict schema/domain v1 JSON with exactly:

```json
{"schemaVersion":1,"domain":"codex-usb-revocation-registration-v1","product":"codex-usb-oneclick","installationId":"...","licenseKeyId":"...","licenseSignatureSha256":"64 lowercase hex chars","fingerprintSha256":"64 lowercase hex chars","issuedAt":"1770000000000"}
```

`issuedAt` is copied exactly from `UsbLicense.issuedAt`: a non-empty ASCII decimal string parsed as an unsigned 64-bit millisecond timestamp. Whitespace, signs, fractions, RFC3339 dates, non-ASCII digits, and values above `18446744073709551615` are rejected. The service verifies the registration signature over the original decoded payload bytes and does not normalize this string.

`licenseKeyId` selects an Ed25519 public key in `license-keyring.json`. The payload is verified before import and on every check. The request's product, installation ID, license signature hash, and observed fingerprint must match the signed registration and stored record.

Registrations whose signed USB license contains a 404token account binding use schema/domain v2. The v2 payload keeps the v1 fields and appends only the signed asset number:

```json
{"schemaVersion":2,"domain":"codex-usb-revocation-registration-v2","product":"codex-usb-oneclick","installationId":"...","licenseKeyId":"...","licenseSignatureSha256":"64 lowercase hex chars","fingerprintSha256":"64 lowercase hex chars","issuedAt":"1770000000000","usbAssetId":"USB-0010"}
```

`usbAssetId` must match `USB-` followed by decimal digits. No account name, email, Sub2API user ID, credential ID, or API key is sent. The VPS accepts v1 and v2 proofs; v1 remains byte-compatible and appears with a blank 404token number in the admin list. If a v2 proof for the same active license arrives after a v1 proof, the server records a one-time audited asset-number binding without changing the license status version. Different numbers or revoked records are not overwritten.

## Automatic signer registration

After writing and locally verifying the signed license and registration sidecar, the trusted signer posts `{"registration": <RegistrationEnvelope>}` to `POST /v1/licenses/register` over HTTPS. This endpoint requires no administrator/API bearer key: possession of a valid license-signing Ed25519 proof is the authorization. The signer never uploads the full `UsbLicense` or account binding; v2 contains only the public `usbAssetId`, never email, Sub2API user ID, credentials, or the signing private key. Redirects are not followed.

The service creates a new `active / version 1` record and returns HTTP 201. An exact retry of the same installation ID, key ID, license-signature digest, and fingerprint digest is idempotent and returns HTTP 200 without an extra audit event. A conflicting proof returns HTTP 409 `registration_conflict`; a previously revoked exact record returns HTTP 409 `license_revoked` and is never changed back to active by registration retry. Invalid proof returns 400, oversized input 413, rate limit 429, and unknown storage state 503. The endpoint has its own small process-wide rate limit.

If the server or network is unavailable, local signing remains complete but activation is reported as pending. The signer keeps the signed registration sidecar on the USB and offers a retry action; retry sends the same proof and is safe. The authenticated admin console's manual registration upload remains available for recovery.

## Fingerprint bytes

The fingerprint is `SHA256` of the following bytes, with unsigned 32-bit integers in big-endian order:

```text
ASCII("codex-usb-fingerprint-v1") || 00
|| anchorSchema:u32be || volumeSerial:u32be
|| hex_decode(lowercase(anchorSha256))
|| utf8_byte_length(canonical_bundle_path):u32be
|| canonical_bundle_path:utf8
```

Before encoding, backslashes become `/`; ASCII `A`–`Z` in each path byte become `a`–`z`. Reject absolute paths, empty components, `.` and `..` components, `:`, NUL, invalid UTF-8, and malformed 32-byte anchor hashes. The returned digest is lowercase hex. This matches the fixed vector in `internal/revocation/canonical_test.go`.

## Check request and signed response

The request JSON contains only schema version, 32 lowercase hex request ID, product, installation ID, license signature SHA-256, signed registration envelope, and the observed fingerprint (`volumeSerial`, `anchorSha256`, `bundleRelativePath`, `anchorSchema`). The service never receives a full `UsbLicense` or account binding.

On success, HTTP 200 returns an envelope with Base64 of the exact JSON payload bytes and Base64 Ed25519 signature over those raw bytes. Payload schema fields are `schemaVersion`, `product`, `installationId`, `licenseSignatureSha256`, exact `requestId`, `status` (`active` or `revoked`), `stateVersion`, UTC RFC3339 `issuedAt`, and `signingKeyId`. Key rotation uses a new key ID and separate client trust update.

## Durable state and privacy

SQLite uses WAL, `synchronous=FULL`, foreign-key mode, and a 2500 ms busy timeout. `licenses` stores only product, installation ID, key ID, the optional 404token asset number, license signature digest, fingerprint digest, status/version/timestamps and short revoke reason. `audit_events` is transactional SQL history. Every registration, asset-number binding, and status mutation is also hashed into two append-only JSONL files on separate configured storage paths. Startup and every check verify both journals match and that the journal, SQL history, and records are consistent. Mutations serialize with API checks using an exclusive lock marker; a crash leaves a fail-closed marker that an operator removes only after confirming no writer is alive.

An append is fsynced to both journals before the SQLite transaction; failure or a crash cannot produce a signed state before state and audit agree. Startup may replay a database snapshot that is an exact prefix of the fully verified dual journals. It never signs a divergent or journal-ahead state. A restore marker always disables signing until an operator recovers and verifies state. The optional admin console has separate account/password authentication, in-memory sessions, strict configured Origin checks, and CSRF tokens; see the VPS admin-console development design. The CLI remains available for server recovery and local maintenance.

The service logs only outcome category, installation ID, state version when available, request ID, elapsed time, and service lifecycle messages. It does not log registration bytes, signature, fingerprint, private key, reason, or database content. SQLite/journal data contains no full fingerprint or bearer credential.

## Fixed local test vector

The deterministic fixture is test-only. License signing public keyring:

```json
{"keys":{"license-test-2026":"0EqyMnQrtKs6E2i9RhXk5tAiSrcaAWuvhSCjMsl3hzc="}}
```

Status response test public key is `revocation-test-2026` with Base64 public key `oJql9HpnWYAv+VX43C0qFKXJnSO+l/hkEn/5ODRVpPA=`. Given the payload bytes below, the expected local test signature is `xpUQ2sKA/5I6GCh76LTmoUYnMnVia7tfQEtTbkQsmhTuiK+HFRW6hYUB6PdzS5d9IY63rPa3RioJxvkwBn71CQ==`:

```json
{"schemaVersion":1,"product":"codex-usb-oneclick","installationId":"usb-install-001","licenseSignatureSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","requestId":"00112233445566778899aabbccddeeff","status":"active","stateVersion":1,"issuedAt":"2026-10-09T00:00:00Z","signingKeyId":"revocation-test-2026"}
```

The exact test envelope is:

```json
{"payloadB64":"eyJzY2hlbWFWZXJzaW9uIjoxLCJwcm9kdWN0IjoiY29kZXgtdXNiLW9uZWNsaWNrIiwiaW5zdGFsbGF0aW9uSWQiOiJ1c2ItaW5zdGFsbC0wMDEiLCJsaWNlbnNlU2lnbmF0dXJlU2hhMjU2IjoiYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYSIsInJlcXVlc3RJZCI6IjAwMTEyMjMzNDQ1NTY2Nzc4ODk5YWFiYmNjZGRlZWZmIiwic3RhdHVzIjoiYWN0aXZlIiwic3RhdGVWZXJzaW9uIjoxLCJpc3N1ZWRBdCI6IjIwMjYtMTAtMDlUMDA6MDA6MDBaIiwic2lnbmluZ0tleUlkIjoicmV2b2NhdGlvbi10ZXN0LTIwMjYifQ==","signatureB64":"xpUQ2sKA/5I6GCh76LTmoUYnMnVia7tfQEtTbkQsmhTuiK+HFRW6hYUB6PdzS5d9IY63rPa3RioJxvkwBn71CQ=="}
```

The fixture signing seeds exist only in Go tests and are public test material; never copy either fixture key into a deployment.
