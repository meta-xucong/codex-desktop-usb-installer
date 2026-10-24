# Windows network preflight repair (2026-10-23)

## Diagnosis

The USB install preflight requested `https://404token.xyz/` through the regular
small-text fetcher. That fetcher uses curl `-f`, which reports HTTP 4xx/5xx as
transfer failures. A reachable host returning 404 could therefore be reported
as “no internet”. The attached manager log also recorded a timeout and no curl
proxy environment variables; this is consistent with a Windows user proxy/PAC
not being applied by curl's empty `System` proxy configuration, although the
log alone does not prove that the test computer depended on a proxy.

The preflight runs before the install staging command and again at the backend
mutation boundary. The UI maps its network result to a boolean, so the prior
failure looked like a generic internet problem. Account-binding and signed USB
guards are separate checks and remain in place.

## Change

- Added a dedicated HTTPS reachability probe. A completed request with an
  origin HTTP response (including 404, 429, or 5xx) passes; DNS, connection,
  TLS, timeout, missing HTTP response, and proxy authentication (407) fail.
  Redirects are not followed: an origin 3xx itself proves reachability.
- In Windows `System` proxy mode without explicit proxy environment overrides,
  the probe uses WinHTTP automatic proxy so Windows system/user proxy and PAC
  settings are applied. `HTTPS_PROXY`/`ALL_PROXY` environment overrides
  continue to be honored; `NO_PROXY` alone does not disable Windows proxy/PAC
  discovery. `Direct` and `Custom` modes retain their existing curl behavior.
- Switched only the USB installation network gate to this probe. Regular curl
  downloads retain `-f` and continue to reject HTTP errors.
- Kept the existing account-key binding, USB signature, and install-stage
  gates unchanged.

## Scope and limitations

This repairs install preflight reachability, not every network request in the
manager. Other metadata and package transfers still use their existing curl
transport and may not resolve Windows PAC settings when no proxy environment
variable is configured. Those requests should be addressed separately if logs
show failures on a proxy-only network; broadening transport behavior here would
change unrelated update and download paths.

WinHTTP applies per-stage DNS/connect/send/receive timeouts. These do not
provide a strict cancellable wall-clock deadline around PAC discovery and all
automatic proxy retries. A pathological or stalled PAC script may therefore
take longer than the individual request-stage limits. This is a remaining
limitation requiring a separate asynchronous/cancellable proxy-resolution
change; the current probe records the WinHTTP error code when it fails.

An HTTP response proves connectivity to the host, not 404token API health or
account validity. Those are validated by their own API-key and service calls.

## Verification

- The focused HTTP-status regression test passes for 200, redirects, 404, 405,
  429, 5xx and rejects 407/out-of-range values.
- The Windows app and engine compile with `cargo check`.
- An explicitly run live probe on the development host failed: its inherited
  curl proxy points to `127.0.0.1` and is unavailable. With that process-local
  proxy override removed, WinHTTP also could not reach the host from this
  environment. This is evidence about the development environment's outbound
  route, not the user's new test machine.
- The full engine test suite ran 245 tests; 203 passed, 40 failed on sandbox
  filesystem/AppX fixture operations with access-denied or unavailable-path
  errors, and 2 were ignored. The new network regression test passed separately.
- A successful real proxy/PAC install on a separate test machine remains the
  final environment-level confirmation.
