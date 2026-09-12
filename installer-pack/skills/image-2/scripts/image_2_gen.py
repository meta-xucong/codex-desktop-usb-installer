import argparse
import base64
import hashlib
import hmac
import ipaddress
import json
import mimetypes
import os
import socket
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path


IMAGE_2_API_BASE_URL = os.environ.get("IMAGE_2_API_BASE_URL", "").strip().rstrip("/")
IMAGE_GENERATION_ASYNC_URL = f"{IMAGE_2_API_BASE_URL}/api/llm/openai/v1/images/generations"
IMAGE_EDIT_ASYNC_URL = f"{IMAGE_2_API_BASE_URL}/api/llm/openai/v1/images/edits"
IMAGE_TASKS_BASE_URL = f"{IMAGE_2_API_BASE_URL}/api/llm/openai/v1/images/tasks"
DEFAULT_MODEL = os.environ.get("IMAGE_2_MODEL", "").strip()
DEFAULT_AGENT_ID = os.environ.get("IMAGE_2_AGENT_ID", "").strip()
try:
    MAX_REFERENCE_IMAGE_BYTES = max(int(os.environ.get("IMAGE_2_MAX_REFERENCE_BYTES", 25 * 1024 * 1024)), 1)
except (TypeError, ValueError):
    MAX_REFERENCE_IMAGE_BYTES = 25 * 1024 * 1024
try:
    MAX_REFERENCE_IMAGES = max(int(os.environ.get("IMAGE_2_MAX_REFERENCES", 8)), 1)
except (TypeError, ValueError):
    MAX_REFERENCE_IMAGES = 8
SUPPORTED_SIZE_PRESETS = {
    "1:1": "2048x2048",
    "2:3": "1360x2048",
    "3:2": "2048x1360",
    "3:4": "1536x2048",
    "4:3": "2048x1536",
    "4:5": "1632x2048",
    "5:4": "2048x1632",
    "9:16": "1152x2048",
    "16:9": "2048x1152",
    "21:9": "2048x864",
}
SUPPORTED_SIZES = set(SUPPORTED_SIZE_PRESETS.values())

TASK_SUCCESS_STATUSES = {"completed", "complete", "success", "succeeded"}
TASK_FAILURE_STATUSES = {"failed", "failure", "error", "canceled", "cancelled", "expired"}
SUPPORTED_REFERENCE_MIME_TYPES = {"image/jpeg", "image/png", "image/webp", "image/gif"}
SKILL_DIRECTORY = Path(__file__).resolve().parents[1]


def env_float(name, default):
    try:
        return float(str(os.environ.get(name) or "").strip())
    except (TypeError, ValueError):
        return float(default)


TASK_POLL_INTERVAL = max(env_float("IMAGE_2_TASK_POLL_INTERVAL", 3), 1)
TASK_POLL_TIMEOUT = max(env_float("IMAGE_2_TASK_POLL_TIMEOUT", 600), 30)


API_KEY = str(
    os.environ.get("IMAGE_2_API_KEY") or ""
).strip()


def validate_service_base_url(value):
    parsed = urllib.parse.urlparse(str(value or "").strip())
    if parsed.scheme != "https" or not parsed.hostname:
        raise RuntimeError("IMAGE_2_API_BASE_URL 必须是带主机名的 HTTPS URL。")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise RuntimeError("IMAGE_2_API_BASE_URL 不得包含凭据、查询参数或 fragment。")
    if parsed.path not in {"", "/"}:
        raise RuntimeError("IMAGE_2_API_BASE_URL 必须是服务根地址，不得包含 API 路径。")
    return parsed


def validate_remote_image_url(value):
    parsed = urllib.parse.urlparse(str(value or "").strip())
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.fragment
    ):
        raise RuntimeError("远程参考图必须是无内嵌凭据的 HTTPS URL。")
    hostname = parsed.hostname.casefold()
    if hostname == "localhost" or hostname.endswith(".localhost"):
        raise RuntimeError("拒绝从本机地址下载参考图。")
    try:
        address = ipaddress.ip_address(hostname.strip("[]"))
    except ValueError:
        address = None
    if address and (address.is_private or address.is_loopback or address.is_link_local or address.is_reserved or address.is_multicast):
        raise RuntimeError("拒绝从私有、环回、链路本地或保留地址下载参考图。")
    return parsed


def validate_public_dns_target(parsed):
    """拒绝解析到非公网地址的远程参考图主机。"""
    hostname = str(parsed.hostname or "").strip()
    if not hostname:
        raise RuntimeError("远程参考图 URL 缺少主机名。")
    port = parsed.port or 443
    try:
        addresses = {
            item[4][0]
            for item in socket.getaddrinfo(hostname, port, type=socket.SOCK_STREAM)
            if item and len(item) >= 5 and item[4]
        }
    except OSError as error:
        raise RuntimeError(f"无法解析远程参考图主机: {hostname}") from error
    if not addresses:
        raise RuntimeError(f"远程参考图主机没有可用地址: {hostname}")
    for raw_address in addresses:
        address = ipaddress.ip_address(str(raw_address).split("%", 1)[0])
        if not address.is_global:
            raise RuntimeError("拒绝从解析到私有、环回、链路本地或保留地址的主机下载参考图。")
    return addresses


class ValidatingImageRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        parsed = validate_remote_image_url(new_url)
        validate_public_dns_target(parsed)
        return super().redirect_request(request, file_pointer, code, message, headers, new_url)


def detect_raster_image_mime(image_bytes):
    if image_bytes.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if image_bytes.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if len(image_bytes) >= 12 and image_bytes.startswith(b"RIFF") and image_bytes[8:12] == b"WEBP":
        return "image/webp"
    if image_bytes.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    return ""


def validate_raster_image_bytes(image_bytes, source="参考图"):
    detected = detect_raster_image_mime(image_bytes)
    if detected not in SUPPORTED_REFERENCE_MIME_TYPES:
        raise RuntimeError(f"{source} 不是受支持的 PNG/JPEG/WEBP/GIF 图片。")
    return detected


def resolve_image_data_dir():
    explicit = str(os.environ.get("IMAGE_2_DATA_DIR") or "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    codex_data = str(os.environ.get("CODEX_DATA_DIR") or "").strip()
    if codex_data:
        return (Path(codex_data).expanduser() / "image-2").resolve()
    return (Path.cwd() / "codex-data" / "image-2").resolve()


def ensure_outside_installed_skill(path, label="数据路径"):
    resolved = Path(path).expanduser().resolve()
    try:
        resolved.relative_to(SKILL_DIRECTORY)
    except ValueError:
        return resolved
    raise RuntimeError(f"{label} 不得位于已安装 Skill 目录内: {resolved}")


def request_json(url, method="GET", headers=None, payload=None, timeout=600):
    body = None
    if payload is not None:
        if isinstance(payload, (dict, list)):
            body = json.dumps(payload).encode("utf-8")
        elif isinstance(payload, str):
            body = payload.encode("utf-8")
        else:
            body = payload

    request = urllib.request.Request(
        url,
        data=body,
        headers=headers or {},
        method=method,
    )

    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        message = f"HTTP Error {error.code}: {error.reason}"
        try:
            error_body = error.read().decode("utf-8")
            parsed = json.loads(error_body) if error_body else {}
            if isinstance(parsed, dict):
                message = parsed.get("error", {}).get("message") if isinstance(parsed.get("error"), dict) else None
                message = message or parsed.get("msg") or parsed.get("message") or f"HTTP Error {error.code}: {error.reason}"
        except Exception:
            pass
        raise RuntimeError(message) from error
    except urllib.error.URLError as error:
        raise RuntimeError(str(error.reason or error)) from error


def resolve_file_mime_type(file_path=""):
    mime_type, _ = mimetypes.guess_type(file_path)
    if mime_type:
        return mime_type

    extension = str(Path(file_path).suffix or "").strip().lower()
    if extension == ".jpg":
        return "image/jpeg"
    if extension == ".png":
        return "image/png"
    if extension == ".webp":
        return "image/webp"
    if extension == ".svg":
        return "image/svg+xml"
    if extension == ".gif":
        return "image/gif"
    return "application/octet-stream"


def guess_image_extension(image_bytes):
    if image_bytes.startswith(b"\x89PNG\r\n\x1a\n"):
        return ".png"
    if image_bytes.startswith(b"\xff\xd8\xff"):
        return ".jpg"
    if image_bytes.startswith(b"RIFF") and image_bytes[8:12] == b"WEBP":
        return ".webp"
    if image_bytes.startswith(b"GIF87a") or image_bytes.startswith(b"GIF89a"):
        return ".gif"
    return ".png"


def b64_image_to_output_file(b64_json):
    if not b64_json:
        raise RuntimeError("响应中缺少 b64_json")

    encoded = str(b64_json)
    if len(encoded) > ((MAX_REFERENCE_IMAGE_BYTES + 2) // 3) * 4 + 4:
        raise RuntimeError("响应中的 b64_json 超过图片大小限制")
    image_bytes = base64.b64decode(encoded, validate=False)
    if len(image_bytes) > MAX_REFERENCE_IMAGE_BYTES:
        raise RuntimeError("响应中的图片超过大小限制")
    validate_raster_image_bytes(image_bytes, source="响应内容")
    extension = guess_image_extension(image_bytes)
    output_dir = resolve_image_data_dir() / "outputs"
    output_dir = ensure_outside_installed_skill(output_dir, "图片输出目录")
    output_dir.mkdir(parents=True, exist_ok=True)
    target = output_dir / f"image-{uuid.uuid4().hex}{extension}"
    temp_file = tempfile.NamedTemporaryFile(delete=False, dir=output_dir, suffix=".tmp")
    try:
        temp_file.write(image_bytes)
        temp_file.flush()
        os.fsync(temp_file.fileno())
        temp_file.close()
        os.replace(temp_file.name, target)
    finally:
        if not temp_file.closed:
            temp_file.close()
        Path(temp_file.name).unlink(missing_ok=True)
    return str(target.resolve())


def normalize_agent_id(value):
    normalized = str(value or "").strip()
    if not normalized:
        raise RuntimeError("请求 image-2 时必须提供外部网关的 agent_id。")
    if len(normalized) > 256 or any(ord(char) < 32 or ord(char) == 127 for char in normalized):
        raise RuntimeError("agent_id 长度或字符无效。")
    return normalized


def normalize_idempotency_key(value):
    normalized = normalize_fingerprint(value)
    if len(normalized) != 64 or any(char not in "0123456789abcdef" for char in normalized):
        raise RuntimeError("正式图像请求缺少有效的确认指纹幂等键。")
    return normalized


def image_submission_record_path(idempotency_key):
    normalized = normalize_idempotency_key(idempotency_key)
    directory = ensure_outside_installed_skill(
        resolve_image_data_dir() / "submissions",
        "图像提交记录目录",
    )
    return directory / f"{normalized}.json"


def write_image_submission_record(path, record):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_file = tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        delete=False,
        dir=path.parent,
        suffix=".tmp",
    )
    try:
        json.dump(record, temp_file, ensure_ascii=False, indent=2, sort_keys=True)
        temp_file.flush()
        os.fsync(temp_file.fileno())
        temp_file.close()
        os.replace(temp_file.name, path)
    finally:
        if not temp_file.closed:
            temp_file.close()
        Path(temp_file.name).unlink(missing_ok=True)


def claim_image_submission(idempotency_key, confirmation_id=None):
    normalized = normalize_idempotency_key(idempotency_key)
    path = image_submission_record_path(normalized)
    path.parent.mkdir(parents=True, exist_ok=True)
    record = {
        "schema_version": 1,
        "idempotency_key": normalized,
        "confirmation_id": str(confirmation_id or "").strip() or None,
        "state": "submitting",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        try:
            existing = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise RuntimeError("已有图像提交记录无法读取；为避免重复计费，已拒绝再次 POST。") from error
        if not isinstance(existing, dict) or existing.get("idempotency_key") != normalized:
            raise RuntimeError("已有图像提交记录无效；为避免重复计费，已拒绝再次 POST。")
        return {"created": False, "path": path, "record": existing}
    with os.fdopen(descriptor, "w", encoding="utf-8") as file:
        json.dump(record, file, ensure_ascii=False, indent=2, sort_keys=True)
        file.flush()
        os.fsync(file.fileno())
    return {"created": True, "path": path, "record": record}


def update_image_submission(idempotency_key, state, **values):
    path = image_submission_record_path(idempotency_key)
    try:
        record = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError("无法更新图像提交记录；请勿重新提交该确认清单。") from error
    if not isinstance(record, dict) or record.get("idempotency_key") != normalize_idempotency_key(idempotency_key):
        raise RuntimeError("图像提交记录校验失败；请勿重新提交该确认清单。")
    record.update(values)
    record["state"] = state
    record["updated_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    write_image_submission_record(path, record)
    return record


def build_image_auth_headers(content_type=None, agent_id=None, idempotency_key=None):
    normalized_agent_id = normalize_agent_id(agent_id)
    normalized_idempotency_key = normalize_idempotency_key(idempotency_key) if idempotency_key else None
    headers = {
        "Accept": "application/json",
        "Authorization": f"Bearer {API_KEY}",
        "agent-id": normalized_agent_id,
        "X-Async": "true"
    }
    if normalized_idempotency_key:
        headers["Idempotency-Key"] = normalized_idempotency_key
    if content_type:
        headers["Content-Type"] = content_type
    return headers


def normalize_task_status(task):
    if not isinstance(task, dict):
        return ""
    return str(task.get("status") or "").strip().lower()


def extract_task_error(task):
    if not isinstance(task, dict):
        return "异步任务失败"
    for key in ("error", "message", "msg"):
        value = task.get(key)
        if isinstance(value, dict):
            nested = value.get("message") or value.get("msg")
            if nested:
                return str(nested)
        elif value:
            return str(value)
    result = task.get("result")
    if isinstance(result, dict):
        error = result.get("error")
        if isinstance(error, dict) and (error.get("message") or error.get("msg")):
            return str(error.get("message") or error.get("msg"))
        if isinstance(error, str) and error:
            return error
    return f"异步任务失败（status={task.get('status')}）"


def submit_image_task(url, payload, content_type="application/json", agent_id=None,
                      idempotency_key=None, confirmation_id=None):
    normalized_key = normalize_idempotency_key(idempotency_key)
    claim = claim_image_submission(normalized_key, confirmation_id=confirmation_id)
    if not claim["created"]:
        existing = claim["record"]
        state = str(existing.get("state") or "").strip().lower()
        if state == "completed" and isinstance(existing.get("result"), dict):
            return {"kind": "completed", "result": existing["result"], "deduplicated": True}
        if state == "task_created" and str(existing.get("task_id") or "").strip():
            return {
                "kind": "async",
                "task_id": str(existing["task_id"]).strip(),
                "deduplicated": True,
            }
        raise RuntimeError(
            f"该确认清单已有状态为 {state or 'unknown'} 的提交记录；"
            "为避免重复计费，已拒绝再次 POST。"
        )

    try:
        response_json = request_json(
            url,
            method="POST",
            headers=build_image_auth_headers(
                content_type=content_type,
                agent_id=agent_id,
                idempotency_key=normalized_key,
            ),
            payload=payload,
            timeout=120,
        )
    except Exception:
        update_image_submission(normalized_key, "submission_unknown")
        raise

    # 检查是否为同步响应（直接返回 data[].url/b64_json 或 image_url）
    data_list = response_json.get("data")
    if isinstance(data_list, list) and len(data_list) > 0:
        first_item = data_list[0]
        if isinstance(first_item, dict) and (first_item.get("b64_json") or first_item.get("url")):
            update_image_submission(normalized_key, "sync_received")
            print("同步响应，直接处理图片结果...", file=sys.stderr)
            return {"kind": "sync", "response": response_json}
    if str(response_json.get("image_url") or "").strip():
        update_image_submission(normalized_key, "sync_received")
        return {"kind": "sync", "response": response_json}
    task_id = str(response_json.get("task_id") or response_json.get("id") or "").strip()
    if not task_id:
        update_image_submission(normalized_key, "submission_rejected")
        brief = json.dumps(response_json, ensure_ascii=False)[:300]
        raise RuntimeError(f"异步任务创建失败，响应缺少 task_id: {brief}")
    if normalize_task_status(response_json) in TASK_FAILURE_STATUSES:
        update_image_submission(normalized_key, "submission_rejected", task_id=task_id)
        raise RuntimeError(extract_task_error(response_json))
    update_image_submission(normalized_key, "task_created", task_id=task_id)
    print(f"异步任务已创建: {task_id}，等待生成完成...", file=sys.stderr)
    return {"kind": "async", "task_id": task_id}


def wait_for_image_task(task_id, agent_id=None):
    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id or len(normalized_task_id) > 512:
        raise RuntimeError("图像服务返回了无效的 task_id。")
    poll_url = f"{IMAGE_TASKS_BASE_URL}/{urllib.parse.quote(normalized_task_id, safe='')}"
    headers = build_image_auth_headers(agent_id=agent_id)
    started = time.monotonic()
    deadline = started + TASK_POLL_TIMEOUT
    last_status = ""
    last_report = -15.0

    while True:
        task = request_json(poll_url, method="GET", headers=headers, timeout=60)
        status = normalize_task_status(task)
        if status in TASK_SUCCESS_STATUSES:
            print(f"图片生成完成: {task_id}", file=sys.stderr)
            return task
        if status in TASK_FAILURE_STATUSES:
            raise RuntimeError(extract_task_error(task))

        elapsed = time.monotonic() - started
        if time.monotonic() >= deadline:
            raise RuntimeError(f"等待图片生成超时（{int(TASK_POLL_TIMEOUT)} 秒），task_id={task_id}")
        if status != last_status or elapsed - last_report >= 15:
            print(f"图片生成中... status={status or 'unknown'} 已等待 {int(elapsed)}s", file=sys.stderr)
            last_status = status
            last_report = elapsed
        time.sleep(TASK_POLL_INTERVAL)


def extract_images_from_data_list(data_list):
    images = []
    if not isinstance(data_list, list):
        return images
    for item in data_list:
        if not isinstance(item, dict):
            continue
        image_url = str(item.get("url") or "").strip()
        if image_url:
            images.append(image_url)
            continue
        if item.get("b64_json"):
            # 兜底：个别模型可能仍返回 b64_json，解码为本地临时文件
            images.append(b64_image_to_output_file(item["b64_json"]))
    return images


def collect_task_images(task):
    result = task.get("result") if isinstance(task, dict) else None
    if not isinstance(result, dict):
        result = {}

    images = extract_images_from_data_list(result.get("data"))
    if isinstance(task, dict):
        images.extend(extract_images_from_data_list(task.get("data")))
    top_level_url = str(task.get("image_url") or "").strip() if isinstance(task, dict) else ""
    if top_level_url:
        images.append(top_level_url)

    images = dedupe_images(images)
    usage = result.get("usage")
    created = result.get("created") or task.get("completed_at") or task.get("created_at")
    return images, usage, created


def is_image_url(data):
    """检查数据是否为有效的图片URL"""
    if not data or not isinstance(data, str):
        return False
    if data.startswith("https://"):
        try:
            validate_remote_image_url(data)
            return True
        except Exception:
            return False
    return False


def download_image_to_tempfile(url):
    """下载远程图片到临时文件，返回临时文件路径"""
    parsed = validate_remote_image_url(url)
    validate_public_dns_target(parsed)
    request = urllib.request.Request(
        url,
        headers={"Accept": "*/*", "User-Agent": "image-2-skill/1.0"},
        method="GET",
    )
    opener = urllib.request.build_opener(ValidatingImageRedirectHandler())
    with opener.open(request, timeout=600) as response:
        final_parsed = validate_remote_image_url(response.geturl())
        validate_public_dns_target(final_parsed)
        declared_length = response.headers.get("Content-Length")
        if declared_length and int(declared_length) > MAX_REFERENCE_IMAGE_BYTES:
            raise RuntimeError(f"远程参考图超过大小限制（{MAX_REFERENCE_IMAGE_BYTES} bytes）。")
        content = response.read(MAX_REFERENCE_IMAGE_BYTES + 1)
        if len(content) > MAX_REFERENCE_IMAGE_BYTES:
            raise RuntimeError(f"远程参考图超过大小限制（{MAX_REFERENCE_IMAGE_BYTES} bytes）。")
        content_type = str(response.headers.get("Content-Type", "")).split(";", 1)[0].strip().lower()

    detected_type = validate_raster_image_bytes(content, source="远程参考图")
    if content_type and content_type not in SUPPORTED_REFERENCE_MIME_TYPES:
        raise RuntimeError(f"远程参考图 Content-Type 不受支持: {content_type}")
    if content_type and content_type != detected_type:
        raise RuntimeError("远程参考图的 Content-Type 与文件内容不一致。")

    # 从 URL 或 Content-Type 推断扩展名
    parsed_url = urllib.parse.urlparse(str(url or "").strip())
    url_suffix = Path(urllib.parse.unquote(parsed_url.path)).suffix.lower()
    extension = url_suffix if url_suffix in {".jpg", ".jpeg", ".png", ".webp", ".gif"} else ""
    if not extension:
        ct = content_type or detected_type
        ct_map = {
            "image/jpeg": ".jpg", "image/png": ".png",
            "image/webp": ".webp", "image/gif": ".gif",
        }
        extension = ct_map.get(ct, "") or mimetypes.guess_extension(ct) or ".png"

    temp_file = tempfile.NamedTemporaryFile(delete=False, suffix=extension)
    try:
        temp_file.write(content)
        temp_file.flush()
        return temp_file.name
    finally:
        temp_file.close()


def dedupe_images(images):
    """去重图片 URL"""
    deduped = []
    seen = set()
    for image in images:
        if image in seen:
            continue
        seen.add(image)
        deduped.append(image)
    return deduped


def build_multipart_form(fields, files):
    boundary = f"----codex-image-2-{uuid.uuid4().hex}"
    parts = []

    for field_name, field_value in fields:
        parts.append(f"--{boundary}\r\n".encode("utf-8"))
        parts.append(
            f'Content-Disposition: form-data; name="{field_name}"\r\n\r\n{field_value}\r\n'.encode("utf-8")
        )

    for file_index, (field_name, file_path) in enumerate(files, start=1):
        target_path = Path(file_path).expanduser()
        if not target_path.exists() or not target_path.is_file():
            raise RuntimeError(f"参考图片不存在: {target_path}")

        if target_path.stat().st_size > MAX_REFERENCE_IMAGE_BYTES:
            raise RuntimeError(f"参考图片超过大小限制（{MAX_REFERENCE_IMAGE_BYTES} bytes）: {target_path}")
        file_bytes = target_path.read_bytes()
        file_mime_type = validate_raster_image_bytes(file_bytes, source=f"参考图片 {target_path}")
        extension = {
            "image/jpeg": ".jpg",
            "image/png": ".png",
            "image/webp": ".webp",
            "image/gif": ".gif",
        }[file_mime_type]
        file_name = f"reference-{file_index}{extension}"

        parts.append(f"--{boundary}\r\n".encode("utf-8"))
        parts.append(
            (
                f'Content-Disposition: form-data; name="{field_name}"; filename="{file_name}"\r\n'
                f"Content-Type: {file_mime_type}\r\n\r\n"
            ).encode("utf-8")
        )
        parts.append(file_bytes)
        parts.append(b"\r\n")

    parts.append(f"--{boundary}--\r\n".encode("utf-8"))
    return boundary, b"".join(parts)


def validate_size(size):
    if size not in SUPPORTED_SIZES:
        allowed = ", ".join(
            f"{ratio}={preset}" for ratio, preset in SUPPORTED_SIZE_PRESETS.items()
        )
        raise ValueError(f"不支持的尺寸 '{size}'。当前仅支持以下比例与像素：{allowed}")
    return size


def generate_image(prompt, size="2048x2048", model=DEFAULT_MODEL, agent_id=None,
                   idempotency_key=None, confirmation_id=None):
    if not IMAGE_2_API_BASE_URL:
        return {"error": "缺少 IMAGE_2_API_BASE_URL；请先配置实际服务端点。"}
    if not API_KEY:
        return {"error": "缺少 IMAGE_2_API_KEY；请通过环境变量显式配置此服务的专用凭据。"}
    if not str(model or "").strip():
        return {"error": "缺少 IMAGE_2_MODEL；不得猜测服务端模型名称。"}
    try:
        validate_service_base_url(IMAGE_2_API_BASE_URL)
    except RuntimeError as error:
        return {"error": str(error)}

    size = validate_size(size)

    payload = {
        "model": model,
        "prompt": prompt,
        "size": size,
    }

    try:
        submission = submit_image_task(
            IMAGE_GENERATION_ASYNC_URL,
            payload,
            agent_id=agent_id,
            idempotency_key=idempotency_key,
            confirmation_id=confirmation_id,
        )
        if submission["kind"] == "completed":
            result = dict(submission["result"])
            result["api_request_sent"] = False
            result["deduplicated"] = True
            return result
        if submission["kind"] == "sync":
            task_id = "sync"
            task = submission["response"]
        else:
            task_id = submission["task_id"]
            task = wait_for_image_task(task_id, agent_id=agent_id)

        images, usage, created = collect_task_images(task)
        if not images:
            return {"error": "Failed to extract image url from completed task"}

        result = {
            "success": True,
            "mode": "generation",
            "task_id": task_id,
            "images": images,
            "usage": usage,
            "created": created,
            "api_request_sent": not bool(submission.get("deduplicated")),
        }
        update_image_submission(idempotency_key, "completed", result=result)
        return result
    except Exception as error:
        return {"error": str(error)}


def edit_image(prompt, image_paths, model=DEFAULT_MODEL, size=None, agent_id=None,
               idempotency_key=None, confirmation_id=None):
    if not IMAGE_2_API_BASE_URL:
        return {"error": "缺少 IMAGE_2_API_BASE_URL；请先配置实际服务端点。"}
    if not API_KEY:
        return {"error": "缺少 IMAGE_2_API_KEY；请通过环境变量显式配置此服务的专用凭据。"}
    if not str(model or "").strip():
        return {"error": "缺少 IMAGE_2_MODEL；不得猜测服务端模型名称。"}
    try:
        validate_service_base_url(IMAGE_2_API_BASE_URL)
    except RuntimeError as error:
        return {"error": str(error)}
    if not image_paths:
        return {"error": "图生图需要至少一个 --image 参数"}

    try:
        form_fields = [
            ("model", model),
            ("prompt", prompt),
        ]
        if size:
            size = validate_size(size)
            form_fields.append(("size", size))

        boundary, body = build_multipart_form(
            fields=form_fields,
            files=[
                (f"image[{index}]", image_path)
                for index, image_path in enumerate(image_paths)
            ],
        )

        submission = submit_image_task(
            IMAGE_EDIT_ASYNC_URL,
            body,
            content_type=f"multipart/form-data; boundary={boundary}",
            agent_id=agent_id,
            idempotency_key=idempotency_key,
            confirmation_id=confirmation_id,
        )
        if submission["kind"] == "completed":
            result = dict(submission["result"])
            result["api_request_sent"] = False
            result["deduplicated"] = True
            return result
        if submission["kind"] == "sync":
            task_id = "sync"
            task = submission["response"]
        else:
            task_id = submission["task_id"]
            task = wait_for_image_task(task_id, agent_id=agent_id)

        images, usage, created = collect_task_images(task)
        if not images:
            return {"error": "Failed to extract image url from completed task"}

        result = {
            "success": True,
            "mode": "edit",
            "task_id": task_id,
            "images": images,
            "usage": usage,
            "created": created,
            "api_request_sent": not bool(submission.get("deduplicated")),
        }
        update_image_submission(idempotency_key, "completed", result=result)
        return result
    except Exception as error:
        return {"error": str(error)}


# ---- 两阶段确认机制（参考 seedance 交互模式）----

CONFIRMATION_MANIFEST_VERSION = 3
CONFIRMATION_DIR = resolve_image_data_dir() / ".confirmations"


def sha256_text(value):
    return hashlib.sha256(str(value or "").encode("utf-8")).hexdigest()


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def normalize_confirmation_images(images):
    if len(images or []) > MAX_REFERENCE_IMAGES:
        raise ValueError(f"参考图数量超过上限（{MAX_REFERENCE_IMAGES}）。")
    normalized_images = []
    integrity = []
    for index, raw_image in enumerate(images or [], start=1):
        value = str(raw_image or "").strip()
        if not value:
            raise ValueError(f"参考图 {index} 为空。")
        if is_image_url(value):
            normalized_images.append(value)
            integrity.append({
                "kind": "url",
                "locator": value,
                "content_pinned": False,
            })
            continue
        if "://" in value:
            raise ValueError(f"参考图 {index} 的远程地址必须使用 HTTPS，且不得指向本机或私有地址。")
        path = Path(value).expanduser().resolve()
        if not path.is_file():
            raise ValueError(f"参考图片不存在: {path}")
        size = path.stat().st_size
        if size > MAX_REFERENCE_IMAGE_BYTES:
            raise ValueError(f"参考图片超过大小限制（{MAX_REFERENCE_IMAGE_BYTES} bytes）: {path}")
        with open(path, "rb") as file:
            validate_raster_image_bytes(file.read(16), source=f"参考图片 {path}")
        normalized = str(path)
        normalized_images.append(normalized)
        integrity.append({
            "kind": "local-file",
            "locator": normalized,
            "size_bytes": size,
            "sha256": sha256_file(path),
            "content_pinned": True,
        })
    return normalized_images, integrity


def verify_confirmation_images(request):
    images = list(request.get("images") or [])
    integrity = request.get("image_integrity")
    if not isinstance(integrity, list) or len(integrity) != len(images):
        raise RuntimeError("确认清单缺少完整的参考图完整性记录；请重新准备并确认。")
    for index, (image, record) in enumerate(zip(images, integrity), start=1):
        if not isinstance(record, dict) or record.get("locator") != image:
            raise RuntimeError(f"参考图 {index} 的定位信息与确认清单不一致。")
        kind = record.get("kind")
        if kind == "url":
            validate_remote_image_url(image)
            if record.get("content_pinned") is not False:
                raise RuntimeError(f"参考图 {index} 的 URL 完整性标记无效。")
            continue
        if kind != "local-file":
            raise RuntimeError(f"参考图 {index} 的完整性记录类型无效。")
        path = Path(image).expanduser().resolve()
        if not path.is_file():
            raise RuntimeError(f"已确认的本地参考图不存在: {path}")
        if path.stat().st_size != record.get("size_bytes"):
            raise RuntimeError(f"已确认的本地参考图大小发生变化: {path}")
        actual_hash = sha256_file(path)
        expected_hash = str(record.get("sha256") or "").strip().lower()
        if not expected_hash or not hmac.compare_digest(actual_hash, expected_hash):
            raise RuntimeError(f"已确认的本地参考图内容发生变化: {path}")
        with open(path, "rb") as file:
            validate_raster_image_bytes(file.read(16), source=f"参考图片 {path}")


def canonical_json_bytes(value):
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def normalize_fingerprint(value):
    normalized = str(value or "").strip().lower()
    if normalized.startswith("sha256:"):
        normalized = normalized.split(":", 1)[1]
    return normalized


def calculate_confirmation_fingerprint(manifest):
    unsigned = {key: value for key, value in manifest.items() if key != "fingerprint"}
    return hashlib.sha256(canonical_json_bytes(unsigned)).hexdigest()


def build_confirmation_manifest(prompt, images, size, model, agent_id=None):
    normalized_prompt = str(prompt or "")
    normalized_images, image_integrity = normalize_confirmation_images(images)
    normalized_agent_id = normalize_agent_id(agent_id)
    normalized_model = str(model or "").strip()
    if not normalized_model:
        raise ValueError("准备确认清单时必须通过 IMAGE_2_MODEL 或 --model 提供实际模型名称")
    manifest = {
        "version": CONFIRMATION_MANIFEST_VERSION,
        "confirmation_id": f"image2-{uuid.uuid4().hex}",
        "created_at": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime()),
        "request": {
            "agent_id": normalized_agent_id,
            "mode": "edit" if normalized_images else "generation",
            "prompt": normalized_prompt,
            "prompt_sha256": sha256_text(normalized_prompt),
            "images": normalized_images,
            "image_count": len(normalized_images),
            "image_integrity": image_integrity,
            "size": str(size or ""),
            "model": normalized_model,
        },
    }
    manifest["fingerprint"] = calculate_confirmation_fingerprint(manifest)
    return manifest


def write_confirmation_manifest(manifest, confirmation_output=None):
    if confirmation_output:
        target_path = Path(confirmation_output).expanduser().resolve()
        if target_path.is_dir():
            target_path = target_path / f"{manifest['confirmation_id']}.json"
    else:
        target_dir = CONFIRMATION_DIR.resolve()
        target_dir.mkdir(parents=True, exist_ok=True)
        target_path = target_dir / f"{manifest['confirmation_id']}.json"

    target_path = target_path.expanduser().resolve()
    target_path = ensure_outside_installed_skill(target_path, "确认清单路径")
    if target_path.exists():
        raise RuntimeError(
            f"确认清单文件已存在，为避免覆盖已确认输入已拒绝写入: {target_path}"
        )
    target_path.parent.mkdir(parents=True, exist_ok=True)
    temp_file = tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        delete=False,
        dir=target_path.parent,
        suffix=".tmp",
    )
    try:
        json.dump(manifest, temp_file, ensure_ascii=False, indent=2, sort_keys=True)
        temp_file.flush()
        os.fsync(temp_file.fileno())
        temp_file.close()
        os.replace(temp_file.name, target_path)
    finally:
        if not temp_file.closed:
            temp_file.close()
        Path(temp_file.name).unlink(missing_ok=True)
    try:
        target_path.chmod(0o600)
    except OSError:
        pass
    return target_path


def load_confirmation_manifest(confirmation_file):
    path = Path(str(confirmation_file or "")).expanduser().resolve()
    if not path.exists():
        raise RuntimeError(f"确认清单文件不存在: {path}")
    with open(path, "r", encoding="utf-8") as file:
        manifest = json.load(file)
    if not isinstance(manifest, dict):
        raise RuntimeError("确认清单根对象必须是 JSON object")
    return manifest


def validate_confirmation_manifest(manifest, expected_fingerprint=None):
    if manifest.get("version") != CONFIRMATION_MANIFEST_VERSION:
        raise RuntimeError(f"不支持的确认清单版本: {manifest.get('version')}")
    stored = normalize_fingerprint(manifest.get("fingerprint"))
    calculated = calculate_confirmation_fingerprint(manifest)
    if not stored or not hmac.compare_digest(stored, calculated):
        raise RuntimeError(
            "确认清单指纹校验失败：prompt、参考图或参数在准备后可能被修改。"
            "请重新准备并取得新确认，不要继续生成。"
        )
    expected = normalize_fingerprint(expected_fingerprint)
    if expected and not hmac.compare_digest(expected, stored):
        raise RuntimeError("传入的 --confirm-fingerprint 与清单指纹不一致，拒绝执行。")
    request = manifest.get("request")
    if not isinstance(request, dict) or not str(request.get("agent_id") or "").strip():
        raise RuntimeError("确认清单缺少 agent_id，必须重新准备并取得新确认。")
    verify_confirmation_images(request)
    return manifest


def prepare_confirmation(prompt, images, size, model, agent_id=None, confirmation_output=None):
    validated_size = validate_size(size)
    manifest = build_confirmation_manifest(
        prompt=prompt,
        images=images,
        size=validated_size,
        model=model,
        agent_id=agent_id,
    )
    target_path = write_confirmation_manifest(manifest, confirmation_output=confirmation_output)
    request = manifest["request"]

    warnings = []
    for index, record in enumerate(request["image_integrity"], start=1):
        if record.get("kind") == "url":
            warnings.append(
                f"参考图 {index} 是远程 URL：清单会锁定 URL 字符串，但无法离线锁定远端字节内容。"
            )

    return {
        "success": True,
        "confirmation_id": manifest["confirmation_id"],
        "confirmation_file": str(target_path),
        "confirmation_fingerprint": manifest["fingerprint"],
        "api_request_sent": False,
        "mode": request["mode"],
        "prompt": request["prompt"],
        "prompt_sha256": request["prompt_sha256"],
        "agent_id": request["agent_id"],
        "images": request["images"],
        "image_count": request["image_count"],
        "size": request["size"],
        "model": request["model"],
        "warnings": warnings,
    }


def run_with_confirmation(confirmation_file, confirm_fingerprint, dry_run=False):
    manifest = load_confirmation_manifest(confirmation_file)
    validate_confirmation_manifest(manifest, expected_fingerprint=confirm_fingerprint)
    request = manifest.get("request") or {}
    mode = request.get("mode") or ("edit" if request.get("images") else "generation")
    prompt = request.get("prompt", "")
    agent_id = str(request.get("agent_id") or "").strip()
    images = list(request.get("images") or [])
    size = request.get("size") or "2048x2048"
    model = str(request.get("model") or "").strip()
    idempotency_key = str(manifest.get("fingerprint") or "").strip()
    confirmation_id = str(manifest.get("confirmation_id") or "").strip()
    if not model:
        return {"error": "确认清单缺少模型名称；请重新准备并取得新确认。"}

    if dry_run:
        return {
            "success": True,
            "dry_run": True,
            "api_request_sent": False,
            "confirmation_id": manifest.get("confirmation_id"),
            "confirmation_fingerprint": manifest.get("fingerprint"),
            "agent_id": agent_id,
            "mode": mode,
            "prompt": prompt,
            "prompt_sha256": request.get("prompt_sha256"),
            "images": images,
            "image_count": len(images),
            "size": size,
            "model": model,
        }

    if not IMAGE_2_API_BASE_URL:
        return {"error": "缺少 IMAGE_2_API_BASE_URL；请先配置实际服务端点。"}
    if not API_KEY:
        return {"error": "缺少 IMAGE_2_API_KEY；请通过环境变量显式配置此服务的专用凭据。"}
    try:
        validate_service_base_url(IMAGE_2_API_BASE_URL)
    except RuntimeError as error:
        return {"error": str(error)}

    temp_files = []
    local_image_paths = []
    try:
        for img in images:
            if is_image_url(img):
                temp_path = download_image_to_tempfile(img)
                temp_files.append(temp_path)
                local_image_paths.append(temp_path)
            else:
                local_image_paths.append(img)

        if mode == "edit":
            if not local_image_paths:
                return {"error": "确认清单为图生图模式但无可用参考图"}
            verify_confirmation_images(request)
            result = edit_image(
                prompt,
                local_image_paths,
                model=model,
                size=size,
                agent_id=agent_id,
                idempotency_key=idempotency_key,
                confirmation_id=confirmation_id,
            )
        else:
            result = generate_image(
                prompt,
                size=size,
                model=model,
                agent_id=agent_id,
                idempotency_key=idempotency_key,
                confirmation_id=confirmation_id,
            )
    except Exception as error:
        result = {"error": str(error)}
    finally:
        for temp_path in temp_files:
            try:
                Path(temp_path).unlink(missing_ok=True)
            except Exception:
                pass

    if isinstance(result, dict) and result.get("success"):
        result["confirmation_id"] = manifest.get("confirmation_id")
        result["confirmation_fingerprint"] = manifest.get("fingerprint")
    return result


def main():
    parser = argparse.ArgumentParser(
        description="Image-2 图像生成与编辑（异步任务 + 两阶段用户确认）",
        formatter_class=argparse.RawTextHelpFormatter,
    )
    parser.add_argument(
        "prompt",
        type=str,
        nargs="?",
        default=None,
        help="图片描述文字或编辑要求；准备阶段必填，正式生成阶段不传",
    )
    parser.add_argument(
        "--image",
        action="append",
        default=[],
        metavar="PATH_OR_URL",
        help="参考图本地路径或 URL，可重复；仅准备阶段使用，按上传顺序保留",
    )
    parser.add_argument(
        "--agent-id",
        type=str,
        default=None,
        help="外部图像网关要求的路由 ID；准备阶段可省略并使用 IMAGE_2_AGENT_ID，正式阶段从清单读取",
    )
    parser.add_argument(
        "--size",
        type=str,
        default="2048x2048",
        help="尺寸，仅支持 1:1/2:3/3:2/3:4/4:3/4:5/5:4/9:16/16:9/21:9 对应的 2K 预设像素",
    )
    parser.add_argument("--model", type=str, default=DEFAULT_MODEL, help="模型名称")
    parser.add_argument("--json", action="store_true", help="以纯 JSON 格式输出结果")

    parser.add_argument(
        "--prepare-confirmation",
        action="store_true",
        help="离线固化 prompt/参考图/参数并生成确认清单，不请求 API",
    )
    parser.add_argument("--confirmation-file", type=str, default=None, help="使用已确认清单正式生成或 dry-run")
    parser.add_argument("--confirm-fingerprint", type=str, default=None, help="准备阶段返回、仅供执行器内部校验的完整 SHA-256 指纹")
    parser.add_argument("--confirmation-output", type=str, default=None, help="准备阶段确认清单的输出目录或文件路径")
    parser.add_argument("--confirm", action="store_true", help="正式生成必填，表示用户已确认当前清单")
    parser.add_argument("--dry-run", action="store_true", help="校验确认清单并展示将提交的输入，不请求 API")

    args = parser.parse_args()

    # 模式一：离线准备确认清单（不请求 API）
    if args.prepare_confirmation:
        if not args.prompt:
            print("错误：准备确认清单时必须提供 prompt", file=sys.stderr)
            return 1
        effective_agent_id = str(args.agent_id or DEFAULT_AGENT_ID).strip()
        if not effective_agent_id:
            print("错误：准备确认清单时必须提供 --agent-id 或配置 IMAGE_2_AGENT_ID", file=sys.stderr)
            return 1
        images = list(args.image)
        try:
            result = prepare_confirmation(
                prompt=args.prompt,
                images=images,
                size=args.size,
                model=args.model,
                agent_id=effective_agent_id,
                confirmation_output=args.confirmation_output,
            )
        except Exception as e:
            result = {"error": str(e)}
        print(json.dumps(result, ensure_ascii=False))
        return 1 if "error" in result else 0

    # 模式二：使用已确认清单正式生成（或 dry-run）
    if args.confirmation_file:
        if args.prompt or args.image or args.agent_id:
            print(
                "错误：使用 --confirmation-file 时禁止再传 prompt、--image 或 --agent-id，已确认输入不可变（尺寸/模型以清单为准）",
                file=sys.stderr,
            )
            return 1
        if not args.confirm_fingerprint:
            print(
                "错误：使用 --confirmation-file 必须传 --confirm-fingerprint（与确认清单一致的 SHA-256 指纹）",
                file=sys.stderr,
            )
            return 1
        if args.dry_run:
            if args.confirm:
                print("错误：--dry-run 不得携带 --confirm（dry-run 不正式生成）", file=sys.stderr)
                return 1
        elif not args.confirm:
            print("错误：正式生成必须传 --confirm 表示用户已确认当前清单", file=sys.stderr)
            return 1
        result = run_with_confirmation(
            confirmation_file=args.confirmation_file,
            confirm_fingerprint=args.confirm_fingerprint,
            dry_run=args.dry_run,
        )
        if args.json:
            print(json.dumps(result, ensure_ascii=False))
        else:
            if "success" in result:
                for url in result.get("images", []):
                    print(url)
            else:
                print(f"错误: {result.get('error')}")
        return 1 if "error" in result else 0

    # 未走两阶段确认：拒绝直接生成
    parser.print_help(sys.stderr)
    print(
        "\n错误：必须先 --prepare-confirmation 准备确认清单并取得用户确认，"
        "再用 --confirmation-file 正式生成。禁止直接传 prompt/--image 生成。",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
