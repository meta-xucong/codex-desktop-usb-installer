#!/usr/bin/env python3
"""Local, stdio MCP facade for configured image and video API gateways.

Only Python's standard library is required. The server never guesses a provider
model, price, refund policy, or secret. Paid requests use an immutable local
confirmation record and a single-use submission state machine.
"""

from __future__ import annotations

import base64
import contextlib
import hashlib
import hmac
import ipaddress
import json
import mimetypes
import os
import re
import socket
import struct
import subprocess
import sys
import threading
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib
from pathlib import Path
from typing import Any, Callable


SERVER_VERSION = "1.3.9"
PROTOCOL_VERSION = "2025-03-26"
MAX_JSON_BYTES = 32 * 1024 * 1024
HTTP_TIMEOUT_SECONDS = 600
MAX_REFERENCE_BYTES = 25 * 1024 * 1024
MAX_GROK_VIDEO_LOCAL_IMAGE_BYTES = 8 * 1024 * 1024
MAX_GROK_VIDEO_REFERENCE_IMAGES = 14
MAX_GROK_VIDEO_TOTAL_LOCAL_IMAGE_BYTES = 25 * 1024 * 1024
MAX_DOWNLOAD_BYTES = 512 * 1024 * 1024
MAX_PNG_PIXELS = 100_000_000
MAX_PNG_DECODED_BYTES = 256 * 1024 * 1024
CONFIRMATION_TTL_SECONDS = 30 * 60
MODEL_CATALOG_TTL_SECONDS = 24 * 60 * 60
INTENT_LOCK_TIMEOUT_SECONDS = 10
_CONFIRMATION_LOCKS: dict[str, threading.Lock] = {}
_CONFIRMATION_LOCKS_GUARD = threading.Lock()
PROVIDERS = {
    "image-2": {"family": "image", "model_env": "MEDIA_IMAGE2_MODEL"},
    "grok-image": {"family": "image", "model_env": "MEDIA_GROK_IMAGE_MODEL"},
    "grok-video": {"family": "video", "model_env": "MEDIA_GROK_VIDEO_MODEL"},
    "seedance-video": {"family": "video", "model_env": "MEDIA_SEEDANCE_MODEL"},
    # MiniMax is intentionally just a shared-gateway slot. It remains unavailable
    # until the user configures a model exposed by the selected unified upstream.
    "minimax-video": {"family": "video", "model_env": "MEDIA_MINIMAX_VIDEO_MODEL"},
}
IMAGE_SIZES = {
    "2048x2048", "1360x2048", "2048x1360", "1536x2048", "2048x1536",
    "1632x2048", "2048x1632", "1152x2048", "2048x1152", "2048x864",
}
IMAGE_TYPES = {"image/png", "image/jpeg", "image/webp", "image/gif"}
VIDEO_TYPES = {"video/mp4", "video/webm", "video/quicktime", "video/x-m4v"}
AUDIO_TYPES = {"audio/mpeg", "audio/wav", "audio/mp4", "audio/aac", "audio/ogg", "audio/flac"}


class MediaError(Exception):
    def __init__(self, message: str, *, code: str = "MEDIA_REQUEST_REJECTED", provider: str | None = None,
                 model: str | None = None, http_status: int | None = None,
                 provider_request_id: str | None = None, gateway_request_id: str | None = None,
                 retryable: bool = False, submission_state: str | None = None,
                 api_request_sent: bool | None = None, details: dict[str, Any] | None = None):
        super().__init__(message)
        self.code = code
        self.safe_message = message
        self.provider = provider
        self.model = model
        self.http_status = http_status
        self.provider_request_id = provider_request_id
        self.gateway_request_id = gateway_request_id
        self.retryable = retryable
        self.submission_state = submission_state
        self.api_request_sent = api_request_sent
        self.details = details or {}


def _config_path() -> Path:
    configured = os.environ.get("MEDIA_GENERATION_CONFIG_PATH", "").strip()
    return Path(configured).resolve() if configured else Path(__file__).resolve().parent.parent / "connections" / "media-generation.json"


def _runtime_config(*, allow_gateway_mismatch: bool = False) -> dict[str, Any]:
    path = _config_path()
    if not path.exists():
        return {}
    value = _load_json(path)
    if value.get("schemaVersion") != 1:
        raise MediaError("媒体配置版本不受支持。")
    configured_host = value.get("gateway", "").rstrip("/")
    active_host = os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip().rstrip("/")
    if configured_host and active_host and configured_host != active_host:
        if allow_gateway_mismatch:
            return {}
        raise MediaError("媒体模型缓存与当前网关不一致；请重新发现模型。", code="MODEL_CATALOG_ROUTE_MISMATCH")
    return value


def _catalog_snapshot() -> dict[str, Any]:
    config = _runtime_config()
    ids = config.get("catalogIds")
    verified = config.get("catalogVerifiedAt")
    if not isinstance(ids, list) or not all(isinstance(item, str) for item in ids):
        raise MediaError("尚未发现当前网关的媒体模型；请先刷新模型列表。", code="MODEL_CATALOG_MISSING")
    try:
        verified_at = int(verified)
    except (TypeError, ValueError):
        raise MediaError("媒体模型目录记录无效；请重新刷新模型列表。", code="MODEL_CATALOG_INVALID")
    if int(time.time()) - verified_at > MODEL_CATALOG_TTL_SECONDS:
        raise MediaError("模型列表已超过 24 小时；请先刷新模型列表。", code="MODEL_CATALOG_STALE")
    if config.get("gateway") != _base() or config.get("profile") != _api_profile():
        raise MediaError("媒体模型目录与当前网关配置不匹配；请重新刷新模型列表。", code="MODEL_CATALOG_ROUTE_MISMATCH")
    return {"verified_at": verified_at, "ids": set(ids), "fingerprint": _token({
        "gateway": _base(), "profile": _api_profile(), "verified_at": verified_at, "ids": sorted(set(ids))
    })}


def _configured_model(provider: str) -> str:
    explicit = os.environ.get(PROVIDERS[provider]["model_env"], "").strip()
    if explicit:
        return explicit
    models = _runtime_config().get("models", {})
    return str(models.get(provider, "")).strip() if isinstance(models, dict) else ""


def _route_context() -> dict[str, str]:
    return {
        "gateway": os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip().rstrip("/"),
        "profile": _api_profile(),
        "agent_id": os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip(),
    }


def media_refresh_models(args: dict[str, Any]) -> dict[str, Any]:
    """Explicit read-only discovery; never run network I/O in tools/list/prepare."""
    # Wokey's default model catalog is text-only. Ask explicitly for all
    # modalities so the media model slots can be matched against its live list.
    catalog_path = "/v1/models?output_modalities=all" if _api_profile() == "wokey" else "/v1/models"
    response = _http_json("GET", catalog_path, None, "catalog")
    rows = response.get("data")
    if not isinstance(rows, list):
        raise MediaError("模型目录缺少 data 数组；未更新配置。")
    if len(rows) > 10000:
        raise MediaError("模型目录项目数超过上限；未更新配置。", code="MODEL_CATALOG_INVALID")
    available = set()
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("id"), str):
            continue
        model_id = row["id"].strip()
        if 0 < len(model_id) <= 256 and not any(ord(char) < 32 for char in model_id):
            available.add(model_id)
    # Conservative defaults: choose exact advertised IDs, not vendor-branded chat models.
    preferred = {
        "image-2": ["gpt-image-2.5", "gpt-image-2"],
        "grok-image": ["grok-imagine-image-2.0"],
        "grok-video": ["grok-imagine-video-1.5", "grok-imagine-video-1.5-lite"],
    }
    # Discovery must be able to repair a cache left by a different gateway.
    # Never carry defaults from the other gateway into the new catalog.
    previous = _runtime_config(allow_gateway_mismatch=True)
    previous_models = previous.get("models", {})
    if not isinstance(previous_models, dict):
        previous_models = {}
    models = {}
    for provider in PROVIDERS:
        old = os.environ.get(PROVIDERS[provider]["model_env"], "").strip() or str(previous_models.get(provider, "")).strip()
        if old in available:
            models[provider] = old
        else:
            models[provider] = next((model for model in preferred.get(provider, []) if model in available), "")
    config = {
        "schemaVersion": 1, "gateway": _base(), "profile": _api_profile(), "models": models,
        "catalogVerifiedAt": int(time.time()), "catalogIds": sorted(available),
    }
    _atomic_json(_config_path(), config)
    return {"success": True, "api_request_sent": True, "generation_request_sent": False,
            "gateway_catalog_status": "verified", "catalog_verified_at": config["catalogVerifiedAt"],
            "models": models, "catalog_count": len(available), "config_path": str(_config_path()),
            "note": "目录认证通过；模型生成路由仍需实际验收。"}


def _data_root() -> Path:
    configured = os.environ.get("MEDIA_GENERATION_DATA_DIR", "").strip()
    if configured:
        root = Path(configured).expanduser().resolve()
    else:
        root = (Path.home() / ".codex" / "codex-agent-kit" / "data" / "media-generation").resolve()
    root.mkdir(parents=True, exist_ok=True)
    return root


def _canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _safe_identifier(value: Any, limit: int = 256) -> str | None:
    if not isinstance(value, str):
        return None
    value = value.strip()
    if not value or len(value) > limit or not re.fullmatch(r"[A-Za-z0-9._:-]+", value):
        return None
    return value


def _atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    with temporary.open("wb") as stream:
        stream.write(json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


@contextlib.contextmanager
def _intent_index_lock():
    """Cross-process lock serializing intent scans and confirmation creation."""
    identity = hashlib.sha256(str(_data_root().resolve()).casefold().encode("utf-8")).hexdigest()
    lock_path = _data_root() / ".intent-index.lock"
    deadline = time.monotonic() + INTENT_LOCK_TIMEOUT_SECONDS
    if _uses_windows_mutex():
        import ctypes
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.CreateMutexW.argtypes = (ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p)
        kernel32.CreateMutexW.restype = ctypes.c_void_p
        kernel32.WaitForSingleObject.argtypes = (ctypes.c_void_p, ctypes.c_uint32)
        kernel32.WaitForSingleObject.restype = ctypes.c_uint32
        kernel32.ReleaseMutex.argtypes = (ctypes.c_void_p,)
        kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
        handle = kernel32.CreateMutexW(None, False, f"Local\\CodexMediaIntent-v1-{identity}")
        if not handle:
            raise MediaError("无法创建媒体意图互斥锁。", code="INTENT_LOCK_UNAVAILABLE")
        acquired = False
        try:
            remaining = max(0, int((deadline - time.monotonic()) * 1000))
            result = kernel32.WaitForSingleObject(handle, remaining)
            if result not in (0x00000000, 0x00000080):
                raise MediaError("媒体任务索引正被其他进程更新；请稍后重试。", code="INTENT_LOCK_TIMEOUT")
            acquired = True
            yield
        finally:
            if acquired:
                kernel32.ReleaseMutex(handle)
            kernel32.CloseHandle(handle)
        return
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    acquired = False
    try:
        import fcntl
        while time.monotonic() < deadline:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                acquired = True
                break
            except OSError:
                time.sleep(0.025)
        if not acquired:
            raise MediaError("媒体任务索引正被其他进程更新；请稍后重试。", code="INTENT_LOCK_TIMEOUT")
        yield
    finally:
        if acquired:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)


def _intent_fingerprint(request: dict[str, Any], *, legacy_roleless: bool = False, legacy_all_roles: bool = False) -> str:
    refs = []
    for item in request.get("reference_media", []):
        reference_url = item.get("url")
        if item.get("url_protected"):
            reference_url = _unprotect_reference_url(item["url_protected"])
        reference = {"kind": item.get("kind"), "source": item.get("source"),
                     "sha256": item.get("sha256"), "url": reference_url}
        # Explicit roles affect only Grok video intent. One short-lived build
        # included the default role for every media provider; its exact legacy
        # form remains verifiable below for already signed records.
        if not legacy_roleless and (request.get("provider") == "grok-video" or legacy_all_roles):
            reference["role"] = item.get("role", "first_frame")
        refs.append(reference)
    payload = {"gateway": request["route"]["gateway"], "profile": request["route"]["profile"],
               "provider": request["provider"], "model": request["model"], "operation": request["mode"],
               "parameters": request["parameters"], "prompt": request["prompt"], "references": refs}
    return _token(payload)


def _verified_intent_fingerprint(request: dict[str, Any], stored_intent: str) -> str | None:
    """Verify a stored intent HMAC and return its current semantic fingerprint.

    The legacy fallback is deliberately narrow: it is accepted only for records
    whose immutable request contains no role keys, and thus retain the old
    first-frame meaning. The enclosing record HMAC and key id must be verified by
    the caller before this helper is used.
    """
    current = _intent_fingerprint(request)
    if hmac.compare_digest(str(stored_intent), current):
        return current
    refs = request.get("reference_media", [])
    if not isinstance(refs, list) or any(not isinstance(item, dict) or "role" in item for item in refs):
        return None
    for legacy in (
        _intent_fingerprint(request, legacy_roleless=True),
        _intent_fingerprint(request, legacy_all_roles=True),
    ):
        if hmac.compare_digest(str(stored_intent), legacy):
            return current
    return None


def _url_origin(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    host = parsed.hostname or ""
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    port = f":{parsed.port}" if parsed.port else ""
    return f"{parsed.scheme}://{host}{port}"


def _protect_reference_url(value: str) -> str:
    """Protect signed/reference URLs at rest; production Windows uses user DPAPI."""
    raw = value.encode("utf-8")
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        class DataBlob(ctypes.Structure):
            _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_ubyte))]

        source = ctypes.create_string_buffer(raw)
        source_blob = DataBlob(len(raw), ctypes.cast(source, ctypes.POINTER(ctypes.c_ubyte)))
        output_blob = DataBlob()
        crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        crypt32.CryptProtectData.argtypes = (ctypes.POINTER(DataBlob), wintypes.LPCWSTR, ctypes.POINTER(DataBlob), ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DataBlob))
        crypt32.CryptProtectData.restype = wintypes.BOOL
        kernel32.LocalFree.argtypes = (ctypes.c_void_p,)
        kernel32.LocalFree.restype = ctypes.c_void_p
        if not crypt32.CryptProtectData(ctypes.byref(source_blob), "Codex media reference URL", None, None, None, 0, ctypes.byref(output_blob)):
            raise MediaError("无法使用当前 Windows 用户保护媒体参考链接。", code="REFERENCE_PROTECTION_FAILED")
        try:
            protected = ctypes.string_at(output_blob.pbData, output_blob.cbData)
        finally:
            kernel32.LocalFree(output_blob.pbData)
        return "dpapi:" + base64.b64encode(protected).decode("ascii")
    if os.environ.get("MEDIA_GENERATION_TEST_MODE") == "1":
        # Reversible test-only envelope for non-Windows CI; never enabled in a
        # normal install and never treated as production encryption.
        nonce = os.urandom(16)
        key = _secret_key()
        stream = bytearray()
        counter = 0
        while len(stream) < len(raw):
            stream.extend(hmac.new(key, b"test-reference-url" + nonce + counter.to_bytes(4, "big"), hashlib.sha256).digest())
            counter += 1
        cipher = bytes(a ^ b for a, b in zip(raw, stream))
        tag = hmac.new(key, b"test-reference-url-tag" + nonce + cipher, hashlib.sha256).digest()
        return "test:" + base64.b64encode(nonce + tag + cipher).decode("ascii")
    raise MediaError("当前平台无法保护持久化的签名参考链接；没有保存请求。", code="REFERENCE_PROTECTION_UNAVAILABLE")


def _unprotect_reference_url(value: str) -> str:
    try:
        prefix, encoded = value.split(":", 1)
        protected = base64.b64decode(encoded, validate=True)
        if prefix == "dpapi" and os.name == "nt":
            import ctypes
            from ctypes import wintypes

            class DataBlob(ctypes.Structure):
                _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_ubyte))]

            source = ctypes.create_string_buffer(protected)
            source_blob = DataBlob(len(protected), ctypes.cast(source, ctypes.POINTER(ctypes.c_ubyte)))
            output_blob = DataBlob()
            crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            description = wintypes.LPWSTR()
            crypt32.CryptUnprotectData.argtypes = (ctypes.POINTER(DataBlob), ctypes.POINTER(wintypes.LPWSTR), ctypes.POINTER(DataBlob), ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DataBlob))
            crypt32.CryptUnprotectData.restype = wintypes.BOOL
            kernel32.LocalFree.argtypes = (ctypes.c_void_p,)
            kernel32.LocalFree.restype = ctypes.c_void_p
            if not crypt32.CryptUnprotectData(ctypes.byref(source_blob), ctypes.byref(description), None, None, None, 0, ctypes.byref(output_blob)):
                raise MediaError("当前 Windows 用户无法解密媒体参考链接。", code="REFERENCE_PROTECTION_FAILED")
            try:
                raw = ctypes.string_at(output_blob.pbData, output_blob.cbData)
            finally:
                kernel32.LocalFree(output_blob.pbData)
                if description:
                    kernel32.LocalFree(description)
            return raw.decode("utf-8")
        if prefix == "test" and os.environ.get("MEDIA_GENERATION_TEST_MODE") == "1":
            if len(protected) < 48:
                raise ValueError("short test envelope")
            nonce, tag, cipher = protected[:16], protected[16:48], protected[48:]
            key = _secret_key()
            expected = hmac.new(key, b"test-reference-url-tag" + nonce + cipher, hashlib.sha256).digest()
            if not hmac.compare_digest(tag, expected):
                raise ValueError("test envelope tag mismatch")
            stream = bytearray()
            counter = 0
            while len(stream) < len(cipher):
                stream.extend(hmac.new(key, b"test-reference-url" + nonce + counter.to_bytes(4, "big"), hashlib.sha256).digest())
                counter += 1
            return bytes(a ^ b for a, b in zip(cipher, stream)).decode("utf-8")
    except MediaError:
        raise
    except Exception as exc:
        raise MediaError("媒体参考链接保护数据损坏；为避免错误提交已停止。", code="REFERENCE_PROTECTION_FAILED") from exc
    raise MediaError("无法读取当前媒体参考链接保护数据。", code="REFERENCE_PROTECTION_UNAVAILABLE")


def _protect_request_urls(request: dict[str, Any]) -> dict[str, Any]:
    protected = json.loads(_canonical(request).decode("utf-8"))
    for item in protected.get("reference_media", []):
        if item.get("source") == "url" and item.get("url"):
            item["url_protected"] = _protect_reference_url(item.pop("url"))
    return protected


def _restore_request_urls(request: dict[str, Any]) -> dict[str, Any]:
    restored = json.loads(_canonical(request).decode("utf-8"))
    for item in restored.get("reference_media", []):
        if item.get("source") == "url" and item.get("url_protected"):
            item["url"] = _unprotect_reference_url(item.pop("url_protected"))
    return restored


def _protect_artifact_urls(artifacts: list[dict[str, Any]]) -> list[dict[str, Any]]:
    protected = json.loads(_canonical(artifacts).decode("utf-8"))
    for item in protected:
        if item.get("url"):
            item["url_protected"] = _protect_reference_url(item.pop("url"))
    return protected


def _restore_artifact_urls(artifacts: list[dict[str, Any]]) -> list[dict[str, Any]]:
    restored = json.loads(_canonical(artifacts).decode("utf-8"))
    for item in restored:
        if item.get("url_protected"):
            item["url"] = _unprotect_reference_url(item.pop("url_protected"))
    return restored


def _public_artifacts(artifacts: list[dict[str, Any]]) -> list[dict[str, Any]]:
    public = []
    for artifact in artifacts:
        item = {key: value for key, value in artifact.items() if key not in {"url", "url_protected", "value"}}
        url = artifact.get("url")
        if url:
            item["url_origin"] = _url_origin(url)
        elif artifact.get("url_protected"):
            try:
                item["url_origin"] = _url_origin(_unprotect_reference_url(artifact["url_protected"]))
            except MediaError:
                item["url_origin"] = None
        public.append(item)
    return public


def _intent_conflicts(intent: str) -> list[dict[str, Any]]:
    folder = _data_root() / "confirmations"
    if not folder.exists():
        return []
    conflicts = []
    active_states = {"prepared", "submitting", "task_created", "in_progress", "submission_unknown", "completed_pending_save"}
    for path in folder.glob("media-*.json"):
        try:
            record = _load_json(path)
            state = str(record.get("state", ""))
            if not record.get("intent_fingerprint"):
                if state in {"submission_unknown", "submitting", "prepared", "task_created", "in_progress", "completed_pending_save"}:
                    raise MediaError("发现无法识别版本的活动媒体记录；为避免重复计费，不能判断其是否与当前请求相同。", code="UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
                continue
            if record.get("intent_fingerprint_key_id") != hashlib.sha256(_secret_key()).hexdigest()[:16]:
                raise MediaError("媒体任务索引密钥版本不匹配；为避免重复计费，已停止准备新任务。", code="UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
            expected_record = _token({key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")})
            stored_intent = str(record.get("intent_fingerprint", ""))
            normalized_intent = _verified_intent_fingerprint(record.get("request", {}), stored_intent)
            if not hmac.compare_digest(str(record.get("fingerprint", "")), expected_record) or normalized_intent is None:
                raise MediaError("媒体任务索引校验失败；为避免重复计费，已停止准备新任务。", code="UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
            if not hmac.compare_digest(normalized_intent, intent):
                continue
            if state == "submitting":
                if _confirmation_operation_is_active(str(record.get("confirmation_id", ""))):
                    raise MediaError("相同请求仍在发送或等待上游响应；请等待当前请求返回后再处理。", code="INTENT_SUBMISSION_IN_PROGRESS", provider=record.get("request", {}).get("provider"), model=record.get("request", {}).get("model"), submission_state="submitting", api_request_sent=record.get("api_request_sent"))
                if record.get("api_request_sent") is not False:
                    record["state"] = "submission_unknown"
                    record["recovered_at"] = int(time.time())
                    record["last_error"] = "Recovered interrupted submission; acceptance may have occurred."
                    _atomic_json(path, record)
                    state = "submission_unknown"
                else:
                    record["state"] = "interrupted_before_send"
                    _atomic_json(path, record)
                    state = "interrupted_before_send"
            elif state == "prepared" and int(record.get("expires_at", 0)) < int(time.time()):
                record["state"] = "expired"
                _atomic_json(path, record)
                state = "expired"
            if state in active_states:
                conflicts.append(record)
        except MediaError:
            raise
        except Exception as exc:
            raise MediaError("媒体任务索引包含损坏记录；为避免重复计费，已停止准备新任务。", code="UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE") from exc
    return conflicts


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise MediaError(f"无法读取本地媒体任务记录: {type(exc).__name__}") from exc
    if not isinstance(value, dict):
        raise MediaError("本地媒体任务记录格式无效。")
    return value


def _secret_key() -> bytes:
    path = _data_root() / ".confirmation-key"
    if not path.exists():
        records = _data_root() / "confirmations"
        if records.exists() and any(records.glob("media-*.json")):
            raise MediaError("本地媒体确认密钥缺失但已有任务记录；为避免重复计费风险，已停止操作。", code="UNRESOLVED_SUBMISSION_INDEX_UNAVAILABLE")
        candidate = os.urandom(32)
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(candidate)
        except FileExistsError:
            pass
    try:
        key = path.read_bytes()
    except OSError as exc:
        raise MediaError("无法读取本地确认密钥。") from exc
    if len(key) != 32:
        raise MediaError("本地确认密钥长度无效；为保护计费安全，拒绝继续。")
    return key


def _token(payload: dict[str, Any]) -> str:
    return hmac.new(_secret_key(), _canonical(payload), hashlib.sha256).hexdigest()


def _validate_base_url(value: str, *, for_download: bool = False) -> str:
    raw = str(value or "").strip().rstrip("/")
    parsed = urllib.parse.urlsplit(raw)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        if os.environ.get("MEDIA_GENERATION_TEST_MODE") == "1" and parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
            pass
        else:
            raise MediaError("媒体服务地址必须是无凭据、无查询参数的 HTTPS URL。")
    if parsed.path not in {"", "/"}:
        raise MediaError("MEDIA_GENERATION_API_BASE_URL 只填写域名根地址，不含 API 路径。")
    host = parsed.hostname or ""
    if host.casefold() == "localhost" or host.endswith(".localhost"):
        if os.environ.get("MEDIA_GENERATION_TEST_MODE") != "1":
            raise MediaError("拒绝连接本机或 localhost 媒体服务地址。")
    try:
        address = ipaddress.ip_address(host.strip("[]"))
        if not address.is_global and os.environ.get("MEDIA_GENERATION_TEST_MODE") != "1":
            raise MediaError("媒体服务地址必须指向公网 HTTPS 主机。")
    except ValueError:
        if os.environ.get("MEDIA_GENERATION_TEST_MODE") != "1":
            try:
                addresses = {ipaddress.ip_address(item[4][0]) for item in socket.getaddrinfo(host, parsed.port or 443, type=socket.SOCK_STREAM)}
            except OSError as exc:
                raise MediaError("媒体服务地址主机名解析失败。") from exc
            if not addresses or any(not address.is_global for address in addresses):
                raise MediaError("媒体服务地址必须解析到公网 IP。")
    return raw


def _validate_public_url(value: str, *, resolve: bool = False) -> str:
    raw = str(value or "").strip()
    parsed = urllib.parse.urlsplit(raw)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise MediaError("媒体 URL 必须是无凭据、无 fragment 的 HTTPS URL。")
    host = parsed.hostname.casefold()
    if host == "localhost" or host.endswith(".localhost") or host.endswith(".local"):
        raise MediaError("拒绝 localhost 或本地域名媒体 URL。")
    try:
        addresses = [ipaddress.ip_address(host.strip("[]"))]
    except ValueError:
        addresses = []
        if resolve:
            try:
                addresses = [ipaddress.ip_address(item[4][0]) for item in socket.getaddrinfo(host, parsed.port or 443, type=socket.SOCK_STREAM)]
            except OSError as exc:
                raise MediaError("媒体 URL 主机名解析失败。") from exc
    if any(not item.is_global for item in addresses):
        raise MediaError("拒绝解析到非公网地址的媒体 URL。")
    return raw


def _base() -> str:
    return _validate_base_url(os.environ.get("MEDIA_GENERATION_API_BASE_URL", ""))


def _api_profile() -> str:
    """Resolve the gateway route contract without guessing from model names."""
    configured = os.environ.get("MEDIA_GENERATION_API_PROFILE", "auto").strip().casefold() or "auto"
    if configured == "auto":
        host = urllib.parse.urlsplit(os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip()).hostname or ""
        if host.casefold() == "api.wokey.ai":
            return "wokey"
        if host.casefold() in {"404token.xyz", "api.404token.xyz"}:
            return "responses"
        return "legacy"
    if configured in {"legacy", "wokey", "responses"}:
        return configured
    raise MediaError("MEDIA_GENERATION_API_PROFILE 只允许 auto、legacy、wokey 或 responses。")


def _api_key() -> str:
    return os.environ.get("MEDIA_GENERATION_API_KEY", "").strip()


def _model(provider: str, requested: str = "") -> str:
    configured = _configured_model(provider)
    if provider == "minimax-video" and not configured:
        raise MediaError("统一媒体上游尚未配置 MiniMax 视频模型；没有发送 API 请求，也不会回退到 Grok 或 Seedance。")
    if requested and configured and requested.strip() != configured:
        raise MediaError("请求模型必须与当前 provider 明确配置的模型完全一致；请在连接设置中选择模型后刷新目录。", code="MODEL_NOT_CONFIGURED_FOR_PROVIDER", provider=provider, model=requested)
    if requested and not configured:
        raise MediaError("当前 provider 尚未配置模型；不能仅因模型出现在目录中就推断其媒体能力。", code="MODEL_NOT_CONFIGURED_FOR_PROVIDER", provider=provider, model=requested)
    model = str(requested or configured).strip()
    if not model:
        raise MediaError(f"未配置 {PROVIDERS[provider]['model_env']}；请填写服务方提供的模型名。")
    if len(model) > 256 or any(ord(char) < 32 for char in model):
        raise MediaError("模型名格式无效。")
    catalog = _catalog_snapshot()
    if model not in catalog["ids"]:
        raise MediaError("所选模型不在当前网关最近验证的模型列表中；请刷新列表并选择准确模型 ID。", code="MODEL_NOT_AVAILABLE", provider=provider, model=model)
    return model


def _image_mime_from_bytes(data: bytes) -> str:
    """Identify supported image containers from their bytes, not their filename."""
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if data.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if data.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    if len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    return "application/octet-stream"


def _safe_reference(item: Any) -> dict[str, Any]:
    if not isinstance(item, dict):
        raise MediaError("reference_media 的每一项都必须是对象。")
    kind = str(item.get("kind", "image")).strip().lower()
    if kind not in {"image", "video", "audio"}:
        raise MediaError("参考素材 kind 只允许 image、video 或 audio。")
    role = str(item.get("role", "")).strip().lower()
    if role and role not in {"first_frame", "reference"}:
        raise MediaError("Grok 视频图片 role 只允许 first_frame 或 reference。", code="INVALID_REFERENCE_ROLE")
    url = str(item.get("url", "")).strip()
    local_path = str(item.get("local_path", "")).strip()
    if bool(url) == bool(local_path):
        raise MediaError("每项参考素材必须且只能提供 url 或 local_path 之一。")
    if url:
        _validate_public_url(url)
        result = {"kind": kind, "source": "url", "url": url}
        if role:
            result["role"] = role
        return result
    path = Path(local_path).expanduser().resolve()
    if not path.is_file():
        raise MediaError(f"参考文件不存在或不是普通文件: {path.name}")
    raw = path.read_bytes()
    size = len(raw)
    if size <= 0 or size > MAX_REFERENCE_BYTES:
        raise MediaError(f"参考文件大小必须在 1 字节至 {MAX_REFERENCE_BYTES} 字节之间。")
    mime = _image_mime_from_bytes(raw[:16]) if kind == "image" else (mimetypes.guess_type(path.name)[0] or "application/octet-stream")
    allowed = IMAGE_TYPES if kind == "image" else VIDEO_TYPES if kind == "video" else AUDIO_TYPES
    if mime not in allowed:
        if kind == "image":
            raise MediaError("本地图片内容格式无法识别；请使用 PNG、JPEG、WebP 或 GIF 文件。")
        raise MediaError(f"参考文件 MIME 类型与 {kind} 不匹配。")
    digest = hashlib.sha256(raw).hexdigest()
    result = {"kind": kind, "source": "file", "path": str(path), "bytes": size, "sha256": digest, "mime_type": mime}
    if role:
        result["role"] = role
    return result


def _grok_video_dimensions(params: dict[str, Any], provider: str, model: str) -> dict[str, str]:
    missing = sorted(
        name for name in ("aspect_ratio", "resolution")
        if not str(params.get(name, "")).strip()
    )
    if missing:
        raise MediaError(
            "Grok video 必须在 parameters 中明确提供 aspect_ratio 和 resolution；请根据本次视频需求选择后再准备。",
            code="VIDEO_PARAMETERS_REQUIRED",
            provider=provider,
            model=model,
            details={
                "missing_parameters": missing,
                "aspect_ratio_options": ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
                "resolution_options": ["480p", "720p", "1080p"],
            },
        )
    clean = {
        "aspect_ratio": str(params["aspect_ratio"]).strip(),
        "resolution": str(params["resolution"]).strip(),
    }
    for name, allowed in (("aspect_ratio", {"1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"}), ("resolution", {"480p", "720p", "1080p"})):
        if clean[name] not in allowed:
            raise MediaError(f"Grok video 参数 {name} 不受支持。", code="INVALID_PARAMETER", provider=provider, model=model)
    return clean


def _validate_grok_video_local_image_sizes(reference_media: list[dict[str, Any]], provider: str, model: str) -> None:
    local_images = [
        item for item in reference_media
        if item.get("kind") == "image" and item.get("source") == "file"
    ]
    if any(int(item.get("bytes", 0)) > MAX_GROK_VIDEO_LOCAL_IMAGE_BYTES for item in local_images):
        raise MediaError(
            "Grok 视频每张本地图片不能超过 8 MiB；请压缩图片后重新准备。",
            code="REFERENCE_SIZE_LIMIT", provider=provider, model=model,
        )
    total_bytes = sum(int(item.get("bytes", 0)) for item in local_images)
    if total_bytes > MAX_GROK_VIDEO_TOTAL_LOCAL_IMAGE_BYTES:
        raise MediaError(
            "Grok 视频本地图片总大小不能超过 25 MiB。",
            code="REFERENCE_SIZE_LIMIT", provider=provider, model=model,
        )


def _validate_request(args: dict[str, Any]) -> dict[str, Any]:
    provider = str(args.get("provider", "")).strip().lower()
    if provider not in PROVIDERS:
        raise MediaError("provider 必须是 image-2、grok-image、grok-video、seedance-video 或 minimax-video。")
    prompt = str(args.get("prompt", ""))
    if not prompt.strip() or len(prompt) > 30000:
        raise MediaError("prompt 不能为空，且最多 30000 个字符。")
    model = _model(provider, str(args.get("model", "")))
    refs = args.get("reference_media", [])
    if not isinstance(refs, list):
        raise MediaError("reference_media 必须是数组。")
    max_references = 15 if provider == "grok-video" else 5 if provider == "grok-image" else 12
    if len(refs) > max_references:
        if provider == "grok-video":
            raise MediaError("Grok 视频 reference_media 最多 15 项（1 张首帧 + 14 张参考图）。")
        if provider == "grok-image":
            raise MediaError("Grok Imagine 图片编辑最多接受 5 张参考图。")
        raise MediaError("此 provider 的 reference_media 最多 12 项。")
    normalized_refs = [_safe_reference(item) for item in refs]
    params = args.get("parameters", {})
    if not isinstance(params, dict):
        raise MediaError("parameters 必须是对象。")
    allowed_parameters = {
        "image-2": {"size", "n"}, "grok-image": {"n", "aspect_ratio", "resolution", "quality"},
        "grok-video": {"duration", "aspect_ratio", "resolution", "generate_audio"},
        "seedance-video": {"duration", "ratio", "watermark", "generate_audio"},
        "minimax-video": {"duration", "ratio", "aspect_ratio", "resolution"},
    }[provider]
    extra_parameters = sorted(set(params) - allowed_parameters)
    if extra_parameters:
        raise MediaError(f"不支持的 {provider} 参数: {', '.join(extra_parameters[:8])}。", code="UNSUPPORTED_PARAMETER", provider=provider, model=model)
    request: dict[str, Any] = {
        "provider": provider,
        "model": model,
        "prompt": prompt,
        "reference_media": normalized_refs,
        "parameters": {},
        "route": _route_context(),
    }
    if provider != "grok-video" and any("role" in item for item in normalized_refs):
        raise MediaError("图片 role 仅适用于 Grok 视频；其他 provider 不接受该字段。", code="INVALID_REFERENCE_ROLE", provider=provider, model=model)
    if provider == "image-2":
        if any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Image-2 仅接受 image 参考素材。")
        size = str(params.get("size", "auto" if _api_profile() == "responses" else "2048x2048"))
        if size not in IMAGE_SIZES and size != "auto":
            raise MediaError("Image-2 size 不在已支持的 2K 尺寸清单中。")
        try:
            n = int(params.get("n", 1))
        except (TypeError, ValueError):
            raise MediaError("图片数量 n 必须是整数。", code="INVALID_PARAMETER", provider=provider, model=model)
        if not 1 <= n <= 8:
            raise MediaError("图片数量 n 必须在 1–8 之间。")
        if _api_profile() == "responses" and n != 1:
            raise MediaError("当前 Responses 图片适配只支持 n=1；多张需分别准备并确认。")
        if _api_profile() == "wokey" and n != 1:
            raise MediaError("Wokey Image API 当前只支持 n=1。")
        request["mode"] = "edit" if normalized_refs else "generate"
        request["parameters"] = {"size": size, "n": n}
        if normalized_refs and any(item["source"] == "url" for item in normalized_refs):
            raise MediaError("Image-2 图生图暂只接受本地参考图片；先将素材保存到本机后再准备。")
    elif provider == "grok-image":
        if any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Grok image 仅接受 image 参考素材。")
        if len(normalized_refs) > 5:
            raise MediaError("Grok Imagine 单次最多接受 5 张参考图。")
        try:
            n = int(params.get("n", 1))
        except (TypeError, ValueError):
            raise MediaError("图片数量 n 必须是整数。", code="INVALID_PARAMETER", provider=provider, model=model)
        if not 1 <= n <= 10:
            raise MediaError("图片数量 n 必须在 1–10 之间。")
        if _api_profile() == "wokey" and (n != 1 or normalized_refs):
            raise MediaError("Wokey Grok 图片适配目前仅支持单张文生图；参考图编辑暂不可用。")
        clean = {"n": n}
        for name, allowed in (("aspect_ratio", {"auto", "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "2:1", "1:2", "19.5:9", "9:19.5", "20:9", "9:20", "21:9", "5:2"}), ("resolution", {"1k", "2k"}), ("quality", {"low", "medium", "auto"})):
            value = str(params.get(name, "")).strip()
            if value:
                if value not in allowed:
                    raise MediaError(f"Grok image 参数 {name} 不受支持。")
                clean[name] = value
        request["mode"] = "edit" if normalized_refs else "generate"
        request["parameters"] = clean
    elif provider == "grok-video":
        if any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Grok video 只接受图片素材。", code="INVALID_REFERENCE_KIND", provider=provider, model=model)
        for item in normalized_refs:
            # Legacy clients omitted role and historically meant one pinned start frame.
            item["role"] = item.get("role", "first_frame")
        first_frames = [item for item in normalized_refs if item["role"] == "first_frame"]
        references = [item for item in normalized_refs if item["role"] == "reference"]
        if len(first_frames) > 1:
            raise MediaError("Grok 视频最多只能有一张首帧图；其他图片请显式标为 reference。", code="TOO_MANY_FIRST_FRAMES", provider=provider, model=model)
        _validate_grok_video_local_image_sizes(normalized_refs, provider, model)
        if _api_profile() == "wokey" and normalized_refs:
            raise MediaError("当前 Wokey 视频路由尚未认证图片/参考图协议；没有发送请求。", code="REFERENCE_MODE_UNSUPPORTED", provider=provider, model=model)
        if references:
            if _api_profile() != "responses":
                raise MediaError("当前媒体路由尚未认证 Grok reference_images；没有发送请求。", code="REFERENCE_MODE_UNSUPPORTED", provider=provider, model=model)
            reference_limits = {"grok-imagine-video-1.5": 14, "grok-imagine-video": 7}
            reference_limit = reference_limits.get(model)
            if reference_limit is None:
                raise MediaError("当前 Grok 视频模型没有已认证的独立参考图能力；没有发送请求。", code="REFERENCE_MODE_UNSUPPORTED", provider=provider, model=model)
            if len(references) > reference_limit:
                raise MediaError(f"当前模型最多接受 {reference_limit} 张参考图。", code="TOO_MANY_REFERENCES", provider=provider, model=model)
            if first_frames and model != "grok-imagine-video-1.5":
                raise MediaError("当前 Grok 视频模型不支持首帧与参考图混合；请明确选一种输入语义。", code="MIXED_REFERENCE_MODE_UNSUPPORTED", provider=provider, model=model)
        clean = _grok_video_dimensions(params, provider, model)
        if "duration" in params:
            try:
                duration = int(params["duration"])
            except (TypeError, ValueError):
                raise MediaError("Grok 视频时长必须是整数。", code="INVALID_PARAMETER", provider=provider, model=model)
            if not 1 <= duration <= 15:
                raise MediaError("Grok 视频时长必须为 1–15 秒整数。")
            clean["duration"] = duration
        if "generate_audio" in params:
            if not isinstance(params["generate_audio"], bool):
                raise MediaError("generate_audio 必须是布尔值。")
            clean["generate_audio"] = params["generate_audio"]
        if not normalized_refs:
            request["mode"] = "text-to-video"
        elif references and first_frames:
            request["mode"] = "image-to-video-with-references"
        elif references:
            request["mode"] = "reference-to-video"
        else:
            request["mode"] = "image-to-video"
        request["parameters"] = clean
    elif provider == "seedance-video":
        if _api_profile() == "wokey" and normalized_refs:
            raise MediaError("Wokey 视频参考素材需要 multipart；当前 MCP 适配暂只支持文生视频。")
        if any(item["source"] == "file" for item in normalized_refs):
            raise MediaError("Seedance 参考素材目前必须使用服务方可访问的 HTTPS URL；MCP 不会擅自上传素材。")
        try:
            duration = int(params.get("duration", 8))
        except (TypeError, ValueError):
            raise MediaError("Seedance 时长必须是整数。", code="INVALID_PARAMETER", provider=provider, model=model)
        if not 4 <= duration <= 15:
            raise MediaError("Seedance duration 当前校验为 4–15 秒整数；请以目标模型文档为准。")
        ratio = str(params.get("ratio", "16:9"))
        if ratio not in {"16:9", "9:16", "1:1", "adaptive"}:
            raise MediaError("Seedance ratio 只允许 16:9、9:16、1:1 或 adaptive。")
        clean = {"duration": duration, "ratio": ratio, "watermark": False, "generate_audio": True}
        for name in ("watermark", "generate_audio"):
            if name in params:
                if not isinstance(params[name], bool):
                    raise MediaError(f"{name} 必须是布尔值。")
                if name == "generate_audio" and params[name] is not True:
                    raise MediaError("当前 Seedance 网关契约要求生成音频；generate_audio 必须为 true。")
                clean[name] = params[name]
        # Preserve the previous adapter's request contract. The returned final
        # frame is useful for chaining another video-generation call.
        clean["return_last_frame"] = True
        request["mode"] = "video-generation"
        request["parameters"] = clean
    else:
        if _api_profile() != "wokey":
            raise MediaError("MiniMax 视频模型暂只支持统一上游的 Wokey 兼容模式；请勿尝试官方 MiniMax API 或其他猜测端点。")
        if normalized_refs:
            raise MediaError("当前统一上游的 MiniMax 视频槽位暂只支持文生视频。")
        try:
            duration = int(params.get("duration", 4))
        except (TypeError, ValueError):
            raise MediaError("MiniMax 视频 duration 必须是整数。", code="INVALID_PARAMETER", provider=provider, model=model)
        if not 1 <= duration <= 15:
            raise MediaError("MiniMax 视频 duration 必须是 1–15 秒整数；请遵循上游模型限制。")
        ratio = str(params.get("ratio", params.get("aspect_ratio", "16:9")))
        if ratio not in {"16:9", "9:16", "1:1", "4:3", "3:4", "3:2", "2:3", "adaptive"}:
            raise MediaError("MiniMax 视频 ratio 不受支持；请遵循上游模型限制。")
        resolution = str(params.get("resolution", "720p"))
        if resolution not in {"480p", "720p", "1080p"}:
            raise MediaError("MiniMax 视频 resolution 只允许 480p、720p 或 1080p；请遵循上游模型限制。")
        request["mode"] = "text-to-video"
        request["parameters"] = {"duration": duration, "ratio": ratio, "resolution": resolution}
    return request


def _confirmation_path(confirmation_id: str) -> Path:
    if not re.fullmatch(r"media-[0-9a-f]{32}", str(confirmation_id or "")):
        raise MediaError("confirmation_id 格式无效。")
    return _data_root() / "confirmations" / f"{confirmation_id}.json"


def _uses_windows_mutex() -> bool:
    return os.name == "nt"


@contextlib.contextmanager
def _confirmation_operation_lock(confirmation_id: str, operation: str):
    """Serialize submit/check/save across processes; OS releases lock on exit/crash."""
    lock_identity = hashlib.sha256(confirmation_id.encode("utf-8")).hexdigest()
    lock_path = _confirmation_path(confirmation_id).with_suffix(".operation-lock")
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    lock_key = str(lock_path.resolve())
    with _CONFIRMATION_LOCKS_GUARD:
        local_lock = _CONFIRMATION_LOCKS.setdefault(lock_key, threading.Lock())
    if not local_lock.acquire(blocking=False):
        code = "SUBMISSION_ALREADY_STARTED" if operation == "提交" else "OPERATION_IN_PROGRESS"
        raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。", code=code)
    if _uses_windows_mutex():
        import ctypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.CreateMutexW.argtypes = (ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p)
        kernel32.CreateMutexW.restype = ctypes.c_void_p
        kernel32.WaitForSingleObject.argtypes = (ctypes.c_void_p, ctypes.c_uint32)
        kernel32.WaitForSingleObject.restype = ctypes.c_uint32
        kernel32.ReleaseMutex.argtypes = (ctypes.c_void_p,)
        kernel32.ReleaseMutex.restype = ctypes.c_bool
        kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
        kernel32.CloseHandle.restype = ctypes.c_bool
        mutex_name = f"Local\\CodexMediaMcp-{lock_identity}"
        handle = kernel32.CreateMutexW(None, False, mutex_name)
        if not handle:
            local_lock.release()
            raise OSError(ctypes.get_last_error(), "CreateMutexW failed")
        acquired = False
        try:
            wait_result = kernel32.WaitForSingleObject(handle, 0)
            if wait_result in (0x00000000, 0x00000080):  # WAIT_OBJECT_0 / WAIT_ABANDONED
                acquired = True
            elif wait_result == 0x00000102:  # WAIT_TIMEOUT
                code = "SUBMISSION_ALREADY_STARTED" if operation == "提交" else "OPERATION_IN_PROGRESS"
                raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。", code=code)
            else:
                raise OSError(ctypes.get_last_error(), f"WaitForSingleObject failed: {wait_result:#x}")
            yield
        finally:
            try:
                if acquired:
                    kernel32.ReleaseMutex(handle)
            finally:
                kernel32.CloseHandle(handle)
                local_lock.release()
        return
    try:
        fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    except Exception:
        local_lock.release()
        raise
    acquired = False
    try:
        if os.fstat(fd).st_size == 0:
            os.write(fd, b"\0")
        os.lseek(fd, 0, os.SEEK_SET)
        try:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            acquired = True
        except OSError as exc:
            code = "SUBMISSION_ALREADY_STARTED" if operation == "提交" else "OPERATION_IN_PROGRESS"
            raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。", code=code) from exc
        yield
    finally:
        if acquired:
            try:
                os.lseek(fd, 0, os.SEEK_SET)
                import fcntl
                fcntl.flock(fd, fcntl.LOCK_UN)
            finally:
                os.close(fd)
        else:
            os.close(fd)
        local_lock.release()


def _confirmation_operation_is_active(confirmation_id: str) -> bool:
    """Check a confirmation's operation lock without waiting or changing state."""
    if not re.fullmatch(r"media-[0-9a-f]{32}", confirmation_id):
        return False
    lock_identity = hashlib.sha256(confirmation_id.encode("utf-8")).hexdigest()
    lock_path = _confirmation_path(confirmation_id).with_suffix(".operation-lock")
    with _CONFIRMATION_LOCKS_GUARD:
        local_lock = _CONFIRMATION_LOCKS.get(str(lock_path.resolve()))
        if local_lock is not None and local_lock.locked():
            return True
    if _uses_windows_mutex():
        import ctypes
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.CreateMutexW.argtypes = (ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p)
        kernel32.CreateMutexW.restype = ctypes.c_void_p
        kernel32.WaitForSingleObject.argtypes = (ctypes.c_void_p, ctypes.c_uint32)
        kernel32.WaitForSingleObject.restype = ctypes.c_uint32
        kernel32.ReleaseMutex.argtypes = (ctypes.c_void_p,)
        kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
        handle = kernel32.CreateMutexW(None, False, f"Local\\CodexMediaMcp-{lock_identity}")
        if not handle:
            raise MediaError("无法检查媒体任务互斥锁。", code="INTENT_LOCK_UNAVAILABLE")
        try:
            outcome = kernel32.WaitForSingleObject(handle, 0)
            if outcome in (0x00000000, 0x00000080):
                kernel32.ReleaseMutex(handle)
                return False
            if outcome == 0x00000102:
                return True
            raise MediaError("无法检查媒体任务互斥锁。", code="INTENT_LOCK_UNAVAILABLE")
        finally:
            kernel32.CloseHandle(handle)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
    try:
        import fcntl
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return True
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    finally:
        os.close(fd)


def media_prepare(args: dict[str, Any]) -> dict[str, Any]:
    request = _validate_request(args)
    persisted_request = _protect_request_urls(request)
    created = int(time.time())
    confirmation_id = "media-" + uuid.uuid4().hex
    catalog = _catalog_snapshot()
    payload = {"version": 1, "confirmation_id": confirmation_id, "created_at": created, "expires_at": created + CONFIRMATION_TTL_SECONDS, "request": persisted_request,
               "catalog_fingerprint": catalog["fingerprint"], "catalog_verified_at": catalog["verified_at"]}
    fingerprint = _token(payload)
    intent = _intent_fingerprint(request)
    key_id = hashlib.sha256(_secret_key()).hexdigest()[:16]
    ack_text = "确认此前任务可能已计费且仍要求新生成"
    with _intent_index_lock():
        conflicts = _intent_conflicts(intent)
        unknown = [item for item in conflicts if item.get("state") == "submission_unknown"]
        prepared = next((item for item in conflicts if item.get("state") == "prepared"), None)
        if prepared:
            raise MediaError("相同请求已有待确认任务；请继续使用该确认单，不要重复准备。", code="INTENT_ALREADY_PREPARED", provider=request["provider"], model=request["model"], submission_state="prepared", api_request_sent=False, details={"confirmation_id": prepared.get("confirmation_id"), "expires_at": prepared.get("expires_at")})
        if conflicts:
            prior = conflicts[0]
            allow_repeat = bool(unknown) and all(not item.get("repeat_ack_consumed") for item in unknown) and str(args.get("repeat_after_unknown_ack", "")).strip() == ack_text
            if not allow_repeat:
                raise MediaError("相同请求存在未解决的上游任务；请求可能已计费。先核对上游；如确需再次生成，必须明确确认可能重复计费。", code="UNRESOLVED_PRIOR_SUBMISSION", provider=request["provider"], model=request["model"], submission_state=prior.get("state"), api_request_sent=prior.get("api_request_sent"), details={"confirmation_id": prior.get("confirmation_id"), "submission_id": prior.get("submission_id"), "task_id": prior.get("task_id"), "created_at": prior.get("created_at"), "may_have_been_billed": True})
            payload["supersedes_unknown_submission_ids"] = [str(item.get("submission_id") or item["confirmation_id"]) for item in unknown]
            payload["repeat_after_unknown_acknowledged"] = True
            payload["acknowledged_at"] = created
        record = dict(payload, fingerprint=fingerprint, intent_fingerprint=intent,
                      intent_fingerprint_key_id=key_id, fingerprint_key_id=key_id,
                      submission_id=uuid.uuid4().hex, request_digest_hmac=_token(request),
                      idempotency_forwarded=False, state="prepared", api_request_sent=False)
        _atomic_json(_confirmation_path(confirmation_id), record)
        if payload.get("repeat_after_unknown_acknowledged"):
            for previous in unknown:
                previous["repeat_ack_consumed"] = True
                previous["repeat_ack_consumed_at"] = created
                _atomic_json(_confirmation_path(str(previous["confirmation_id"])), previous)
    refs = [{"kind": item["kind"], "role": item.get("role"), "source": item["source"], "url_origin": _url_origin(item["url"]) if item.get("url") else None, "file_name": Path(item["path"]).name if item.get("path") else None, "bytes": item.get("bytes"), "sha256": item.get("sha256")} for item in request["reference_media"]]
    return {
        "success": True,
        "mode": "prepared",
        "api_request_sent": False,
        "confirmation_id": confirmation_id,
        "submission_id": record["submission_id"],
        "confirmation_fingerprint": fingerprint,
        "expires_at_epoch": payload["expires_at"],
        "provider": request["provider"],
        "model": request["model"],
        "operation": request["mode"],
        "prompt": request["prompt"],
        "reference_media": refs,
        "parameters": request["parameters"],
        "price_estimate": "unknown; query the configured provider's current pricing before approval",
        "url_content_note": "HTTPS reference URLs are locked as strings; their remote bytes can change before submission.",
        "confirmation_required": "show the exact request and cost uncertainty to the user; submit only after their explicit approval",
    }


def _validated_record(args: dict[str, Any], *, allow_submitting: bool = False) -> tuple[Path, dict[str, Any]]:
    confirmation_id = str(args.get("confirmation_id", "")).strip()
    path = _confirmation_path(confirmation_id)
    record = _load_json(path)
    fingerprint = str(args.get("confirmation_fingerprint", "")).strip()
    expected = _token({key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request", "catalog_fingerprint", "catalog_verified_at")})
    if not fingerprint or not hmac.compare_digest(fingerprint, expected) or not hmac.compare_digest(str(record.get("fingerprint", "")), expected):
        raise MediaError("确认指纹不匹配；拒绝继续。")
    if not allow_submitting and int(record.get("expires_at", 0)) < int(time.time()):
        raise MediaError("确认清单已过期，请重新准备并再次确认。")
    route = record.get("request", {}).get("route")
    if route is not None and route != _route_context():
        raise MediaError("网关或路由已变化，拒绝改变原确认请求；请恢复原配置。")
    if route is None and not allow_submitting:
        raise MediaError("旧版确认未绑定网关路由；请重新准备并确认。")
    if record.get("state") == "submitting":
        # A crash can happen after the durable intent marker and before the
        # response/task_id is saved. Fail closed and never repeat a possibly
        # billed request.
        if record.get("api_request_sent") is False:
            record["state"] = "interrupted_before_send"
            record["submission_state"] = "interrupted_before_send"
        else:
            record["state"] = "submission_unknown"
            record["last_error"] = "Process ended while submission may have reached the upstream."
            record["recovered_at"] = int(time.time())
        _atomic_json(path, record)
    if record.get("operation_state") == "checking":
        record["operation_state"] = ""
        record["operation_token"] = ""
        _atomic_json(path, record)
    elif record.get("operation_state") == "saving":
        record["operation_state"] = ""
        record["operation_token"] = ""
        if record.get("state") in {"completed", "completed_pending_save"}:
            record["state"] = "completed_pending_save"
        output = _data_root() / "outputs" / str(record.get("confirmation_id", ""))
        if output.exists():
            for partial in output.glob("*.part"):
                partial.unlink(missing_ok=True)
        _atomic_json(path, record)
    allowed_states = {"prepared"}
    if allow_submitting:
        allowed_states |= {"submitting", "submission_unknown", "submission_rejected", "task_created", "in_progress", "provider_status_unknown", "completed", "completed_pending_save", "completed_saved", "completed_missing_artifacts", "completed_without_artifact", "failed", "interrupted_before_send"}
    if record.get("state") not in allowed_states:
        raise MediaError(f"该确认清单已进入 {record.get('state')} 状态，不能重复提交。")
    # Once submitted, retrieval belongs to the upstream task and must survive
    # deletion of the original local reference. Submission remains strict.
    for item in ([] if allow_submitting else record["request"].get("reference_media", [])):
        if item.get("source") == "file":
            path_value = Path(item["path"])
            if not path_value.is_file() or path_value.stat().st_size != int(item["bytes"]) or hashlib.sha256(path_value.read_bytes()).hexdigest() != item["sha256"]:
                raise MediaError("已确认的本地参考素材发生变化或不可用；请重新准备并确认。")
    return path, record


def _endpoint(path: str) -> str:
    base = _base()
    if not path.startswith("/") or ".." in path.split("/"):
        raise MediaError("内部 API 路径无效。")
    return base + path


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise MediaError("媒体服务 API 返回重定向；为避免泄露凭据，已拒绝跟随。")


def _http_json(method: str, path: str, payload: dict[str, Any] | bytes | None, provider: str, idempotency_key: str | None = None, content_type: str = "application/json", before_request: Callable[[], None] | None = None, sensitive_texts: tuple[str, ...] = ()) -> dict[str, Any]:
    key = _api_key()
    if not key:
        raise MediaError("未配置 MEDIA_GENERATION_API_KEY；没有发送 API 请求。")
    url = _endpoint(path)
    headers = {"Accept": "application/json", "Authorization": f"Bearer {key}"}
    if payload is not None:
        headers["Content-Type"] = content_type
    profile = _api_profile()
    if provider == "image-2" and profile not in {"wokey", "responses"}:
        headers["X-Async"] = "true"
        agent = os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip()
        if not agent:
            raise MediaError("Image-2 需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
        headers["agent-id"] = agent
    if provider == "seedance-video" and profile != "wokey":
        agent = os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip()
        if not agent:
            raise MediaError("Seedance 网关需要 MEDIA_GENERATION_AGENT_ID。")
        headers["agent-id"] = agent
    # The local idempotency key is persisted for correlation only. Do not emit an
    # header named `Idempotency-Key`: no provider in the current kit has a verified
    # upstream idempotency contract, so retries remain forbidden after uncertainty.
    # The local single-use state machine is not an upstream duplicate-charge guarantee.
    data = _canonical(payload) if isinstance(payload, dict) else payload
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    opener = urllib.request.build_opener(_NoRedirect)
    try:
        if before_request is not None:
            before_request()
        with opener.open(request, timeout=HTTP_TIMEOUT_SECONDS) as response:
            raw = response.read(MAX_JSON_BYTES + 1)
            if len(raw) > MAX_JSON_BYTES:
                raise MediaError("媒体服务 JSON 响应超过大小上限。")
            parsed = json.loads(raw.decode("utf-8-sig"))
    except urllib.error.HTTPError as exc:
        try:
            provider_body = json.loads(exc.read(8192).decode("utf-8", errors="replace"))
        except Exception:
            provider_body = {}
        provider_error = provider_body.get("error", {}) if isinstance(provider_body, dict) else {}
        provider_code = str(provider_error.get("code", "")).casefold() if isinstance(provider_error, dict) else ""
        remote_detail = _safe_upstream_error_detail(provider_body, payload, api_key=key, sensitive_texts=sensitive_texts)
        headers_obj = exc.headers or {}
        provider_request_id = next((_safe_identifier(headers_obj.get(name)) for name in ("x-request-id", "request-id", "x-amzn-requestid") if _safe_identifier(headers_obj.get(name))), None)
        gateway_request_id = next((_safe_identifier(headers_obj.get(name)) for name in ("x-gateway-request-id", "x-wokey-request-id") if _safe_identifier(headers_obj.get(name))), None)
        if method == "POST" and (exc.code >= 500 or exc.code in {408, 425, 429}):
            message = f"上游 HTTP {exc.code}，提交结果未知；请勿重新提交。"
            if remote_detail:
                message += f" 服务端诊断：{remote_detail}"
            raise MediaError(message, code="SUBMISSION_UNKNOWN", provider=provider, http_status=exc.code, provider_request_id=provider_request_id, gateway_request_id=gateway_request_id, submission_state="submission_unknown", api_request_sent=True) from exc
        if method != "POST" and exc.code >= 500:
            raise MediaError(f"上游暂时无法响应（HTTP {exc.code}）；可稍后查询同一任务。", code="UPSTREAM_UNAVAILABLE", provider=provider, http_status=exc.code, provider_request_id=provider_request_id, gateway_request_id=gateway_request_id, retryable=True, api_request_sent=False) from exc
        mapping = {401: ("UPSTREAM_AUTH_FAILED", "上游认证失败。"), 403: ("UPSTREAM_AUTH_FAILED", "上游拒绝当前账户或能力权限。"), 404: ("UPSTREAM_MODEL_NOT_FOUND", "上游未找到请求的模型或接口。"), 400: ("UPSTREAM_INPUT_REJECTED", "上游拒绝请求参数或内容。"), 422: ("UPSTREAM_INPUT_REJECTED", "上游拒绝请求参数或内容。")}
        code, safe_message = mapping.get(exc.code, ("UPSTREAM_REQUEST_REJECTED", f"上游拒绝请求（HTTP {exc.code}）。"))
        if provider_code in {"content_policy_violation", "safety_violation", "content_policy"}:
            code, safe_message = "UPSTREAM_CONTENT_POLICY", "上游内容政策拒绝了本次请求。"
        if remote_detail:
            safe_message += f" 服务端诊断：{remote_detail}"
        raise MediaError(safe_message, code=code, provider=provider, http_status=exc.code, provider_request_id=provider_request_id, gateway_request_id=gateway_request_id, submission_state="submission_rejected" if method == "POST" else None, api_request_sent=method == "POST") from exc
    except MediaError:
        raise
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as exc:
        if method == "POST":
            raise MediaError("提交状态未知：连接在提交后中断，无法确认上游是否接受；请勿重新提交。", code="SUBMISSION_UNKNOWN", provider=provider, submission_state="submission_unknown", api_request_sent=True) from exc
        raise MediaError("媒体服务暂时无法响应；原任务状态未更改。", code="UPSTREAM_UNAVAILABLE", provider=provider, retryable=True, api_request_sent=False) from exc
    if not isinstance(parsed, dict):
        raise MediaError("媒体服务返回了非对象 JSON。")
    return parsed


def _safe_upstream_error_detail(error_body: Any, request_payload: dict[str, Any] | bytes | None, *, api_key: str = "", sensitive_texts: tuple[str, ...] = ()) -> str | None:
    """Return a short upstream error reason without echoing credentials or request text."""
    if not isinstance(error_body, dict):
        return None
    error = error_body.get("error")
    candidates = []
    if isinstance(error, dict):
        candidates.append(error.get("message"))
    elif isinstance(error, str):
        candidates.append(error)
    candidates.extend((error_body.get("message"), error_body.get("detail")))
    message = next((value.strip() for value in candidates if isinstance(value, str) and value.strip()), None)
    if not message:
        return None

    private_texts: set[str] = {value for value in sensitive_texts if isinstance(value, str) and value}
    def collect_private_text(value: Any) -> None:
        if isinstance(value, dict):
            for key, item in value.items():
                if str(key).casefold() in {"prompt", "text"} and isinstance(item, str) and item:
                    private_texts.add(item)
                else:
                    collect_private_text(item)
        elif isinstance(value, list):
            for item in value:
                collect_private_text(item)
    if isinstance(request_payload, dict):
        collect_private_text(request_payload)
    if api_key:
        message = message.replace(api_key, "[API key redacted]")
    for private_text in sorted(private_texts, key=len, reverse=True):
        message = message.replace(private_text, "[request text omitted]")

    message = re.sub(r"(?i)\bBearer\s+[^\s,;]+", "Bearer [REDACTED]", message)
    message = re.sub(r"(?i)\b(?:sk|rk|pk)-[A-Za-z0-9._-]{8,}", "[REDACTED]", message)
    message = re.sub(r"(?i)\b(?:signed[-_ ]?query|query)[-_=][^\s,;]+", "[query redacted]", message)
    message = re.sub(r"(?i)\b(api[_-]?key|authorization|access[_-]?token|secret)\s*[:=]\s*[^\s,;]+", r"\1=[REDACTED]", message)
    message = re.sub(r"(?i)data:[^\s,;]+;base64,[A-Za-z0-9+/=_-]+", "[media data omitted]", message)
    message = re.sub(r"https?://[^\s\"'<>]+", "[URL omitted]", message)
    message = " ".join("".join(" " if unicodedata.category(char) == "Cc" else char for char in message).split())
    if not message:
        return None
    return message[:200].rstrip() + ("…" if len(message) > 200 else "")


def _multipart_image_request(record: dict[str, Any], idempotency_key: str, before_request: Callable[[], None] | None = None) -> dict[str, Any]:
    request = record["request"]
    boundary = "----CodexMedia" + uuid.uuid4().hex
    chunks: list[bytes] = []
    fields = {"model": request["model"], "prompt": request["prompt"], "size": request["parameters"]["size"]}
    for name, value in fields.items():
        chunks.extend([f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n".encode("utf-8")])
    for index, reference in enumerate(request["reference_media"]):
        file_path = Path(reference["path"])
        raw = file_path.read_bytes()
        if len(raw) > MAX_REFERENCE_BYTES or hashlib.sha256(raw).hexdigest() != reference["sha256"]:
            raise MediaError("Image-2 参考图片在提交前改变；请重新准备并确认。")
        mime = reference["mime_type"]
        safe_name = re.sub(r"[^A-Za-z0-9._-]", "_", file_path.name)[:100] or f"reference-{index}.img"
        chunks.append(f"--{boundary}\r\nContent-Disposition: form-data; name=\"image[{index}]\"; filename=\"{safe_name}\"\r\nContent-Type: {mime}\r\n\r\n".encode("utf-8") + raw + b"\r\n")
    chunks.append(f"--{boundary}--\r\n".encode("ascii"))
    url_path = "/api/llm/openai/v1/images/edits"
    return _http_json("POST", url_path, b"".join(chunks), "image-2", idempotency_key, f"multipart/form-data; boundary={boundary}", before_request, sensitive_texts=(str(request.get("prompt", "")),))


def _request_payload(request: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    provider = request["provider"]
    params = request["parameters"]
    if provider == "image-2" and _api_profile() == "responses":
        content: list[dict[str, Any]] = [{"type": "input_text", "text": request["prompt"]}]
        for reference in request["reference_media"]:
            path = Path(reference["path"])
            raw = path.read_bytes()
            if len(raw) > MAX_REFERENCE_BYTES or hashlib.sha256(raw).hexdigest() != reference["sha256"]:
                raise MediaError("Image-2 参考图片在提交前改变；请重新准备并确认。")
            content.append({
                "type": "input_image",
                "image_url": f"data:{reference['mime_type']};base64," + base64.b64encode(raw).decode("ascii"),
            })
        payload = {
            "model": request["model"],
            "input": [{"role": "user", "content": content}],
            "tools": [{"type": "image_generation"}],
            "tool_choice": {"type": "image_generation"},
        }
        if params.get("size") not in {None, "auto"}:
            payload["tools"][0]["size"] = params["size"]
        return "/v1/responses", payload
    if _api_profile() == "wokey":
        if provider == "image-2":
            return "/v1/images/generations", {"model": request["model"], "prompt": request["prompt"], "size": params["size"], "n": 1}
        if provider == "grok-image":
            return "/v1/images/generations", {"model": request["model"], "prompt": request["prompt"], "n": 1}
        if provider == "grok-video":
            dimensions = _grok_video_dimensions(params, provider, request["model"])
            body = {"model": request["model"], "prompt": request["prompt"], "ratio": dimensions["aspect_ratio"], "video_resolution": dimensions["resolution"]}
            if "duration" in params:
                body["duration_seconds"] = int(params["duration"])
            mode = "image_to_video" if request["reference_media"] else "text_to_video"
            body["mode"] = mode
            return "/v1/videos", body
        duration = int(params.get("duration", 4))
        ratio = str(params.get("aspect_ratio", params.get("ratio", "16:9")))
        resolution = str(params.get("resolution", "720p" if provider in {"seedance-video", "minimax-video"} else "480p"))
        mode = "image_to_video" if request["reference_media"] else "text_to_video"
        return "/v1/videos", {
            "model": request["model"],
            "prompt": request["prompt"],
            "duration_seconds": duration,
            "ratio": ratio,
            "video_resolution": resolution,
            "mode": mode,
        }
    if provider == "image-2":
        path = "/api/llm/openai/v1/images/edits" if request["mode"] == "edit" else "/api/llm/openai/v1/images/generations"
        return path, {"model": request["model"], "prompt": request["prompt"], "size": params["size"], "n": params["n"]}
    if provider == "grok-image":
        if request["mode"] == "edit":
            images = []
            for item in request["reference_media"]:
                if item["source"] == "url":
                    image_url = item["url"]
                else:
                    path = Path(item["path"])
                    data = path.read_bytes()
                    if len(data) > MAX_REFERENCE_BYTES or hashlib.sha256(data).hexdigest() != item["sha256"]:
                        raise MediaError("Grok 参考图片在提交前改变；请重新准备并确认。")
                    image_url = f"data:{item['mime_type']};base64," + base64.b64encode(data).decode("ascii")
                images.append({"type": "image_url", "url": image_url})
            body = {"model": request["model"], "prompt": request["prompt"], **params}
            if len(images) == 1:
                body["image"] = images[0]
            else:
                body["images"] = images
            return "/v1/images/edits", body
        return "/v1/images/generations", {"model": request["model"], "prompt": request["prompt"], **params}
    if provider == "minimax-video":
        if _api_profile() != "wokey":
            raise MediaError("MiniMax 视频暂只支持统一上游的 Wokey 兼容模式；没有发送 API 请求。")
        raise MediaError("MiniMax 视频路由尚未配置模型；没有发送 API 请求。")
    if provider == "grok-video":
        body = {"model": request["model"], "prompt": request["prompt"], **params}
        first_frame = next((item for item in request["reference_media"] if item.get("role", "first_frame") == "first_frame"), None)
        reference_images = [item for item in request["reference_media"] if item.get("role", "first_frame") == "reference"]

        def image_url(item: dict[str, Any]) -> str:
            if item["source"] == "file":
                path = Path(item["path"])
                raw = path.read_bytes()
                if len(raw) > MAX_REFERENCE_BYTES or hashlib.sha256(raw).hexdigest() != item["sha256"]:
                    raise MediaError("Grok 视频本地图片在提交前改变；请重新准备并确认。")
                return f"data:{item['mime_type']};base64," + base64.b64encode(raw).decode("ascii")
            return item["url"]

        if first_frame:
            body["image"] = {"url": image_url(first_frame)}
        if reference_images:
            body["reference_images"] = [{"url": image_url(item)} for item in reference_images]
        return "/v1/videos/generations", body
    content = [{"type": "text", "text": request["prompt"]}]
    role_by_kind = {"image": "reference_image", "video": "reference_video", "audio": "reference_audio"}
    for item in request["reference_media"]:
        if item["kind"] == "image":
            content.append({"type": "image_url", "image_url": {"url": item["url"]}, "role": role_by_kind[item["kind"]]})
        elif item["kind"] == "video":
            content.append({"type": "video_url", "video_url": {"url": item["url"]}, "role": role_by_kind[item["kind"]]})
        else:
            content.append({"type": "audio_url", "audio_url": {"url": item["url"]}, "role": role_by_kind[item["kind"]]})
    body = {"model": request["model"], "content": content, "duration": request["parameters"]["duration"], "ratio": request["parameters"]["ratio"]}
    for name in ("watermark", "generate_audio", "return_last_frame"):
        if name in request["parameters"]:
            body[name] = request["parameters"][name]
    return "/api/llm/doubao/contents/generations/tasks", body


def _extract_task_id(provider: str, response: dict[str, Any]) -> str:
    task_id = response.get("request_id") if provider == "grok-video" else None
    task_id = task_id or response.get("task_id") or response.get("id")
    if not isinstance(task_id, str) or not task_id.strip() or len(task_id) > 512 or any(ord(char) < 32 for char in task_id):
        raise MediaError("服务响应未包含有效 task_id；请求结果不确定，请勿直接重新提交。", code="SUBMISSION_UNKNOWN", submission_state="submission_unknown", api_request_sent=True)
    return task_id.strip()


def _base64_image_mime(value: str) -> str:
    """Infer the image container from a bounded base64 prefix, never from the provider name."""
    if not isinstance(value, str) or not value:
        return "application/octet-stream"
    try:
        signature = base64.b64decode(value[:64], validate=True)
    except (ValueError, TypeError):
        return "application/octet-stream"
    if signature.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if signature.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    return "application/octet-stream"


def _artifacts(response: Any, provider: str | None = None, *, output_scope: bool = False) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    if isinstance(response, dict):
        if response.get("type") == "image_generation_call":
            result = response.get("result")
            if isinstance(result, str) and result:
                found.append({"kind": "base64", "value": result, "mime_type": _base64_image_mime(result)})
        for key, value in response.items():
            is_output = key in {"video_url", "content_url", "b64_json"} or (output_scope and key == "url")
            if is_output and isinstance(value, str) and value:
                if key == "b64_json":
                    found.append({"kind": "base64", "value": value, "mime_type": _base64_image_mime(value)})
                elif key == "content_url" and value.startswith("/") and (
                    _api_profile() == "wokey" or (provider and PROVIDERS[provider]["family"] == "video")
                ):
                    found.append({"kind": "video", "url": _endpoint(value)})
                elif value.startswith("https://"):
                    kind = "video" if key in {"video_url", "content_url"} or (provider and PROVIDERS[provider]["family"] == "video") else "image"
                    found.append({"kind": kind, "url": value})
            elif key in {"output", "data", "result", "video", "image", "images", "content"} and isinstance(value, (dict, list)):
                found.extend(_artifacts(value, provider, output_scope=True))
    elif isinstance(response, list):
        for item in response:
            found.extend(_artifacts(item, provider, output_scope=output_scope))
    # Keep order but don't duplicate a provider's same URL echoed in nested fields.
    unique: list[dict[str, Any]] = []
    seen = set()
    for item in found:
        marker = item.get("url") or hashlib.sha256(str(item.get("value", "")).encode()).hexdigest()
        if marker not in seen:
            seen.add(marker)
            unique.append(item)
    return unique


def _response_summary(response: dict[str, Any], artifacts: list[dict[str, Any]]) -> dict[str, Any]:
    """Persist only bounded metadata; never retain provider payloads/base64 blobs."""
    status = response.get("status", response.get("task_status"))
    task_id = response.get("task_id", response.get("id"))
    request_id = response.get("request_id")
    encoded = _canonical(response)
    return {
        "status": str(status)[:80] if status is not None else None,
        "task_id": _safe_identifier(task_id),
        "request_id": _safe_identifier(request_id),
        "response_bytes": len(encoded),
        "response_sha256": hashlib.sha256(encoded).hexdigest(),
        "top_level_keys": sorted(str(key)[:80] for key in response)[:100],
        "artifact_count": len(artifacts),
    }


def media_submit(args: dict[str, Any]) -> dict[str, Any]:
    # Share one OS-backed lock with check/save to prevent stale-record overwrites.
    confirmation_id = str(args.get("confirmation_id", ""))
    with _confirmation_operation_lock(confirmation_id, "提交"):
        return _media_submit_locked(args)


def _media_submit_locked(args: dict[str, Any]) -> dict[str, Any]:
    path, record = _validated_record(args)
    if record.get("state") != "prepared" or record.get("operation_token"):
        raise MediaError("该确认单已开始处理；请勿重复提交。", code="SUBMISSION_ALREADY_STARTED", provider=record.get("request", {}).get("provider"), model=record.get("request", {}).get("model"), submission_state=record.get("state"), api_request_sent=record.get("api_request_sent"))
    if str(args.get("user_confirmation", "")).strip() not in {"确认", "就按这份生成"}:
        raise MediaError("正式提交只接受用户已明确回复“确认”或“就按这份生成”的执行调用。")
    request = _restore_request_urls(record["request"])
    provider = request["provider"]
    if provider == "grok-video":
        # Recheck at submit time as well: confirmations created by an older
        # MCP release can remain valid during the confirmation TTL.
        _validate_grok_video_local_image_sizes(request.get("reference_media", []), provider, request.get("model", ""))
        try:
            _grok_video_dimensions(request.get("parameters", {}), provider, request.get("model", ""))
        except MediaError as exc:
            raise MediaError(
                "这份 Grok 视频确认单缺少有效画幅或分辨率，不能继续提交。请重新准备请求并明确选择这两个参数。",
                code="CONFIRMATION_STALE",
                provider=provider,
                model=request.get("model"),
                api_request_sent=False,
                details=exc.details,
            ) from exc
    _base()
    if not _api_key():
        raise MediaError("未配置 MEDIA_GENERATION_API_KEY；没有发送 API 请求。")
    model = _model(provider, request["model"])
    catalog = _catalog_snapshot()
    if (catalog["fingerprint"] != record.get("catalog_fingerprint") or model not in catalog["ids"]):
        raise MediaError("模型目录在确认后发生变化；请重新发现模型并再次确认。", code="CONFIRMATION_STALE", provider=provider, model=model)
    if _api_profile() not in {"wokey", "responses"} and provider == "image-2" and not os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip():
        raise MediaError("当前图像网关需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
    if _api_profile() != "wokey" and provider == "seedance-video" and not os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip():
        raise MediaError("当前网关此 provider 需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
    for reference in request.get("reference_media", []):
        if reference.get("source") == "url":
            _validate_public_url(reference["url"], resolve=True)
    # Intent scanning/recovery and the submitting transition use the same
    # cross-process lock. The operation token is checked again immediately
    # before the first network byte is sent.
    with _intent_index_lock():
        path, record = _validated_record(args)
        if record.get("state") != "prepared" or record.get("operation_token"):
            raise MediaError("该确认单已开始处理；请勿重复提交。", code="SUBMISSION_ALREADY_STARTED", provider=provider, model=model, submission_state=record.get("state"), api_request_sent=record.get("api_request_sent"))
        if not hmac.compare_digest(str(record.get("request_digest_hmac", "")), _token(request)):
            raise MediaError("确认单请求摘要不匹配；拒绝提交。", code="CONFIRMATION_INTEGRITY_FAILED")
        stored_intent = str(record.get("intent_fingerprint", ""))
        if not stored_intent or record.get("intent_fingerprint_key_id") != hashlib.sha256(_secret_key()).hexdigest()[:16]:
            raise MediaError("该确认单没有可验证的唯一意图预留；为避免重复计费，拒绝提交旧确认单。", code="INTENT_RESERVATION_UNAVAILABLE", provider=provider, model=model)
        intent = _verified_intent_fingerprint(record.get("request", {}), stored_intent)
        if intent is None:
            raise MediaError("该确认单的唯一意图摘要无法验证；为避免重复计费，拒绝提交。", code="INTENT_RESERVATION_UNAVAILABLE", provider=provider, model=model)
        conflicts = _intent_conflicts(intent)
        own = [item for item in conflicts if item.get("confirmation_id") == record.get("confirmation_id")]
        foreign = [item for item in conflicts if item.get("confirmation_id") != record.get("confirmation_id")]
        if not own:
            raise MediaError("该确认单已不再持有有效的意图预留；拒绝提交。", code="INTENT_RESERVATION_LOST", provider=provider, model=model)
        if foreign:
            allowed_unknowns = {str(value) for value in record.get("supersedes_unknown_submission_ids", [])}
            foreign_ids = {str(item.get("submission_id") or item.get("confirmation_id")) for item in foreign}
            if all(item.get("state") == "prepared" for item in foreign):
                # Older versions could leave multiple prepared confirmations
                # for one intent. The explicitly submitted one wins; retire
                # the others before any request can leave this process.
                for item in foreign:
                    item["state"] = "superseded"
                    item["superseded_by_confirmation_id"] = record["confirmation_id"]
                    item["superseded_at"] = int(time.time())
                    _atomic_json(_confirmation_path(str(item["confirmation_id"])), item)
            else:
                repeat_ack_valid = bool(record.get("repeat_after_unknown_acknowledged")) and all(
                    item.get("state") == "submission_unknown"
                    and item.get("repeat_ack_consumed") is True
                    and str(item.get("submission_id") or item.get("confirmation_id")) in allowed_unknowns
                    for item in foreign
                )
                if not repeat_ack_valid or not foreign_ids.issubset(allowed_unknowns):
                    raise MediaError("相同请求存在另一份活动确认单；为避免重复计费，本确认单不能提交。", code="INTENT_RESERVATION_CONFLICT", provider=provider, model=model, submission_state="prepared", api_request_sent=False)
        record["state"] = "submitting"
        record["operation_state"] = "submitting"
        record["operation_token"] = uuid.uuid4().hex
        operation_token = record["operation_token"]
        record["submitted_at"] = int(time.time())
        record["api_request_sent"] = False
        _atomic_json(path, record)
    idempotency_key = hmac.new(_secret_key(), record["submission_id"].encode("utf-8"), hashlib.sha256).hexdigest()
    record["idempotency_key_sha256"] = hashlib.sha256(idempotency_key.encode()).hexdigest()
    record["idempotency_forwarded"] = False
    _atomic_json(path, record)

    def mark_request_started() -> None:
        # Conservative durable marker: once this is written, no failure may
        # cause an automatic retry, even if the socket failed before send.
        with _intent_index_lock():
            current = _load_json(path)
            if current.get("state") != "submitting" or current.get("operation_token") != operation_token or current.get("api_request_sent") is not False:
                raise MediaError("提交状态已被恢复或改变；未发送网络请求。", code="SUBMISSION_CANCELLED_BEFORE_SEND", provider=provider, model=model, submission_state=current.get("state"), api_request_sent=current.get("api_request_sent"))
            current["api_request_sent"] = True
            current["submission_state"] = "request_started"
            current["request_started_at"] = int(time.time())
            _atomic_json(path, current)
            record.update(current)

    try:
        if provider == "image-2" and request["mode"] == "edit" and _api_profile() == "legacy":
            response = _multipart_image_request(dict(record, request=request), idempotency_key, mark_request_started)
        elif provider == "image-2" and request["mode"] == "edit" and _api_profile() == "wokey":
            raise MediaError("当前 Wokey 图片编辑尚未验证契约；没有发送 API 请求。")
        else:
            api_path, body = _request_payload(request)
            response = _http_json("POST", api_path, body, provider, idempotency_key, before_request=mark_request_started)
    except MediaError as exc:
        record["state"] = exc.submission_state or ("submission_unknown" if record.get("api_request_sent") else "submission_rejected")
        record["operation_state"] = ""
        record["operation_token"] = ""
        record["last_error"] = exc.safe_message[:300]
        record["last_error_code"] = exc.code
        record["provider_request_id"] = exc.provider_request_id
        record["gateway_request_id"] = exc.gateway_request_id
        _atomic_json(path, record)
        raise
    except Exception as exc:
        record["state"] = "submission_unknown"
        record["operation_state"] = ""
        record["operation_token"] = ""
        record["last_error"] = f"{type(exc).__name__}: response processing failed"
        _atomic_json(path, record)
        raise MediaError("服务已响应但无法可靠解析；提交状态未知，不会自动重试。") from exc
    artifacts = _artifacts(response, provider)
    task_id = None
    status = str(response.get("status", response.get("task_status", ""))).strip().lower()
    failed = status in {"failed", "error", "cancelled", "expired", "rejected"}
    pending = status in {"queued", "pending", "processing", "running", "in_progress", "submitted"}
    if failed:
        record["state"] = "failed"
        record["last_error"] = "上游将任务标记为失败；请查看服务方任务详情。"
    elif status in {"completed", "succeeded", "success", "done", "finished"} and not artifacts:
        record["state"] = "completed_without_artifact"
        record["last_error"] = "上游报告任务完成但没有可识别的媒体产物。"
    elif artifacts and not pending:
        record["state"] = "completed_pending_save"
    elif provider == "image-2" and _api_profile() == "responses":
        record["state"] = "submission_unknown"
        record["last_error"] = "Responses API returned no image_generation_call.result"
        record["response_summary"] = _response_summary(response, _artifacts(response, provider))
        record["api_request_sent"] = True
        _atomic_json(path, record)
        raise MediaError("图像请求已发送，但 Responses 响应中没有 image_generation_call.result；请先核对网关任务和账单，不要重试。", code="SUBMISSION_UNKNOWN", provider=provider, model=model, submission_state="submission_unknown", api_request_sent=True)
    else:
        try:
            task_id = _extract_task_id(provider, response)
        except MediaError as exc:
            record["state"] = "submission_unknown"
            record["last_error"] = str(exc)
            _atomic_json(path, record)
            raise
        record["state"] = "in_progress" if pending else "task_created"
        record["task_id"] = task_id
    record["api_request_sent"] = True
    record["response_summary"] = _response_summary(response, artifacts)
    record["artifacts"] = _protect_artifact_urls(artifacts)
    record["operation_state"] = ""
    record["operation_token"] = ""
    _atomic_json(path, record)
    saved = []
    if provider == "image-2" and _api_profile() == "responses" and record["state"] == "completed_pending_save":
        try:
            saved = _save_artifacts(record)
            record["saved_artifacts"] = saved
            record["state"] = "completed_saved"
            _atomic_json(path, record)
        except MediaError:
            record["state"] = "completed_pending_save"
            record["last_error_code"] = "ARTIFACT_SAVE_PENDING"
            _atomic_json(path, record)
            raise
    return {"success": record["state"] not in {"failed", "completed_without_artifact", "submission_unknown"}, "state": record["state"], "provider": provider, "model": model, "confirmation_id": record["confirmation_id"], "submission_id": record["submission_id"], "task_id": task_id, "artifacts": _public_artifacts(artifacts), "saved": saved, "api_request_sent": True, "note": "异步任务请调用 media_check；completed_pending_save 可用 media_save 恢复；未知提交不得重发。"}


def _query_path(provider: str, task_id: str) -> str:
    encoded = urllib.parse.quote(task_id, safe="")
    if _api_profile() == "wokey" and provider in {"grok-video", "seedance-video", "minimax-video"}:
        return f"/v1/videos/{encoded}"
    if provider == "image-2" and _api_profile() == "responses":
        raise MediaError("Responses API 图像生成是同步请求，没有异步轮询接口。")
    if provider == "image-2":
        return f"/api/llm/openai/v1/images/tasks/{encoded}"
    if provider == "grok-video":
        return f"/v1/videos/{encoded}"
    if provider == "seedance-video":
        return f"/api/llm/doubao/contents/generations/tasks/{encoded}"
    raise MediaError("该图像服务返回同步结果，没有异步轮询接口。")


def media_check(args: dict[str, Any]) -> dict[str, Any]:
    with _confirmation_operation_lock(str(args.get("confirmation_id", "")), "检查"):
        path, record = _validated_record(args, allow_submitting=True)
        state = str(record.get("state", ""))
        if state in {"completed", "completed_saved"}:
            return {"success": True, "state": state, "artifacts": _public_artifacts(record.get("artifacts", [])), "api_request_sent": False, "deduplicated": True}
        if state == "completed_pending_save":
            return {"success": True, "state": state, "artifacts": _public_artifacts(record.get("artifacts", [])), "api_request_sent": False, "next_tool": "media_save"}
        if state not in {"task_created", "in_progress", "provider_status_unknown"} or not record.get("task_id"):
            raise MediaError(f"当前任务状态为 {state}；没有可查询的 task_id。未知提交只能等待人工查账，不能重发。")
        provider = record["request"]["provider"]
        record["operation_state"] = "checking"
        record["operation_token"] = uuid.uuid4().hex
        token = record["operation_token"]
        _atomic_json(path, record)
        try:
            response = _http_json("GET", _query_path(provider, record["task_id"]), None, provider)
        except MediaError:
            current = _load_json(path)
            if current.get("operation_token") == token:
                current["operation_state"] = ""
                current["operation_token"] = ""
                _atomic_json(path, current)
            raise
        status = str(response.get("status", response.get("task_status", "unknown"))).strip().lower()
        arts = _artifacts(response, provider)
        if status in {"failed", "error", "cancelled", "expired", "rejected"}:
            record["state"] = "failed"
            record["last_error"] = "上游将任务标记为失败；请查看服务方任务详情。"
            record["response_summary"] = _response_summary(response, arts)
            _atomic_json(path, record)
        elif status in {"completed", "succeeded", "success", "done", "finished"}:
            record["state"] = "completed_pending_save" if arts else "completed_without_artifact"
            record["artifacts"] = _protect_artifact_urls(arts)
            record["response_summary"] = _response_summary(response, arts)
        elif status in {"queued", "pending", "processing", "running", "in_progress", "submitted"}:
            record["state"] = "in_progress"
        else:
            record["state"] = "provider_status_unknown"
        record["operation_state"] = ""
        record["operation_token"] = ""
        record["last_provider_status"] = status[:80]
        record["last_checked_at"] = int(time.time())
        _atomic_json(path, record)
        return {"success": record["state"] not in {"failed", "submission_unknown", "provider_status_unknown", "completed_missing_artifacts", "completed_without_artifact"}, "state": record["state"], "provider_status": status, "provider": provider, "task_id": record["task_id"], "artifacts": _public_artifacts(arts), "api_request_sent": True, "poll_after_seconds": 10 if record["state"] == "in_progress" else None}


def _safe_download(url: str, destination: Path) -> tuple[int, str]:
    _validate_public_url(url, resolve=True)
    opener = urllib.request.build_opener(_NoRedirect)
    headers = {"Accept": "*/*", "User-Agent": "Codex-Media-MCP/1.0"}
    target = urllib.parse.urlsplit(url)
    base = urllib.parse.urlsplit(_base())
    if (target.scheme, target.hostname, target.port or 443) == (base.scheme, base.hostname, base.port or 443):
        key = _api_key()
        if key:
            headers["Authorization"] = f"Bearer {key}"
    request = urllib.request.Request(url, headers=headers, method="GET")
    temporary = destination.with_suffix(destination.suffix + ".part")
    total = 0
    try:
        with opener.open(request, timeout=HTTP_TIMEOUT_SECONDS) as response, temporary.open("wb") as stream:
            content_type = str(response.headers.get("Content-Type", "")).split(";", 1)[0].strip().casefold()
            declared = int(response.headers.get("Content-Length") or 0)
            if declared > MAX_DOWNLOAD_BYTES:
                raise MediaError("结果文件超过本地下载上限。")
            while True:
                block = response.read(1024 * 1024)
                if not block:
                    break
                total += len(block)
                if total > MAX_DOWNLOAD_BYTES:
                    raise MediaError("结果文件超过本地下载上限。")
                stream.write(block)
        if total <= 0:
            raise MediaError("结果文件为空。")
        with temporary.open("rb") as check_stream:
            signature = check_stream.read(32)
        content_type = content_type.casefold()
        signatures = {
            "image/png": signature.startswith(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg": signature.startswith(b"\xff\xd8\xff"),
            "image/webp": len(signature) >= 12 and signature[:4] == b"RIFF" and signature[8:12] == b"WEBP",
            "image/gif": signature.startswith((b"GIF87a", b"GIF89a")),
            "video/mp4": b"ftyp" in signature[:16],
            "video/quicktime": b"ftyp" in signature[:16],
            "video/x-m4v": b"ftyp" in signature[:16],
            "video/webm": signature.startswith(b"\x1aE\xdf\xa3"),
        }
        known = [(mime, matches) for mime, matches in signatures.items() if matches]
        if not known or (content_type in signatures and not signatures[content_type]) or (content_type and content_type != "application/octet-stream" and content_type not in signatures):
            raise MediaError("下载结果的 MIME 类型或文件头无效。", code="ARTIFACT_INTEGRITY_FAILED")
        os.replace(temporary, destination)
        return total, content_type
    except Exception:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise


def _validate_png(raw: bytes) -> None:
    signature = b"\x89PNG\r\n\x1a\n"
    if len(raw) < 45 or not raw.startswith(signature):
        raise MediaError("图像结果不是有效的 PNG 文件。")
    offset = len(signature)
    saw_ihdr = False
    saw_idat = False
    idat_closed = False
    saw_plte = False
    saw_iend = False
    width = height = bit_depth = color_type = interlace = 0
    compressed_parts: list[bytes] = []
    while offset + 12 <= len(raw):
        length = struct.unpack_from(">I", raw, offset)[0]
        chunk_type = raw[offset + 4:offset + 8]
        chunk_end = offset + 12 + length
        if length > MAX_DOWNLOAD_BYTES or chunk_end > len(raw):
            raise MediaError("PNG 图像包含越界或不完整的数据块。")
        chunk_data = raw[offset + 8:offset + 8 + length]
        expected_crc = struct.unpack_from(">I", raw, offset + 8 + length)[0]
        if (zlib.crc32(chunk_type + chunk_data) & 0xFFFFFFFF) != expected_crc:
            raise MediaError("PNG 图像校验失败。")
        if not saw_ihdr:
            if chunk_type != b"IHDR" or length != 13:
                raise MediaError("PNG 图像缺少有效的 IHDR。")
            width, height, bit_depth, color_type, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", chunk_data
            )
            if width < 1 or height < 1 or width * height > MAX_PNG_PIXELS:
                raise MediaError("PNG 图像尺寸无效。")
            valid_depths = {
                0: {1, 2, 4, 8, 16},
                2: {8, 16},
                3: {1, 2, 4, 8},
                4: {8, 16},
                6: {8, 16},
            }
            if bit_depth not in valid_depths.get(color_type, set()) or compression != 0 or filtering != 0 or interlace not in {0, 1}:
                raise MediaError("PNG 图像头参数无效。")
            saw_ihdr = True
        elif chunk_type == b"IHDR":
            raise MediaError("PNG 图像包含重复 IHDR。")
        elif chunk_type == b"PLTE":
            if saw_plte or saw_idat or not length or length % 3 or length > 768:
                raise MediaError("PNG 图像的调色板无效。")
            if color_type in {0, 4} or (color_type == 3 and length // 3 > (1 << bit_depth)):
                raise MediaError("PNG 图像的调色板与颜色类型不匹配。")
            saw_plte = True
        elif chunk_type == b"IDAT":
            if idat_closed:
                raise MediaError("PNG 图像的数据块顺序无效。")
            saw_idat = True
            compressed_parts.append(chunk_data)
        elif saw_idat and chunk_type != b"IEND":
            idat_closed = True
        if chunk_type[0:1].isupper() and chunk_type not in {b"IHDR", b"PLTE", b"IDAT", b"IEND"}:
            raise MediaError("PNG 图像包含不支持的关键数据块。")
        offset = chunk_end
        if chunk_type == b"IEND":
            if length != 0 or not saw_idat:
                raise MediaError("PNG 图像的 IEND 数据无效。")
            saw_iend = True
            break
    if not (saw_ihdr and saw_idat and saw_iend) or offset != len(raw):
        raise MediaError("PNG 图像结构不完整。")
    if color_type == 3 and not saw_plte:
        raise MediaError("索引色 PNG 图像缺少调色板。")

    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color_type]
    passes = (
        ((0, 0, 1, 1),)
        if interlace == 0
        else ((0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2))
    )
    scanlines: list[tuple[int, int]] = []
    decoded_size = 0
    for x_start, y_start, x_step, y_step in passes:
        pass_width = 0 if width <= x_start else (width - x_start + x_step - 1) // x_step
        pass_height = 0 if height <= y_start else (height - y_start + y_step - 1) // y_step
        if pass_width and pass_height:
            row_bytes = (pass_width * channels * bit_depth + 7) // 8
            decoded_size += pass_height * (row_bytes + 1)
            if decoded_size > MAX_PNG_DECODED_BYTES:
                raise MediaError("PNG 解码数据超过本地上限。")
            scanlines.append((pass_height, row_bytes))

    try:
        decoder = zlib.decompressobj()
        decoded = decoder.decompress(b"".join(compressed_parts), decoded_size + 1)
    except zlib.error as exc:
        raise MediaError("PNG 图像压缩数据无法解码。") from exc
    if len(decoded) != decoded_size or not decoder.eof or decoder.unconsumed_tail or decoder.unused_data:
        raise MediaError("PNG 图像解码长度或压缩流无效。")
    scan_offset = 0
    for rows, row_bytes in scanlines:
        for _ in range(rows):
            if decoded[scan_offset] > 4:
                raise MediaError("PNG 图像包含无效的扫描线过滤器。")
            scan_offset += row_bytes + 1
    if scan_offset != len(decoded):
        raise MediaError("PNG 图像扫描线结构不完整。")


def _save_artifacts(record: dict[str, Any]) -> list[dict[str, Any]]:
    output_root = _data_root() / "outputs" / record["confirmation_id"]
    output_root.mkdir(parents=True, exist_ok=True)
    saved = []
    for index, artifact in enumerate(_restore_artifact_urls(record.get("artifacts", [])), 1):
        if artifact.get("kind") == "base64":
            try:
                raw = base64.b64decode(artifact.get("value", ""), validate=True)
            except (ValueError, TypeError) as exc:
                raise MediaError("图像 Base64 数据无效。") from exc
            if len(raw) > MAX_DOWNLOAD_BYTES:
                raise MediaError("图像结果超过本地文件上限。")
            # Re-sniff persisted base64 data on save as well. Older records may
            # have been written by versions that labeled every image result as
            # PNG, even when a provider returned JPEG bytes.
            mime = _base64_image_mime(str(artifact.get("value", "")))
            if mime == "image/png":
                _validate_png(raw)
                extension = ".png"
            elif mime == "image/jpeg":
                if not raw.startswith(b"\xff\xd8\xff") or not raw.endswith(b"\xff\xd9"):
                    raise MediaError("JPEG 图像结果不完整或缺少结束标记。", code="ARTIFACT_INTEGRITY_FAILED")
                extension = ".jpg"
            else:
                raise MediaError("图像结果格式不受支持。")
            target = output_root / f"result-{index}{extension}"
            if target.exists():
                existing = target.read_bytes()
                if hashlib.sha256(existing).digest() != hashlib.sha256(raw).digest():
                    raise MediaError("同一确认记录的本地结果文件已被修改；未覆盖现有文件。")
            else:
                temporary = target.with_name(target.name + ".tmp-" + uuid.uuid4().hex)
                temporary.write_bytes(raw)
                os.replace(temporary, target)
            saved.append({"path": str(target.resolve()), "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest(), "kind": artifact.get("kind", "image")})
        elif artifact.get("url"):
            url = _validate_public_url(artifact["url"])
            parsed = urllib.parse.urlsplit(url)
            suffix = Path(parsed.path).suffix.lower()
            allowed_exts = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".mp4", ".mov", ".webm", ".m4v"}
            if suffix not in allowed_exts:
                suffix = ".mp4" if artifact.get("kind") == "video" else ".png"
            target = output_root / f"result-{index}{suffix}"
            kind = artifact.get("kind", "image")
            prior = (record.get("saved_artifacts") or [])
            previous = prior[index - 1] if len(prior) >= index else None
            if target.exists():
                raw = target.read_bytes()
                digest = hashlib.sha256(raw).hexdigest()
                if not isinstance(previous, dict) or previous.get("sha256") != digest or int(previous.get("bytes", -1)) != len(raw):
                    raise MediaError("已存在的本地媒体结果缺少可信校验值或内容已被修改；拒绝复用。", code="ARTIFACT_INTEGRITY_FAILED")
                content_type = mimetypes.guess_type(target.name)[0] or ""
                size = len(raw)
            else:
                size, content_type = _safe_download(url, target)
                raw = target.read_bytes()
                digest = hashlib.sha256(raw).hexdigest()
            allowed_types = IMAGE_TYPES if kind == "image" else VIDEO_TYPES
            if content_type and content_type not in allowed_types and content_type != "application/octet-stream":
                target.unlink(missing_ok=True)
                raise MediaError("结果 Content-Type 与图像/视频类型不匹配。", code="ARTIFACT_INTEGRITY_FAILED")
            if kind == "image":
                valid_magic = raw.startswith((b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff", b"GIF87a", b"GIF89a")) or (len(raw) >= 12 and raw[:4] == b"RIFF" and raw[8:12] == b"WEBP")
            else:
                valid_magic = b"ftyp" in raw[:16] or raw.startswith(b"\x1aE\xdf\xa3")
            if not valid_magic:
                target.unlink(missing_ok=True)
                raise MediaError("结果文件头与图像/视频格式不匹配。", code="ARTIFACT_INTEGRITY_FAILED")
            if kind == "image" and raw.startswith(b"\x89PNG\r\n\x1a\n"):
                _validate_png(raw)
            if kind == "image" and raw.startswith(b"\xff\xd8\xff") and not raw.endswith(b"\xff\xd9"):
                target.unlink(missing_ok=True)
                raise MediaError("JPEG 结果缺少结束标记。", code="ARTIFACT_INTEGRITY_FAILED")
            saved.append({"path": str(target.resolve()), "bytes": size, "sha256": digest, "kind": kind})
    return saved


def media_save(args: dict[str, Any]) -> dict[str, Any]:
    with _confirmation_operation_lock(str(args.get("confirmation_id", "")), "保存"):
        path, record = _validated_record(args, allow_submitting=True)
        if record.get("state") not in {"completed", "completed_saved", "completed_pending_save", "task_created"}:
            raise MediaError("仅已完成或已有异步 task_id 的任务可保存；先调用 media_check。")
        if record.get("state") == "task_created":
            raise MediaError("任务尚未完成；先调用 media_check 获取结果。")
        output_root = _data_root() / "outputs" / record["confirmation_id"]
        if output_root.exists():
            for partial in output_root.glob("*.part"):
                partial.unlink(missing_ok=True)
        if record.get("operation_state"):
            raise MediaError("该媒体任务正在检查或保存；请稍后重试。", code="OPERATION_IN_PROGRESS", submission_state=record.get("state"))
        record["operation_state"] = "saving"
        record["operation_token"] = uuid.uuid4().hex
        token = record["operation_token"]
        _atomic_json(path, record)
        try:
            saved = record.get("saved_artifacts") or _save_artifacts(record)
        except Exception:
            current = _load_json(path)
            if current.get("operation_token") == token:
                current["operation_state"] = ""
                current["operation_token"] = ""
                current["state"] = "completed_pending_save"
                _atomic_json(path, current)
            raise
        record["saved_artifacts"] = saved
        record["state"] = "completed_saved"
        record["operation_state"] = ""
        record["operation_token"] = ""
        record["saved_at"] = int(time.time())
        _atomic_json(path, record)
        for item in saved:
            saved_path = Path(str(item.get("path", ""))).resolve()
            try:
                saved_path.relative_to(output_root.resolve())
            except ValueError as exc:
                raise MediaError("保存记录指向任务目录之外；拒绝报告成功。", code="ARTIFACT_INTEGRITY_FAILED") from exc
            if not saved_path.is_file() or saved_path.stat().st_size != int(item.get("bytes", -1)) or hashlib.sha256(saved_path.read_bytes()).hexdigest() != item.get("sha256"):
                raise MediaError("已保存结果与本地校验值不一致；拒绝报告成功。", code="ARTIFACT_INTEGRITY_FAILED")
        return {"success": True, "saved": saved, "output_dir": str(output_root.resolve()), "api_request_sent": False}


def _configured() -> dict[str, Any]:
    base_ok = bool(os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip())
    key_ok = bool(_api_key())
    providers = {}
    for provider, config in PROVIDERS.items():
        model = _configured_model(provider)
        model_ok = bool(model)
        route_ok = (
            _api_profile() in {"wokey", "responses"}
            or provider not in {"image-2", "seedance-video"}
            or bool(os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip())
        )
        if provider == "seedance-video" and _api_profile() == "responses":
            route_ok = bool(os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip())
        if provider == "minimax-video" and _api_profile() != "wokey":
            route_ok = False
        provider_configured = base_ok and key_ok and model_ok and route_ok
        missing = [name for name, ok in (("MEDIA_GENERATION_API_BASE_URL", base_ok), ("MEDIA_GENERATION_API_KEY", key_ok), (config["model_env"], model_ok)) if not ok]
        if not route_ok:
            missing.append("MEDIA_GENERATION_API_PROFILE=wokey" if provider == "minimax-video" else "MEDIA_GENERATION_AGENT_ID")
        catalog_ok = False
        try:
            catalog_ok = model in _catalog_snapshot()["ids"]
        except MediaError:
            pass
        providers[provider] = {"configured": provider_configured, "missing": missing, "model": model,
                               "catalog_verified_model": catalog_ok,
                               "can_accept_explicit_model": base_ok and key_ok and route_ok and provider != "minimax-video"}
    cached = _runtime_config(allow_gateway_mismatch=True)
    active_gateway = os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip().rstrip("/")
    catalog_matches = bool(cached.get("gateway")) and cached.get("gateway") == active_gateway
    catalog_at = cached.get("catalogVerifiedAt") if catalog_matches else None
    catalog_fresh = bool(catalog_at and int(time.time()) - int(catalog_at) <= MODEL_CATALOG_TTL_SECONDS and cached.get("profile") == _api_profile())
    catalog_count = len(cached.get("catalogIds", [])) if catalog_matches and isinstance(cached.get("catalogIds"), list) else 0
    return {
        "local_runtime": "available",
        "local_tool_contract": "available",
        "api_key_configured": key_ok,
        "base_url_configured": base_ok,
        "api_profile": _api_profile(),
        "http_timeout_seconds": HTTP_TIMEOUT_SECONDS,
        "gateway_catalog": {"status": "fresh" if catalog_fresh else "stale" if catalog_at else "not_checked", "verified_at": catalog_at, "model_count": catalog_count, "ttl_seconds": MODEL_CATALOG_TTL_SECONDS},
        "generation_capability": "not_tested",
        "providers": providers,
        "price_source": "not available from this MCP; check the provider before approval",
    }


TOOL_SCHEMAS = [
    {"name": "media_refresh_models", "description": "使用统一网关只读模型目录发现并保存默认媒体模型。联网认证，不提交生成任务；目录可用于准备请求 24 小时，目录命中不代表生成权限已实测。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "media_capabilities", "description": "查看本地运行时、凭据/模型配置和最近一次网关目录验证状态；不联网、不计费。真实生图/视频能力默认未测试。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "media_prepare", "description": "离线准备图片或视频请求，要求模型在当前网关最近 24 小时目录中；创建 30 分钟有效的本地确认记录，不提交计费任务。Grok 视频图片请显式设置 role=first_frame 或 role=reference；旧调用省略 role 时按首帧处理。Grok 视频必须在 parameters 中明确提供非空 aspect_ratio 和 resolution。正式生成前必须展示实际请求摘要并等待用户明确确认；未知提交不自动重发。", "inputSchema": {"type": "object", "properties": {"provider": {"type": "string", "enum": list(PROVIDERS)}, "prompt": {"type": "string", "minLength": 1}, "model": {"type": "string"}, "reference_media": {"type": "array", "items": {"type": "object", "properties": {"kind": {"type": "string", "enum": ["image", "video", "audio"]}, "role": {"type": "string", "enum": ["first_frame", "reference"]}, "url": {"type": "string"}, "local_path": {"type": "string"}}, "required": ["kind"], "additionalProperties": False}}, "parameters": {"type": "object"}, "repeat_after_unknown_ack": {"type": "string", "enum": ["确认此前任务可能已计费且仍要求新生成"]}}, "required": ["provider", "prompt"], "additionalProperties": False}},
    {"name": "media_submit", "description": "使用 media_prepare 返回的确认 ID/指纹提交一次计费请求。只有用户明确确认该确切请求后才调用；图像同步结果会在本机验证并保存，状态未知时禁止重发。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}, "user_confirmation": {"type": "string", "enum": ["确认", "就按这份生成"]}}, "required": ["confirmation_id", "confirmation_fingerprint", "user_confirmation"], "additionalProperties": False}},
    {"name": "media_check", "description": "仅查询同一确认记录已创建的异步任务；不会创建或重发任务。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}}, "required": ["confirmation_id", "confirmation_fingerprint"], "additionalProperties": False}},
    {"name": "media_save", "description": "把已完成任务返回的图像/视频 URL 或 base64 结果保存到本机媒体输出目录；仅下载结果，不提交新的计费任务。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}}, "required": ["confirmation_id", "confirmation_fingerprint"], "additionalProperties": False}},
]


def _call_tool(name: str, args: dict[str, Any]) -> Any:
    if name == "media_refresh_models":
        return media_refresh_models(args)
    if name == "media_capabilities":
        return _configured()
    if name == "media_prepare":
        return media_prepare(args)
    if name == "media_submit":
        return media_submit(args)
    if name == "media_check":
        return media_check(args)
    if name == "media_save":
        return media_save(args)
    raise MediaError(f"未知工具: {name}")


def _media_error_envelope(exc: MediaError) -> dict[str, Any]:
    result: dict[str, Any] = {"success": False, "state": exc.submission_state or "rejected",
                              "error": {"code": exc.code, "safe_message": exc.safe_message,
                                        "retryable": bool(exc.retryable)}}
    for key, value in (("http_status", exc.http_status), ("provider_request_id", exc.provider_request_id),
                       ("gateway_request_id", exc.gateway_request_id)):
        if value is not None:
            result["error"][key] = value
    if exc.provider is not None:
        result["provider"] = exc.provider
    if exc.model is not None:
        result["model"] = exc.model
    if exc.api_request_sent is not None:
        result["api_request_sent"] = exc.api_request_sent
    result.update({key: value for key, value in exc.details.items() if value is not None})
    return result


def _normalize_tool_result(name: str, args: dict[str, Any], value: dict[str, Any]) -> dict[str, Any]:
    """Stable top-level tool envelope while preserving legacy result fields."""
    result = dict(value)
    result.setdefault("success", True)
    result.setdefault("state", "reported")
    for key in ("provider", "model", "confirmation_id", "submission_id", "task_id"):
        if result.get(key) is None:
            candidate = args.get(key)
            if candidate is not None:
                result[key] = candidate
    result.setdefault("api_request_sent", False)
    result.setdefault("artifacts", [])
    result.setdefault("saved", [])
    result.setdefault("error", None)
    result.setdefault("timestamps", {"returned_at_epoch": int(time.time())})
    return result


def _send(message: dict[str, Any]) -> None:
    raw = (json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")
    sys.stdout.buffer.write(raw)
    sys.stdout.buffer.flush()


def _jsonrpc_error(request_id: Any, code: int, message: str) -> None:
    _send({"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}})


def main() -> None:
    while True:
        raw = sys.stdin.buffer.readline(MAX_JSON_BYTES + 1)
        if not raw:
            return
        if len(raw) > MAX_JSON_BYTES:
            while raw and not raw.endswith(b"\n"):
                raw = sys.stdin.buffer.readline(MAX_JSON_BYTES + 1)
            _jsonrpc_error(None, -32600, "Request too large")
            continue
        try:
            message = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            _jsonrpc_error(None, -32700, "Parse error")
            continue
        if not isinstance(message, dict):
            _jsonrpc_error(None, -32600, "Invalid Request")
            continue
        method_value = message.get("method")
        method = method_value if isinstance(method_value, str) else ""
        request_id = message.get("id")
        if message.get("jsonrpc") != "2.0" or not method:
            _jsonrpc_error(request_id, -32600, "Invalid Request")
            continue
        if method == "notifications/initialized" or method.startswith("notifications/"):
            continue
        if "id" not in message:
            # Notifications never invoke tools (especially billable tools).
            continue
        if method == "initialize":
            params = message.get("params") if isinstance(message.get("params"), dict) else {}
            version = params.get("protocolVersion") if isinstance(params.get("protocolVersion"), str) else PROTOCOL_VERSION
            _send({"jsonrpc": "2.0", "id": request_id, "result": {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}}, "serverInfo": {"name": "codex-starter-media-generation", "version": SERVER_VERSION}, "instructions": "Use media_prepare, wait for user confirmation, then media_submit. Unknown submissions must never be retried."}})
        elif method == "ping":
            _send({"jsonrpc": "2.0", "id": request_id, "result": {}})
        elif method == "tools/list":
            _send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOL_SCHEMAS}})
        elif method == "tools/call":
            params = message.get("params") if isinstance(message.get("params"), dict) else {}
            name = str(params.get("name", ""))
            args = params.get("arguments") if isinstance(params.get("arguments"), dict) else {}
            try:
                result = _normalize_tool_result(name, args, _call_tool(name, args))
                _send({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False)}], "isError": False}})
            except MediaError as exc:
                envelope = _normalize_tool_result(name, args, _media_error_envelope(exc))
                envelope["success"] = False
                envelope["error"] = envelope.pop("error")
                _send({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": json.dumps(envelope, ensure_ascii=False, separators=(",", ":"))}], "structuredContent": envelope, "isError": True}})
            except Exception as exc:  # Do not leak traceback or environment data into the MCP stream.
                _send({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": f"媒体 MCP 内部错误: {type(exc).__name__}"}], "isError": True}})
        elif request_id is not None:
            _send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "Method not found"}})


if __name__ == "__main__":
    if "--healthcheck" in sys.argv[1:]:
        runtime_ready = sys.implementation.name == "cpython" and sys.version_info >= (3, 12)
        protocol_ready = False
        protocol_error = "runtime_not_ready"
        if runtime_ready:
            requests = [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": PROTOCOL_VERSION, "clientInfo": {"name": "健康检查-中文", "version": "1"}}},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
                {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "media_capabilities", "arguments": {}}},
            ]
            environment = os.environ.copy()
            # Simulate a non-UTF-8 Python text environment (common on older
            # Chinese Windows installs). The protocol must still be raw UTF-8.
            environment["PYTHONIOENCODING"] = "gbk"
            environment["PYTHONUTF8"] = "0"
            environment.pop("MEDIA_GENERATION_API_KEY", None)
            try:
                probe = subprocess.run(
                    [sys.executable, str(Path(__file__).resolve())],
                    input=("\n".join(json.dumps(item, ensure_ascii=False) for item in requests) + "\n").encode("utf-8"),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=environment,
                    timeout=10,
                    check=True,
                )
                replies = [json.loads(line.decode("utf-8", errors="strict")) for line in probe.stdout.splitlines() if line.strip()]
                tools = replies[1]["result"]["tools"] if len(replies) == 3 else []
                capability_text = replies[2]["result"]["content"][0]["text"] if len(replies) == 3 else ""
                capabilities = json.loads(capability_text) if capability_text else {}
                protocol_ready = (
                    replies[0].get("id") == 1
                    and replies[1].get("id") == 2
                    and replies[2].get("id") == 3
                    and isinstance(tools, list)
                    and len(tools) == len(TOOL_SCHEMAS)
                    and any("统一" in str(item.get("description", "")) for item in tools)
                    and capabilities.get("generation_capability") == "not_tested"
                    and capabilities.get("gateway_catalog", {}).get("status") in {"not_checked", "fresh", "stale"}
                )
                protocol_error = "protocol_contract_mismatch" if not protocol_ready else ""
            except Exception as exc:
                protocol_error = type(exc).__name__
        ready = runtime_ready and protocol_ready
        print(json.dumps({
            "ready": ready,
            "healthScope": "local-runtime-protocol",
            "runtimeStatus": "passed" if runtime_ready else "failed",
            "protocolStatus": "passed" if protocol_ready else "failed",
            "gatewayCatalogStatus": "not_checked",
            "generationCapability": "not_tested",
            "python": ".".join(str(part) for part in sys.version_info[:3]),
            "implementation": sys.implementation.name,
            "serverVersion": SERVER_VERSION,
            "dependencies": "python-standard-library",
            "protocol": "utf-8" if protocol_ready else "failed",
            "toolCount": len(TOOL_SCHEMAS),
            "protocolError": protocol_error,
        }, separators=(",", ":")))
        raise SystemExit(0 if ready else 1)
    main()
