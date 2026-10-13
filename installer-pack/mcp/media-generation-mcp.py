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
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib
from pathlib import Path
from typing import Any, Callable


SERVER_VERSION = "1.3.3"
PROTOCOL_VERSION = "2025-03-26"
MAX_JSON_BYTES = 32 * 1024 * 1024
HTTP_TIMEOUT_SECONDS = 60
MAX_REFERENCE_BYTES = 25 * 1024 * 1024
MAX_DOWNLOAD_BYTES = 512 * 1024 * 1024
MAX_PNG_PIXELS = 100_000_000
MAX_PNG_DECODED_BYTES = 256 * 1024 * 1024
CONFIRMATION_TTL_SECONDS = 30 * 60
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
    pass


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
        raise MediaError("媒体模型缓存与当前网关不一致；请重新发现模型。")
    return value


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
    response = _http_json("GET", "/v1/models", None, "catalog")
    rows = response.get("data")
    if not isinstance(rows, list):
        raise MediaError("模型目录缺少 data 数组；未更新配置。")
    available = {row["id"] for row in rows if isinstance(row, dict) and isinstance(row.get("id"), str)}
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
        "schemaVersion": 1, "gateway": _base(), "models": models,
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


def _atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    temporary.write_bytes(json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8"))
    os.replace(temporary, path)


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
    if provider == "minimax-video" and requested and requested.strip() != configured:
        raise MediaError("MiniMax 视频请求只能使用统一媒体配置中的 MEDIA_MINIMAX_VIDEO_MODEL。")
    model = str(requested or configured).strip()
    if not model:
        raise MediaError(f"未配置 {PROVIDERS[provider]['model_env']}；请填写服务方提供的模型名。")
    if len(model) > 256 or any(ord(char) < 32 for char in model):
        raise MediaError("模型名格式无效。")
    return model


def _safe_reference(item: Any) -> dict[str, Any]:
    if not isinstance(item, dict):
        raise MediaError("reference_media 的每一项都必须是对象。")
    kind = str(item.get("kind", "image")).strip().lower()
    if kind not in {"image", "video", "audio"}:
        raise MediaError("参考素材 kind 只允许 image、video 或 audio。")
    url = str(item.get("url", "")).strip()
    local_path = str(item.get("local_path", "")).strip()
    if bool(url) == bool(local_path):
        raise MediaError("每项参考素材必须且只能提供 url 或 local_path 之一。")
    if url:
        _validate_public_url(url)
        return {"kind": kind, "source": "url", "url": url}
    path = Path(local_path).expanduser().resolve()
    if not path.is_file():
        raise MediaError(f"参考文件不存在或不是普通文件: {path.name}")
    size = path.stat().st_size
    if size <= 0 or size > MAX_REFERENCE_BYTES:
        raise MediaError(f"参考文件大小必须在 1 字节至 {MAX_REFERENCE_BYTES} 字节之间。")
    mime = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
    allowed = IMAGE_TYPES if kind == "image" else VIDEO_TYPES if kind == "video" else AUDIO_TYPES
    if mime not in allowed:
        raise MediaError(f"参考文件 MIME 类型与 {kind} 不匹配。")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"kind": kind, "source": "file", "path": str(path), "bytes": size, "sha256": digest, "mime_type": mime}


def _validate_request(args: dict[str, Any]) -> dict[str, Any]:
    provider = str(args.get("provider", "")).strip().lower()
    if provider not in PROVIDERS:
        raise MediaError("provider 必须是 image-2、grok-image、grok-video、seedance-video 或 minimax-video。")
    prompt = str(args.get("prompt", ""))
    if not prompt.strip() or len(prompt) > 30000:
        raise MediaError("prompt 不能为空，且最多 30000 个字符。")
    model = _model(provider, str(args.get("model", "")))
    refs = args.get("reference_media", [])
    if not isinstance(refs, list) or len(refs) > 12:
        raise MediaError("reference_media 必须是数组，最多 12 项。")
    normalized_refs = [_safe_reference(item) for item in refs]
    params = args.get("parameters", {})
    if not isinstance(params, dict):
        raise MediaError("parameters 必须是对象。")
    request: dict[str, Any] = {
        "provider": provider,
        "model": model,
        "prompt": prompt,
        "reference_media": normalized_refs,
        "parameters": {},
        "route": _route_context(),
    }
    if provider == "image-2":
        if any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Image-2 仅接受 image 参考素材。")
        size = str(params.get("size", "auto" if _api_profile() == "responses" else "2048x2048"))
        if size not in IMAGE_SIZES and size != "auto":
            raise MediaError("Image-2 size 不在已支持的 2K 尺寸清单中。")
        n = int(params.get("n", 1))
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
        n = int(params.get("n", 1))
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
        if len(normalized_refs) > 1 or any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Grok video 当前只接受 0 或 1 张起始帧图片。")
        if _api_profile() == "wokey" and normalized_refs:
            raise MediaError("Wokey 视频参考素材需要 multipart；当前 MCP 适配暂只支持文生视频。")
        clean = {}
        if "duration" in params:
            duration = int(params["duration"])
            if not 1 <= duration <= 15:
                raise MediaError("Grok 视频时长必须为 1–15 秒整数。")
            clean["duration"] = duration
        for name, allowed in (("aspect_ratio", {"1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"}), ("resolution", {"480p", "720p", "1080p"})):
            value = str(params.get(name, "")).strip()
            if value:
                if value not in allowed:
                    raise MediaError(f"Grok video 参数 {name} 不受支持。")
                clean[name] = value
        if "generate_audio" in params:
            if not isinstance(params["generate_audio"], bool):
                raise MediaError("generate_audio 必须是布尔值。")
            clean["generate_audio"] = params["generate_audio"]
        request["mode"] = "image-to-video" if normalized_refs else "text-to-video"
        request["parameters"] = clean
    elif provider == "seedance-video":
        if _api_profile() == "wokey" and normalized_refs:
            raise MediaError("Wokey 视频参考素材需要 multipart；当前 MCP 适配暂只支持文生视频。")
        if any(item["source"] == "file" for item in normalized_refs):
            raise MediaError("Seedance 参考素材目前必须使用服务方可访问的 HTTPS URL；MCP 不会擅自上传素材。")
        duration = int(params.get("duration", 8))
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
        duration = int(params.get("duration", 4))
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
        raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。")
    if os.name == "nt":
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
                raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。")
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
            fcntl.flock(fd, fcntl.LOCK_EX, fcntl.LOCK_NB)
            acquired = True
        except OSError as exc:
            raise MediaError(f"该任务正在{operation}；请稍后重试，不要重复提交。") from exc
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


def media_prepare(args: dict[str, Any]) -> dict[str, Any]:
    request = _validate_request(args)
    created = int(time.time())
    confirmation_id = "media-" + uuid.uuid4().hex
    payload = {"version": 1, "confirmation_id": confirmation_id, "created_at": created, "expires_at": created + CONFIRMATION_TTL_SECONDS, "request": request}
    fingerprint = _token(payload)
    record = dict(payload, fingerprint=fingerprint, state="prepared", api_request_sent=False)
    _atomic_json(_confirmation_path(confirmation_id), record)
    refs = [{"kind": item["kind"], "source": item["source"], "url": item.get("url"), "file_name": Path(item["path"]).name if item.get("path") else None, "bytes": item.get("bytes"), "sha256": item.get("sha256")} for item in request["reference_media"]]
    return {
        "success": True,
        "mode": "prepared",
        "api_request_sent": False,
        "confirmation_id": confirmation_id,
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
    expected = _token({key: record.get(key) for key in ("version", "confirmation_id", "created_at", "expires_at", "request")})
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
        record["state"] = "submission_unknown"
        record["last_error"] = "Process ended while submission was in progress; verify upstream before any manual recovery."
        record["recovered_at"] = int(time.time())
        record["api_request_sent"] = True
        _atomic_json(path, record)
    allowed_states = {"prepared"}
    if allow_submitting:
        allowed_states |= {"submitting", "submission_unknown", "submission_rejected", "task_created", "completed", "failed"}
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


def _http_json(method: str, path: str, payload: dict[str, Any] | bytes | None, provider: str, idempotency_key: str | None = None, content_type: str = "application/json", before_request: Callable[[], None] | None = None) -> dict[str, Any]:
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
    if idempotency_key and (profile == "wokey" or provider in {"image-2", "seedance-video"}):
        headers["Idempotency-Key"] = idempotency_key
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
        body = exc.read(8192).decode("utf-8", errors="replace")
        body = body.replace(key, "<redacted>")
        if method == "POST" and (exc.code >= 500 or exc.code in {408, 425, 429}):
            raise MediaError(f"提交状态未知（HTTP {exc.code}）；为避免重复计费，不会自动重试。") from exc
        raise MediaError(f"媒体服务拒绝请求（HTTP {exc.code}）：{body[:600]}") from exc
    except MediaError:
        raise
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as exc:
        if method == "POST":
            raise MediaError(f"提交状态未知（{type(exc).__name__}）；为避免重复计费，不会自动重试。") from exc
        raise MediaError(f"媒体服务暂时无法响应（{type(exc).__name__}）；原任务状态未更改。") from exc
    if not isinstance(parsed, dict):
        raise MediaError("媒体服务返回了非对象 JSON。")
    return parsed


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
    return _http_json("POST", url_path, b"".join(chunks), "image-2", idempotency_key, f"multipart/form-data; boundary={boundary}", before_request)


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
            body = {"model": request["model"], "prompt": request["prompt"], "images": images, **params}
            return "/v1/images/edits", body
        return "/v1/images/generations", {"model": request["model"], "prompt": request["prompt"], **params}
    if provider == "minimax-video":
        if _api_profile() != "wokey":
            raise MediaError("MiniMax 视频暂只支持统一上游的 Wokey 兼容模式；没有发送 API 请求。")
        raise MediaError("MiniMax 视频路由尚未配置模型；没有发送 API 请求。")
    if provider == "grok-video":
        body = {"model": request["model"], "prompt": request["prompt"], **params}
        if request["reference_media"]:
            item = request["reference_media"][0]
            if item["source"] == "file":
                path = Path(item["path"])
                raw = path.read_bytes()
                if len(raw) > MAX_REFERENCE_BYTES or hashlib.sha256(raw).hexdigest() != item["sha256"]:
                    raise MediaError("Grok 起始帧在提交前改变；请重新准备并确认。")
                url = f"data:{item['mime_type']};base64," + base64.b64encode(raw).decode("ascii")
            else:
                url = item["url"]
            body["image"] = {"url": url}
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
    if not isinstance(task_id, str) or not task_id.strip() or len(task_id) > 512:
        raise MediaError("服务响应未包含有效 task_id；请求结果不确定，请勿直接重新提交。")
    return task_id.strip()


def _artifacts(response: Any, provider: str | None = None, *, output_scope: bool = False) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    if isinstance(response, dict):
        if response.get("type") == "image_generation_call":
            result = response.get("result")
            if isinstance(result, str) and result:
                found.append({"kind": "base64", "value": result, "mime_type": "image/png"})
        for key, value in response.items():
            is_output = key in {"video_url", "content_url", "b64_json"} or (output_scope and key == "url")
            if is_output and isinstance(value, str) and value:
                if key == "b64_json":
                    found.append({"kind": "base64", "value": value, "mime_type": "image/png"})
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
        "task_id": str(task_id)[:256] if task_id is not None else None,
        "request_id": str(request_id)[:256] if request_id is not None else None,
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
    if str(args.get("user_confirmation", "")).strip() not in {"确认", "就按这份生成"}:
        raise MediaError("正式提交只接受用户已明确回复“确认”或“就按这份生成”的执行调用。")
    provider = record["request"]["provider"]
    _base()
    if not _api_key():
        raise MediaError("未配置 MEDIA_GENERATION_API_KEY；没有发送 API 请求。")
    model = _model(provider, record["request"]["model"])
    if _api_profile() not in {"wokey", "responses"} and provider == "image-2" and not os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip():
        raise MediaError("当前图像网关需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
    if _api_profile() != "wokey" and provider == "seedance-video" and not os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip():
        raise MediaError("当前网关此 provider 需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
    for reference in record["request"].get("reference_media", []):
        if reference.get("source") == "url":
            _validate_public_url(reference["url"], resolve=True)
    # Persist before network I/O. Any crash or ambiguous network result blocks another POST.
    record["state"] = "submitting"
    record["submitted_at"] = int(time.time())
    record["api_request_sent"] = False
    _atomic_json(path, record)
    idempotency_key = hmac.new(_secret_key(), record["confirmation_id"].encode("utf-8"), hashlib.sha256).hexdigest()

    def mark_request_started() -> None:
        # Conservative durable marker: once this is written, no failure may
        # cause an automatic retry, even if the socket failed before send.
        record["api_request_sent"] = True
        record["request_started_at"] = int(time.time())
        _atomic_json(path, record)

    try:
        request = record["request"]
        if provider == "image-2" and request["mode"] == "edit" and _api_profile() == "legacy":
            response = _multipart_image_request(record, idempotency_key, mark_request_started)
        elif provider == "image-2" and request["mode"] == "edit" and _api_profile() == "wokey":
            raise MediaError("当前 Wokey 图片编辑尚未验证契约；没有发送 API 请求。")
        else:
            api_path, body = _request_payload(request)
            response = _http_json("POST", api_path, body, provider, idempotency_key, before_request=mark_request_started)
    except MediaError as exc:
        text = str(exc)
        record["state"] = "submission_unknown" if "状态未知" in text else "submission_rejected"
        record["last_error"] = text[:800]
        _atomic_json(path, record)
        raise
    except Exception as exc:
        record["state"] = "submission_unknown"
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
    elif artifacts and not pending:
        record["state"] = "completed"
    elif provider == "image-2" and _api_profile() == "responses":
        record["state"] = "submission_unknown"
        record["last_error"] = "Responses API returned no image_generation_call.result"
        record["response_summary"] = _response_summary(response, _artifacts(response, provider))
        record["api_request_sent"] = True
        _atomic_json(path, record)
        raise MediaError("图像请求已发送，但 Responses 响应中没有 image_generation_call.result；请先核对网关任务和账单，不要重试。")
    else:
        try:
            task_id = _extract_task_id(provider, response)
        except MediaError as exc:
            record["state"] = "submission_unknown"
            record["last_error"] = str(exc)
            _atomic_json(path, record)
            raise
        record["state"] = "task_created"
        record["task_id"] = task_id
    record["api_request_sent"] = True
    record["response_summary"] = _response_summary(response, artifacts)
    record["artifacts"] = artifacts
    record["idempotency_key_sha256"] = hashlib.sha256(idempotency_key.encode()).hexdigest()
    _atomic_json(path, record)
    saved = []
    if provider == "image-2" and _api_profile() == "responses" and record["state"] == "completed":
        saved = _save_artifacts(record)
        record["saved_artifacts"] = saved
        _atomic_json(path, record)
    return {"success": not failed, "state": record["state"], "provider": provider, "model": model, "task_id": task_id, "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in artifacts], "saved": saved, "api_request_sent": True, "note": "异步任务请调用 media_check；已完成结果可用 media_save 保存；未知提交不得重发。"}


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
        if state == "completed":
            return {"success": True, "state": state, "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in record.get("artifacts", [])], "api_request_sent": False, "deduplicated": True}
        if state != "task_created" or not record.get("task_id"):
            raise MediaError(f"当前任务状态为 {state}；没有可查询的 task_id。未知提交只能等待人工查账，不能重发。")
        provider = record["request"]["provider"]
        response = _http_json("GET", _query_path(provider, record["task_id"]), None, provider)
        status = str(response.get("status", response.get("task_status", "unknown"))).strip().lower()
        arts = _artifacts(response, provider)
        if status in {"failed", "error", "cancelled", "expired", "rejected"}:
            record["state"] = "failed"
            record["last_error"] = str(response.get("error") or response.get("message") or status)[:800]
            record["response_summary"] = _response_summary(response, arts)
            _atomic_json(path, record)
        elif status in {"completed", "succeeded", "success", "done", "finished"}:
            record["state"] = "completed" if arts else "task_created"
            record["artifacts"] = arts
            record["response_summary"] = _response_summary(response, arts)
            _atomic_json(path, record)
        return {"success": record["state"] not in {"failed", "submission_unknown"}, "state": record["state"], "provider_status": status, "provider": provider, "task_id": record["task_id"], "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in arts], "api_request_sent": True}


def _safe_download(url: str, destination: Path) -> int:
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
        with opener.open(request, timeout=60) as response, temporary.open("wb") as stream:
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
        os.replace(temporary, destination)
        return total
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
    for index, artifact in enumerate(record.get("artifacts", []), 1):
        if artifact.get("kind") == "base64":
            try:
                raw = base64.b64decode(artifact.get("value", ""), validate=True)
            except (ValueError, TypeError) as exc:
                raise MediaError("图像 Base64 数据无效。") from exc
            if len(raw) > MAX_DOWNLOAD_BYTES:
                raise MediaError("图像结果超过本地文件上限。")
            mime = str(artifact.get("mime_type", ""))
            if mime == "image/png":
                _validate_png(raw)
                extension = ".png"
            elif mime == "image/jpeg":
                if not raw.startswith(b"\xff\xd8\xff"):
                    raise MediaError("图像结果不是有效的 JPEG 文件。")
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
            saved.append({"path": str(target.resolve()), "bytes": len(raw), "kind": artifact.get("kind", "image")})
        elif artifact.get("url"):
            url = _validate_public_url(artifact["url"])
            parsed = urllib.parse.urlsplit(url)
            suffix = Path(parsed.path).suffix.lower()
            allowed_exts = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".mp4", ".mov", ".webm", ".m4v"}
            if suffix not in allowed_exts:
                suffix = ".mp4" if artifact.get("kind") == "video" else ".png"
            target = output_root / f"result-{index}{suffix}"
            size = target.stat().st_size if target.exists() else _safe_download(url, target)
            saved.append({"path": str(target.resolve()), "bytes": size, "kind": artifact.get("kind", "image")})
    return saved


def media_save(args: dict[str, Any]) -> dict[str, Any]:
    with _confirmation_operation_lock(str(args.get("confirmation_id", "")), "保存"):
        path, record = _validated_record(args, allow_submitting=True)
        if record.get("state") not in {"completed", "task_created"}:
            raise MediaError("仅已完成或已有异步 task_id 的任务可保存；先调用 media_check。")
        if record.get("state") == "task_created":
            raise MediaError("任务尚未完成；先调用 media_check 获取结果。")
        saved = record.get("saved_artifacts") or _save_artifacts(record)
        record["saved_artifacts"] = saved
        _atomic_json(path, record)
        output_root = _data_root() / "outputs" / record["confirmation_id"]
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
        providers[provider] = {"configured": provider_configured, "missing": missing, "model": model,
                               "can_accept_explicit_model": base_ok and key_ok and route_ok and provider != "minimax-video"}
    cached = _runtime_config(allow_gateway_mismatch=True)
    active_gateway = os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip().rstrip("/")
    catalog_matches = bool(cached.get("gateway")) and cached.get("gateway") == active_gateway
    catalog_at = cached.get("catalogVerifiedAt") if catalog_matches else None
    catalog_count = len(cached.get("catalogIds", [])) if catalog_matches and isinstance(cached.get("catalogIds"), list) else 0
    return {
        "local_runtime": "available",
        "local_tool_contract": "available",
        "api_key_configured": key_ok,
        "base_url_configured": base_ok,
        "api_profile": _api_profile(),
        "gateway_catalog": {"status": "previously_verified" if catalog_at else "not_checked", "verified_at": catalog_at, "model_count": catalog_count},
        "generation_capability": "not_tested",
        "providers": providers,
        "price_source": "not available from this MCP; check the provider before approval",
    }


TOOL_SCHEMAS = [
    {"name": "media_refresh_models", "description": "使用统一网关只读模型目录发现并保存默认媒体模型；联网认证但不提交生成任务。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "media_capabilities", "description": "查看本地运行时、凭据/模型配置和最近一次网关目录验证状态；不联网、不计费。真实生图/视频能力默认未测试。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "media_prepare", "description": "离线准备图片或视频请求，创建 30 分钟有效、不可变的本地确认记录；不联网、不提交计费任务。正式生成前必须向用户展示摘要并等待明确确认。", "inputSchema": {"type": "object", "properties": {"provider": {"type": "string", "enum": list(PROVIDERS)}, "prompt": {"type": "string", "minLength": 1}, "model": {"type": "string"}, "reference_media": {"type": "array", "items": {"type": "object", "properties": {"kind": {"type": "string", "enum": ["image", "video", "audio"]}, "url": {"type": "string"}, "local_path": {"type": "string"}}, "required": ["kind"], "additionalProperties": False}}, "parameters": {"type": "object"}}, "required": ["provider", "prompt"], "additionalProperties": False}},
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
                result = _call_tool(name, args)
                _send({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False)}], "isError": False}})
            except MediaError as exc:
                _send({"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": str(exc)}], "isError": True}})
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
                    and capabilities.get("gateway_catalog", {}).get("status") in {"not_checked", "previously_verified"}
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
