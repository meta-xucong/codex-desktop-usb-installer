# Unified Media MCP version status

Date: 2026-10-18  
Repository: `meta-xucong/codex-desktop-usb-installer`

## Canonical current version

The canonical current source/package version is **1.3.11**. The server, PowerShell launcher, installer health gate, MCP manifest, and media-tool manifest all expect `1.3.11`.

Current server source SHA-256:

```text
0E735597E053DD8E0D7F5319F6FA1482AA22DBCFF4BE02AD234B4FF522E3F470
```

Version 1.3.11 includes the portable static `media_prepare` schema, actionable structured validation errors, and a bounded in-memory guard against repeated invalid `media_prepare` calls. It also retains the previously accepted image-edit and video-reference behavior, confirmation/approval gate, one-submit rule, and unknown-submission protections.

## Verification for this exact source snapshot

- `python -B installer-pack/test-media-generation-mcp.py`: **64/64 passed**. This includes `test_kimi_shaped_tool_call_can_prepare_grok_video_without_retry_loop` and the static-schema, repeated-invalid-call, corrected-input, concurrency, image-edit, video-reference, and submission-safety regressions.
- `python -B installer-pack/test-adapter-contracts.py`: **76 passed, 25 skipped**. The skipped legacy MCP fixtures are explicitly superseded by the dedicated 1.3.11 suite.
- The package contract test confirms the MCP source, wrapper, installer gate, and manifests agree on version 1.3.11.

These are local source/package tests. They do not prove that a real Kimi-driven video was completed and saved.

## Live Kimi result and limitation

The recorded Kimi K3 attempt made one `media_prepare` call and did **not** loop. The subsequent single `media_submit` reached the media request path, but the acceptance harness crashed while reading the response, before recording the task ID or completing `media_check`/`media_save`. The video result is therefore **unverified**; do not treat the attempt as full end-to-end generation acceptance or submit it again without first resolving whether the prior request was accepted.

At the time of that recorded attempt, the normally registered Codex MCP still reported 1.3.9; the test explicitly launched the local 1.3.11 source instead. The recorded test therefore validates 1.3.11's tool flow, not that an existing Codex registration has already been upgraded.

## Deprecation

- **1.3.9 — deprecated; do not use for new installs or verification.**
- **1.3.10 — superseded by 1.3.11; deprecated.**
- **1.3.11 — canonical current source/package version.**

An existing Codex registration may continue running its old copied server until the current installer updates the managed MCP files and Codex is restarted. The installer health gate rejects a server version other than 1.3.11. This status document does not itself modify an already-installed user's Codex configuration, build a release, or publish the package.

## Audit trail note

The earlier `MEDIA-MCP-CROSS-MODEL-LOOP-HARDENING-VERIFICATION-20261018.md` records an earlier source snapshot (`29D4F961...`) and 63 passing MCP tests. The exact current source hash above differs and the dedicated suite now contains 64 tests; use this status record for the present working-tree snapshot, not the earlier hash as if it described current bytes.
