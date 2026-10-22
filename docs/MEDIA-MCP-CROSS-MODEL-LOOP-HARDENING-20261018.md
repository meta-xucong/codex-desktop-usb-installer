# Unified Media MCP cross-model loop hardening

Date: 2026-10-18
Status: design baseline for implementation and audit

## 1. Goal

Prevent repeated invalid `media_prepare` calls across supported chat models by making the MCP tool schema portable, returning a precise one-step correction, and stopping repeated failures in the MCP process. Keep all provider validation and media semantics in the MCP runtime.

The fix is MCP-only. Do not modify Sub2API, provider adapters, account routing, upstream request formats, or Codex model settings.

## 2. Frozen baseline and evidence

- Repository: `meta-xucong/codex-desktop-usb-installer`.
- Local branch: `codex/windows-usb-ci-scope-20261016`.
- Baseline commit: `b61eaf783db12d35c46405f4effbf4973b876d4b`.
- MCP server version: `1.3.10`.
- MCP source SHA-256: `35bebf0ca354beed98792c98c5383f42da84b61c` (Git blob hash from the current clean MCP path; use a file SHA-256 in the implementation receipt).
- The local `github/main` tracking ref contains `896086e15b38f954aef5411edede3b2e6fefa7d7`, which changes the MCP source and records version `1.3.10`; `b61eaf7` is the current local tracking tip. Direct GitHub/API access was blocked by the machine proxy during this task, so these are local fetched-ref records, not a live remote refresh.
- Existing workspace changes outside the MCP scope are preserved. At baseline, the MCP source itself is clean.
- Existing Kimi design and verification record: `docs/KIMI-MCP-TOOL-LOOP-REPAIR-20261018.md`.

## 3. Scope and non-goals

### Allowed changes

1. `installer-pack/mcp/media-generation-mcp.py`.
2. `installer-pack/test-media-generation-mcp.py`.
3. MCP-specific package version/integrity metadata only if the repository's existing packaging checks require it.
4. This design document and a short implementation/audit evidence note.

Do not touch the pre-existing installer/model-manager/signer changes or unrelated untracked audit documents.

### Explicit non-goals

- No Sub2API source, database, route, provider, or VPS changes.
- No edits to image/video payload construction, upload/download, references, or provider-specific adapters.
- No weakening, bypassing, or replacing runtime validation.
- No change to confirmation, authorization, exactly-once submit, unknown-submission, or recovery behavior.
- No new provider, model, credential, dependency, or network endpoint.
- No real request during offline tests.
- No Git commit, push, deployment, or EXE/signer release build in this task.

## 4. Hard invariants: previously accepted media behavior

The implementation and audit must prove all of the following remain unchanged:

1. Image-2 text-to-image and image-to-image still select `generate`/`edit` from references, preserve supported local-image validation and size handling, and retain the current profile-specific `n=1` restrictions.
2. Grok image text-to-image and image-to-image retain the current accepted model, resolution/ratio/quality behavior and Wokey text-only restriction.
3. Grok video retains explicit `reference_media[].role` semantics: `first_frame` and `reference` remain distinct; mixed-mode and model/route limitations remain enforced; legacy role omission remains interpreted as `first_frame`.
4. Grok video continues to require valid `aspect_ratio` and `resolution` at runtime, including the same allowed values and local image checks.
5. Seedance and MiniMax retain their existing text/reference limitations, duration/ratio/resolution/audio behavior, and route restrictions.
6. Request payload builders and provider-specific wire formats remain byte-for-byte or semantically unchanged outside test-only fixtures.
7. `media_prepare` remains non-billable. It must not send a generation request. It may create a confirmation only after the existing runtime validation succeeds.
8. `media_submit` remains gated by the existing exact confirmation and user-approval fields; an already-sent or unknown submission is never automatically repeated.
9. The loop guard must affect only repeated local validation failures for `media_prepare`. It must not block valid corrected input, unrelated prompts, `media_check`, `media_save`, or a prepared/approved submit.

If a proposed change requires relaxing or changing any invariant, stop and return to the user for a revised scope.

## 5. Root cause addressed

The current MCP advertises provider-conditioned arguments through root-level `allOf` containing `if/then`. Some tool-call paths may not preserve or act on these conditionals. The MCP runtime already performs authoritative provider-specific validation, so the portable contract should not depend on conditional JSON Schema support.

Separately, invalid arguments return an MCP error but do not create a confirmation record. Consequently, the existing duplicate-confirmation protection cannot stop a model from issuing the same invalid `media_prepare` repeatedly. Add a small in-process guard for this local preflight loop only.

## 6. Required implementation

### 6.1 Portable static `media_prepare` schema

Replace the root conditional `allOf` contract with one stable root object schema. Keep the same top-level fields and runtime-required fields (`provider`, `prompt`). Keep `parameters` as an object with `additionalProperties: false` and explicitly declared optional fields. Use field-level union enums/ranges across supported providers where useful, but do not encode per-provider `if`, `then`, root `allOf`, or root `oneOf`/`anyOf`.

The tool description must be concise and state:

- use `media_capabilities` for current configured providers/models;
- use only the parameter names described for the selected provider;
- server validation is authoritative and a rejected preflight sends no media request;
- after a validation error, correct the listed fields once; do not repeat unchanged arguments;
- image/video reference roles and route limitations remain as they are today.

Do not remove `MEDIA_PROVIDER_PARAMETER_SCHEMAS` information if it is reused to build the static field-level unions or runtime tests. Do not duplicate provider validation in the schema builder in a way that can silently drift from `_validate_request`.

The runtime `_validate_request` behavior remains the source of truth. If a model sends a union-valid but provider-invalid value, return the provider-specific validation result described below.

### 6.2 Actionable validation result

For local `media_prepare` validation errors, return structured details in addition to the current safe message:

- stable error code;
- `retryable: false` for unchanged arguments and a `recovery_action` of `correct_arguments_once` when a concrete correction exists, otherwise `stop_and_ask_user`;
- `api_request_sent: false`;
- `confirmation_created: false`;
- `missing_parameters`, `invalid_parameters`, and `allowed_values` only when applicable;
- a short instruction to change the named fields and retry once.

Never claim `api_request_sent: false` for an error that occurred after a generation POST. Keep submit/check/save envelopes and their established meanings unchanged. Do not include prompt bodies, local file paths, API credentials, image/video bytes, or signed URLs in diagnostic logs or fingerprints.

### 6.3 In-process loop guard

Implement a bounded, in-memory `media_prepare` validation-failure guard. It must not persist data to disk and must not log raw input.

Key each attempt by a privacy-safe intent digest made from normalized provider, prompt digest, and reference-media identity digests; track the exact argument digest separately. On validation failures only:

- allow at most three validation failures for one intent within the window: the initial attempt plus at most two changed-argument corrections;
- if the exact same arguments fail twice for the same intent, return `LOOP_GUARD` on the repeated call;
- if three validation failures occur for the same intent within 120 seconds, return `LOOP_GUARD` even when the model makes superficial argument changes;
- explain that no media request was sent and ask the model to stop calling the tool until the user changes the request or parameters;
- clear that intent's failure state after a successful `media_prepare` and expire stale state.

Do not count network/catalog failures as argument-validation failures. Do not count submit/check/save failures. Protect the map/counters with a lock so concurrent MCP calls cannot exceed the budget. Keep a fixed upper bound on stored intent entries and evict expired/oldest entries without touching confirmation records.

The two-attempt exact-argument and three-failure intent limits are deliberate public behavior. An unchanged invalid call is stopped on its second failure; changed arguments may fail up to three times total for that intent. Tests and docs must use these exact limits; do not invent additional retries, backoff, or persistence.

### 6.4 Tool error protocol

Keep returning validation failures as MCP tool results with `isError: true` and `structuredContent`. The error's human-readable text and structured fields must agree. `LOOP_GUARD` must be a terminal instruction; it must not mark the error retryable.

## 7. Required tests

Add tests to `installer-pack/test-media-generation-mcp.py`; use the existing in-process fake HTTP transport. Tests must be deterministic and make zero real network calls.

### Schema tests

- `tools/list` schema has a root object and contains no conditional root `allOf`, `if`, or `then`.
- All existing provider names and reference role fields remain present.
- The static schema does not assert that a field is valid for every provider; provider-invalid union values are still rejected by runtime validation.

### Error and loop-guard tests

- Missing Grok video `aspect_ratio`/`resolution` returns their names and supported values, with no confirmation file and zero POSTs.
- Unsupported cross-provider parameters return a named correction and no POST.
- One corrected call can succeed and creates exactly one normal confirmation.
- The exact same invalid call twice returns `LOOP_GUARD` on the repeated call.
- Three validation errors for one intent with superficial parameter changes return `LOOP_GUARD`.
- A different prompt/provider is not blocked; successful prepare clears that intent's counter.
- `media_refresh_models`, catalog/network failure, submit, check, and save do not increment the validation counter.
- Concurrent identical invalid calls cannot exceed the configured limit.
- MCP `tools/call` response uses `isError: true`, structured fields match its text, and no network request is made.
- Expired entries are cleared, the in-memory store remains bounded, and raw prompt/path/key canaries do not appear in serialized diagnostics.

### Protected media regression tests

Retain and run the existing tests covering Image-2 image-to-image, Grok image-to-image, Wokey restrictions, Grok video first-frame versus reference mode, mixed references, local image MIME/size, Seedance and MiniMax constraints, payload construction, exactly-once submit, `submission_unknown`, media checking and saving. Add only the minimal regression assertion needed to prove the loop guard did not intercept these valid paths.

## 8. Verification and release gates

Run and record:

1. MCP offline suite: `python -B installer-pack/test-media-generation-mcp.py`.
2. Adapter contract suite and capability-pack integrity/audit commands documented by the repository.
3. Python bytecode compilation of the MCP and its test file.
4. `git diff --check`; review exact diff and confirm no protected functions/payload code changed except required error-envelope wiring.
5. Independent read-only code audit against this document and the frozen baseline. Auditor returns `PASS`, `FAIL`, or `INSUFFICIENT_EVIDENCE`; a finding must be corrected and audited again on the new frozen diff.
6. After code audit passes, run a real Kimi K3 text-model tool-call flow using the updated local MCP and existing configured gateway. First verify model/tool route without a media generation POST. Then prepare one simple video through MCP, explicitly show model/provider/prompt/duration/ratio/resolution and cost uncertainty, and submit at most once. If the user-provided authorization in this task is treated as the approval for this exact test, use the displayed test request; otherwise pause before `media_submit` for approval.
7. The actual video must be generated through `media_prepare` → `media_submit` → `media_check`/`media_save`, with the MCP's configured video provider, while Kimi K3 is the chat model making the tool calls. Do not call the generation API directly. Record Kimi tool-call count, MCP version/hash, confirmation ID, submission state, provider request ID, and saved file path; redact credentials and URLs.

For live Kimi verification, if this session cannot select or prove that Kimi K3 is the calling chat model, report `ROUTE_UNVERIFIED`; do not relabel a GPT-driven MCP call as a Kimi test. If a submit returns `submission_unknown`, stop and do not resubmit. A failed real test is a finding, not permission for repeated billed attempts.

Only after the implementation audit, offline tests, and the Kimi-driven real video test pass may the work be handed to the user for acceptance. Do not push, deploy, merge, or build a release installer/signer in this task.

## 9. Audit checklist for the design and implementation

- [ ] Current MCP baseline is tied to the local GitHub-tracking commit record.
- [ ] Schema removes conditional-keyword reliance without dropping fields needed by the provider validators.
- [ ] Runtime provider validators and media payload builders retain their behavior.
- [ ] Image-to-image, Grok first-frame/reference video, and route-specific reference limits are explicitly regression-tested.
- [ ] Loop guard is in-memory, bounded, privacy-safe, concurrency-safe, and scoped only to local prepare validation failures.
- [ ] Corrected valid input can proceed after a prior validation error.
- [ ] No local failure can create a confirmation or send a generation request.
- [ ] Exactly-once submission and unknown-submission protections are unchanged.
- [ ] Independent code audit is bound to the exact final diff.
- [ ] Actual Kimi K3 route provenance is established separately from functional test success.
- [ ] One successful saved video artifact is required for a full pass.
