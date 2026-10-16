# Windows USB CI scope

The Codex USB edition is a Windows product. Its pull-request gate is the
Windows Rust workspace checks, frontend checks, and the Windows installer
lifecycle workflow. macOS packaged validation is optional and manually
dispatchable; it is not a release requirement for the USB edition.

Some standalone checks are intentionally scoped to changes in the code they
cover:

- `codex-win-engine` standalone tests run when `crates/codex-win-engine/**`
  changes. This keeps AppX/WpSystem deployment-fixture failures from blocking
  unrelated installer and model-manager work while retaining coverage for
  changes to that engine.
- `codex-theme-engine` standalone tests run when
  `crates/codex-theme-engine/**` changes.
- Rust and npm dependency audits run when dependency manifests, lockfiles, or
  audit policy files change. Existing advisories therefore do not block
  application-only changes; dependency changes still receive the audit gate.

These checks are scoped, not deleted. The separate macOS app release workflow
and its release validation remain available and unchanged.
