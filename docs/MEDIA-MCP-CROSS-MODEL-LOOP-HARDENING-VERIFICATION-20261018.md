# Unified Media MCP cross-model loop hardening — verification

Date: 2026-10-18
Repository: `meta-xucong/codex-desktop-usb-installer`
GitHub-tracking baseline: `b61eaf783db12d35c46405f4effbf4973b876d4b`
Previously recorded MCP release: `1.3.10`, commit `896086e15b38f954aef5411edede3b2e6fefa7d7`
Implementation version: `1.3.11`
Implementation source SHA-256: `29D4F961255B01FEA84AE57734E45C1C63B8A19E228F5981C64D3B6B67040ADD`

## Review and offline verification

- Design review: **PASS**. The reviewer confirmed the static schema and loop-guard design preserve the frozen image-edit, video-reference, route, and submit-state invariants.
- Independent read-only code review: **PASS** on the final implementation diff. It confirmed the static `n` union covers legacy Image-2 `1..8` and non-Wokey Grok image `1..10`, while runtime route restrictions remain authoritative; the bounded guard and one-read preflight are correct; and first-use confirmation-key initialization is serialized.
- `python -B installer-pack/test-media-generation-mcp.py`: **63/63 passed**.
- `python -B installer-pack/test-adapter-contracts.py`: **76 passed, 25 skipped**. The skips are legacy MCP fixtures explicitly superseded by the dedicated suite.
- Concurrency stress: the loop-guard concurrency test and the 16-thread first-key initialization test each passed 40 repeated runs (**80/80**).
- Python syntax compilation: passed for the MCP and both test modules.
- `python -B tools/refresh-capability-pack-metadata.py installer-pack --check`: passed, 309 manifest entries and 310 checksum entries.
- `git diff --check`: passed; only Git line-ending normalization warnings remain.

Protected regression coverage passed for Image-2 and Grok image-to-image, Wokey restrictions, Grok video `first_frame` versus `reference`, payload construction, unknown-submission handling, and the single-submit state machine. No provider payload builder or media role conversion was changed by this work.

## Kimi K3 live test — incomplete, do not count as pass

The Codex-registered MCP on this workstation still reports **1.3.9**. To avoid testing that stale installation as if it were the new code, the live attempt launched the local `1.3.11` source as a separate stdio MCP process, with its API credential decrypted only in the current-user process environment.

The Kimi chat-completions response identified its model as `kimi-k3` and made one `media_prepare` tool call. The local `1.3.11` MCP then reported that a media request had been sent (`api_request_sent: true`) during the single `media_submit` call. The acceptance harness crashed immediately afterward while reading the normal success envelope (`error: null` as though it were a dictionary), before it captured the task ID or queried/saved the result. Its cleanup also removed the isolated temporary confirmation record. No second submit was attempted.

Therefore the actual generation outcome, returned task/model summary, and saved artifact are **unverified**. For duplicate-charge safety, treat the one request as potentially accepted and do not resubmit it. This is an acceptance-harness failure after one request reached the MCP submission path; it is not evidence of an MCP validation-loop failure or a provider failure. The Kimi-driven `media_prepare` did not loop.

## Release gate

The source implementation, offline regressions, independent code audit, and package metadata checks pass. The requested Kimi K3 end-to-end video acceptance does **not** pass because its sole real submit was not captured or polled. Do not claim full live acceptance or package a release on the strength of that attempt. A further paid attempt requires first resolving whether the prior request was accepted, then obtaining authorization for a new exact request if needed.

This verification note records the observed test limitation; it does not alter the approved implementation scope in `MEDIA-MCP-CROSS-MODEL-LOOP-HARDENING-20261018.md`.
