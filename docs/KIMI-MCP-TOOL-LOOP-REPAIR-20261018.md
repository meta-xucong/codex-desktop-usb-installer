# Kimi media tool-call loop: diagnosis and repair

Date: 2026-10-18

## Finding

The reproduced loop was caused by an incomplete MCP tool schema, not by a Kimi model-routing failure.

`media_prepare` required `parameters.aspect_ratio` and `parameters.resolution` for Grok video at runtime, but its MCP schema advertised `parameters` as an empty generic object. Kimi therefore had no declared field names, types, or allowed values to follow. The server correctly rejected the missing fields before creating a confirmation, so repeated calls could not progress.

The production 404token Sub2API logs also show successful Kimi requests and the expected model mapping (`kimi-k3` to the upstream Kimi model). The active Kimi route includes a Kimi Coding Plan Chat Completions account and a YeToken-compatible account. Both 404token and aiself were running images built from commit `2bedb713`; no Sub2API code or account-routing change was justified by the evidence.

## Change

- Advertise explicit per-provider conditional schemas for parameters accepted by the existing provider validators, including allowed enums, ranges, and required fields.
- Prevent schema-valid but runtime-invalid cross-provider values (for example image resolutions on video providers, Seedance-only ratios on Grok video, and `generate_audio=false` on Seedance).
- Restrict `role` to Grok video references and constrain image count to one per request so the static schema remains compatible with both `responses` and `wokey` routes.
- Clearly describe route-specific reference-media limits: Wokey Grok image, Seedance video, and MiniMax video currently support text-only requests; Grok video references are supported only on the authenticated Responses route.
- Keep the existing runtime validation, confirmation flow, and paid submit boundary unchanged.
- Increment the media MCP version to `1.3.10` and refresh the embedded capability-pack integrity metadata.

## Verification

- Packaged USB media MCP suite: 54 tests passed.
- Development-copy media MCP suite: 53 tests passed.
- Installer adapter contracts: 76 tests passed, 25 skipped by their existing policy.
- Independent audit found that the initial union enums could still advertise invalid cross-provider values; the final schema now narrows values per provider and adds tests for those boundaries.
- Capability-pack integrity check and offline pack audit passed.
- Python bytecode compilation passed for service and tests.
- Three live text-only Kimi K3 tool-call probes through 404token returned a single `media_prepare` call with valid `duration`, `aspect_ratio`, and `resolution`; the actual packaged MCP `media_prepare` accepted all three. Each returned `api_request_sent=false`; no image/video generation was submitted.
- Production logs show successful Kimi `/v1/responses` requests and expected account/model mapping. This proves route transport and model mapping, not every possible Kimi conversation.

## Deployment boundary

The MCP fix belongs in the USB installer capability pack. It does not run inside Sub2API, so deploying Sub2API images cannot install this fix. 404token and aiself already use the same Sub2API source commit and their Kimi route does not need a server deployment for this schema defect.

The local Docker Desktop/WSL engine was unavailable to this session (`docker` pipe access denied; WSL access denied). Therefore a local Sub2API database/account import and full local gateway replay were not completed. No production credentials were copied into a local file, and no Sub2API production configuration was changed.
