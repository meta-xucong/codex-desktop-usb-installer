#!/usr/bin/env python3
"""Local, stdio MCP facade for configured image and video API gateways.

Only Python's standard library is required. The server never guesses a provider
model, price, refund policy, or secret. Paid requests use an immutable local
confirmation record and a single-use submission state machine.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import ipaddress
import json
import mimetypes
import os
import re
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any


SERVER_VERSION = "1.2.0"
PROTOCOL_VERSION = "2025-03-26"
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_REFERENCE_BYTES = 25 * 1024 * 1024
MAX_DOWNLOAD_BYTES = 512 * 1024 * 1024
CONFIRMATION_TTL_SECONDS = 30 * 60
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
        return "wokey" if host.casefold() == "api.wokey.ai" else "legacy"
    if configured in {"legacy", "wokey"}:
        return configured
    raise MediaError("MEDIA_GENERATION_API_PROFILE 只允许 auto、legacy 或 wokey。")


def _api_key() -> str:
    return os.environ.get("MEDIA_GENERATION_API_KEY", "").strip()


def _model(provider: str, requested: str = "") -> str:
    configured = os.environ.get(PROVIDERS[provider]["model_env"], "").strip()
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
    }
    if provider == "image-2":
        if any(item["kind"] != "image" for item in normalized_refs):
            raise MediaError("Image-2 仅接受 image 参考素材。")
        size = str(params.get("size", "2048x2048"))
        if size not in IMAGE_SIZES:
            raise MediaError("Image-2 size 不在已支持的 2K 尺寸清单中。")
        n = int(params.get("n", 1))
        if not 1 <= n <= 8:
            raise MediaError("图片数量 n 必须在 1–8 之间。")
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
    if int(record.get("expires_at", 0)) < int(time.time()):
        raise MediaError("确认清单已过期，请重新准备并再次确认。")
    allowed_states = {"prepared"}
    if allow_submitting:
        allowed_states |= {"submitting", "submission_unknown", "submission_rejected", "task_created", "completed", "failed"}
    if record.get("state") not in allowed_states:
        raise MediaError(f"该确认清单已进入 {record.get('state')} 状态，不能重复提交。")
    for item in record["request"].get("reference_media", []):
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


def _http_json(method: str, path: str, payload: dict[str, Any] | bytes | None, provider: str, idempotency_key: str | None = None, content_type: str = "application/json") -> dict[str, Any]:
    key = _api_key()
    if not key:
        raise MediaError("未配置 MEDIA_GENERATION_API_KEY；没有发送 API 请求。")
    url = _endpoint(path)
    headers = {"Accept": "application/json", "Authorization": f"Bearer {key}"}
    if payload is not None:
        headers["Content-Type"] = content_type
    profile = _api_profile()
    if provider == "image-2" and profile != "wokey":
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
        with opener.open(request, timeout=120) as response:
            raw = response.read(MAX_JSON_BYTES + 1)
            if len(raw) > MAX_JSON_BYTES:
                raise MediaError("媒体服务 JSON 响应超过大小上限。")
            parsed = json.loads(raw.decode("utf-8-sig"))
    except urllib.error.HTTPError as exc:
        body = exc.read(8192).decode("utf-8", errors="replace")
        body = body.replace(key, "<redacted>")
        if exc.code >= 500 or exc.code in {408, 425, 429}:
            raise MediaError(f"提交状态未知（HTTP {exc.code}）；为避免重复计费，不会自动重试。") from exc
        raise MediaError(f"媒体服务拒绝请求（HTTP {exc.code}）：{body[:600]}") from exc
    except MediaError:
        raise
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as exc:
        raise MediaError(f"提交状态未知（{type(exc).__name__}）；为避免重复计费，不会自动重试。") from exc
    if not isinstance(parsed, dict):
        raise MediaError("媒体服务返回了非对象 JSON。")
    return parsed


def _multipart_image_request(record: dict[str, Any], idempotency_key: str) -> dict[str, Any]:
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
    return _http_json("POST", url_path, b"".join(chunks), "image-2", idempotency_key, f"multipart/form-data; boundary={boundary}")


def _request_payload(request: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    provider = request["provider"]
    params = request["parameters"]
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


def _artifacts(response: Any) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    if isinstance(response, dict):
        for key, value in response.items():
            if key in {"url", "image_url", "video_url", "content_url", "b64_json"} and isinstance(value, str) and value:
                if key == "b64_json":
                    found.append({"kind": "base64", "value": value, "mime_type": "image/png"})
                elif key == "content_url" and value.startswith("/") and _api_profile() == "wokey":
                    found.append({"kind": "video", "url": _endpoint(value)})
                elif value.startswith("https://"):
                    kind = "video" if key in {"video_url", "content_url"} or "video" in value.casefold() else "image"
                    found.append({"kind": kind, "url": value})
            elif isinstance(value, (dict, list)):
                found.extend(_artifacts(value))
    elif isinstance(response, list):
        for item in response:
            found.extend(_artifacts(item))
    # Keep order but don't duplicate a provider's same URL echoed in nested fields.
    unique: list[dict[str, Any]] = []
    seen = set()
    for item in found:
        marker = item.get("url") or hashlib.sha256(str(item.get("value", "")).encode()).hexdigest()
        if marker not in seen:
            seen.add(marker)
            unique.append(item)
    return unique


def media_submit(args: dict[str, Any]) -> dict[str, Any]:
    path, record = _validated_record(args)
    if str(args.get("user_confirmation", "")).strip() not in {"确认", "就按这份生成"}:
        raise MediaError("正式提交只接受用户已明确回复“确认”或“就按这份生成”的执行调用。")
    provider = record["request"]["provider"]
    _base()
    if not _api_key():
        raise MediaError("未配置 MEDIA_GENERATION_API_KEY；没有发送 API 请求。")
    model = _model(provider, record["request"]["model"])
    if _api_profile() != "wokey" and provider in {"image-2", "seedance-video"} and not os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip():
        raise MediaError("当前网关此 provider 需要 MEDIA_GENERATION_AGENT_ID；没有发送 API 请求。")
    for reference in record["request"].get("reference_media", []):
        if reference.get("source") == "url":
            _validate_public_url(reference["url"], resolve=True)
    # Persist before network I/O. Any crash or ambiguous network result blocks another POST.
    record["state"] = "submitting"
    record["submitted_at"] = int(time.time())
    _atomic_json(path, record)
    idempotency_key = hmac.new(_secret_key(), record["confirmation_id"].encode("utf-8"), hashlib.sha256).hexdigest()
    try:
        request = record["request"]
        if provider == "image-2" and request["mode"] == "edit":
            response = _multipart_image_request(record, idempotency_key)
        else:
            api_path, body = _request_payload(request)
            response = _http_json("POST", api_path, body, provider, idempotency_key)
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
    artifacts = _artifacts(response)
    task_id = None
    if artifacts:
        record["state"] = "completed"
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
    record["response"] = response
    record["artifacts"] = artifacts
    record["idempotency_key_sha256"] = hashlib.sha256(idempotency_key.encode()).hexdigest()
    _atomic_json(path, record)
    return {"success": True, "state": record["state"], "provider": provider, "model": model, "task_id": task_id, "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in artifacts], "api_request_sent": True, "note": "异步任务请调用 media_check；状态未知时不得重新提交。"}


def _query_path(provider: str, task_id: str) -> str:
    encoded = urllib.parse.quote(task_id, safe="")
    if _api_profile() == "wokey" and provider in {"grok-video", "seedance-video", "minimax-video"}:
        return f"/v1/videos/{encoded}"
    if provider == "image-2":
        return f"/api/llm/openai/v1/images/tasks/{encoded}"
    if provider == "grok-video":
        return f"/v1/videos/{encoded}"
    if provider == "seedance-video":
        return f"/api/llm/doubao/contents/generations/tasks/{encoded}"
    raise MediaError("该图像服务返回同步结果，没有异步轮询接口。")


def media_check(args: dict[str, Any]) -> dict[str, Any]:
    path, record = _validated_record(args, allow_submitting=True)
    state = str(record.get("state", ""))
    if state == "completed":
        return {"success": True, "state": state, "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in record.get("artifacts", [])], "api_request_sent": False, "deduplicated": True}
    if state != "task_created" or not record.get("task_id"):
        raise MediaError(f"当前任务状态为 {state}；没有可查询的 task_id。未知提交只能等待人工查账，不能重发。")
    provider = record["request"]["provider"]
    response = _http_json("GET", _query_path(provider, record["task_id"]), None, provider)
    status = str(response.get("status", response.get("task_status", "unknown"))).strip().lower()
    arts = _artifacts(response)
    if status in {"completed", "succeeded", "success", "done", "finished"} or arts:
        record["state"] = "completed"
        record["artifacts"] = arts
        record["response"] = response
        _atomic_json(path, record)
    elif status in {"failed", "error", "cancelled", "expired", "rejected"}:
        record["state"] = "failed"
        record["last_error"] = str(response.get("error") or response.get("message") or status)[:800]
        record["response"] = response
        _atomic_json(path, record)
    return {"success": record["state"] not in {"failed", "submission_unknown"}, "state": record["state"], "provider_status": status, "provider": provider, "task_id": record["task_id"], "artifacts": [{k: v for k, v in item.items() if k != "value"} for item in arts], "api_request_sent": True}


def _safe_download(url: str, destination: Path) -> int:
    _validate_public_url(url, resolve=True)
    opener = urllib.request.build_opener(_NoRedirect)
    headers = {"Accept": "*/*", "User-Agent": "Codex-Media-MCP/1.0"}
    if _api_profile() == "wokey":
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


def media_save(args: dict[str, Any]) -> dict[str, Any]:
    path, record = _validated_record(args, allow_submitting=True)
    if record.get("state") not in {"completed", "task_created"}:
        raise MediaError("仅已完成或已有异步 task_id 的任务可保存；先调用 media_check。")
    if record.get("state") == "task_created":
        raise MediaError("任务尚未完成；先调用 media_check 获取结果。")
    output_root = _data_root() / "outputs" / record["confirmation_id"]
    output_root.mkdir(parents=True, exist_ok=True)
    saved = []
    for index, artifact in enumerate(record.get("artifacts", []), 1):
        if artifact.get("kind") == "base64":
            raw = base64.b64decode(artifact.get("value", ""), validate=True)
            if len(raw) > MAX_DOWNLOAD_BYTES:
                raise MediaError("图像结果超过本地文件上限。")
            ext = ".png" if artifact.get("mime_type") == "image/png" else ".jpg"
            target = output_root / f"result-{index}{ext}"
            if not target.exists():
                target.write_bytes(raw)
            size = target.stat().st_size
        elif artifact.get("url"):
            url = _validate_public_url(artifact["url"])
            parsed = urllib.parse.urlsplit(url)
            suffix = Path(parsed.path).suffix.lower()
            allowed_exts = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".mp4", ".mov", ".webm", ".m4v"}
            if suffix not in allowed_exts:
                suffix = ".mp4" if artifact.get("kind") == "video" else ".png"
            target = output_root / f"result-{index}{suffix}"
            if target.exists():
                size = target.stat().st_size
            else:
                size = _safe_download(url, target)
        else:
            continue
        saved.append({"path": str(target.resolve()), "bytes": size, "kind": artifact.get("kind", "image")})
    return {"success": True, "saved": saved, "output_dir": str(output_root.resolve()), "api_request_sent": False}


def _configured() -> dict[str, Any]:
    base_ok = bool(os.environ.get("MEDIA_GENERATION_API_BASE_URL", "").strip())
    key_ok = bool(_api_key())
    providers = {}
    for provider, config in PROVIDERS.items():
        model_ok = bool(os.environ.get(config["model_env"], "").strip())
        route_ok = _api_profile() == "wokey" or provider not in {"image-2", "seedance-video"} or bool(os.environ.get("MEDIA_GENERATION_AGENT_ID", "").strip())
        if provider == "minimax-video" and _api_profile() != "wokey":
            route_ok = False
        provider_configured = base_ok and key_ok and model_ok and route_ok
        missing = [name for name, ok in (("MEDIA_GENERATION_API_BASE_URL", base_ok), ("MEDIA_GENERATION_API_KEY", key_ok), (config["model_env"], model_ok)) if not ok]
        if not route_ok:
            missing.append("MEDIA_GENERATION_API_PROFILE=wokey")
        providers[provider] = {"configured": provider_configured, "missing": missing}
    return {"api_key_configured": key_ok, "base_url_configured": base_ok, "api_profile": _api_profile(), "providers": providers, "price_source": "not available from this MCP; check the provider before approval"}


TOOL_SCHEMAS = [
    {"name": "media_capabilities", "description": "查看统一图片/视频 MCP 的连接状态、可用 provider 与缺少的本机配置；不联网、不计费。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "media_prepare", "description": "离线准备图片或视频请求，创建 30 分钟有效、不可变的本地确认记录；不联网、不提交计费任务。正式生成前必须向用户展示摘要并等待明确确认。", "inputSchema": {"type": "object", "properties": {"provider": {"type": "string", "enum": list(PROVIDERS)}, "prompt": {"type": "string", "minLength": 1}, "model": {"type": "string"}, "reference_media": {"type": "array", "items": {"type": "object", "properties": {"kind": {"type": "string", "enum": ["image", "video", "audio"]}, "url": {"type": "string"}, "local_path": {"type": "string"}}, "required": ["kind"], "additionalProperties": False}}, "parameters": {"type": "object"}}, "required": ["provider", "prompt"], "additionalProperties": False}},
    {"name": "media_submit", "description": "使用 media_prepare 返回的确认 ID/指纹提交一次计费请求。只有用户明确确认该确切请求后才调用；确认记录先落盘，状态未知时禁止重发。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}, "user_confirmation": {"type": "string", "enum": ["确认", "就按这份生成"]}}, "required": ["confirmation_id", "confirmation_fingerprint", "user_confirmation"], "additionalProperties": False}},
    {"name": "media_check", "description": "仅查询同一确认记录已创建的异步任务；不会创建或重发任务。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}}, "required": ["confirmation_id", "confirmation_fingerprint"], "additionalProperties": False}},
    {"name": "media_save", "description": "把已完成任务返回的图像/视频 URL 或 base64 结果保存到本机媒体输出目录；仅下载结果，不提交新的计费任务。", "inputSchema": {"type": "object", "properties": {"confirmation_id": {"type": "string"}, "confirmation_fingerprint": {"type": "string"}}, "required": ["confirmation_id", "confirmation_fingerprint"], "additionalProperties": False}},
]


def _call_tool(name: str, args: dict[str, Any]) -> Any:
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
    sys.stdout.write(json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def main() -> None:
    for raw in sys.stdin.buffer:
        if len(raw) > MAX_JSON_BYTES:
            continue
        try:
            message = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            _send({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Parse error"}})
            continue
        if not isinstance(message, dict):
            continue
        method = str(message.get("method", ""))
        request_id = message.get("id")
        if not method:
            continue
        if method == "notifications/initialized" or method.startswith("notifications/"):
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
    main()
