# Provider Profile

A provider profile is a scoped compatibility record. It is not a global model capability table.

## Scope

Every visual-input decision is bound to:

```text
host + profile + model + api_contract + date
```

The same model name at two hosts, or the same host behind two profiles, may expose different fields, limits, routing, or image semantics.

## Contract layers

Keep these layers separate:

- `client_contract`: the request that the local MCP sends to the configured host, for example JSON `image.image_url`.
- `gateway_transport`: an internal conversion performed by a gateway, for example downloading a URL and forwarding `multipart/form-data` with `image[]`.
- `upstream_contract`: the protocol required by the actual media account or upstream model.

A gateway fix must not be copied into every client or treated as proof that another host has the same bridge. Conversely, an upstream field name must not be sent to a compatible host until that host's client contract is confirmed.

## Capability states

- `certified`: the exact profile/model/mode has recent evidence from a completed request and post-generation checks. Record the completed sample scope separately when it is narrower than a documented contract limit.
- `uncertified`: the profile declares the mode, but it has not been tested enough for paid generation.
- `unknown`: no reliable profile evidence exists.
- `unsupported`: the profile explicitly does not expose the mode.

Certification is per visual mode. T2V success does not certify I2V. I2V success does not certify independent identity references, multiple reference images, or temporal keyframes.

A documented provider limit is an admission constraint, while completed-task coverage is evidence about the subset that has been exercised. Keep them as separate fields such as `max_reference_images` and `verified_reference_image_count`: do not lower a documented request limit merely because only fewer inputs have been tested, and do not claim that a higher limit has completed visual QA without such a run.

Prompt length has two distinct notions. `provider.prompt_soft_budget_bytes` is a local quality signal for repetition or over-constrained language and must never reject a request. Set `provider.max_prompt_bytes` only when the active host, profile, and model have an evidenced transport limit from provider documentation, a gateway response, or a controlled probe. A missing value means no hard prompt-length limit has been established.

## Aiself Observed Profile

```json
{
  "host": "https://aiself.vip/v1",
  "profile": "aiself-grok",
  "model": "grok-imagine-video-1.5",
  "api_contract": "aiself-wokey-video-v1",
  "mode": "reference_to_video",
  "client_fields": ["reference_images[].url"],
  "gateway_transport": "validate public HTTPS URLs, preserve order, then send one multipart image[] part per reference with mode=multimodal_reference",
  "upstream_contract": "Wokey multipart/form-data multimodal_reference; Studio shows Grok Imagine Video 1.5 up to 7 images",
  "limits": {"max_reference_images": 7, "verified_reference_image_count": 2, "max_prompt_bytes": 4096},
  "verified_at": "2026-08-15",
  "status": "certified",
  "evidence": "A controlled two-image public-reference task completed: Aiself status returned mode=multimodal_reference, model grok-imagine-video-1.5, 5s, 720p, 16:9. A later 4203-byte reference-to-video request was rejected before task creation with an explicit 4096 UTF-8-byte Grok video limit for this host route."
}
```

This record is intentionally separate from direct xAI documentation. Aiself accepts compatible JSON `reference_images`, then adapts them to Wokey multipart rather than forwarding xAI JSON unchanged. The active Wokey contract limit is seven references; the completed runtime evidence currently covers two references at 5 seconds, 720p, 16:9. Its `max_prompt_bytes=4096` is a measured admission limit for this Aiself/profile/model/reference mode only; do not apply it to direct xAI, a different gateway, I2V, or T2V without their own evidence. Keep contract limits and runtime sample coverage separate. Do not merge reference images into an I2V first frame or retry failures as T2V.

## Evidence record

Keep a compact record with:

```json
{
  "host": "https://video.example/v1",
  "profile": "example-grok",
  "model": "grok-imagine-video-1.5",
  "api_contract": "legacy-video-v1",
  "mode": "image_to_video",
  "client_fields": ["ratio", "image.image_url"],
  "gateway_transport": "unknown",
  "limits": {"duration_seconds": {"min": 1, "max": 15}},
  "verified_at": "YYYY-MM-DD",
  "status": "certified",
  "evidence": "Completed task ID plus opening-frame and media checks"
}
```

Do not copy `limits`, `status`, or `evidence` from another host. A successful HTTP response without a task ID is not generation evidence. A completed task without visual QA is not proof that the reference image had the intended meaning.

## Preflight and cost gate

Before a visual-input task:

1. Read the actual loaded MCP/profile configuration.
2. Run `get_video_preflight` with the exact model, mode, duration, resolution, ratio, and reference count.
3. If the mode is `unknown` or `uncertified`, stop before creating a paid task. Use `allow_uncertified_visual_input=true` only for an explicit, bounded certification attempt.
4. Create one task, retain its request ID, and poll/download only that task.
5. After completion, check both technical metadata and the semantic role of the reference in the opening and key frames.

For T2V, the visual-input cost gate does not apply, but the selected profile and model still control the request fields and limits.
