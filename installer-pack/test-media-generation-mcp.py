#!/usr/bin/env python3
"""Offline regression tests for the unified media-generation MCP.

This suite uses only Python's standard library. Every HTTP interaction is
intercepted by an in-process fake transport; it never opens sockets, contacts
a real provider, or uses a real credential. Run with:

    python scripts/test-media-generation-mcp.py
"""

from __future__ import annotations

import base64
import copy
import contextlib
import email.message
import hashlib
import io
import importlib.util
import json
import os
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import unittest.mock
import zlib
from pathlib import Path
from typing import Any


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent if HERE.name.casefold() == "scripts" else HERE
SERVICE_PATH = ROOT / "scripts" / "media-generation-mcp.py"
if not SERVICE_PATH.is_file():
    SERVICE_PATH = ROOT / "mcp" / "media-generation-mcp.py"
SPEC = importlib.util.spec_from_file_location("media_generation_mcp_test_subject", SERVICE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"Cannot load service: {SERVICE_PATH}")
media = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = media
SPEC.loader.exec_module(media)


def load_fresh_service(module_name: str):
    """Load a second module instance to approximate a fresh MCP process."""
    fresh_spec = importlib.util.spec_from_file_location(module_name, SERVICE_PATH)
    if fresh_spec is None or fresh_spec.loader is None:
        raise RuntimeError(f"Cannot load fresh service: {SERVICE_PATH}")
    fresh = importlib.util.module_from_spec(fresh_spec)
    sys.modules[module_name] = fresh
    fresh_spec.loader.exec_module(fresh)
    return fresh


CANARY_KEY = "sk-test-canary-do-not-leak-7f1c"
CANARY_PROMPT = "prompt-canary-秘密-49b2"
CANARY_QUERY = "signed-query-canary-秘密-8a31"


def _chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def valid_png() -> bytes:
    """Return a minimal, structurally valid 1x1 RGBA PNG."""
    signature = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    return signature + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", zlib.compress(b"\x00\x00\x00\x00\xff")) + _chunk(b"IEND", b"")


class FixtureState:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.requests: list[dict[str, Any]] = []
        self.responses: dict[str, list[Any]] = {}
        self.barriers: dict[str, tuple[threading.Event, threading.Event]] = {}

    def enqueue(self, path: str, status: int, payload: Any, headers: dict[str, str] | None = None) -> None:
        if isinstance(payload, bytes):
            body = payload
        else:
            body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        with self.lock:
            self.responses.setdefault(path, []).append((status, headers or {}, body))

    def enqueue_exception(self, path: str, exception: BaseException) -> None:
        with self.lock:
            self.responses.setdefault(path, []).append(exception)

    def block_next(self, path: str) -> tuple[threading.Event, threading.Event]:
        entered = threading.Event()
        release = threading.Event()
        with self.lock:
            self.barriers[path] = (entered, release)
        return entered, release

    def take(self, path: str, method: str, headers: dict[str, str], body: bytes, query: str = "") -> tuple[int, dict[str, str], bytes]:
        with self.lock:
            self.requests.append({"path": path, "query": query, "method": method, "headers": headers, "body": body})
            queue = self.responses.get(path, [])
            if queue:
                response = queue.pop(0)
            else:
                response = (404, {"Content-Type": "application/json"}, b'{"error":{"code":"not_found"}}')
            barrier = self.barriers.pop(path, None)
        if barrier:
            entered, release = barrier
            entered.set()
            if not release.wait(timeout=5):
                raise TimeoutError("fixture response barrier timed out")
        if isinstance(response, BaseException):
            raise response
        return response

    def count(self, method: str, path: str | None = None) -> int:
        with self.lock:
            return sum(1 for request in self.requests if request["method"] == method and (path is None or request["path"] == path))


class FixtureResponse:
    def __init__(self, status: int, headers: dict[str, str], body: bytes) -> None:
        self.status = status
        self.code = status
        self.headers = email.message.Message()
        for name, value in headers.items():
            self.headers[name] = value
        self._body = io.BytesIO(body)

    def read(self, size: int = -1) -> bytes:
        return self._body.read(size)

    def __enter__(self) -> "FixtureResponse":
        return self

    def __exit__(self, *_: Any) -> None:
        self._body.close()


class FakeHttpServer:
    """In-process stdlib HTTP transport fixture; performs no socket I/O."""
    _next_port = 18763
    _port_lock = threading.Lock()

    def __init__(self) -> None:
        self.state = FixtureState()
        self.server = self
        with self._port_lock:
            port = self._next_port
            type(self)._next_port += 1
        self.origin = f"http://127.0.0.1:{port}"

    @property
    def url(self) -> str:
        return self.origin

    def open(self, request: Any, timeout: float | None = None) -> FixtureResponse:
        parsed = media.urllib.parse.urlsplit(request.full_url)
        body = request.data or b""
        status, headers, payload = self.state.take(parsed.path, request.get_method(), dict(request.header_items()), body, parsed.query)
        if status >= 400:
            message = "Fixture HTTP error"
            response_headers = email.message.Message()
            for name, value in headers.items():
                response_headers[name] = value
            raise media.urllib.error.HTTPError(request.full_url, status, message, response_headers, io.BytesIO(payload))
        return FixtureResponse(status, headers, payload)

    def __enter__(self) -> "FakeHttpServer":
        self._patch = unittest.mock.patch.object(media.urllib.request, "build_opener", side_effect=lambda *_args, **_kwargs: self)
        self._patch.start()
        return self

    def __exit__(self, *_: Any) -> None:
        self._patch.stop()


class MediaMcpTestCase(unittest.TestCase):
    def setUp(self) -> None:
        with media._PREPARE_FAILURES_LOCK:
            media._PREPARE_FAILURES.clear()
        self.temp = tempfile.TemporaryDirectory(prefix="media-mcp-test-", dir=ROOT)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = self.root / "media-config.json"
        self.data = self.root / "data"
        self.env = {
            "MEDIA_GENERATION_TEST_MODE": "1",
            "MEDIA_GENERATION_CONFIG_PATH": str(self.config),
            "MEDIA_GENERATION_DATA_DIR": str(self.data),
            "MEDIA_GENERATION_API_KEY": CANARY_KEY,
            "MEDIA_GENERATION_API_PROFILE": "responses",
            "MEDIA_IMAGE2_MODEL": "gpt-image-2.5",
            "PYTHONIOENCODING": "utf-8",
            "PYTHONUTF8": "1",
        }
        self.env_patch = unittest.mock.patch.dict(os.environ, self.env, clear=False)
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.http = FakeHttpServer()
        self.http.__enter__()
        self.addCleanup(self.http.__exit__, None, None, None)
        os.environ["MEDIA_GENERATION_API_BASE_URL"] = self.http.url

    def refresh_catalog(self, model_ids: list[str] | None = None) -> dict[str, Any]:
        ids = model_ids or ["gpt-image-2.5", "grok-imagine-image-2.0", "grok-imagine-video-1.5", "seedance-2.0-mini"]
        self.http.state.enqueue("/v1/models", 200, {"data": [{"id": item} for item in ids]})
        return media.media_refresh_models({})

    def prepared_image(self, prompt: str = CANARY_PROMPT) -> dict[str, Any]:
        self.refresh_catalog()
        return media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": prompt, "parameters": {"size": "auto"}})

    def submit_args(self, prepared: dict[str, Any]) -> dict[str, Any]:
        return {
            "confirmation_id": prepared["confirmation_id"],
            "confirmation_fingerprint": prepared["confirmation_fingerprint"],
            "user_confirmation": "确认",
        }

    def record_path(self, confirmation_id: str) -> Path:
        return self.data / "confirmations" / f"{confirmation_id}.json"

    def test_minimax_reserved_slot_stays_unavailable_even_if_model_is_injected(self) -> None:
        environment = {
            "MEDIA_GENERATION_API_PROFILE": "wokey",
            "MEDIA_MINIMAX_VIDEO_MODEL": "MiniMax-H3",
        }
        with unittest.mock.patch.dict(os.environ, environment, clear=False), unittest.mock.patch.object(
            media, "_http_json", side_effect=AssertionError("MiniMax availability checks must not make network requests")
        ):
            capabilities = media._configured()
            slot = capabilities["providers"]["minimax-video"]
            self.assertFalse(slot["available"])
            self.assertFalse(slot["configured"])
            self.assertEqual(slot["model"], "")
            self.assertEqual(slot["missing"], ["PROVIDER_ADAPTER_UNAVAILABLE"])
            with self.assertRaisesRegex(media.MediaError, "没有已验证的官方 API 或统一网关适配"):
                media.media_prepare({"provider": "minimax-video", "model": "MiniMax-H3", "prompt": "Offline contract fixture"})
        self.assertFalse((self.data / "confirmations").exists())

    def test_image_mime_detector_recognizes_supported_signatures(self) -> None:
        fixtures = (
            (b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0d", "image/png"),
            (b"\xff\xd8\xff\xe0\x00\x10JFIF", "image/jpeg"),
            (b"GIF89a\x01\x00\x01\x00", "image/gif"),
            (b"RIFF\x00\x00\x00\x00WEBP", "image/webp"),
        )
        for data, expected in fixtures:
            with self.subTest(expected=expected):
                self.assertEqual(media._image_mime_from_bytes(data), expected)

    def test_local_image_mime_uses_content_not_extension(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        mislabeled = self.root / "jpeg-content-with-png-extension.png"
        jpeg_bytes = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01"
        mislabeled.write_bytes(jpeg_bytes)

        request = media._validate_request({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "MIME regression",
            "reference_media": [{"kind": "image", "role": "reference", "local_path": str(mislabeled)}],
            "parameters": {"duration": 5, "aspect_ratio": "1:1", "resolution": "480p"},
        })
        self.assertEqual(request["reference_media"][0]["mime_type"], "image/jpeg")
        path, payload = media._request_payload(request)
        self.assertEqual(path, "/v1/videos/generations")
        data_url = payload["reference_images"][0]["url"]
        self.assertTrue(data_url.startswith("data:image/jpeg;base64,"))
        self.assertEqual(base64.b64decode(data_url.split(",", 1)[1]), jpeg_bytes)
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_local_image_with_unknown_content_is_rejected(self) -> None:
        mislabeled = self.root / "not-an-image.png"
        mislabeled.write_bytes(b"plain text, not image bytes")
        with self.assertRaises(media.MediaError) as rejected:
            media._safe_reference({"kind": "image", "local_path": str(mislabeled)})
        self.assertIn("PNG、JPEG、WebP 或 GIF", str(rejected.exception))

    def test_model_catalog_gate_exact_match_stale_and_gateway_change(self) -> None:
        self.refresh_catalog(["gpt-image-2.5"])
        good = media._validate_request({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "catalog exact"})
        self.assertEqual(good["model"], "gpt-image-2.5")
        self.assertEqual(self.http.state.count("GET", "/v1/models"), 1)
        self.assertEqual(self.http.state.requests[-1]["query"], "")

        with self.assertRaises(media.MediaError) as missing:
            os.environ["MEDIA_IMAGE2_MODEL"] = "gpt-image-2.50"
            media._validate_request({"provider": "image-2", "model": "gpt-image-2.50", "prompt": "not in catalog"})
        self.assertEqual(getattr(missing.exception, "code", None), "MODEL_NOT_AVAILABLE")
        os.environ["MEDIA_IMAGE2_MODEL"] = "gpt-image-2.5"
        self.assertEqual(self.http.state.count("POST"), 0)

        config = json.loads(self.config.read_text(encoding="utf-8"))
        config["catalogVerifiedAt"] = int(time.time()) - media.MODEL_CATALOG_TTL_SECONDS - 1
        self.config.write_text(json.dumps(config), encoding="utf-8")
        with self.assertRaises(media.MediaError) as stale:
            media._validate_request({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "stale catalog"})
        self.assertEqual(getattr(stale.exception, "code", None), "MODEL_CATALOG_STALE")

        config["catalogVerifiedAt"] = int(time.time())
        self.config.write_text(json.dumps(config), encoding="utf-8")
        os.environ["MEDIA_GENERATION_API_BASE_URL"] = "http://127.0.0.1:9"
        with self.assertRaises(media.MediaError) as changed:
            media._validate_request({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "route changed"})
        self.assertEqual(getattr(changed.exception, "code", None), "MODEL_CATALOG_ROUTE_MISMATCH")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_wokey_catalog_refresh_includes_media_models(self) -> None:
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "wokey"
        ids = ["gpt-image-2.5", "grok-imagine-image-2.0", "grok-imagine-video-1.5", "grok-imagine-video-1.5-lite"]
        self.http.state.enqueue("/v1/models", 200, {"data": [{"id": item} for item in ids]})

        result = media.media_refresh_models({})

        self.assertTrue(result["success"])
        self.assertEqual(self.http.state.requests[-1]["query"], "output_modalities=all")
        self.assertEqual(result["models"]["image-2"], "gpt-image-2.5")
        self.assertEqual(result["models"]["grok-image"], "grok-imagine-image-2.0")
        self.assertEqual(result["models"]["grok-video"], "grok-imagine-video-1.5")
        self.assertEqual(result["catalog_count"], len(ids))
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_grok_video_explicit_parameters_are_in_confirmed_request_for_all_profiles(self) -> None:
        for profile, endpoint, expected_ratio_field, expected_resolution_field, duration_field in (
            ("responses", "/v1/videos/generations", "aspect_ratio", "resolution", "duration"),
            ("wokey", "/v1/videos", "ratio", "video_resolution", "duration_seconds"),
        ):
            with self.subTest(profile=profile):
                os.environ["MEDIA_GENERATION_API_PROFILE"] = profile
                self.refresh_catalog(["grok-imagine-video-1.5"])
                prepared = media.media_prepare({
                    "provider": "grok-video",
                    "model": "grok-imagine-video-1.5",
                    "prompt": f"explicit video parameters {profile}",
                    "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
                })

                self.assertEqual(prepared["parameters"], {
                    "duration": 4,
                    "aspect_ratio": "16:9",
                    "resolution": "480p",
                })
                record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
                self.assertEqual(record["request"]["parameters"], prepared["parameters"])
                path, body = media._request_payload(record["request"])
                self.assertEqual(path, endpoint)
                self.assertEqual(body[expected_ratio_field], prepared["parameters"]["aspect_ratio"])
                self.assertEqual(body[expected_resolution_field], prepared["parameters"]["resolution"])
                self.assertEqual(body[duration_field], prepared["parameters"]["duration"])

                task_id = f"task-explicit-video-params-{profile}"
                self.http.state.enqueue(endpoint, 200, {"id": task_id, "status": "queued"})
                submitted = media.media_submit(self.submit_args(prepared))
                self.assertEqual(submitted["task_id"], task_id)
                posted_body = json.loads(self.http.state.requests[-1]["body"].decode("utf-8"))
                self.assertEqual(posted_body[expected_ratio_field], prepared["parameters"]["aspect_ratio"])
                self.assertEqual(posted_body[expected_resolution_field], prepared["parameters"]["resolution"])
                self.assertEqual(posted_body[duration_field], prepared["parameters"]["duration"])

        os.environ["MEDIA_GENERATION_API_PROFILE"] = "responses"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        no_duration = media._validate_request({
            "provider": "grok-video",
            "model": "grok-imagine-video-1.5",
            "prompt": "duration stays caller-controlled",
            "parameters": {"aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertNotIn("duration", no_duration["parameters"])
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "wokey"
        path, payload = media._request_payload(copy.deepcopy(no_duration))
        self.assertEqual(path, "/v1/videos")
        self.assertEqual(payload["ratio"], "16:9")
        self.assertEqual(payload["video_resolution"], "480p")
        self.assertNotIn("duration_seconds", payload)

    def test_grok_video_reference_roles_use_xai_fields_and_keep_order(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        images = [
            {"kind": "image", "role": "reference", "url": f"https://assets.example.invalid/ref-{index}.png"}
            for index in range(3)
        ]
        prepared = media.media_prepare({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Reference role fixture",
            "reference_media": images,
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertEqual(prepared["operation"], "reference-to-video")
        self.assertEqual([item["role"] for item in prepared["reference_media"]], ["reference"] * 3)
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        path, body = media._request_payload(media._restore_request_urls(record["request"]))
        self.assertEqual(path, "/v1/videos/generations")
        self.assertNotIn("image", body)
        self.assertEqual(body["reference_images"], [{"url": item["url"]} for item in images])
        self.assertEqual(self.http.state.count("POST"), 0, "payload mapping must not submit a generation")

    def test_grok_video_local_reference_file_becomes_data_url_without_changing_role(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        raw_image = valid_png()
        local_path = self.root / "product-reference.png"
        local_path.write_bytes(raw_image)
        prepared = media.media_prepare({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Use as a visual reference",
            "reference_media": [{"kind": "image", "role": "reference", "local_path": str(local_path)}],
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertEqual(prepared["operation"], "reference-to-video")
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        path, body = media._request_payload(media._restore_request_urls(record["request"]))
        self.assertEqual(path, "/v1/videos/generations")
        self.assertNotIn("image", body)
        self.assertEqual(len(body["reference_images"]), 1)
        image_url = body["reference_images"][0]["url"]
        self.assertTrue(image_url.startswith("data:image/png;base64,"))
        self.assertEqual(base64.b64decode(image_url.split(",", 1)[1]), raw_image)
        self.assertEqual(self.http.state.count("POST"), 0, "preparing payload must not submit a generation")

    def test_grok_video_local_reference_over_gateway_limit_is_rejected_during_prepare(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        local_path = self.root / "oversized-reference.png"
        with local_path.open("wb") as image_file:
            image_file.write(b"\x89PNG\r\n\x1a\n")
            image_file.truncate(media.MAX_GROK_VIDEO_LOCAL_IMAGE_BYTES + 1)

        with self.assertRaises(media.MediaError) as rejected:
            media.media_prepare({
                "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Reject oversize reference",
                "reference_media": [{"kind": "image", "role": "reference", "local_path": str(local_path)}],
                "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
            })
        self.assertEqual(getattr(rejected.exception, "code", None), "REFERENCE_SIZE_LIMIT")
        self.assertIn("8 MiB", str(rejected.exception))
        self.assertEqual(self.http.state.count("POST"), 0, "oversized reference must be rejected before any generation request")

        # Confirmations from the previous MCP release can remain valid for
        # 30 minutes, so submit must enforce the same gateway bound too.
        legacy_record = {
            "state": "prepared",
            "request": {
                "provider": "grok-video", "model": "grok-imagine-video-1.5",
                "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
                "reference_media": [{"kind": "image", "role": "reference", "source": "file", "bytes": media.MAX_GROK_VIDEO_LOCAL_IMAGE_BYTES + 1}],
            },
        }
        with unittest.mock.patch.object(media, "_validated_record", return_value=(self.root / "legacy.json", legacy_record)):
            with self.assertRaises(media.MediaError) as submit_rejected:
                media._media_submit_locked({"user_confirmation": "确认"})
        self.assertEqual(getattr(submit_rejected.exception, "code", None), "REFERENCE_SIZE_LIMIT")
        self.assertEqual(self.http.state.count("POST"), 0, "old oversized confirmation must also be blocked before POST")

    def test_grok_video_first_frame_and_reference_mix_is_explicit_and_model_scoped(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        opening = "https://assets.example.invalid/opening.png"
        product = "https://assets.example.invalid/product.png"
        mixed = media._validate_request({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Mixed roles fixture",
            "reference_media": [
                {"kind": "image", "role": "first_frame", "url": opening},
                {"kind": "image", "role": "reference", "url": product},
            ],
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertEqual(mixed["mode"], "image-to-video-with-references")
        path, body = media._request_payload(mixed)
        self.assertEqual(path, "/v1/videos/generations")
        self.assertEqual(body["image"], {"url": opening})
        self.assertEqual(body["reference_images"], [{"url": product}])

        with self.assertRaises(media.MediaError) as duplicate_first:
            media._validate_request({
                "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Multiple first frames",
                "reference_media": [
                    {"kind": "image", "role": "first_frame", "url": opening},
                    {"kind": "image", "role": "first_frame", "url": product},
                ],
                "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
            })
        self.assertEqual(getattr(duplicate_first.exception, "code", None), "TOO_MANY_FIRST_FRAMES")

        for model, profile, roles, expected_code in (
            ("grok-imagine-video", "responses", ["first_frame", "reference"], "MIXED_REFERENCE_MODE_UNSUPPORTED"),
            ("grok-imagine-video-1.5-lite", "responses", ["reference"], "REFERENCE_MODE_UNSUPPORTED"),
            ("grok-imagine-video-1.5", "legacy", ["reference"], "REFERENCE_MODE_UNSUPPORTED"),
            ("grok-imagine-video-1.5", "wokey", ["reference"], "REFERENCE_MODE_UNSUPPORTED"),
        ):
            with self.subTest(model=model, profile=profile, roles=roles):
                os.environ["MEDIA_GENERATION_API_PROFILE"] = profile
                os.environ["MEDIA_GROK_VIDEO_MODEL"] = model
                self.refresh_catalog([model])
                refs = [{"kind": "image", "role": role, "url": f"https://assets.example.invalid/{index}.png"} for index, role in enumerate(roles)]
                with self.assertRaises(media.MediaError) as rejected:
                    media.media_prepare({
                        "provider": "grok-video", "model": model, "prompt": f"Reject {model} {profile} {roles}",
                        "reference_media": refs,
                        "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
                    })
                self.assertEqual(getattr(rejected.exception, "code", None), expected_code)
        self.assertEqual(self.http.state.count("POST"), 0, "unsupported reference modes must be rejected before submit")

    def test_grok_video_reference_limits_backward_compatibility_and_intent_binding(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        legacy = media._validate_request({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Legacy implicit first frame",
            "reference_media": [{"kind": "image", "url": "https://assets.example.invalid/first.png"}],
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertEqual(legacy["reference_media"][0]["role"], "first_frame")
        path, body = media._request_payload(legacy)
        self.assertEqual(path, "/v1/videos/generations")
        self.assertIn("image", body)
        self.assertNotIn("reference_images", body)

        ref_only = media._validate_request({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Intent role fixture",
            "reference_media": [{"kind": "image", "role": "reference", "url": "https://assets.example.invalid/same.png"}],
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertNotEqual(media._intent_fingerprint(legacy), media._intent_fingerprint(ref_only))

        references = [{"kind": "image", "role": "reference", "url": f"https://assets.example.invalid/{index}.png"} for index in range(14)]
        first = {"kind": "image", "role": "first_frame", "url": "https://assets.example.invalid/start.png"}
        accepted = media._validate_request({
            "provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "Fourteen refs plus opening frame",
            "reference_media": [first, *references],
            "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
        })
        self.assertEqual(len(accepted["reference_media"]), 15)
        self.assertEqual(len(media._request_payload(accepted)[1]["reference_images"]), 14)
        for model, max_refs in (("grok-imagine-video", 7), ("grok-imagine-video-1.5", 14)):
            os.environ["MEDIA_GROK_VIDEO_MODEL"] = model
            self.refresh_catalog([model])
            with self.assertRaises(media.MediaError) as too_many:
                media._validate_request({
                    "provider": "grok-video", "model": model, "prompt": f"Too many refs {model}",
                    "reference_media": [{"kind": "image", "role": "reference", "url": f"https://assets.example.invalid/too-many-{index}.png"} for index in range(max_refs + 1)],
                    "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
                })
            self.assertEqual(getattr(too_many.exception, "code", None), "TOO_MANY_REFERENCES")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_grok_video_first_frame_and_reference_each_submit_poll_and_save(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        os.environ["MEDIA_GENERATION_API_BASE_URL"] = "https://gateway.example.test"
        raw_image = valid_png()
        raw_video = b"\x00\x00\x00\x18ftypmp42fixture"
        expected_content_type = "video/mp4"

        for role in ("first_frame", "reference"):
            with self.subTest(role=role):
                self.refresh_catalog(["grok-imagine-video-1.5"])
                image_path = self.root / f"{role}-input-with-jpg-suffix.jpg"
                image_path.write_bytes(raw_image)
                prepared = media.media_prepare({
                    "provider": "grok-video", "model": "grok-imagine-video-1.5",
                    "prompt": f"End-to-end mocked {role} video request",
                    "reference_media": [{"kind": "image", "role": role, "local_path": str(image_path)}],
                    "parameters": {"duration": 5, "aspect_ratio": "1:1", "resolution": "480p"},
                })
                self.assertEqual(prepared["operation"], "image-to-video" if role == "first_frame" else "reference-to-video")

                task_id = f"task-{role}"
                self.http.state.enqueue("/v1/videos/generations", 200, {"request_id": task_id, "status": "queued"})
                submitted = media.media_submit(self.submit_args(prepared))
                self.assertEqual(submitted["task_id"], task_id)
                self.assertEqual(submitted["state"], "in_progress")

                post = next(item for item in reversed(self.http.state.requests) if item["method"] == "POST" and item["path"] == "/v1/videos/generations")
                body = json.loads(post["body"].decode("utf-8"))
                self.assertEqual(post["headers"].get("Authorization"), f"Bearer {CANARY_KEY}")
                self.assertEqual(body["model"], "grok-imagine-video-1.5")
                self.assertEqual(body["duration"], 5)
                self.assertEqual(body["aspect_ratio"], "1:1")
                self.assertEqual(body["resolution"], "480p")
                data_url = body["image"]["url"] if role == "first_frame" else body["reference_images"][0]["url"]
                self.assertTrue(data_url.startswith("data:image/png;base64,"))
                self.assertEqual(base64.b64decode(data_url.split(",", 1)[1]), raw_image)
                if role == "first_frame":
                    self.assertNotIn("reference_images", body)
                else:
                    self.assertNotIn("image", body)
                    self.assertEqual(len(body["reference_images"]), 1)

                content_path = f"/v1/videos/{task_id}/content"
                self.http.state.enqueue(f"/v1/videos/{task_id}", 200, {"status": "completed", "content_url": content_path})
                checked = media.media_check(self.submit_args(prepared))
                self.assertEqual(checked["state"], "completed_pending_save")

                self.http.state.enqueue(content_path, 200, raw_video, {"Content-Type": expected_content_type})
                # Exercise the actual HTTP downloader while replacing only DNS
                # resolution; the fake opener still observes URL, headers, MIME,
                # magic validation, and the saved bytes without socket access.
                original_validate = media._validate_public_url
                media._validate_public_url = lambda value, **_: value  # type: ignore[method-assign]
                try:
                    saved = media.media_save(self.submit_args(prepared))
                finally:
                    media._validate_public_url = original_validate  # type: ignore[method-assign]
                self.assertTrue(saved["success"])
                download = self.http.state.requests[-1]
                self.assertEqual(download["method"], "GET")
                self.assertEqual(download["path"], content_path)
                self.assertEqual(download["headers"].get("Authorization"), f"Bearer {CANARY_KEY}")
                artifact_path = Path(saved["saved"][0]["path"])
                self.assertTrue(artifact_path.is_file())
                self.assertEqual(artifact_path.read_bytes(), raw_video)
                record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
                self.assertEqual(record["state"], "completed_saved")
                self.assertEqual(record["saved_artifacts"][0]["sha256"], hashlib.sha256(raw_video).hexdigest())

        self.assertEqual(self.http.state.count("POST", "/v1/videos/generations"), 2)

    def test_grok_video_parameters_are_required_and_invalid_values_fail_before_submit(self) -> None:
        self.refresh_catalog(["grok-imagine-video-1.5"])
        with self.assertRaises(media.MediaError) as missing:
            media.media_prepare({
                "provider": "grok-video",
                "model": "grok-imagine-video-1.5",
                "prompt": "missing video parameters",
                "parameters": {"duration": 4},
            })
        self.assertEqual(getattr(missing.exception, "code", None), "VIDEO_PARAMETERS_REQUIRED")
        self.assertEqual(missing.exception.details["missing_parameters"], ["aspect_ratio", "resolution"])

        prepared = media.media_prepare({
            "provider": "grok-video",
            "model": "grok-imagine-video-1.5",
            "prompt": "explicit video parameters",
            "parameters": {"duration": 4, "aspect_ratio": "9:16", "resolution": "720p"},
        })
        self.assertEqual(prepared["parameters"]["aspect_ratio"], "9:16")
        self.assertEqual(prepared["parameters"]["resolution"], "720p")

        with self.assertRaises(media.MediaError) as blank:
            media.media_prepare({
                "provider": "grok-video",
                "model": "grok-imagine-video-1.5",
                "prompt": "invalid video parameters",
                "parameters": {"duration": 4, "aspect_ratio": "", "resolution": "480p"},
            })
        self.assertEqual(getattr(blank.exception, "code", None), "VIDEO_PARAMETERS_REQUIRED")
        self.assertEqual(blank.exception.details["missing_parameters"], ["aspect_ratio"])
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_legacy_grok_video_confirmation_without_dimensions_is_rejected_before_post(self) -> None:
        for profile in ("responses", "wokey"):
            with self.subTest(profile=profile):
                os.environ["MEDIA_GENERATION_API_PROFILE"] = profile
                self.refresh_catalog(["grok-imagine-video-1.5"])
                legacy_request = media._validate_request({
                    "provider": "grok-video",
                    "model": "grok-imagine-video-1.5",
                    "prompt": f"legacy confirmation missing dimensions {profile}",
                    "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"},
                })
                legacy_request["parameters"].pop("aspect_ratio")
                legacy_request["parameters"].pop("resolution")
                with unittest.mock.patch.object(media, "_validate_request", return_value=copy.deepcopy(legacy_request)):
                    prepared = media.media_prepare({"provider": "grok-video", "model": legacy_request["model"], "prompt": legacy_request["prompt"]})

                self.assertEqual(prepared["parameters"], {"duration": 4})
                with self.assertRaises(media.MediaError) as rejected:
                    media.media_submit(self.submit_args(prepared))
                self.assertEqual(getattr(rejected.exception, "code", None), "CONFIRMATION_STALE")
                self.assertFalse(getattr(rejected.exception, "api_request_sent", True))
                self.assertEqual(rejected.exception.details["missing_parameters"], ["aspect_ratio", "resolution"])
                self.assertEqual(self.http.state.count("POST"), 0)

    def test_refresh_failure_does_not_replace_last_verified_catalog(self) -> None:
        self.refresh_catalog(["gpt-image-2.5"])
        before = self.config.read_bytes()
        self.http.state.enqueue("/v1/models", 503, {"error": {"message": f"failed {CANARY_KEY}"}})
        with self.assertRaises(media.MediaError):
            media.media_refresh_models({})
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(self.http.state.count("GET", "/v1/models"), 2)

    def test_unknown_provider_parameters_rejected_before_network(self) -> None:
        self.refresh_catalog()
        with self.assertRaises(media.MediaError) as raised:
            media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "unknown parameter", "parameters": {"size": "auto", "imaginary_option": True}})
        self.assertEqual(getattr(raised.exception, "code", None), "UNSUPPORTED_PARAMETER")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_gateway_change_after_prepare_is_rejected_without_post(self) -> None:
        prepared = self.prepared_image("route drift")
        os.environ["MEDIA_GENERATION_API_BASE_URL"] = "http://127.0.0.1:9"
        with self.assertRaises(media.MediaError):
            media.media_submit(self.submit_args(prepared))
        self.assertEqual(self.http.state.count("POST"), 0)
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertEqual(record["state"], "prepared")

    def test_http_error_status_matrix_and_safe_request_ids(self) -> None:
        self.refresh_catalog()
        cases = [
            (400, "UPSTREAM_INPUT_REJECTED", "submission_rejected", False),
            (401, "UPSTREAM_AUTH_FAILED", "submission_rejected", False),
            (403, "UPSTREAM_AUTH_FAILED", "submission_rejected", False),
            (404, "UPSTREAM_MODEL_NOT_FOUND", "submission_rejected", False),
            (422, "UPSTREAM_INPUT_REJECTED", "submission_rejected", False),
            (429, "SUBMISSION_UNKNOWN", "submission_unknown", False),
            (500, "SUBMISSION_UNKNOWN", "submission_unknown", False),
            (502, "SUBMISSION_UNKNOWN", "submission_unknown", False),
            (503, "SUBMISSION_UNKNOWN", "submission_unknown", False),
        ]
        for status, expected_code, expected_state, retryable in cases:
            with self.subTest(status=status):
                prompt = f"HTTP status fixture {status}"
                prepared = media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": prompt, "parameters": {"size": "auto"}})
                request_id = f"fixture-request-{status}"
                self.http.state.enqueue("/v1/responses", status, {
                    "error": {"code": "invalid_request" if status < 500 else "gateway_error",
                              "message": f"safe fixture only: {CANARY_KEY} {prompt} {CANARY_QUERY}"},
                    "request_id": request_id,
                }, {"X-Request-ID": request_id, "X-Gateway-Request-ID": f"gateway-{status}", "Content-Type": "application/json"})
                with self.assertRaises(media.MediaError) as raised:
                    media.media_submit(self.submit_args(prepared))
                error = raised.exception
                self.assertEqual(getattr(error, "http_status", None), status)
                self.assertEqual(getattr(error, "code", None), expected_code)
                self.assertEqual(getattr(error, "submission_state", None), expected_state)
                self.assertEqual(getattr(error, "retryable", None), retryable)
                self.assertEqual(getattr(error, "provider_request_id", None), request_id)
                self.assertEqual(getattr(error, "gateway_request_id", None), f"gateway-{status}")
                self.assertNotIn(CANARY_KEY, str(error))
                self.assertNotIn(prompt, str(error))
                self.assertNotIn(CANARY_QUERY, str(error))
                record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
                self.assertEqual(record["state"], expected_state)
                persisted = json.dumps(record, ensure_ascii=False)
                self.assertNotIn(CANARY_KEY, persisted)
                self.assertNotIn(CANARY_QUERY, persisted)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), len(cases))

    def test_http_502_unknown_state_preserves_sanitized_gateway_diagnostic(self) -> None:
        prepared = self.prepared_image("image URL MIME mismatch")
        diagnostic = "reference image data URL MIME type does not match its image data"
        self.http.state.enqueue("/v1/responses", 502, {"error": {"code": "gateway_error", "message": diagnostic}},
                                {"X-Request-ID": "provider-502", "X-Gateway-Request-ID": "gateway-502"})
        with self.assertRaises(media.MediaError) as raised:
            media.media_submit(self.submit_args(prepared))
        self.assertEqual(raised.exception.code, "SUBMISSION_UNKNOWN")
        self.assertEqual(raised.exception.submission_state, "submission_unknown")
        self.assertTrue(raised.exception.api_request_sent)
        self.assertFalse(raised.exception.retryable)
        self.assertIn(diagnostic, raised.exception.safe_message)
        envelope = media._media_error_envelope(raised.exception)
        self.assertEqual(envelope["state"], "submission_unknown")
        self.assertIn(diagnostic, envelope["error"]["safe_message"])
        self.assertTrue(envelope["request_attempt_started"])
        self.assertTrue(envelope["mcp_http_response_received"])
        self.assertEqual(envelope["upstream_acceptance"], "unknown")
        self.assertTrue(envelope["manual_reconciliation_required"])
        self.assertEqual(envelope["error"]["provider_request_id"], "provider-502")
        self.assertEqual(envelope["error"]["gateway_request_id"], "gateway-502")
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertEqual(record["state"], "submission_unknown")
        self.assertIn(diagnostic, record["last_error"])
        self.assertEqual(record["http_status"], 502)
        self.assertEqual(record["provider_request_id"], "provider-502")
        self.assertEqual(record["gateway_request_id"], "gateway-502")
        self.assertTrue(record["submission_diagnostics"]["mcp_http_response_received"])
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

        get_count_before_check = self.http.state.count("GET")
        with self.assertRaises(media.MediaError) as check_error:
            media.media_check({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
        self.assertEqual(check_error.exception.code, "SUBMISSION_STATUS_UNAVAILABLE")
        self.assertEqual(check_error.exception.http_status, 502)
        self.assertEqual(check_error.exception.provider_request_id, "provider-502")
        self.assertEqual(check_error.exception.gateway_request_id, "gateway-502")
        check_envelope = media._media_error_envelope(check_error.exception)
        self.assertFalse(check_envelope["api_request_sent"])
        self.assertFalse(check_envelope["check_request_sent"])
        self.assertTrue(check_envelope["original_request_attempt_started"])
        self.assertTrue(check_envelope["mcp_http_response_received"])
        self.assertEqual(check_envelope["upstream_acceptance"], "unknown")
        self.assertTrue(check_envelope["manual_reconciliation_required"])
        self.assertEqual(self.http.state.count("GET"), get_count_before_check)

    def test_upstream_diagnostic_redacts_short_multipart_text_and_actual_key(self) -> None:
        short_prompt = "hi"
        custom_key = "custom-secret-canary-12345"
        multipart_payload = b"multipart body without parsed prompt field"
        detail = media._safe_upstream_error_detail(
            {"error": {"message": f"invalid {short_prompt} {custom_key} reason\u0085continued"}},
            multipart_payload,
            api_key=custom_key,
            sensitive_texts=(short_prompt,),
        )
        self.assertIsNotNone(detail)
        self.assertNotIn(short_prompt, detail)
        self.assertNotIn(custom_key, detail)
        self.assertIn("invalid", detail)
        self.assertIn("reason continued", detail)

        json_detail = media._safe_upstream_error_detail(
            {"error": {"message": "echoed cat"}},
            {"prompt": "cat"},
        )
        self.assertEqual(json_detail, "echoed [request text omitted]")

        self.http.state.enqueue("/v1/responses", 502, {
            "error": {"message": f"gateway rejected {short_prompt} {custom_key} MIME mismatch"}
        })
        with unittest.mock.patch.object(media, "_api_key", return_value=custom_key):
            with self.assertRaises(media.MediaError) as raised:
                media._http_json("POST", "/v1/responses", {"prompt": short_prompt}, "image-2")
        self.assertEqual(raised.exception.code, "SUBMISSION_UNKNOWN")
        self.assertIn("MIME mismatch", raised.exception.safe_message)
        self.assertNotIn(short_prompt, raised.exception.safe_message)
        self.assertNotIn(custom_key, raised.exception.safe_message)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_invalid_utf8_and_oversized_error_body_are_bounded(self) -> None:
        self.refresh_catalog()
        for label, body in (
            ("invalid-utf8", b"\xff\xfe" + CANARY_KEY.encode("ascii")),
            ("oversized", b"x" * 8192 + CANARY_QUERY.encode("utf-8")),
        ):
            with self.subTest(label=label):
                prepared = media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": label, "parameters": {"size": "auto"}})
                self.http.state.enqueue("/v1/responses", 400, body, {"X-Request-ID": f"req-{label}"})
                with self.assertRaises(media.MediaError) as raised:
                    media.media_submit(self.submit_args(prepared))
                self.assertEqual(getattr(raised.exception, "http_status", None), 400)
                self.assertEqual(getattr(raised.exception, "provider_request_id", None), f"req-{label}")
                self.assertNotIn(CANARY_KEY, str(raised.exception))
                self.assertNotIn(CANARY_QUERY, str(raised.exception))
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 2)

    def test_explicit_content_policy_error_has_distinct_code(self) -> None:
        prepared = self.prepared_image(CANARY_PROMPT)
        self.http.state.enqueue("/v1/responses", 400, {
            "error": {"code": "content_policy_violation", "message": f"{CANARY_KEY} {CANARY_PROMPT}"},
            "request_id": "policy-request-fixture",
        }, {"X-Request-ID": "policy-request-fixture"})
        with self.assertRaises(media.MediaError) as raised:
            media.media_submit(self.submit_args(prepared))
        self.assertEqual(getattr(raised.exception, "code", None), "UPSTREAM_CONTENT_POLICY")
        self.assertEqual(getattr(raised.exception, "submission_state", None), "submission_rejected")
        self.assertFalse(getattr(raised.exception, "retryable", True))
        self.assertNotIn(CANARY_KEY, str(raised.exception))
        self.assertNotIn(CANARY_PROMPT, str(raised.exception))

    def test_unknown_submit_is_structured_redacted_and_never_reposted(self) -> None:
        prepared = self.prepared_image()
        self.http.state.enqueue("/v1/responses", 500, {
            "error": {"code": "upstream_failure", "message": f"{CANARY_KEY} {CANARY_PROMPT} {CANARY_QUERY}"},
            "request_id": "provider-req-fixture-500",
        })
        args = self.submit_args(prepared)
        with self.assertRaises(media.MediaError) as first:
            media.media_submit(args)
        self.assertEqual(getattr(first.exception, "code", None), "SUBMISSION_UNKNOWN")
        self.assertFalse(getattr(first.exception, "retryable", True))
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertEqual(record["state"], "submission_unknown")
        self.assertTrue(record["api_request_sent"])

        # Same confirmation must never be POSTed again, including after a new
        # Python process has loaded the durable record.
        with self.assertRaises(media.MediaError):
            media.media_submit(args)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)
        restarted = load_fresh_service("media_generation_mcp_test_restart")
        with self.assertRaises(restarted.MediaError):
            restarted.media_submit(args)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

        # The correlated error contract may preserve request IDs but must not
        # expose secret, prompt, or signed URL query material.
        exposed = json.dumps({"error": str(first.exception), "code": getattr(first.exception, "code", None),
                             "request_id": getattr(first.exception, "provider_request_id", None)}, ensure_ascii=False)
        for secret in (CANARY_KEY, CANARY_PROMPT, CANARY_QUERY):
            self.assertNotIn(secret, exposed)

    def test_connection_reset_after_request_start_is_unknown_and_not_retried(self) -> None:
        prepared = self.prepared_image("network reset fixture")
        self.http.state.enqueue_exception("/v1/responses", ConnectionResetError("fixture reset"))
        args = self.submit_args(prepared)
        with self.assertRaises(media.MediaError) as raised:
            media.media_submit(args)
        self.assertEqual(getattr(raised.exception, "code", None), "SUBMISSION_UNKNOWN")
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertEqual(record["state"], "submission_unknown")
        self.assertTrue(record["api_request_sent"])
        with self.assertRaises(media.MediaError):
            media.media_submit(args)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_missing_intent_key_does_not_silently_clear_unknown_reservation(self) -> None:
        prepared = self.prepared_image("key loss must fail closed")
        self.http.state.enqueue("/v1/responses", 500, {"error": {"code": "unknown"}})
        with self.assertRaises(media.MediaError):
            media.media_submit(self.submit_args(prepared))
        key_path = self.data / ".confirmation-key"
        self.assertTrue(key_path.exists())
        key_path.unlink()
        with self.assertRaises(media.MediaError) as raised:
            media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "key loss must fail closed", "parameters": {"size": "auto"}, "repeat_after_unknown_ack": "确认此前任务可能已计费且仍要求新生成"})
        self.assertEqual(getattr(raised.exception, "code", None), "UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
        self.assertFalse(key_path.exists(), "missing key must not be silently regenerated while unresolved records exist")
        self.assertEqual(len(list((self.data / "confirmations").glob("media-*.json"))), 1)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_request_started_marker_write_failure_prevents_http_post(self) -> None:
        prepared = self.prepared_image("durable marker failure")
        original_atomic_json = media._atomic_json

        def fail_request_started(path: Path, value: dict[str, Any]) -> None:
            if value.get("state") == "submitting" and value.get("api_request_sent") is True:
                raise OSError("fixture disk write failure")
            original_atomic_json(path, value)

        media._atomic_json = fail_request_started  # type: ignore[assignment]
        try:
            with self.assertRaises(media.MediaError):
                media.media_submit(self.submit_args(prepared))
        finally:
            media._atomic_json = original_atomic_json  # type: ignore[assignment]
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 0)
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertNotEqual(record["state"], "prepared")

    def test_concurrent_submit_has_one_cas_winner_and_one_post(self) -> None:
        prepared = self.prepared_image("concurrent submit fixture")
        image = valid_png()
        self.http.state.enqueue("/v1/responses", 200, {"output": [{"type": "image_generation_call", "result": base64.b64encode(image).decode("ascii")}]})
        entered, release = self.http.state.block_next("/v1/responses")
        args = self.submit_args(prepared)
        first_outcome: list[Any] = []

        def first_submit() -> None:
            try:
                first_outcome.append(media.media_submit(args))
            except Exception as exc:  # surfaced in the assertions below
                first_outcome.append(exc)

        thread = threading.Thread(target=first_submit)
        thread.start()
        try:
            self.assertTrue(entered.wait(timeout=3), "first submit never reached fake HTTP")
            with self.assertRaises(media.MediaError) as second:
                media.media_submit(args)
            self.assertEqual(getattr(second.exception, "code", None), "SUBMISSION_ALREADY_STARTED")
            self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)
        finally:
            release.set()
            thread.join(timeout=8)
        self.assertFalse(thread.is_alive(), "submit worker did not finish")
        self.assertEqual(len(first_outcome), 1)
        self.assertNotIsInstance(first_outcome[0], Exception, repr(first_outcome[0]))
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_check_marker_blocks_concurrent_check(self) -> None:
        self.refresh_catalog(["grok-imagine-video-1.5"])
        prepared = media.media_prepare({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "check marker fixture", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}})
        self.http.state.enqueue("/v1/videos/generations", 200, {"id": "task-check-marker", "status": "queued"})
        media.media_submit(self.submit_args(prepared))
        self.http.state.enqueue("/v1/videos/task-check-marker", 200, {"status": "processing"})
        entered, release = self.http.state.block_next("/v1/videos/task-check-marker")
        args = {"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]}
        first_outcome: list[Any] = []

        def first_check() -> None:
            try:
                first_outcome.append(media.media_check(args))
            except Exception as exc:
                first_outcome.append(exc)

        thread = threading.Thread(target=first_check)
        thread.start()
        try:
            self.assertTrue(entered.wait(timeout=3), "first check never reached fake HTTP")
            with self.assertRaises(media.MediaError) as second:
                media.media_check(args)
            self.assertEqual(getattr(second.exception, "code", None), "OPERATION_IN_PROGRESS")
            self.assertEqual(self.http.state.count("GET", "/v1/videos/task-check-marker"), 1)
        finally:
            release.set()
            thread.join(timeout=8)
        self.assertFalse(thread.is_alive(), "check worker did not finish")
        self.assertEqual(len(first_outcome), 1)
        self.assertNotIsInstance(first_outcome[0], Exception, repr(first_outcome[0]))
        self.assertEqual(self.http.state.count("GET", "/v1/videos/task-check-marker"), 1)

    def test_same_intent_unknown_blocks_new_prepare_without_ack(self) -> None:
        prepared = self.prepared_image("same intended image")
        self.http.state.enqueue("/v1/responses", 500, {"error": {"code": "busy"}})
        with self.assertRaises(media.MediaError):
            media.media_submit(self.submit_args(prepared))
        with self.assertRaises(media.MediaError) as duplicate:
            media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "same intended image", "parameters": {"size": "auto"}})
        self.assertEqual(getattr(duplicate.exception, "code", None), "UNRESOLVED_PRIOR_SUBMISSION")
        records = list((self.data / "confirmations").glob("media-*.json"))
        self.assertEqual(len(records), 1)

        ack_text = "确认此前任务可能已计费且仍要求新生成"
        repeated = media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "same intended image", "parameters": {"size": "auto"}, "repeat_after_unknown_ack": ack_text})
        self.assertNotEqual(repeated["confirmation_id"], prepared["confirmation_id"])
        repeated_record = json.loads(self.record_path(repeated["confirmation_id"]).read_text(encoding="utf-8"))
        prior_record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertEqual(repeated_record.get("supersedes_unknown_submission_ids"), [prior_record.get("submission_id")])
        self.assertTrue(repeated_record.get("repeat_after_unknown_acknowledged"))
        self.assertTrue(prior_record.get("repeat_ack_consumed"))
        with self.assertRaises(media.MediaError) as second_repeat:
            media.media_prepare({"provider": "image-2", "model": "gpt-image-2.5", "prompt": "same intended image", "parameters": {"size": "auto"}, "repeat_after_unknown_ack": ack_text})
        self.assertEqual(getattr(second_repeat.exception, "code", None), "INTENT_ALREADY_PREPARED")
        self.assertEqual(len(list((self.data / "confirmations").glob("media-*.json"))), 2)
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_legacy_roleless_grok_video_reservation_is_verified_as_first_frame(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        image = self.root / "legacy-first-frame.png"
        image.write_bytes(valid_png())
        args = {
            "provider": "grok-video", "model": "grok-imagine-video-1.5",
            "prompt": "Legacy first-frame intent compatibility fixture",
            "reference_media": [{"kind": "image", "role": "first_frame", "local_path": str(image)}],
            "parameters": {"duration": 5, "aspect_ratio": "1:1", "resolution": "480p"},
        }
        prepared = media.media_prepare(args)
        path = self.record_path(prepared["confirmation_id"])
        record = json.loads(path.read_text(encoding="utf-8"))
        reference = record["request"]["reference_media"][0]
        self.assertEqual(reference["role"], "first_frame")

        # Simulate an authenticated confirmation written before explicit video
        # roles existed: its immutable request has no role key, and its legacy
        # intent HMAC omitted that key. The record HMAC itself remains valid.
        del reference["role"]
        record["intent_fingerprint"] = media._intent_fingerprint(record["request"], legacy_roleless=True)
        record["request_digest_hmac"] = media._token(record["request"])
        payload = {key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")}
        record["fingerprint"] = media._token(payload)
        media._atomic_json(path, record)

        with self.assertRaises(media.MediaError) as duplicate:
            media.media_prepare(args)
        self.assertEqual(getattr(duplicate.exception, "code", None), "INTENT_ALREADY_PREPARED")
        self.assertEqual(duplicate.exception.details.get("confirmation_id"), prepared["confirmation_id"])

        self.http.state.enqueue("/v1/videos/generations", 200, {"request_id": "legacy-first-frame-task", "status": "queued"})
        submit_args = self.submit_args(prepared)
        submit_args["confirmation_fingerprint"] = record["fingerprint"]
        submitted = media.media_submit(submit_args)
        self.assertEqual(submitted["task_id"], "legacy-first-frame-task")
        posted = next(item for item in self.http.state.requests if item["method"] == "POST" and item["path"] == "/v1/videos/generations")
        body = json.loads(posted["body"].decode("utf-8"))
        self.assertIn("image", body)
        self.assertNotIn("reference_images", body)
        self.assertEqual(self.http.state.count("POST", "/v1/videos/generations"), 1)

    def test_intermediate_all_roles_hmac_is_accepted_for_roleless_grok_intent(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        image = self.root / "intermediate-first-frame.png"
        image.write_bytes(valid_png())
        args = {
            "provider": "grok-video", "model": "grok-imagine-video-1.5",
            "prompt": "Intermediate role fingerprint compatibility fixture",
            "reference_media": [{"kind": "image", "role": "first_frame", "local_path": str(image)}],
            "parameters": {"duration": 5, "aspect_ratio": "1:1", "resolution": "480p"},
        }
        prepared = media.media_prepare(args)
        path = self.record_path(prepared["confirmation_id"])
        record = json.loads(path.read_text(encoding="utf-8"))
        del record["request"]["reference_media"][0]["role"]
        record["intent_fingerprint"] = media._intent_fingerprint(record["request"], legacy_all_roles=True)
        record["request_digest_hmac"] = media._token(record["request"])
        payload = {key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")}
        record["fingerprint"] = media._token(payload)
        media._atomic_json(path, record)

        with self.assertRaises(media.MediaError) as duplicate:
            media.media_prepare(args)
        self.assertEqual(getattr(duplicate.exception, "code", None), "INTENT_ALREADY_PREPARED")
        self.assertEqual(duplicate.exception.details.get("confirmation_id"), prepared["confirmation_id"])
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_intermediate_all_roles_hmac_is_accepted_for_roleless_image_reference(self) -> None:
        self.refresh_catalog(["gpt-image-2.5"])
        image = self.root / "intermediate-image-reference.png"
        image.write_bytes(valid_png())
        args = {
            "provider": "image-2", "model": "gpt-image-2.5",
            "prompt": "Intermediate image reference fingerprint compatibility fixture",
            "reference_media": [{"kind": "image", "local_path": str(image)}],
            "parameters": {"size": "auto"},
        }
        prepared = media.media_prepare(args)
        path = self.record_path(prepared["confirmation_id"])
        record = json.loads(path.read_text(encoding="utf-8"))
        record["intent_fingerprint"] = media._intent_fingerprint(record["request"], legacy_all_roles=True)
        record["request_digest_hmac"] = media._token(record["request"])
        payload = {key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")}
        record["fingerprint"] = media._token(payload)
        media._atomic_json(path, record)

        with self.assertRaises(media.MediaError) as duplicate:
            media.media_prepare(args)
        self.assertEqual(getattr(duplicate.exception, "code", None), "INTENT_ALREADY_PREPARED")
        self.assertEqual(duplicate.exception.details.get("confirmation_id"), prepared["confirmation_id"])
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_invalid_roleless_intent_hmac_still_blocks_new_preparation(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        image = self.root / "invalid-legacy-intent.png"
        image.write_bytes(valid_png())
        prepared = media.media_prepare({
            "provider": "grok-video", "model": "grok-imagine-video-1.5",
            "prompt": "Invalid legacy intent fixture",
            "reference_media": [{"kind": "image", "role": "first_frame", "local_path": str(image)}],
            "parameters": {"duration": 5, "aspect_ratio": "1:1", "resolution": "480p"},
        })
        path = self.record_path(prepared["confirmation_id"])
        record = json.loads(path.read_text(encoding="utf-8"))
        del record["request"]["reference_media"][0]["role"]
        record["intent_fingerprint"] = "0" * 64
        record["request_digest_hmac"] = media._token(record["request"])
        payload = {key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")}
        record["fingerprint"] = media._token(payload)
        media._atomic_json(path, record)

        with self.assertRaises(media.MediaError) as rejected:
            media.media_prepare({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "unrelated request", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}})
        self.assertEqual(getattr(rejected.exception, "code", None), "UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_historical_duplicate_prepared_confirmations_allow_at_most_one_post(self) -> None:
        prepared = self.prepared_image("legacy duplicate reservation")
        original = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        duplicate_id = "media-" + "d" * 32
        duplicate = dict(original)
        duplicate["confirmation_id"] = duplicate_id
        duplicate["submission_id"] = "e" * 32
        payload = {key: duplicate.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")}
        duplicate["fingerprint"] = media._token(payload)
        self.record_path(duplicate_id).write_text(json.dumps(duplicate), encoding="utf-8")
        self.http.state.enqueue("/v1/responses", 200, {"output": [{"type": "image_generation_call", "result": base64.b64encode(valid_png()).decode("ascii")}]})

        media.media_submit(self.submit_args(prepared))
        duplicate_record = json.loads(self.record_path(duplicate_id).read_text(encoding="utf-8"))
        self.assertEqual(duplicate_record["state"], "superseded")
        with self.assertRaises(media.MediaError):
            media.media_submit({
                "confirmation_id": duplicate_id,
                "confirmation_fingerprint": duplicate["fingerprint"],
                "user_confirmation": "确认",
            })
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_provider_artifact_signed_url_is_encrypted_at_rest_and_redacted(self) -> None:
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "wokey"
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog()
        prepared = media.media_prepare({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "signed artifact url", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}})
        signed_url = f"https://cdn.example.test/result.mp4?token={CANARY_QUERY}"
        self.http.state.enqueue("/v1/videos", 200, {"status": "completed", "video_url": signed_url})
        submitted = media.media_submit(self.submit_args(prepared))
        self.assertEqual(submitted["state"], "completed_pending_save")
        serialized_response = json.dumps(submitted, ensure_ascii=False)
        self.assertNotIn(CANARY_QUERY, serialized_response)
        self.assertEqual(submitted["artifacts"][0]["url_origin"], "https://cdn.example.test")

        record_path = self.record_path(prepared["confirmation_id"])
        raw_record = record_path.read_text(encoding="utf-8")
        self.assertNotIn(CANARY_QUERY, raw_record)
        record = json.loads(raw_record)
        artifact = record["artifacts"][0]
        self.assertNotIn("url", artifact)
        self.assertTrue(artifact["url_protected"].startswith("dpapi:"))
        self.assertEqual(media._restore_artifact_urls([artifact])[0]["url"], signed_url)

        checked = media.media_check(self.submit_args(prepared))
        self.assertNotIn(CANARY_QUERY, json.dumps(checked, ensure_ascii=False))
        self.assertEqual(checked["artifacts"][0]["url_origin"], "https://cdn.example.test")

        downloaded: list[str] = []
        def fake_download(url: str, destination: Path) -> tuple[int, str]:
            downloaded.append(url)
            raw = b"\x00\x00\x00\x18ftypmp42"
            destination.write_bytes(raw)
            return len(raw), "video/mp4"
        with unittest.mock.patch.object(media, "_safe_download", side_effect=fake_download):
            saved = media.media_save(self.submit_args(prepared))
        self.assertTrue(saved["success"])
        self.assertEqual(downloaded, [signed_url])
        self.assertNotIn(CANARY_QUERY, record_path.read_text(encoding="utf-8"))

    def test_prepared_same_intent_is_reserved_across_threads(self) -> None:
        self.refresh_catalog()
        request = {"provider": "image-2", "model": "gpt-image-2.5", "prompt": "thread reservation", "parameters": {"size": "auto"}}
        barrier = threading.Barrier(2)
        outcomes: list[tuple[str, Any]] = []
        lock = threading.Lock()

        def worker() -> None:
            barrier.wait(timeout=3)
            try:
                result = media.media_prepare(request)
                value: tuple[str, Any] = ("ok", result)
            except media.MediaError as exc:
                value = ("error", exc)
            with lock:
                outcomes.append(value)

        threads = [threading.Thread(target=worker) for _ in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=5)
        self.assertTrue(all(not thread.is_alive() for thread in threads), "prepare reservation deadlocked")
        records = list((self.data / "confirmations").glob("media-*.json"))
        self.assertEqual(len(records), 1)
        self.assertEqual(sum(outcome[0] == "ok" for outcome in outcomes), 1)
        self.assertEqual(self.http.state.count("POST"), 0)

    @unittest.skipUnless(os.name == "nt", "Named mutex contract is Windows-specific")
    def test_same_intent_prepare_is_reserved_across_processes(self) -> None:
        self.refresh_catalog()
        request = {"provider": "image-2", "model": "gpt-image-2.5", "prompt": "process reservation", "parameters": {"size": "auto"}}
        start_file = self.root / "release-prepare-processes"
        child_code = (
            "import importlib.util,json,pathlib,time\n"
            f"p={str(SERVICE_PATH)!r}\n"
            "s=importlib.util.spec_from_file_location('child_media',p)\n"
            "m=importlib.util.module_from_spec(s)\n"
            "s.loader.exec_module(m)\n"
            f"start=pathlib.Path({str(start_file)!r})\n"
            "deadline=time.monotonic()+10\n"
            "while not start.exists() and time.monotonic()<deadline: time.sleep(0.005)\n"
            f"r=json.loads(json.loads({json.dumps(json.dumps(request))!r}))\n"
            "try:\n"
            " x=m.media_prepare(r)\n"
            " print(json.dumps({'created':True,'id':x.get('confirmation_id')}))\n"
            "except m.MediaError as e:\n"
            " print(json.dumps({'created':False,'code':e.code}))\n"
        )
        env = os.environ.copy()
        processes = [subprocess.Popen([sys.executable, "-c", child_code], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env) for _ in range(2)]
        start_file.write_text("go", encoding="ascii")
        outcomes = []
        for process in processes:
            stdout, stderr = process.communicate(timeout=15)
            self.assertEqual(process.returncode, 0, stderr)
            outcomes.append(json.loads(stdout.strip().splitlines()[-1]))
        self.assertEqual(sum(1 for item in outcomes if item["created"]), 1)
        self.assertEqual([item["code"] for item in outcomes if not item["created"]], ["INTENT_ALREADY_PREPARED"])
        self.assertEqual(len(list((self.data / "confirmations").glob("media-*.json"))), 1)
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_unknown_repeat_ack_is_not_consumed_if_new_confirmation_write_fails(self) -> None:
        prepared = self.prepared_image("repeat acknowledgement write failure")
        self.http.state.enqueue("/v1/responses", 500, {"error": {"code": "busy"}})
        with self.assertRaises(media.MediaError):
            media.media_submit(self.submit_args(prepared))
        prior_path = self.record_path(prepared["confirmation_id"])
        prior_before = json.loads(prior_path.read_text(encoding="utf-8"))
        real_atomic_json = media._atomic_json

        def fail_new_confirmation(path: Path, value: dict[str, Any]) -> None:
            if value.get("state") == "prepared" and value.get("repeat_after_unknown_acknowledged"):
                raise OSError("simulated confirmation disk failure")
            real_atomic_json(path, value)

        with unittest.mock.patch.object(media, "_atomic_json", side_effect=fail_new_confirmation):
            with self.assertRaisesRegex(OSError, "simulated confirmation disk failure"):
                media.media_prepare({
                    "provider": "image-2", "model": "gpt-image-2.5", "prompt": "repeat acknowledgement write failure",
                    "parameters": {"size": "auto"},
                    "repeat_after_unknown_ack": "确认此前任务可能已计费且仍要求新生成",
                })
        prior_after = json.loads(prior_path.read_text(encoding="utf-8"))
        self.assertEqual(prior_after.get("repeat_ack_consumed"), prior_before.get("repeat_ack_consumed"))
        self.assertEqual(len(list((self.data / "confirmations").glob("media-*.json"))), 1)

    def test_reference_query_is_part_of_intent_hmac_without_extra_persistence(self) -> None:
        self.refresh_catalog(["grok-imagine-video-1.5"])
        common = {"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "same prompt", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}}
        first = media.media_prepare({**common, "reference_media": [{"kind": "image", "url": f"https://assets.example.invalid/frame.png?sig={CANARY_QUERY}-one"}]})
        second = media.media_prepare({**common, "reference_media": [{"kind": "image", "url": f"https://assets.example.invalid/frame.png?sig={CANARY_QUERY}-two"}]})
        self.assertNotEqual(first["confirmation_id"], second["confirmation_id"])
        first_record = json.loads(self.record_path(first["confirmation_id"]).read_text(encoding="utf-8"))
        second_record = json.loads(self.record_path(second["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertNotEqual(first_record.get("intent_fingerprint"), second_record.get("intent_fingerprint"))
        for result, record in ((first, first_record), (second, second_record)):
            serialized = json.dumps({"result": result, "record": record}, ensure_ascii=False)
            self.assertNotIn(CANARY_QUERY, serialized)
            protected = record["request"]["reference_media"][0]
            self.assertNotIn("url", protected)
            self.assertIn(protected["url_protected"].split(":", 1)[0], {"dpapi", "test"})
        restored = media._restore_request_urls(first_record["request"])
        self.assertIn(CANARY_QUERY + "-one", restored["reference_media"][0]["url"])
        self.assertEqual(self.http.state.count("POST"), 0)

    @unittest.skipUnless(os.name == "nt", "Named mutex contract is Windows-specific")
    def test_prepare_cannot_recover_live_submit_before_request_marker(self) -> None:
        prepared = self.prepared_image("submit prepare cross-process race")
        request = {"provider": "image-2", "model": "gpt-image-2.5", "prompt": "submit prepare cross-process race", "parameters": {"size": "auto"}}
        self.http.state.enqueue("/v1/responses", 200, {"output": [{"type": "image_generation_call", "result": base64.b64encode(valid_png()).decode("ascii")}]})
        submit_args = self.submit_args(prepared)
        original_http_json = media._http_json
        child_outcomes: list[dict[str, Any]] = []

        def run_prepare_process() -> dict[str, Any]:
            child_code = (
                "import importlib.util,json\n"
                f"p={str(SERVICE_PATH)!r}\n"
                "s=importlib.util.spec_from_file_location('child_media',p)\n"
                "m=importlib.util.module_from_spec(s)\n"
                "s.loader.exec_module(m)\n"
                f"r=json.loads(json.loads({json.dumps(json.dumps(request))!r}))\n"
                "try:\n"
                " x=m.media_prepare(r)\n"
                " print(json.dumps({'created':True,'id':x.get('confirmation_id')}))\n"
                "except m.MediaError as e:\n"
                " print(json.dumps({'created':False,'code':e.code}))\n"
            )
            child = subprocess.run([sys.executable, "-c", child_code], capture_output=True, text=True, env=os.environ.copy(), timeout=10)
            if child.returncode != 0:
                raise AssertionError(child.stderr)
            return json.loads(child.stdout.strip().splitlines()[-1])

        def interleaved_http(method: str, path: str, body: Any, provider: str, idempotency_key: str = "", *, before_request=None):
            child_outcomes.append(run_prepare_process())
            return original_http_json(method, path, body, provider, idempotency_key, before_request=before_request)

        with unittest.mock.patch.object(media, "_http_json", side_effect=interleaved_http):
            submitted = media.media_submit(submit_args)
        self.assertEqual(submitted["state"], "completed_saved")
        self.assertEqual(child_outcomes, [{"created": False, "code": "INTENT_SUBMISSION_IN_PROGRESS"}])
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_completed_image_save_hash_and_tamper_detection(self) -> None:
        prepared = self.prepared_image("valid image artifact")
        image = valid_png()
        response = {"output": [{"type": "image_generation_call", "result": base64.b64encode(image).decode("ascii")}]}
        self.http.state.enqueue("/v1/responses", 200, response)
        submitted = media.media_submit(self.submit_args(prepared))
        self.assertIn(submitted["state"], {"completed_pending_save", "completed_saved"})
        saved = media.media_save({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
        self.assertTrue(saved["saved"])
        artifact_path = Path(saved["saved"][0]["path"])
        self.assertEqual(artifact_path.read_bytes(), image)
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        metadata = record.get("saved_artifacts", [])[0]
        self.assertEqual(metadata.get("sha256"), hashlib.sha256(image).hexdigest())
        artifact_path.write_bytes(image + b"tampered")
        with self.assertRaises(media.MediaError) as tampered:
            media.media_save({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
        self.assertIn(getattr(tampered.exception, "code", ""), {"SAVED_ARTIFACT_TAMPERED", "ARTIFACT_INTEGRITY_FAILED"})
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_grok_image_edit_uses_single_image_field_and_multi_image_array(self) -> None:
        self.refresh_catalog(["grok-imagine-image-2.0"])
        source = self.root / "grok-edit-input.png"
        source.write_bytes(valid_png())
        common = {
            "provider": "grok-image",
            "model": "grok-imagine-image-2.0",
            "prompt": "Edit the supplied image",
            "parameters": {"n": 1, "aspect_ratio": "1:1", "resolution": "1k"},
        }

        single = media._validate_request({
            **common,
            "reference_media": [{"kind": "image", "local_path": str(source)}],
        })
        path, body = media._request_payload(single)
        self.assertEqual(path, "/v1/images/edits")
        self.assertEqual(body["image"]["type"], "image_url")
        self.assertTrue(body["image"]["url"].startswith("data:image/png;base64,"))
        self.assertEqual(base64.b64decode(body["image"]["url"].split(",", 1)[1]), valid_png())
        self.assertNotIn("images", body)

        multiple = media._validate_request({
            **common,
            "reference_media": [
                {"kind": "image", "url": "https://assets.example.invalid/one.png"},
                {"kind": "image", "url": "https://assets.example.invalid/two.png"},
            ],
        })
        path, body = media._request_payload(multiple)
        self.assertEqual(path, "/v1/images/edits")
        self.assertEqual(body["images"], [
            {"type": "image_url", "url": "https://assets.example.invalid/one.png"},
            {"type": "image_url", "url": "https://assets.example.invalid/two.png"},
        ])
        self.assertNotIn("image", body)
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_grok_image_reference_limit_matches_provider_contract(self) -> None:
        self.refresh_catalog(["grok-imagine-image-2.0"])
        common = {"provider": "grok-image", "model": "grok-imagine-image-2.0", "prompt": "Multi-image edit"}
        five = [{"kind": "image", "url": f"https://assets.example.invalid/{index}.png"} for index in range(5)]
        accepted = media._validate_request({**common, "reference_media": five})
        self.assertEqual(len(accepted["reference_media"]), 5)
        six = [{"kind": "image", "url": f"https://assets.example.invalid/{index}.png"} for index in range(6)]
        with self.assertRaises(media.MediaError):
            media._validate_request({**common, "reference_media": six})
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_grok_base64_jpeg_is_sniffed_and_saved_with_jpeg_extension(self) -> None:
        jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xd9"
        encoded = base64.b64encode(jpeg).decode("ascii")
        artifacts = media._artifacts({"data": [{"b64_json": encoded}]}, provider="grok-image")
        self.assertEqual(artifacts, [{"kind": "base64", "value": encoded, "mime_type": "image/jpeg"}])

        saved = media._save_artifacts({"confirmation_id": "media-grok-jpeg-fixture", "artifacts": artifacts})
        self.assertEqual(len(saved), 1)
        path = Path(saved[0]["path"])
        self.assertEqual(path.suffix, ".jpg")
        self.assertEqual(path.read_bytes(), jpeg)
        self.assertEqual(saved[0]["sha256"], hashlib.sha256(jpeg).hexdigest())
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_truncated_base64_jpeg_is_rejected_without_writing_a_file(self) -> None:
        truncated = b"\xff\xd8\xff\xe0\x00\x10JFIF"
        encoded = base64.b64encode(truncated).decode("ascii")
        artifacts = media._artifacts({"data": [{"b64_json": encoded}]}, provider="grok-image")
        with self.assertRaises(media.MediaError) as raised:
            media._save_artifacts({"confirmation_id": "media-truncated-jpeg-fixture", "artifacts": artifacts})
        self.assertEqual(getattr(raised.exception, "code", None), "ARTIFACT_INTEGRITY_FAILED")
        self.assertFalse((media._data_root() / "outputs" / "media-truncated-jpeg-fixture" / "result-1.jpg").exists())
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_unsupported_base64_image_format_is_rejected_without_network(self) -> None:
        payload = b"not-an-image-container"
        encoded = base64.b64encode(payload).decode("ascii")
        artifacts = media._artifacts({"data": [{"b64_json": encoded}]}, provider="grok-image")
        self.assertEqual(artifacts[0]["mime_type"], "application/octet-stream")
        with self.assertRaises(media.MediaError) as raised:
            media._save_artifacts({"confirmation_id": "media-unsupported-image-fixture", "artifacts": artifacts})
        self.assertIn("格式不受支持", str(raised.exception))
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_legacy_persisted_jpeg_mislabeled_as_png_is_saved_as_jpeg(self) -> None:
        jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xd9"
        encoded = base64.b64encode(jpeg).decode("ascii")
        saved = media._save_artifacts({
            "confirmation_id": "media-legacy-jpeg-mislabeled-png-fixture",
            "artifacts": [{"kind": "base64", "value": encoded, "mime_type": "image/png"}],
        })
        self.assertEqual(len(saved), 1)
        path = Path(saved[0]["path"])
        self.assertEqual(path.suffix, ".jpg")
        self.assertEqual(path.read_bytes(), jpeg)
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_completed_without_artifact_is_not_reported_as_delivered(self) -> None:
        prepared = self.prepared_image("completed but no output")
        self.http.state.enqueue("/v1/responses", 200, {"status": "completed", "output": []})
        try:
            submitted = media.media_submit(self.submit_args(prepared))
        except media.MediaError as error:
            self.assertIn(getattr(error, "code", ""), {"COMPLETED_WITHOUT_ARTIFACT", "SUBMISSION_UNKNOWN"})
        else:
            self.assertFalse(submitted.get("success", True))
            self.assertIn(submitted.get("state"), {"completed_without_artifact", "result_unavailable"})
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertIn(record["state"], {"completed_without_artifact", "result_unavailable", "submission_unknown"})
        self.assertEqual(self.http.state.count("POST", "/v1/responses"), 1)

    def test_unknown_provider_status_does_not_masquerade_as_running(self) -> None:
        self.refresh_catalog(["grok-imagine-video-1.5"])
        prepared = media.media_prepare({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "unknown status fixture", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}})
        self.http.state.enqueue("/v1/videos/generations", 200, {"id": "task-fixture-unknown", "status": "queued"})
        submitted = media.media_submit(self.submit_args(prepared))
        self.assertIn(submitted["state"], {"task_created", "in_progress"})
        self.http.state.enqueue("/v1/videos/task-fixture-unknown", 200, {"status": "vendor-added-state"})
        try:
            checked = media.media_check({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
        except media.MediaError as error:
            self.assertEqual(getattr(error, "code", None), "PROVIDER_STATUS_UNKNOWN")
        else:
            self.assertEqual(checked["state"], "provider_status_unknown")
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertNotIn(record["state"], {"in_progress", "completed", "failed"})
        self.assertEqual(self.http.state.count("POST", "/v1/videos/generations"), 1)

    def test_transient_poll_failure_preserves_known_task_without_resubmit(self) -> None:
        self.refresh_catalog(["grok-imagine-video-1.5"])
        prepared = media.media_prepare({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "poll failure fixture", "parameters": {"duration": 4, "aspect_ratio": "16:9", "resolution": "480p"}})
        self.http.state.enqueue("/v1/videos/generations", 200, {"id": "task-poll-failure", "status": "queued"})
        media.media_submit(self.submit_args(prepared))
        self.http.state.enqueue("/v1/videos/task-poll-failure", 503, {"error": {"code": "temporarily_unavailable"}}, {"X-Request-ID": "poll-503-fixture"})
        args = {"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]}
        with self.assertRaises(media.MediaError) as raised:
            media.media_check(args)
        self.assertEqual(getattr(raised.exception, "code", None), "UPSTREAM_UNAVAILABLE")
        self.assertEqual(getattr(raised.exception, "provider_request_id", None), "poll-503-fixture")
        record = json.loads(self.record_path(prepared["confirmation_id"]).read_text(encoding="utf-8"))
        self.assertIn(record["state"], {"task_created", "in_progress"})
        self.assertEqual(record.get("task_id"), "task-poll-failure")
        self.assertEqual(self.http.state.count("POST", "/v1/videos/generations"), 1)
        self.assertEqual(self.http.state.count("GET", "/v1/videos/task-poll-failure"), 1)

    def test_download_validates_content_type_magic_and_auth_boundary(self) -> None:
        self.refresh_catalog()
        image = valid_png()
        self.http.state.enqueue("/result.png", 200, image, {"Content-Type": "image/png"})
        destination = self.data / "download.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        original_validate = media._validate_public_url
        media._validate_public_url = lambda value, **_: value  # type: ignore[method-assign]
        try:
            size, content_type = media._safe_download(self.http.url + "/result.png", destination)
        finally:
            media._validate_public_url = original_validate  # type: ignore[method-assign]
        self.assertEqual(size, len(image))
        self.assertEqual(content_type, "image/png")
        request = self.http.state.requests[-1]
        self.assertEqual(request["headers"].get("Authorization"), f"Bearer {CANARY_KEY}")

        # A different origin must not receive the gateway token, even though
        # it is also served by a loopback fake in this offline test.
        with FakeHttpServer() as external:
            external.state.enqueue("/external.png", 200, image, {"Content-Type": "image/png"})
            external_validate = media._validate_public_url
            media._validate_public_url = lambda value, **_: value  # type: ignore[method-assign]
            try:
                media._safe_download(external.url + "/external.png", self.data / "external.png")
            finally:
                media._validate_public_url = external_validate  # type: ignore[method-assign]
            self.assertNotIn("Authorization", external.state.requests[-1]["headers"])

        self.http.state.enqueue("/wrong.png", 200, b"this is not a PNG", {"Content-Type": "image/png"})
        bad_target = self.data / "wrong.png"
        validate_again = media._validate_public_url
        media._validate_public_url = lambda value, **_: value  # type: ignore[method-assign]
        try:
            with self.assertRaises(media.MediaError):
                media._safe_download(self.http.url + "/wrong.png", bad_target)
        finally:
            media._validate_public_url = validate_again  # type: ignore[method-assign]
        self.assertFalse(bad_target.exists())
        self.assertFalse(bad_target.with_suffix(bad_target.suffix + ".part").exists())

    def test_jsonrpc_utf8_protocol_has_no_non_json_stdout(self) -> None:
        request_lines = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": media.PROTOCOL_VERSION, "clientInfo": {"name": "中文验收", "version": "1"}}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "media_capabilities", "arguments": {}}},
        ]
        env = os.environ.copy()
        env.update(self.env)
        env.update({"MEDIA_GENERATION_API_BASE_URL": self.http.url, "PYTHONIOENCODING": "gbk", "PYTHONUTF8": "0"})
        env.pop("MEDIA_GENERATION_API_KEY", None)
        result = subprocess.run(
            [sys.executable, str(SERVICE_PATH)],
            input=("\n".join(json.dumps(item, ensure_ascii=False) for item in request_lines) + "\n").encode("utf-8"),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=15,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
        lines = result.stdout.decode("utf-8", errors="strict").splitlines()
        self.assertEqual(len(lines), 3)
        replies = [json.loads(line) for line in lines]
        self.assertEqual([reply["id"] for reply in replies], [1, 2, 3])
        self.assertEqual(replies[0]["result"]["serverInfo"]["version"], media.SERVER_VERSION)
        self.assertEqual(len(replies[1]["result"]["tools"]), len(media.TOOL_SCHEMAS))
        prepare_schema = next(tool["inputSchema"] for tool in replies[1]["result"]["tools"] if tool["name"] == "media_prepare")
        role_schema = prepare_schema["properties"]["reference_media"]["items"]["properties"]["role"]
        self.assertEqual(role_schema["enum"], ["first_frame", "reference"])
        parameters_schema = prepare_schema["properties"]["parameters"]
        self.assertEqual(parameters_schema["type"], "object")
        self.assertFalse(parameters_schema["additionalProperties"])
        self.assertEqual(parameters_schema["properties"]["aspect_ratio"]["type"], "string")
        self.assertIn("provider-specific", parameters_schema["description"].lower())
        self.assertEqual(prepare_schema["type"], "object")
        self.assertNotIn("allOf", prepare_schema)
        serialized_schema = json.dumps(prepare_schema)
        self.assertNotIn('"if"', serialized_schema)
        self.assertNotIn('"then"', serialized_schema)
        self.assertNotIn('"oneOf"', serialized_schema)
        self.assertNotIn('"anyOf"', serialized_schema)
        self.assertEqual(prepare_schema["properties"]["provider"]["enum"], list(media.PROVIDERS))
        self.assertEqual(prepare_schema["properties"]["reference_media"]["items"]["properties"]["role"]["enum"], ["first_frame", "reference"])
        generic_params = parameters_schema["properties"]
        self.assertEqual(set(generic_params["aspect_ratio"]["enum"]), {"1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "2:1", "1:2", "19.5:9", "9:19.5", "20:9", "9:20", "21:9", "5:2", "auto"})
        self.assertEqual(generic_params["resolution"]["enum"], ["1080p", "1k", "2k", "480p", "720p"])
        self.assertEqual(generic_params["generate_audio"]["type"], "boolean")
        self.assertNotIn("const", generic_params["generate_audio"])
        self.assertEqual(generic_params["n"]["maximum"], 10)
        capability = json.loads(replies[2]["result"]["content"][0]["text"])
        self.assertEqual(capability["generation_capability"], "not_tested")
        self.assertEqual(result.stderr, b"")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_kimi_shaped_tool_call_can_prepare_grok_video_without_retry_loop(self) -> None:
        # Mirrors the arguments Kimi should produce from the advertised MCP
        # schema. Preparation is local-only and must not submit a paid task.
        self.refresh_catalog()
        kimi_arguments = {
            "provider": "grok-video",
            "model": "grok-imagine-video-1.5",
            "prompt": "Create a short vertical product clip.",
            "parameters": {"duration": 5, "aspect_ratio": "9:16", "resolution": "480p"},
        }
        result = media._call_tool("media_prepare", kimi_arguments)
        self.assertTrue(result["success"])
        self.assertFalse(result["api_request_sent"])
        self.assertEqual(result["parameters"]["duration"], 5)
        self.assertEqual(result["parameters"]["aspect_ratio"], "9:16")
        self.assertEqual(result["parameters"]["resolution"], "480p")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_static_schema_covers_legacy_image2_count_while_runtime_keeps_profile_rules(self) -> None:
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "legacy"
        self.refresh_catalog(["gpt-image-2.5"])
        prepared = media.media_prepare({
            "provider": "image-2", "model": "gpt-image-2.5", "prompt": "legacy batch image",
            "parameters": {"size": "2048x2048", "n": 2},
        })
        self.assertEqual(prepared["parameters"]["n"], 2)
        self.assertEqual(self.http.state.count("POST"), 0)
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "responses"
        self.refresh_catalog(["gpt-image-2.5"])
        with self.assertRaises(media.MediaError) as response_profile_rejected:
            media.media_prepare({
                "provider": "image-2", "model": "gpt-image-2.5", "prompt": "response count restriction",
                "parameters": {"size": "auto", "n": 2},
            })
        self.assertIn("n=1", str(response_profile_rejected.exception))

    def test_static_schema_covers_grok_image_count_while_wokey_keeps_runtime_limit(self) -> None:
        os.environ["MEDIA_GENERATION_API_PROFILE"] = "responses"
        os.environ["MEDIA_GROK_IMAGE_MODEL"] = "grok-imagine-image-2.0"
        self.refresh_catalog(["grok-imagine-image-2.0"])
        prepared = media.media_prepare({
            "provider": "grok-image", "model": "grok-imagine-image-2.0", "prompt": "batch image schema range",
            "parameters": {"n": 10, "aspect_ratio": "1:1", "resolution": "1k"},
        })
        self.assertEqual(prepared["parameters"]["n"], 10)
        self.assertEqual(self.http.state.count("POST"), 0)

        os.environ["MEDIA_GENERATION_API_PROFILE"] = "wokey"
        self.refresh_catalog(["grok-imagine-image-2.0"])
        with self.assertRaises(media.MediaError) as wokey_rejected:
            media.media_prepare({
                "provider": "grok-image", "model": "grok-imagine-image-2.0", "prompt": "wokey runtime limit",
                "parameters": {"n": 2, "aspect_ratio": "1:1", "resolution": "1k"},
            })
        self.assertIn("Wokey", str(wokey_rejected.exception))

    def test_confirmation_key_first_initialization_is_thread_safe(self) -> None:
        path = self.data / ".confirmation-key"
        self.assertFalse(path.exists())
        barrier = threading.Barrier(16)
        results: list[bytes | Exception] = []
        result_lock = threading.Lock()

        def initialize() -> None:
            try:
                barrier.wait(timeout=3)
                key = media._secret_key()
            except Exception as exc:
                key = exc
            with result_lock:
                results.append(key)

        workers = [threading.Thread(target=initialize) for _ in range(16)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(timeout=5)
        self.assertTrue(all(not worker.is_alive() for worker in workers))
        self.assertEqual(len(results), 16)
        self.assertTrue(all(isinstance(item, bytes) and len(item) == 32 for item in results), results)
        self.assertTrue(all(item == results[0] for item in results))
        self.assertEqual(path.read_bytes(), results[0])

    def test_prepare_validation_failure_is_actionable_and_never_creates_confirmation(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        args = {"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": CANARY_PROMPT, "parameters": {"duration": 5}}
        with self.assertRaises(media.MediaError) as rejected:
            media.media_prepare(args)
        error = rejected.exception
        self.assertEqual(error.code, "VIDEO_PARAMETERS_REQUIRED")
        envelope = media._media_error_envelope(error)
        self.assertFalse(envelope["api_request_sent"])
        self.assertFalse(envelope["confirmation_created"])
        self.assertEqual(envelope["missing_parameters"], ["aspect_ratio", "resolution"])
        self.assertEqual(envelope["allowed_values"]["resolution"], ["480p", "720p", "1080p"])
        self.assertEqual(envelope["recovery_action"], "correct_arguments_once")
        self.assertEqual(list((self.data / "confirmations").glob("media-*.json")) if (self.data / "confirmations").exists() else [], [])
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_prepare_loop_guard_stops_exact_repeat_then_allows_corrected_request(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        invalid = {"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "loop exact", "parameters": {"duration": 5}}
        with self.assertRaises(media.MediaError) as first:
            media.media_prepare(invalid)
        self.assertNotEqual(first.exception.code, "LOOP_GUARD")
        with self.assertRaises(media.MediaError) as second:
            media.media_prepare(invalid)
        self.assertEqual(second.exception.code, "LOOP_GUARD")
        self.assertFalse(second.exception.retryable)
        self.assertFalse(second.exception.api_request_sent)
        self.assertEqual(second.exception.details["recovery_action"], "stop_and_ask_user")
        corrected = copy.deepcopy(invalid)
        corrected["parameters"].update({"aspect_ratio": "16:9", "resolution": "480p"})
        prepared = media.media_prepare(corrected)
        self.assertTrue(prepared["success"])
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_prepare_loop_guard_stops_third_changed_attempt_but_not_other_intent(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        base = {"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "loop intent", "parameters": {"duration": 5}}
        for resolution in ("480p", "720p"):
            attempt = copy.deepcopy(base)
            attempt["parameters"]["resolution"] = resolution
            with self.assertRaises(media.MediaError):
                media.media_prepare(attempt)
        third = copy.deepcopy(base)
        third["parameters"]["aspect_ratio"] = "16:9"
        with self.assertRaises(media.MediaError) as blocked:
            media.media_prepare(third)
        self.assertEqual(blocked.exception.code, "LOOP_GUARD")
        other = copy.deepcopy(third)
        other["prompt"] = "different intent"
        with self.assertRaises(media.MediaError) as independent:
            media.media_prepare(other)
        self.assertNotEqual(independent.exception.code, "LOOP_GUARD")
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_prepare_loop_guard_is_concurrency_safe_and_does_not_count_catalog_failures(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        args = {"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "parallel invalid", "parameters": {}}
        with self.assertRaises(media.MediaError) as catalog_error:
            media.media_prepare(args)
        self.assertEqual(catalog_error.exception.code, "MODEL_CATALOG_MISSING")
        self.assertFalse(media._PREPARE_FAILURES)
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        results: list[str] = []
        lock = threading.Lock()

        def invoke() -> None:
            try:
                media.media_prepare(args)
                code = "success"
            except media.MediaError as exc:
                code = exc.code if exc.code == "LOOP_GUARD" else f"{exc.code}: {exc.safe_message}"
            with lock:
                results.append(code)

        workers = [threading.Thread(target=invoke) for _ in range(8)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(timeout=5)
        self.assertTrue(all(not worker.is_alive() for worker in workers))
        self.assertEqual(results.count("LOOP_GUARD"), 7, results)
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_prepare_loop_guard_store_is_bounded_and_expired_entries_are_removed(self) -> None:
        now = time.monotonic()
        with media._PREPARE_FAILURES_LOCK:
            media._PREPARE_FAILURES["expired"] = {"updated_at": now - media.PREPARE_FAILURE_WINDOW_SECONDS - 1, "total": 1, "exact": {}}
            for index in range(media.PREPARE_FAILURE_MAX_INTENTS):
                media._PREPARE_FAILURES[f"entry-{index}"] = {"updated_at": now + index, "total": 1, "exact": {}}
        result = media._prepare_failure_error("new-entry", "exact", media.MediaError("invalid"))
        self.assertIsNone(result)
        self.assertNotIn("expired", media._PREPARE_FAILURES)
        self.assertLessEqual(len(media._PREPARE_FAILURES), media.PREPARE_FAILURE_MAX_INTENTS)
        for index in range(5):
            media._prepare_failure_error("new-entry", f"variant-{index}", media.MediaError("invalid"))
        self.assertLessEqual(len(media._PREPARE_FAILURES["new-entry"]["exact"]), media.PREPARE_FAILURE_MAX_EXACT_PER_INTENT)
        serialized = json.dumps(media._PREPARE_FAILURES)
        self.assertNotIn(CANARY_PROMPT, serialized)
        self.assertNotIn(CANARY_KEY, serialized)

    def test_validation_error_uses_structured_mcp_result_and_never_posts(self) -> None:
        os.environ["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        self.refresh_catalog(["grok-imagine-video-1.5"])
        request = {
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {"name": "media_prepare", "arguments": {
                "provider": "grok-video", "model": "grok-imagine-video-1.5",
                "prompt": CANARY_PROMPT, "parameters": {"duration": 5},
            }},
        }
        env = os.environ.copy()
        env.update(self.env)
        env["MEDIA_GROK_VIDEO_MODEL"] = "grok-imagine-video-1.5"
        result = subprocess.run([sys.executable, str(SERVICE_PATH)], input=(json.dumps(request, ensure_ascii=False) + "\n").encode("utf-8"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=10, check=False)
        self.assertEqual(result.returncode, 0)
        reply = json.loads(result.stdout.decode("utf-8"))
        self.assertTrue(reply["result"]["isError"])
        structured = reply["result"]["structuredContent"]
        self.assertEqual(structured["error"]["code"], "VIDEO_PARAMETERS_REQUIRED")
        self.assertFalse(structured["api_request_sent"])
        self.assertFalse(structured["confirmation_created"])
        text_envelope = json.loads(reply["result"]["content"][0]["text"])
        self.assertEqual(text_envelope, structured)
        self.assertNotIn(CANARY_PROMPT, json.dumps(structured, ensure_ascii=False))
        self.assertEqual(self.http.state.count("POST"), 0)

    def test_structured_input_error_survives_tools_call_jsonrpc(self) -> None:
        request = {
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": {"name": "media_prepare", "arguments": {"provider": "image-2", "model": "gpt-image-2.5", "prompt": CANARY_PROMPT}},
        }
        # Directly run one JSON-RPC request in a subprocess so stdout/stderr
        # boundaries are tested independently of importing the service.
        env = os.environ.copy()
        env.update(self.env)
        env.update({"MEDIA_GENERATION_API_BASE_URL": self.http.url})
        result = subprocess.run([sys.executable, str(SERVICE_PATH)], input=(json.dumps(request, ensure_ascii=False) + "\n").encode("utf-8"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=10, check=False)
        self.assertEqual(result.returncode, 0)
        reply = json.loads(result.stdout.decode("utf-8"))
        self.assertEqual(reply["id"], 10)
        self.assertTrue(reply["result"]["isError"])
        error_text = reply["result"]["content"][0]["text"]
        error = json.loads(error_text)
        self.assertFalse(error["success"])
        self.assertEqual(error["error"]["code"], "MODEL_CATALOG_MISSING")
        self.assertNotIn(CANARY_KEY, error_text)
        self.assertNotIn(CANARY_PROMPT, error_text)
        self.assertEqual(self.http.state.count("POST"), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
