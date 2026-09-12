import sys
import json
import hashlib
import hmac
import mimetypes
import re
import urllib.request
import urllib.error
import urllib.parse
import argparse
import tempfile
import time
import uuid
import os
import calendar
import ipaddress
import socket
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

if sys.platform == "win32":
    import msvcrt
else:
    import fcntl

# API credentials are explicit environment variables only.  Preparation and dry-run
# remain usable without them; real submission/query requires both values.
API_KEY = os.environ.get("SEEDANCE_API_KEY", "").strip()
API_KEY_LOAD_ERROR = None if API_KEY else "缺少 SEEDANCE_API_KEY 环境变量。"
SEEDANCE_MODEL = os.environ.get("SEEDANCE_MODEL", "").strip()
DEFAULT_AGENT_ID = os.environ.get("SEEDANCE_AGENT_ID", "").strip()
SKILL_DIRECTORY = Path(__file__).resolve().parents[1]


def parse_json_response(raw):
    if not raw:
        return {}

    text = raw.strip()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        decoder = json.JSONDecoder()
        values = []
        index = 0
        while index < len(text):
            while index < len(text) and text[index].isspace():
                index += 1
            if index >= len(text):
                break
            try:
                value, next_index = decoder.raw_decode(text, index)
            except json.JSONDecodeError:
                if values:
                    break
                raise
            values.append(value)
            index = next_index

        if not values:
            raise
        for value in reversed(values):
            if isinstance(value, dict):
                return value
        return values[-1]


def print_progress(*args, **kwargs):
    print(*args, file=sys.stderr, **kwargs)


def request_json(url, method="GET", headers=None, payload=None, timeout=60):
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
            return parse_json_response(raw)
    except urllib.error.HTTPError as error:
        # 保留 API 返回的原始错误格式
        try:
            error_body = error.read().decode("utf-8")
            parsed = parse_json_response(error_body)
            if not isinstance(parsed, dict):
                parsed = {
                    "error": {
                        "code": f"HTTP_{error.code}",
                        "message": error.reason,
                        "type": "HTTPError",
                    }
                }
            # 无论 API 是否返回 JSON，都保留传输层状态供幂等判断使用。
            parsed = dict(parsed)
            parsed.setdefault("_http_status", error.code)
            parsed.setdefault("_response_received", True)
            return parsed
        except Exception:
            # 如果无法解析错误响应，返回基础错误信息
            return {
                "error": {"code": f"HTTP_{error.code}", "message": error.reason, "type": "HTTPError"},
                "_http_status": error.code,
                "_response_received": True,
            }
    except urllib.error.URLError as error:
        return {
            "error": {"code": "NetworkError", "message": str(error.reason or error), "type": "URLError"},
            "_request_may_have_been_sent": method.upper() == "POST",
        }


def validate_media_url(value):
    parsed = urllib.parse.urlparse(str(value or "").strip())
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.fragment
    ):
        raise ValueError("媒体必须是无内嵌凭据和 fragment 的 HTTPS URL。")
    hostname = parsed.hostname.casefold()
    if hostname == "localhost" or hostname.endswith(".localhost"):
        raise ValueError("媒体 URL 不得指向本机。")
    try:
        address = ipaddress.ip_address(hostname.strip("[]"))
    except ValueError:
        address = None
    if address and not address.is_global:
        raise ValueError("媒体 URL 不得使用私有、环回、链路本地或保留 IP 地址。")
    return parsed


def validate_public_dns_target(parsed):
    hostname = str(parsed.hostname or "").strip()
    if not hostname:
        raise ValueError("媒体 URL 缺少主机名。")
    try:
        addresses = {
            item[4][0]
            for item in socket.getaddrinfo(hostname, parsed.port or 443, type=socket.SOCK_STREAM)
            if item and len(item) >= 5 and item[4]
        }
    except OSError as error:
        raise ValueError(f"无法解析媒体主机: {hostname}") from error
    if not addresses:
        raise ValueError(f"媒体主机没有可用地址: {hostname}")
    for raw_address in addresses:
        address = ipaddress.ip_address(str(raw_address).split("%", 1)[0])
        if not address.is_global:
            raise ValueError("媒体主机解析到私有、环回、链路本地或保留地址，已拒绝下载。")
    return addresses


class ValidatingMediaRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        parsed = validate_media_url(new_url)
        validate_public_dns_target(parsed)
        return super().redirect_request(request, file_pointer, code, message, headers, new_url)


def is_image_url(data):
    try:
        validate_media_url(data)
        return True
    except (TypeError, ValueError):
        return False


def is_video_url(data):
    try:
        validate_media_url(data)
        return True
    except (TypeError, ValueError):
        return False


def is_audio_url(data):
    try:
        validate_media_url(data)
        return True
    except (TypeError, ValueError):
        return False


def validate_service_base_url(value):
    parsed = urllib.parse.urlparse(str(value or "").strip())
    if parsed.scheme != "https" or not parsed.hostname:
        raise ValueError("SEEDANCE_API_BASE_URL 必须是带主机名的 HTTPS URL。")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("SEEDANCE_API_BASE_URL 不得包含凭据、查询参数或 fragment。")
    if parsed.path not in {"", "/"}:
        raise ValueError("SEEDANCE_API_BASE_URL 必须是服务根地址，不得包含 API 路径。")
    return parsed


def ensure_outside_installed_skill(path, label="数据路径"):
    resolved = Path(path).expanduser().resolve()
    try:
        resolved.relative_to(SKILL_DIRECTORY)
    except ValueError:
        return resolved
    raise ValueError(f"{label} 不得位于已安装 Skill 目录内: {resolved}")


def resolve_seedance_data_dir():
    explicit = str(os.environ.get("SEEDANCE_DATA_DIR") or "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    codex_data = str(os.environ.get("CODEX_DATA_DIR") or "").strip()
    if codex_data:
        return (Path(codex_data).expanduser() / "seedance2-0-video-gen").resolve()
    return (Path.cwd() / "codex-data" / "seedance2-0-video-gen").resolve()


def positive_int_environment(name):
    raw = str(os.environ.get(name) or "").strip()
    try:
        value = int(raw)
    except (TypeError, ValueError):
        return None
    return value if value > 0 else None


def adapter_policy_configuration_error():
    missing = []
    if not SEEDANCE_MODEL:
        missing.append("SEEDANCE_MODEL")
    if VIDEO_GENERATION_PRECHARGE_POINTS is None:
        missing.append("SEEDANCE_PRECHARGE_POINTS")
    if OFFICIAL_LINK_TTL_HOURS is None:
        missing.append("SEEDANCE_OFFICIAL_LINK_TTL_HOURS")
    if not REFUND_RULE:
        missing.append("SEEDANCE_REFUND_RULE")
    if missing:
        return "缺少或无效的显式服务配置：" + ", ".join(missing) + "。Starter Kit 不猜测模型、费用或链接策略。"
    try:
        ensure_outside_installed_skill(TASK_RECORDS_PATH, "Seedance 任务记录路径")
    except ValueError as error:
        return str(error)
    return ""


DEFAULT_API_BASE_URL = os.environ.get("SEEDANCE_API_BASE_URL", "").strip().rstrip("/")
BASE_URL = f"{DEFAULT_API_BASE_URL}/api/llm/doubao/contents/generations/tasks" if DEFAULT_API_BASE_URL else ""
_task_records_override = str(os.environ.get("SEEDANCE_TASK_RECORDS_PATH") or "").strip()
TASK_RECORDS_PATH = (
    Path(_task_records_override).expanduser().resolve()
    if _task_records_override
    else resolve_seedance_data_dir() / "seedance_video_tasks.json"
)
TASK_RECORDS_LOCK_PATH = TASK_RECORDS_PATH.with_suffix(TASK_RECORDS_PATH.suffix + ".lock")
VIDEO_GENERATION_PRECHARGE_POINTS = positive_int_environment("SEEDANCE_PRECHARGE_POINTS")
TASK_RECORDS_VERSION = 3
CONFIRMATION_MANIFEST_VERSION = 3
CONFIRMATION_TTL_SECONDS = 30 * 60
OFFICIAL_LINK_TTL_HOURS = positive_int_environment("SEEDANCE_OFFICIAL_LINK_TTL_HOURS")
OFFICIAL_LINK_EXPIRED_MESSAGE = "官方链接已过期，请到本地产物或云端产物中查看"
REFUND_RULE = os.environ.get("SEEDANCE_REFUND_RULE", "").strip()
MAX_VIDEO_BYTES = positive_int_environment("SEEDANCE_MAX_VIDEO_BYTES") or 512 * 1024 * 1024
TERMINAL_TASK_FAILURE_STATUSES = {"failed", "cancelled", "unknown"}
TERMINAL_TASK_STATUSES = TERMINAL_TASK_FAILURE_STATUSES | {"official_link_expired"}
SUBMISSION_IN_PROGRESS_STATUSES = {"submitting", "submission_unknown"}
EXPLICIT_USER_CONFIRMATIONS = {"确认", "就按这份生成"}
DIAGNOSTIC_PROMPTS = {
    "test", "testing", "demo", "hello", "ping", "123", "1",
    "测试", "测一下", "占位符", "<prompt>", "<提示词>",
}
def error_result(code, message, error_type="BadRequest", **extra):
    result = {
        "error": {
            "code": code,
            "message": message,
            "type": error_type,
        }
    }
    result.update(extra)
    return result


def extract_api_error(result):
    if not isinstance(result, dict):
        return {
            "code": "InvalidResponse",
            "message": "API 返回格式无效。",
            "type": "APIError",
        }

    http_status = result.get("_http_status")
    nested_error = result.get("error")
    if nested_error:
        if isinstance(nested_error, dict):
            error_data = dict(nested_error)
        else:
            error_data = {
                "code": "APIError",
                "message": str(nested_error),
                "type": "APIError",
            }
        if http_status is not None:
            error_data.setdefault("http_status", http_status)
        if result.get("_request_may_have_been_sent") is not None:
            error_data.setdefault("request_may_have_been_sent", result.get("_request_may_have_been_sent"))
        return error_data

    code = result.get("code")
    if code is None:
        return None
    normalized_code = str(code).strip().lower()
    if normalized_code in {"", "0", "200", "201", "202", "ok", "success"}:
        return None
    if "message" not in result and "data" not in result:
        return None
    error_data = {
        "code": f"API_{code}",
        "message": str(result.get("message") or "API 返回错误。"),
        "type": "APIError",
    }
    if http_status is not None:
        error_data["http_status"] = http_status
    if result.get("_request_may_have_been_sent") is not None:
        error_data["request_may_have_been_sent"] = result.get("_request_may_have_been_sent")
    return error_data


def api_key_error_result():
    return error_result(
        "ConfigError",
        API_KEY_LOAD_ERROR or "缺少 SEEDANCE_API_KEY 环境变量。",
        "Configuration",
    )


def sha256_text(value):
    return hashlib.sha256(str(value).encode("utf-8")).hexdigest()


def absolute_path_str(path_value):
    """将本地路径规范为绝对路径字符串；HTTP(S) URL 原样返回。"""
    text = str(path_value or "").strip()
    if not text:
        return text
    if text.startswith(("http://", "https://")):
        return text
    return str(Path(text).expanduser().resolve())


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


def validate_generation_prompt(prompt):
    if not isinstance(prompt, str) or not prompt.strip():
        return error_result("InvalidPrompt", "生成视频时必须提供非空 prompt。")

    normalized = " ".join(prompt.split()).strip().casefold()
    if normalized in DIAGNOSTIC_PROMPTS:
        return error_result(
            "DiagnosticPromptRejected",
            "拒绝使用 test/demo/测试等诊断占位词发起付费生成。"
            "如需排查环境，请使用 --dry-run 或 --help，不要改动已确认输入。",
            "PermissionDenied",
        )
    return None


def is_explicit_user_confirmation(value):
    return str(value or "").strip() in EXPLICIT_USER_CONFIRMATIONS


def utc_iso_timestamp(epoch=None):
    value = time.time() if epoch is None else float(epoch)
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(value))


def local_display_timestamp(epoch=None):
    value = time.time() if epoch is None else float(epoch)
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(value))


def confirmation_expiry_timestamp(created_epoch=None):
    created = time.time() if created_epoch is None else float(created_epoch)
    return utc_iso_timestamp(created + CONFIRMATION_TTL_SECONDS)


def normalize_task_record_schema(record):
    if not isinstance(record, dict):
        return record
    status = str(record.get("status") or "").strip().lower()
    if "remote_status" not in record:
        if status == "official_link_expired":
            record["remote_status"] = "succeeded"
        else:
            record["remote_status"] = status or "unknown"
    if "local_artifact_status" not in record:
        record["local_artifact_status"] = "downloaded" if existing_local_video_path(record) else "pending"
    if "reconciliation_status" not in record:
        record["reconciliation_status"] = "not_checked"
    if "local_artifact_error" not in record:
        record["local_artifact_error"] = None
    if "schema_version" not in record:
        record["schema_version"] = TASK_RECORDS_VERSION
    return record


def build_task_record(agent_id, task_id, prompt, request_time=None, image_paths=None,
                      confirmation_id=None, confirmation_fingerprint=None,
                      video_paths=None, audio_paths=None):
    now_epoch = time.time()
    record = {
        "schema_version": TASK_RECORDS_VERSION,
        "agent_id": str(agent_id or "").strip(),
        "task_id": str(task_id or "").strip(),
        "request_time": request_time or local_display_timestamp(now_epoch),
        "request_time_utc": utc_iso_timestamp(now_epoch),
        "prompt": prompt,
        "prompt_sha256": sha256_text(prompt),
        "images": list(image_paths or []),
        "image_count": len(image_paths or []),
        "videos": list(video_paths or []),
        "video_count": len(video_paths or []),
        "audios": list(audio_paths or []),
        "audio_count": len(audio_paths or []),
        "video_url": None,
        "local_video_path": None,
        "official_video_url": None,
        "official_link_status": "unknown",
        "official_link_expires_at": None,
        "cloud_artifact_ref": None,
        "completed_at": None,
        "completed_at_utc": None,
        "status": "submitted",
        "remote_status": "submitted",
        "local_artifact_status": "pending",
        "reconciliation_status": "not_checked",
        "local_artifact_error": None,
        "error": None,
    }
    if confirmation_id:
        record["confirmation_id"] = str(confirmation_id)
    if confirmation_fingerprint:
        record["confirmation_fingerprint"] = normalize_fingerprint(confirmation_fingerprint)
    return record


def load_task_records_unlocked(file_path):
    if not file_path.exists():
        return {"version": TASK_RECORDS_VERSION, "tasks": [], "submissions": []}

    try:
        with open(file_path, "r", encoding="utf-8") as file:
            data = json.load(file)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"任务记录文件 JSON 格式无效: {error}") from error
    except OSError as error:
        raise RuntimeError(f"读取任务记录文件失败: {error}") from error

    if not isinstance(data, dict):
        raise RuntimeError("任务记录文件格式无效：根对象必须为 JSON object")

    tasks = data.get("tasks")
    if tasks is None:
        data["tasks"] = []
    elif not isinstance(tasks, list):
        raise RuntimeError("任务记录文件格式无效：tasks 字段必须为数组")

    submissions = data.get("submissions")
    if submissions is None:
        data["submissions"] = []
    elif not isinstance(submissions, list):
        raise RuntimeError("任务记录文件格式无效：submissions 字段必须为数组")

    if "version" not in data or not isinstance(data.get("version"), int) or data.get("version", 0) < TASK_RECORDS_VERSION:
        data["version"] = TASK_RECORDS_VERSION

    for record in data.get("tasks", []):
        normalize_task_record_schema(record)
    for record in data.get("submissions", []):
        if not isinstance(record, dict):
            continue
        record.setdefault("schema_version", TASK_RECORDS_VERSION)
        record.setdefault("attempt_no", 1)
        record.setdefault("outcome_status", None)
        record.setdefault("outcome_message", None)
        record.setdefault("result_reported_at", None)
        record.setdefault("confirmation_consumed_at", record.get("claimed_at"))

    return data


def atomic_write_json(file_path, data):
    file_path.parent.mkdir(parents=True, exist_ok=True)
    temp_file = tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        dir=str(file_path.parent),
        prefix=f".{file_path.stem}.",
        suffix=".tmp",
        delete=False,
    )
    temp_path = Path(temp_file.name)
    try:
        with temp_file:
            json.dump(data, temp_file, ensure_ascii=False, indent=2)
            temp_file.flush()
            os.fsync(temp_file.fileno())
        os.replace(temp_path, file_path)
    except Exception:
        try:
            temp_path.unlink(missing_ok=True)
        except OSError:
            pass
        raise


def _acquire_windows_lock(lock_file):
    """Exclusive byte-range lock via msvcrt (Windows has no shared flock)."""
    lock_file.seek(0)
    if lock_file.read(1) != b"\0":
        lock_file.seek(0)
        lock_file.write(b"\0")
        lock_file.flush()
    while True:
        try:
            lock_file.seek(0)
            msvcrt.locking(lock_file.fileno(), msvcrt.LK_NBLCK, 1)
            return
        except OSError:
            time.sleep(0.1)


def _release_windows_lock(lock_file):
    lock_file.seek(0)
    msvcrt.locking(lock_file.fileno(), msvcrt.LK_UNLCK, 1)


@contextmanager
def task_records_lock(exclusive=True):
    """Cross-platform lock for concurrent task-record reads/writes."""
    TASK_RECORDS_LOCK_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(TASK_RECORDS_LOCK_PATH, "a+b") as lock_file:
        if sys.platform == "win32":
            _acquire_windows_lock(lock_file)
            try:
                yield
            finally:
                _release_windows_lock(lock_file)
        else:
            lock_mode = fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH
            fcntl.flock(lock_file.fileno(), lock_mode)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)


def append_task_record(agent_id, task_id, prompt, image_paths=None,
                       confirmation_id=None, confirmation_fingerprint=None,
                       video_paths=None, audio_paths=None):
    record = build_task_record(
        agent_id=agent_id,
        task_id=task_id,
        prompt=prompt,
        image_paths=image_paths,
        video_paths=video_paths,
        audio_paths=audio_paths,
        confirmation_id=confirmation_id,
        confirmation_fingerprint=confirmation_fingerprint,
    )

    with task_records_lock(exclusive=True):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        records.setdefault("tasks", []).append(record)
        atomic_write_json(TASK_RECORDS_PATH, records)

    return record


def confirmation_submission_key(agent_id, confirmation_id, confirmation_fingerprint):
    """同一份用户确认只能拥有一个提交键。"""
    return sha256_text(canonical_json_bytes({
        "agent_id": str(agent_id or "").strip(),
        "confirmation_id": str(confirmation_id or "").strip(),
        "confirmation_fingerprint": normalize_fingerprint(confirmation_fingerprint),
    }))


def find_submission_record(records, submission_key):
    for record in records.setdefault("submissions", []):
        if not isinstance(record, dict):
            continue
        if str(record.get("submission_key") or "").strip() == submission_key:
            return record
    return None


def claim_submission(agent_id, confirmation_id, confirmation_fingerprint, payload,
                     user_confirmation=None, attempt_no=None, retry_context=None):
    """原子占用确认清单，阻止并发或重复的 POST 请求。"""
    normalized_agent_id = str(agent_id or "").strip()
    normalized_confirmation_id = str(confirmation_id or "").strip()
    normalized_fingerprint = normalize_fingerprint(confirmation_fingerprint)
    if not normalized_agent_id or not normalized_confirmation_id or not normalized_fingerprint:
        return error_result(
            "ConfirmationRequired",
            "提交视频前必须提供已确认清单的 agent_id、confirmation_id 和完整确认指纹。",
            "PermissionDenied",
        )
    if not is_explicit_user_confirmation(user_confirmation):
        return error_result(
            "ExplicitUserConfirmationRequired",
            "必须传入用户明确回复“确认”或“就按这份生成”，才能发起视频生成。",
            "PermissionDenied",
        )

    submission_key = confirmation_submission_key(
        normalized_agent_id,
        normalized_confirmation_id,
        normalized_fingerprint,
    )
    payload_sha256 = sha256_text(canonical_json_bytes(payload))
    with task_records_lock(exclusive=True):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        existing = find_submission_record(records, submission_key)
        if existing:
            existing_payload_sha256 = str(existing.get("payload_sha256") or "")
            if existing_payload_sha256 and existing_payload_sha256 != payload_sha256:
                return error_result(
                    "ConfirmationSubmissionConflict",
                    "同一确认清单对应的请求内容不一致，已拒绝提交。",
                    "Conflict",
                )
            state = str(existing.get("state") or "").strip().lower()
            existing_task_id = str(existing.get("task_id") or "").strip()
            if existing_task_id:
                return {
                    "success": True,
                    "claimed": False,
                    "already_submitted": True,
                    "submission_key": submission_key,
                    "task_id": existing_task_id,
                }
            if state in SUBMISSION_IN_PROGRESS_STATUSES:
                return {
                    "success": False,
                    "claimed": False,
                    "submission_key": submission_key,
                    "error": {
                        "code": "SubmissionInProgress",
                        "message": "该确认清单正在提交或等待恢复查询，禁止重复发起 API 请求。",
                        "type": "Conflict",
                    },
                    "requires_submission_recovery": True,
                    "requires_new_confirmation_for_regeneration": True,
                }
            return error_result(
                "ConfirmationAlreadyUsed",
                "该确认清单已经发起过一次提交；如需再次生成，必须先查看本次结果，重新准备并取得新的用户确认。",
                "Conflict",
                submission_key=submission_key,
                previous_state=state or "completed",
                requires_new_confirmation_for_regeneration=True,
            )

        normalized_attempt_no = int(attempt_no or 1)
        records.setdefault("submissions", []).append({
            "schema_version": TASK_RECORDS_VERSION,
            "submission_key": submission_key,
            "agent_id": normalized_agent_id,
            "confirmation_id": normalized_confirmation_id,
            "confirmation_fingerprint": normalized_fingerprint,
            "payload_sha256": payload_sha256,
            "user_confirmation": str(user_confirmation).strip(),
            "state": "submitting",
            "attempt_no": normalized_attempt_no,
            "retry_context": retry_context if isinstance(retry_context, dict) else {},
            "claimed_at": local_display_timestamp(),
            "claimed_at_utc": utc_iso_timestamp(),
            "confirmation_consumed_at": utc_iso_timestamp(),
            "task_id": None,
            "error": None,
            "outcome_status": None,
            "outcome_message": None,
            "result_reported_at": None,
        })
        atomic_write_json(TASK_RECORDS_PATH, records)
    return {
        "success": True,
        "claimed": True,
        "submission_key": submission_key,
        "payload_sha256": payload_sha256,
    }


def update_submission_record(submission_key, state=None, task_id=None, error=None,
                             outcome_status=None, outcome_message=None,
                             result_reported_at=None, **extra):
    """更新一次性确认的提交状态；任务 ID 写入与任务记录保持同一把锁。"""
    normalized_key = str(submission_key or "").strip()
    if not normalized_key:
        return None
    with task_records_lock(exclusive=True):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        record = find_submission_record(records, normalized_key)
        if record is None:
            return None
        if state:
            record["state"] = state
        record["updated_at"] = local_display_timestamp()
        record["updated_at_utc"] = utc_iso_timestamp()
        if task_id:
            record["task_id"] = str(task_id)
        if error is not None:
            record["error"] = error
        if outcome_status is not None:
            record["outcome_status"] = outcome_status
        if outcome_message is not None:
            record["outcome_message"] = outcome_message
        if result_reported_at is not None:
            record["result_reported_at"] = result_reported_at
        for key, value in extra.items():
            if value is not None:
                record[key] = value
        atomic_write_json(TASK_RECORDS_PATH, records)
        return dict(record)


def get_submission_record(submission_key, agent_id=None):
    normalized_key = str(submission_key or "").strip()
    normalized_agent_id = str(agent_id or "").strip()
    if not normalized_key:
        return None
    with task_records_lock(exclusive=False):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        record = find_submission_record(records, normalized_key)
        if record is None:
            return None
        if normalized_agent_id and str(record.get("agent_id") or "").strip() != normalized_agent_id:
            return None
        return dict(record)


def find_submission_key_for_task(task_id):
    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id:
        return None
    with task_records_lock(exclusive=False):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        for record in records.get("submissions", []):
            if not isinstance(record, dict):
                continue
            if str(record.get("task_id") or "").strip() == normalized_task_id:
                return str(record.get("submission_key") or "").strip() or None
    return None


def list_prior_submissions(agent_id, payload_sha256):
    normalized_agent_id = str(agent_id or "").strip()
    normalized_payload_sha256 = str(payload_sha256 or "").strip()
    if not normalized_agent_id or not normalized_payload_sha256:
        return []
    with task_records_lock(exclusive=False):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        matches = []
        for record in records.get("submissions", []):
            if not isinstance(record, dict):
                continue
            if str(record.get("agent_id") or "").strip() != normalized_agent_id:
                continue
            if str(record.get("payload_sha256") or "").strip() != normalized_payload_sha256:
                continue
            matches.append({
                "submission_key": record.get("submission_key"),
                "attempt_no": record.get("attempt_no", 1),
                "state": record.get("state"),
                "task_id": record.get("task_id"),
                "outcome_status": record.get("outcome_status"),
                "outcome_message": record.get("outcome_message"),
                "result_reported_at": record.get("result_reported_at"),
                "error": record.get("error"),
            })
        return matches


def submission_error_is_ambiguous(error_data):
    """判断 POST 的响应是否可能代表服务端已经受理请求。"""
    if not isinstance(error_data, dict):
        return True
    raw_status = error_data.get("http_status")
    try:
        http_status = int(raw_status) if raw_status is not None else None
    except (TypeError, ValueError):
        http_status = None
    code = str(error_data.get("code") or "").strip().lower()
    normalized_code = re.sub(r"[^a-z0-9]", "", code)
    if http_status in {408, 429} or (http_status is not None and 500 <= http_status <= 599):
        return True
    if re.search(r"(?:api|http)?(?:408|429|5\d{2})$", normalized_code):
        return True
    if normalized_code in {"networkerror", "unexpectederror", "invalidresponse", "timeouterror"}:
        return True
    return bool(error_data.get("request_may_have_been_sent"))


def summarize_submission_record(record):
    if not isinstance(record, dict):
        return None
    state = str(record.get("state") or "unknown").strip().lower()
    outcome_status = str(record.get("outcome_status") or state).strip().lower()
    if state in SUBMISSION_IN_PROGRESS_STATUSES:
        message = "本次 POST 的服务端受理状态未知，系统不会自动重新提交，避免重复扣费。"
    elif state in {"retry_eligible", "rejected"}:
        message = "本次提交未确认创建任务；如需重新生成，必须重新准备确认清单并取得用户确认。"
    elif record.get("task_id"):
        message = "本次提交已创建远端任务；如需再次生成，必须重新准备确认清单并取得用户确认。"
    else:
        message = "本次任务尚未创建可查询的远端任务；如需重新生成，必须重新准备确认清单并取得用户确认。"
    return {
        "submission_key": record.get("submission_key"),
        "attempt_no": record.get("attempt_no", 1),
        "state": state,
        "outcome_status": outcome_status,
        "task_id": record.get("task_id"),
        "error": record.get("error"),
        "message": record.get("outcome_message") or message,
        "result_reported_at": record.get("result_reported_at"),
        "requires_new_confirmation_for_regeneration": True,
    }


def report_submission_outcome(submission_key, outcome_status, message, error=None,
                              task_id=None):
    normalized_key = str(submission_key or "").strip()
    if not normalized_key:
        return None
    return update_submission_record(
        normalized_key,
        task_id=task_id,
        error=error,
        outcome_status=outcome_status,
        outcome_message=message,
        result_reported_at=utc_iso_timestamp(),
    )


def record_submitted_task(submission_key, agent_id, task_id, prompt, image_paths=None,
                          confirmation_id=None, confirmation_fingerprint=None,
                          video_paths=None, audio_paths=None):
    """原子记录 task_id 和提交完成状态，供重复调用恢复同一个任务。"""
    task_record = build_task_record(
        agent_id=agent_id,
        task_id=task_id,
        prompt=prompt,
        image_paths=image_paths,
        video_paths=video_paths,
        audio_paths=audio_paths,
        confirmation_id=confirmation_id,
        confirmation_fingerprint=confirmation_fingerprint,
    )
    with task_records_lock(exclusive=True):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        existing_task = None
        for record in records.setdefault("tasks", []):
            if not isinstance(record, dict):
                continue
            if str(record.get("task_id") or "").strip() == str(task_id or "").strip():
                existing_task = record
                break
        if existing_task is None:
            records["tasks"].append(task_record)
            existing_task = task_record

        submission = find_submission_record(records, submission_key)
        if submission is not None:
            submission["state"] = "submitted"
            submission["task_id"] = str(task_id)
            submission["updated_at"] = local_display_timestamp()
            submission["updated_at_utc"] = utc_iso_timestamp()
            submission["error"] = None
        atomic_write_json(TASK_RECORDS_PATH, records)
        return dict(existing_task)


def load_task_records():
    with task_records_lock(exclusive=False):
        return load_task_records_unlocked(TASK_RECORDS_PATH)


def update_task_record_video_url(task_id, video_url=None, original_video_url=None,
                                 status=None, error=None, local_video_path=None,
                                 official_link_status=None, official_link_expires_at=None,
                                 cloud_artifact_ref=None, completed_at=None,
                                 remote_status=None, local_artifact_status=None,
                                 reconciliation_status=None, local_artifact_error=None,
                                 local_artifact_metadata=None):
    """更新任务记录的视频链接、状态和错误信息。

    status 为 succeeded 时会清除此前记录的错误信息；
    status 为 failed/cancelled/unknown 时应同时传入 error 记录具体失败原因。
    """
    video_url = absolute_path_str(video_url)
    local_video_path = absolute_path_str(local_video_path)
    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id:
        return None

    with task_records_lock(exclusive=True):
        records = load_task_records_unlocked(TASK_RECORDS_PATH)
        tasks = records.setdefault("tasks", [])
        updated_record = None
        for record in tasks:
            if not isinstance(record, dict):
                continue
            if str(record.get("task_id") or "").strip() != normalized_task_id:
                continue
            if video_url:
                record["video_url"] = video_url
            if local_video_path:
                record["local_video_path"] = local_video_path
            if original_video_url:
                record["original_video_url"] = original_video_url
                record["official_video_url"] = original_video_url
            if official_link_status:
                record["official_link_status"] = official_link_status
            if official_link_expires_at:
                parsed_expiry = parse_expiry_epoch(official_link_expires_at)
                record["official_link_expires_at"] = format_utc_timestamp(parsed_expiry) if parsed_expiry is not None else official_link_expires_at
            if cloud_artifact_ref:
                record["cloud_artifact_ref"] = cloud_artifact_ref
            if completed_at:
                parsed_completed_at = parse_expiry_epoch(completed_at)
                normalized_completed_at = format_utc_timestamp(parsed_completed_at) if parsed_completed_at is not None else completed_at
                record["completed_at"] = normalized_completed_at
                record["completed_at_utc"] = normalized_completed_at
            if status:
                record["status"] = status
            if remote_status:
                record["remote_status"] = remote_status
            elif status in {"submitted", "processing", "succeeded", "failed", "cancelled", "unknown"}:
                record["remote_status"] = status
            if local_artifact_status:
                record["local_artifact_status"] = local_artifact_status
            if reconciliation_status:
                record["reconciliation_status"] = reconciliation_status
            if local_artifact_error is not None:
                record["local_artifact_error"] = local_artifact_error
            elif local_artifact_status == "downloaded":
                record["local_artifact_error"] = None
            if isinstance(local_artifact_metadata, dict):
                record["local_artifact_metadata"] = dict(local_artifact_metadata)
            if error is not None:
                record["error"] = error
            elif status == "succeeded":
                record["error"] = None
            updated_record = record
        if updated_record is not None:
            atomic_write_json(TASK_RECORDS_PATH, records)
        return updated_record


def existing_local_video_path(record):
    """返回任务记录中仍存在的本地视频文件，不把官方 URL 当成本地路径。"""
    if not isinstance(record, dict):
        return None
    candidates = [record.get("local_video_path"), record.get("video_url")]
    for candidate in candidates:
        value = str(candidate or "").strip()
        if not value or value.startswith(("http://", "https://")):
            continue
        path = Path(value).expanduser()
        if path.is_file() and path.stat().st_size > 0:
            return absolute_path_str(path)
    return None


def parse_expiry_epoch(value):
    """解析 Unix 时间戳或带时区 ISO 时间；旧的无时区字符串按本机本地时区解释。"""
    if value is None:
        return None
    if isinstance(value, (int, float)):
        numeric = float(value)
        return numeric / 1000 if abs(numeric) > 100000000000 else numeric
    text = str(value).strip()
    if not text:
        return None
    try:
        return float(text)
    except ValueError:
        pass
    normalized = text.replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
        if parsed.tzinfo is None:
            # 旧版本把本地时间写成无时区字符串；按生成它的本机时区读取，不能默认为 UTC。
            return time.mktime(parsed.timetuple())
        return parsed.timestamp()
    except (ValueError, TypeError):
        pass
    try:
        return request_time_to_epoch(text)
    except Exception:
        return None


def format_local_timestamp(epoch):
    if epoch is None:
        return None
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(epoch))


def format_utc_timestamp(epoch):
    if epoch is None:
        return None
    return utc_iso_timestamp(epoch)


def infer_official_link_expiry(record=None, result=None, official_video_url=None):
    """优先使用最新 API/签名 URL 的过期时间，旧记录只作为回退。"""
    for source in (result or {},):
        if not isinstance(source, dict):
            continue
        for key in ("official_link_expires_at", "expires_at", "expiration_time", "expires"):
            parsed = parse_expiry_epoch(source.get(key))
            if parsed is not None:
                return format_utc_timestamp(parsed)

    url = str(official_video_url or "").strip()
    query = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
    for key in (
        "Expires", "expires", "X-Amz-Expires", "x-amz-expires",
        "X-Tos-Expires", "x-tos-expires",
    ):
        values = query.get(key)
        if values:
            try:
                seconds = float(values[0])
                base = None
                for date_key in (
                    "X-Amz-Date", "x-amz-date", "X-Tos-Date", "x-tos-date",
                ):
                    if query.get(date_key):
                        base = time.strptime(query[date_key][0], "%Y%m%dT%H%M%SZ")
                        base = calendar.timegm(base)
                        break
                if base is not None:
                    return format_utc_timestamp(base + seconds)
                if seconds > time.time():
                    return format_utc_timestamp(seconds)
            except (TypeError, ValueError, OverflowError):
                pass

    for source in (record or {},):
        if not isinstance(source, dict):
            continue
        for key in ("official_link_expires_at", "expires_at", "expiration_time", "expires"):
            parsed = parse_expiry_epoch(source.get(key))
            if parsed is not None:
                return format_utc_timestamp(parsed)

    request_time = (record or {}).get("request_time") if isinstance(record, dict) else None
    request_epoch = parse_expiry_epoch(request_time)
    if request_epoch is not None and official_video_url:
        return format_utc_timestamp(request_epoch + OFFICIAL_LINK_TTL_HOURS * 3600)
    return None


def record_link_is_expired(record, now=None):
    if not isinstance(record, dict):
        return False
    status = str(record.get("official_link_status") or "").strip().lower()
    if status == "expired" or str(record.get("status") or "").strip().lower() == "official_link_expired":
        return True
    official_url = (
        record.get("official_video_url")
        or record.get("original_video_url")
        or (record.get("video_url") if str(record.get("video_url") or "").startswith(("http://", "https://")) else None)
    )
    expires_at = parse_expiry_epoch(record.get("official_link_expires_at"))
    if expires_at is None:
        request_epoch = parse_expiry_epoch(record.get("request_time"))
        if request_epoch is not None and str(official_url or "").strip():
            expires_at = request_epoch + OFFICIAL_LINK_TTL_HOURS * 3600
    return expires_at is not None and (now if now is not None else time.time()) >= expires_at


def official_link_expired_result(task_id, record=None, official_video_url=None,
                                 cloud_artifact_ref=None):
    local_path = existing_local_video_path(record)
    legacy_video_url = (record or {}).get("video_url")
    original_url = (
        official_video_url
        or (record or {}).get("official_video_url")
        or (record or {}).get("original_video_url")
        or (legacy_video_url if str(legacy_video_url or "").startswith(("http://", "https://")) else None)
    )
    expiry = infer_official_link_expiry(record=record, official_video_url=original_url)
    update_task_record_video_url(
        task_id=task_id,
        status="official_link_expired",
        official_link_status="expired",
        official_link_expires_at=expiry,
        original_video_url=original_url,
        local_video_path=local_path,
        cloud_artifact_ref=cloud_artifact_ref,
        error=None,
        remote_status=(record or {}).get("remote_status") or "succeeded",
        local_artifact_status="downloaded" if local_path else (record or {}).get("local_artifact_status") or "pending",
        reconciliation_status="matched" if local_path else (record or {}).get("reconciliation_status") or "not_checked",
    )
    return {
        "success": True,
        "status": "official_link_expired",
        "terminal": True,
        "official_link_expired": True,
        "requires_regeneration": False,
        "task_id": str(task_id or "").strip(),
        "video_url": local_path,
        "local_video_path": local_path,
        "original_video_url": original_url,
        "official_link_expires_at": expiry,
        "cloud_artifact_ref": cloud_artifact_ref or (record or {}).get("cloud_artifact_ref"),
        "artifact_locations": ["local", "cloud"],
        "message": OFFICIAL_LINK_EXPIRED_MESSAGE,
    }


def is_expired_http_error(error, url=""):
    status_code = getattr(error, "code", None)
    if status_code in {401, 403, 410}:
        return True
    if status_code != 404:
        return False
    query = urllib.parse.parse_qs(urllib.parse.urlparse(str(url or "")).query)
    signed_keys = {key.lower() for key in query}
    return bool(signed_keys & {"expires", "x-amz-expires", "x-amz-date", "signature", "x-amz-signature"})


def api_result_indicates_expired(result):
    if not isinstance(result, dict):
        return False
    text = json.dumps(result, ensure_ascii=False).lower()
    return any(marker in text for marker in (
        "official_link_expired", "link_expired", "url_expired", "expired_url",
        "link expired", "url expired", "链接已过期", "链接过期",
    ))


def get_task_record_by_id(task_id):
    """按 task_id 查找任务记录；不存在时返回 None。"""
    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id:
        return None
    for record in load_task_records().get("tasks", []):
        if not isinstance(record, dict):
            continue
        if str(record.get("task_id") or "").strip() == normalized_task_id:
            return record
    return None


def list_task_records_missing_video_url(agent_id=None):
    records = load_task_records().get("tasks", [])
    normalized_agent_id = str(agent_id or "").strip()
    missing_records = []
    for record in records:
        if not isinstance(record, dict):
            continue
        if normalized_agent_id and str(record.get("agent_id") or "").strip() != normalized_agent_id:
            continue
        if not str(record.get("task_id") or "").strip():
            continue
        if existing_local_video_path(record):
            continue
        status = str(record.get("status") or "").strip().lower()
        if status in TERMINAL_TASK_STATUSES:
            continue
        if record_link_is_expired(record):
            continue
        missing_records.append(record)
    return missing_records


def resolve_missing_task_video_urls_before_generation(agent_id=None, output_dir=None):
    reconcile_task_artifacts(agent_id=agent_id, output_dir=output_dir)
    missing_records = list_task_records_missing_video_url(agent_id=agent_id)
    if not missing_records:
        return {"success": True, "checked": True, "pending_count": 0, "resolved_tasks": []}

    resolved_tasks = []
    unresolved_tasks = []
    expired_tasks = []
    non_regenerable_tasks = []

    for record in missing_records:
        task_id = str(record.get("task_id") or "").strip()
        if not task_id:
            continue

        prompt = str(record.get("prompt") or "")
        if record_link_is_expired(record):
            expired_tasks.append(official_link_expired_result(task_id, record=record))
            continue

        result = query_video_task(
            task_id,
            agent_id=record.get("agent_id") or agent_id,
            output_dir=output_dir,
        )

        if result.get("status") == "succeeded" and str(result.get("video_url") or "").strip():
            updated_record = update_task_record_video_url(
                task_id=task_id,
                video_url=result.get("video_url"),
                original_video_url=result.get("original_video_url"),
                status=result.get("status"),
            ) or record
            resolved_tasks.append({
                "task_id": task_id,
                "prompt": prompt,
                "video_url": absolute_path_str(result.get("video_url")),
                "original_video_url": result.get("original_video_url"),
                "output_dir": result.get("output_dir"),
                "local_video_path": result.get("local_video_path"),
                "status": result.get("status"),
                "request_time": updated_record.get("request_time"),
            })
            continue

        if result.get("official_link_expired") or result.get("status") == "official_link_expired":
            expired_tasks.append(result)
            continue

        if result.get("requires_regeneration") is False:
            non_regenerable_tasks.append({
                "task_id": task_id,
                "prompt": prompt,
                "status": result.get("status") or "artifact_unavailable",
                "error": result.get("error"),
                "message": result.get("message"),
            })
            continue

        unresolved_tasks.append({
            "task_id": task_id,
            "prompt": prompt,
            "status": result.get("status") or "unknown",
            "error": result.get("error"),
            "request_time": record.get("request_time"),
        })

    if resolved_tasks:
        next_prompt = resolved_tasks[-1].get("prompt") or "该提示词"
        return {
            "success": True,
            "checked": True,
            "requires_user_confirmation": True,
            "requires_history_confirmation": True,
            "message": f"我已经将 task_id 对应提示词“{next_prompt}”的视频链接查询到了，是否需要继续生成新视频？",
            "resolved_tasks": resolved_tasks,
            "unresolved_tasks": unresolved_tasks,
            "expired_tasks": expired_tasks,
            "non_regenerable_tasks": non_regenerable_tasks,
            "pending_count": len(unresolved_tasks),
        }

    if (expired_tasks or non_regenerable_tasks) and not unresolved_tasks:
        return {
            "success": True,
            "checked": True,
            "requires_user_confirmation": False,
            "requires_history_confirmation": False,
            "requires_regeneration": False,
            "expired_tasks": expired_tasks,
            "non_regenerable_tasks": non_regenerable_tasks,
            "message": OFFICIAL_LINK_EXPIRED_MESSAGE if expired_tasks else "历史任务已完成，但当前没有可用的官方链接；请到本地产物或云端产物中查看。",
            "pending_count": 0,
        }

    return {
        "success": True,
        "checked": True,
        "requires_user_confirmation": True,
        "requires_history_confirmation": True,
        "message": "发现 seedance_video_tasks.json 中仍有 task_id 缺少视频链接，请先查询这些 task_id 的视频链接；确认后我再继续生成新视频。",
        "resolved_tasks": resolved_tasks,
        "unresolved_tasks": unresolved_tasks,
        "expired_tasks": expired_tasks,
        "non_regenerable_tasks": non_regenerable_tasks,
        "pending_count": len(unresolved_tasks),
    }


def parse_request_time(value):
    return time.strptime(str(value or "").strip(), "%Y-%m-%d %H:%M:%S")


def request_time_to_epoch(value):
    return time.mktime(parse_request_time(value))


def build_prompt_summary(prompt, limit=60):
    text = " ".join(str(prompt or "").split())
    if len(text) <= limit:
        return text
    return text[: limit - 3] + "..."


def parse_time_keyword_range(keyword, now_ts=None):
    normalized = str(keyword or "").strip().lower()
    if not normalized:
        return None

    now_ts = now_ts if now_ts is not None else time.time()
    now_local = time.localtime(now_ts)

    if normalized in {"today", "今天"}:
        start = time.mktime((now_local.tm_year, now_local.tm_mon, now_local.tm_mday, 0, 0, 0, 0, 0, -1))
        return (start, start + 86400)
    if normalized in {"yesterday", "昨天"}:
        today_start = time.mktime((now_local.tm_year, now_local.tm_mon, now_local.tm_mday, 0, 0, 0, 0, 0, -1))
        return (today_start - 86400, today_start)
    if normalized in {"this-hour", "这一小时", "最近一小时"}:
        return (now_ts - 3600, now_ts)
    if normalized in {"last-10-minutes", "最近10分钟", "十分钟前后"}:
        return (now_ts - 600, now_ts)
    return None


def normalize_keywords(raw_keywords=None, prompt_query=None):
    keywords = []
    for item in raw_keywords or []:
        value = str(item or "").strip()
        if value:
            keywords.append(value)
    prompt_value = str(prompt_query or "").strip()
    if prompt_value:
        keywords.append(prompt_value)
    return keywords


def filter_task_records(agent_id=None, task_id=None, prompt_query=None, keywords=None,
                        time_from=None, time_to=None, time_keyword=None, limit=10):
    records = load_task_records().get("tasks", [])
    normalized_agent_id = str(agent_id or "").strip()
    normalized_task_id = str(task_id or "").strip()
    normalized_keywords = [item.lower() for item in normalize_keywords(keywords, prompt_query)]

    parsed_time_from = request_time_to_epoch(time_from) if time_from else None
    parsed_time_to = request_time_to_epoch(time_to) if time_to else None
    keyword_range = parse_time_keyword_range(time_keyword)
    if keyword_range:
        keyword_from, keyword_to = keyword_range
        parsed_time_from = max(parsed_time_from, keyword_from) if parsed_time_from is not None else keyword_from
        parsed_time_to = min(parsed_time_to, keyword_to) if parsed_time_to is not None else keyword_to

    matched = []
    for record in records:
        if not isinstance(record, dict):
            continue
        record_agent_id = str(record.get("agent_id") or "").strip()
        record_task_id = str(record.get("task_id") or "").strip()
        record_prompt = str(record.get("prompt") or "")
        record_request_time = str(record.get("request_time") or "").strip()

        if normalized_agent_id and record_agent_id != normalized_agent_id:
            continue
        if normalized_task_id and record_task_id != normalized_task_id:
            continue
        if normalized_keywords:
            prompt_lower = record_prompt.lower()
            if not all(keyword in prompt_lower for keyword in normalized_keywords):
                continue

        try:
            record_ts = request_time_to_epoch(record_request_time)
        except Exception:
            record_ts = None

        if parsed_time_from is not None and (record_ts is None or record_ts < parsed_time_from):
            continue
        if parsed_time_to is not None and (record_ts is None or record_ts > parsed_time_to):
            continue

        matched.append((record_ts or float("-inf"), record))

    matched.sort(key=lambda item: item[0], reverse=True)
    return [item[1] for item in matched[:max(1, int(limit or 1))]]


def query_history_task(agent_id=None, task_id=None, prompt_query=None, keywords=None,
                       time_from=None, time_to=None, time_keyword=None, limit=10,
                       output_dir=None):
    matched_records = filter_task_records(
        agent_id=agent_id,
        task_id=task_id,
        prompt_query=prompt_query,
        keywords=keywords,
        time_from=time_from,
        time_to=time_to,
        time_keyword=time_keyword,
        limit=limit,
    )

    if not matched_records:
        return {
            "success": False,
            "error": {
                "code": "TaskRecordNotFound",
                "message": "未找到符合条件的历史视频任务记录。",
                "type": "NotFound",
            },
            "task_records_path": absolute_path_str(TASK_RECORDS_PATH),
        }

    selected_record = dict(matched_records[0])
    if selected_record.get("video_url"):
        selected_record["video_url"] = absolute_path_str(selected_record.get("video_url"))
    task_result = query_video_task(
        selected_record.get("task_id"),
        agent_id=agent_id or selected_record.get("agent_id"),
        output_dir=output_dir,
    )
    refreshed_record = get_task_record_by_id(selected_record.get("task_id"))
    if refreshed_record:
        selected_record = dict(refreshed_record)
        if selected_record.get("video_url"):
            selected_record["video_url"] = absolute_path_str(selected_record.get("video_url"))
        if selected_record.get("local_video_path"):
            selected_record["local_video_path"] = absolute_path_str(selected_record.get("local_video_path"))
    task_result["matched_record"] = selected_record
    task_result["matched_count"] = len(matched_records)
    task_result["task_records_path"] = absolute_path_str(TASK_RECORDS_PATH)
    if len(matched_records) > 1:
        task_result["other_matches"] = [
            {
                "agent_id": record.get("agent_id"),
                "task_id": record.get("task_id"),
                "request_time": record.get("request_time"),
                "prompt_summary": build_prompt_summary(record.get("prompt")),
            }
            for record in matched_records[1:]
        ]
    return task_result


def build_video_request_payload(prompt, duration=None, image_paths=None, audio_paths=None,
                                ratio="16:9", watermark=False, generate_audio=True,
                                video_paths=None):
    """构建将要发送给 API 的确定性 payload，不发起网络请求。"""
    prompt_error = validate_generation_prompt(prompt)
    if prompt_error:
        return prompt_error

    if duration is None:
        return error_result(
            "InvalidParameter",
            "必须指定视频时长（--duration 参数），支持 4-15 秒的任意整数。",
        )
    if not isinstance(duration, int) or duration < 4 or duration > 15:
        return error_result(
            "InvalidParameter",
            f"不支持的时长 {duration} 秒。API 支持 4-15 秒的任意整数时长。",
        )
    if ratio not in {"16:9", "9:16"}:
        return error_result("InvalidParameter", f"不支持的画面比例: {ratio}。")

    processed_images = []
    for raw_image in image_paths or []:
        image_url = str(raw_image or "").strip()
        if not is_image_url(image_url):
            return error_result("InvalidImage", f"无效的图片 URL: {image_url[:100]}...")
        processed_images.append(image_url)

    processed_videos = []
    for raw_video in video_paths or []:
        video_url = str(raw_video or "").strip()
        if not is_video_url(video_url):
            return error_result("InvalidVideo", f"无效的视频 URL: {video_url[:100]}...")
        processed_videos.append(video_url)

    processed_audio = []
    for raw_audio in audio_paths or []:
        audio_url = str(raw_audio or "").strip()
        if not is_audio_url(audio_url):
            return error_result("InvalidAudio", f"无效的音频 URL: {audio_url[:100]}...")
        processed_audio.append(audio_url)

    content = [{"type": "text", "text": prompt}]
    for image_url in processed_images:
        content.append({
            "type": "image_url",
            "image_url": {"url": image_url},
            "role": "reference_image",
        })
    for video_url in processed_videos:
        content.append({
            "type": "video_url",
            "video_url": {"url": video_url},
            "role": "reference_video",
        })
    for audio_url in processed_audio:
        content.append({
            "type": "audio_url",
            "audio_url": {"url": audio_url},
            "role": "reference_audio",
        })

    if generate_audio is not True:
        return error_result(
            "GenerateAudioRequired",
            "必须生成有声音的视频：generate_audio 只能为 true，禁止 false，"
            "也禁止规划后期配音/BGM 替代模型出声。",
        )

    if not SEEDANCE_MODEL:
        return error_result(
            "ConfigError",
            "缺少 SEEDANCE_MODEL；Starter Kit 不猜测视频模型名称。",
            "Configuration",
        )

    payload = {
        "model": SEEDANCE_MODEL,
        "ratio": ratio,
        "watermark": bool(watermark),
        "return_last_frame": True,
        "generate_audio": True,
        "content": content,
        "duration": int(duration),
    }
    return {
        "success": True,
        "payload": payload,
        "images": processed_images,
        "videos": processed_videos,
        "audios": processed_audio,
    }


def submit_video_task(prompt, duration=None, image_paths=None, audio_paths=None,
                      ratio="16:9", watermark=False, generate_audio=True,
                      agent_id=None, confirmation_id=None,
                      confirmation_fingerprint=None, video_paths=None,
                      user_confirmation=None, confirmation_file=None,
                      confirm_retry_risk=False):
    """提交视频生成任务，立即返回 task_id。"""
    normalized_confirmation_id = str(confirmation_id or "").strip()
    fingerprint = normalize_fingerprint(confirmation_fingerprint)
    if not str(confirmation_file or "").strip():
        return error_result(
            "ConfirmationManifestRequired",
            "正式生成必须提供已向用户展示的 confirmation_file；禁止直接提交 prompt。",
            "PermissionDenied",
        )
    if not normalized_confirmation_id or len(fingerprint) != 64:
        return error_result(
            "ConfirmationRequired",
            "正式生成必须使用与用户确认内容对应的清单和执行器内部完整指纹；禁止直接提交 prompt。",
            "PermissionDenied",
        )
    if not is_explicit_user_confirmation(user_confirmation):
        return error_result(
            "ExplicitUserConfirmationRequired",
            "必须传入用户明确回复“确认”或“就按这份生成”，才能发起视频生成。",
            "PermissionDenied",
        )
    validated_confirmation = load_and_validate_confirmation_manifest(
        confirmation_file,
        expected_fingerprint=fingerprint,
        expected_agent_id=agent_id,
    )
    if "error" in validated_confirmation:
        return validated_confirmation
    if validated_confirmation["confirmation_id"] != normalized_confirmation_id:
        return error_result(
            "ConfirmationIdMismatch",
            "confirmation_id 与确认清单不一致，已拒绝发起 API 请求。",
            "PermissionDenied",
        )
    retry_context = validated_confirmation["manifest"].get("retry_context") or {}
    if retry_context.get("requires_duplicate_charge_risk_confirmation") and not confirm_retry_risk:
        return error_result(
            "RetryRiskConfirmationRequired",
            "此前存在服务端受理状态未知的同参数提交；必须先向用户说明可能重复扣费风险，"
            "取得新的确认并明确确认该风险后，才能发起新的 POST。",
            "PermissionDenied",
        )

    if not DEFAULT_API_BASE_URL:
        return error_result("ConfigError", "缺少 SEEDANCE_API_BASE_URL 环境变量。", "Configuration")
    try:
        validate_service_base_url(DEFAULT_API_BASE_URL)
    except ValueError as error:
        return error_result("ConfigError", str(error), "Configuration")
    if not API_KEY:
        return api_key_error_result()

    prepared = build_video_request_payload(
        prompt=prompt,
        duration=duration,
        image_paths=image_paths,
        audio_paths=audio_paths,
        video_paths=video_paths,
        ratio=ratio,
        watermark=watermark,
        generate_audio=generate_audio,
    )
    if "error" in prepared:
        return prepared
    if prepared["payload"] != validated_confirmation["payload"]:
        return error_result(
            "ConfirmedInputMismatch",
            "正式提交参数与用户确认清单不一致，已拒绝发起 API 请求。",
            "PermissionDenied",
        )

    claim = claim_submission(
        agent_id=agent_id,
        confirmation_id=normalized_confirmation_id,
        confirmation_fingerprint=fingerprint,
        payload=prepared["payload"],
        user_confirmation=user_confirmation,
        attempt_no=validated_confirmation["manifest"].get("attempt_no", 1),
        retry_context=validated_confirmation["manifest"].get("retry_context"),
    )
    if "error" in claim:
        return claim
    if claim.get("already_submitted"):
        return {
            "success": True,
            "api_request_sent": False,
            "task_id": claim["task_id"],
            "deduplicated": True,
            "confirmation_id": normalized_confirmation_id,
            "confirmation_fingerprint": fingerprint,
            "message": "该确认清单已提交过，继续使用原 task_id，未重复发起 API 请求。",
        }

    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {API_KEY}",
        "agent-id": str(agent_id or "").strip(),
        "Idempotency-Key": claim["submission_key"],
    }
    print_progress(
        "Submitting immutable confirmed request: "
        f"fingerprint={fingerprint}, prompt_sha256={sha256_text(prompt)}, "
        f"image_count={len(prepared['images'])}, "
        f"video_count={len(prepared['videos'])}, "
        f"audio_count={len(prepared['audios'])}",
        flush=True,
    )

    try:
        result = request_json(
            BASE_URL,
            method="POST",
            headers=headers,
            payload=prepared["payload"],
            timeout=60,
        )
        api_error = extract_api_error(result)
        if api_error:
            error_data = api_error
            ambiguous = submission_error_is_ambiguous(error_data)
            state = "submission_unknown" if ambiguous else "retry_eligible"
            outcome_status = "submission_unknown" if ambiguous else "rejected"
            outcome_message = (
                "提交响应异常，服务端是否已受理暂时无法确认；系统不会自动重新提交，"
                "请先查询结果，若需重新生成必须重新确认。"
                if ambiguous
                else "本次提交未创建可确认的远端任务；如需重新生成，必须重新准备确认清单并取得用户确认。"
            )
            update_submission_record(
                claim["submission_key"],
                state,
                error=error_data,
                outcome_status=outcome_status,
                outcome_message=outcome_message,
                result_reported_at=utc_iso_timestamp(),
            )
            return error_result(
                "SubmissionUnknown" if ambiguous else "SubmissionRejected",
                outcome_message,
                "Conflict" if ambiguous else "APIError",
                submission_key=claim["submission_key"],
                api_error=error_data,
                requires_submission_recovery=ambiguous,
                requires_new_confirmation_for_regeneration=True,
                api_request_sent=True,
            )

        task_id = result.get("id")
        if not task_id:
            invalid_result = error_result(
                "InvalidResponse",
                f"Failed to get task_id: {result}",
                "APIError",
            )
            update_submission_record(
                claim["submission_key"],
                "submission_unknown",
                error=invalid_result["error"],
            )
            invalid_result["submission_key"] = claim["submission_key"]
            invalid_result["requires_submission_recovery"] = True
            invalid_result["requires_new_confirmation_for_regeneration"] = True
            return invalid_result
        record_submitted_task(
            submission_key=claim["submission_key"],
            agent_id=agent_id,
            task_id=task_id,
            prompt=prompt,
            image_paths=prepared["images"],
            video_paths=prepared["videos"],
            audio_paths=prepared["audios"],
            confirmation_id=confirmation_id,
            confirmation_fingerprint=fingerprint,
        )
        return {
            "success": True,
            "api_request_sent": True,
            "task_id": task_id,
            "submission_key": claim["submission_key"],
            "confirmation_id": confirmation_id,
            "confirmation_fingerprint": fingerprint or None,
        }
    except Exception as e:
        update_submission_record(
            claim["submission_key"],
            "submission_unknown",
            error={"code": "UnexpectedError", "message": str(e), "type": "Exception"},
            outcome_status="submission_unknown",
            outcome_message="提交结果不确定，系统不会自动重新提交；请先恢复查询并重新确认后再生成。",
            result_reported_at=utc_iso_timestamp(),
        )
        return error_result(
            "SubmissionUnknown",
            f"提交结果不确定，请按 submission_key 恢复查询，不要自动重新提交：{e}",
            "Exception",
            submission_key=claim["submission_key"],
            requires_submission_recovery=True,
            requires_new_confirmation_for_regeneration=True,
        )


# ---- 本地视频保存相关工具函数 ----

def sanitize_filename_stem(name=""):
    normalized = str(name or "").strip().rsplit(".", 1)[0].lower()
    safe_chars = []
    last_dash = False
    for char in normalized:
        if char.isalnum() or char in {"-", "_"}:
            safe_chars.append(char)
            last_dash = False
            continue
        if not last_dash:
            safe_chars.append("-")
            last_dash = True
    result = "".join(safe_chars).strip("-")
    return result or "video"


def normalize_agent_id(value):
    normalized = str(value or "").strip()
    if not normalized:
        raise ValueError("必须提供外部视频网关要求的 agent_id。")
    if len(normalized) > 256 or any(ord(char) < 32 or ord(char) == 127 for char in normalized):
        raise ValueError("agent_id 长度或字符无效。")
    return normalized


def resolve_video_output_dir(agent_id=None, output_dir=None):
    """解析视频本地保存目录，优先级：显式参数 > 环境变量 > 项目数据目录。

    始终返回绝对路径。
    """
    explicit = str(output_dir or os.environ.get("VIDEO_OUTPUT_DIR") or "").strip()
    if explicit:
        return ensure_outside_installed_skill(explicit, "视频输出目录")

    artifact_root = str(os.environ.get("CODEX_ARTIFACT_DIR") or "").strip()
    base = (
        Path(artifact_root).expanduser().resolve() / "seedance2-0-video-gen"
        if artifact_root
        else resolve_seedance_data_dir()
    )
    normalized_agent_id = str(agent_id or "").strip()
    if normalized_agent_id and normalized_agent_id.casefold() != "main":
        return ensure_outside_installed_skill(
            base / "agents" / sanitize_filename_stem(normalized_agent_id) / "video",
            "视频输出目录",
        )
    return ensure_outside_installed_skill(base / "video", "视频输出目录")


def calculate_confirmation_fingerprint(manifest):
    unsigned = {key: value for key, value in manifest.items() if key != "fingerprint"}
    return hashlib.sha256(canonical_json_bytes(unsigned)).hexdigest()


def build_confirmation_manifest(agent_id, prompt, duration, image_paths=None,
                                audio_paths=None, ratio="16:9", watermark=False,
                                generate_audio=True, video_paths=None, output_dir=None):
    try:
        normalized_agent_id = normalize_agent_id(agent_id)
    except ValueError as error:
        return error_result("InvalidAgentId", str(error))
    policy_error = adapter_policy_configuration_error()
    if policy_error:
        return error_result("ConfigError", policy_error, "Configuration")

    prepared = build_video_request_payload(
        prompt=prompt,
        duration=duration,
        image_paths=image_paths,
        video_paths=video_paths,
        audio_paths=audio_paths,
        ratio=ratio,
        watermark=watermark,
        generate_audio=True,
    )
    if "error" in prepared:
        return prepared

    payload_sha256 = sha256_text(canonical_json_bytes(prepared["payload"]))
    prior_attempts = list_prior_submissions(normalized_agent_id, payload_sha256)
    prior_attempt_no = max(
        [int(item.get("attempt_no") or 0) for item in prior_attempts] or [0]
    )
    retry_context = {
        "prior_attempts": prior_attempts,
        "requires_duplicate_charge_risk_confirmation": any(
            str(item.get("state") or "").strip().lower() in SUBMISSION_IN_PROGRESS_STATUSES
            for item in prior_attempts
        ),
        "message": (
            "此前存在服务端受理状态未知的同参数提交；重新生成可能导致重复任务或重复扣费。"
            if any(
                str(item.get("state") or "").strip().lower() in SUBMISSION_IN_PROGRESS_STATUSES
                for item in prior_attempts
            )
            else None
        ),
    }
    now_epoch = time.time()
    resolved_output_dir = absolute_path_str(
        resolve_video_output_dir(agent_id=normalized_agent_id, output_dir=output_dir)
    )
    manifest = {
        "version": CONFIRMATION_MANIFEST_VERSION,
        "confirmation_id": f"seedance-{uuid.uuid4().hex}",
        "created_at": utc_iso_timestamp(now_epoch),
        "expires_at": confirmation_expiry_timestamp(now_epoch),
        "attempt_no": prior_attempt_no + 1,
        "retry_context": retry_context,
        "confirmation_policy": {
            "precharge_points": VIDEO_GENERATION_PRECHARGE_POINTS,
            "refund_rule": REFUND_RULE,
            "official_link_ttl_hours": OFFICIAL_LINK_TTL_HOURS,
            "artifact_locations": ["local", "cloud"],
            "official_link_expiry_notice": (
                f"服务方声明的官方视频链接有效期为 {OFFICIAL_LINK_TTL_HOURS} 小时；"
                "过期后到本地产物或云端产物中查看"
            ),
            "new_confirmation_required_for_every_post": True,
        },
        "request": {
            "agent_id": normalized_agent_id,
            "output_dir": resolved_output_dir,
            "prompt": prompt,
            "images": prepared["images"],
            "videos": prepared["videos"],
            "audios": prepared["audios"],
            "duration": int(duration),
            "ratio": ratio,
            "watermark": bool(watermark),
            "generate_audio": True,
        },
    }
    manifest["fingerprint"] = calculate_confirmation_fingerprint(manifest)
    return {"success": True, "manifest": manifest, "payload": prepared["payload"]}


def write_confirmation_manifest(manifest, agent_id=None, output_dir=None,
                                confirmation_output=None):
    if confirmation_output:
        target_path = Path(confirmation_output).expanduser().resolve()
        if target_path.is_dir():
            target_path = target_path / f"{manifest['confirmation_id']}.json"
    else:
        confirmation_dir = (
            resolve_video_output_dir(agent_id=agent_id, output_dir=output_dir)
            / ".confirmations"
        )
        target_path = confirmation_dir / f"{manifest['confirmation_id']}.json"

    try:
        target_path = ensure_outside_installed_skill(target_path, "确认清单路径")
    except ValueError as error:
        return error_result("InvalidOutputPath", str(error), "Configuration")

    if target_path.exists():
        return error_result(
            "ConfirmationFileExists",
            f"确认清单文件已存在，为避免覆盖已确认输入已拒绝写入: {target_path}",
            "Conflict",
        )

    target_path = target_path.expanduser().resolve()
    atomic_write_json(target_path, manifest)
    try:
        target_path.chmod(0o600)
    except OSError:
        pass
    return {"success": True, "confirmation_file": absolute_path_str(target_path)}


def validate_confirmation_manifest(manifest, expected_fingerprint=None,
                                   expected_agent_id=None):
    if not isinstance(manifest, dict):
        return error_result("InvalidConfirmation", "确认清单根对象必须是 JSON object。")
    if manifest.get("version") != CONFIRMATION_MANIFEST_VERSION:
        return error_result(
            "InvalidConfirmationVersion",
            f"不支持的确认清单版本: {manifest.get('version')}。",
        )

    stored_fingerprint = normalize_fingerprint(manifest.get("fingerprint"))
    calculated_fingerprint = calculate_confirmation_fingerprint(manifest)
    if not stored_fingerprint or not hmac.compare_digest(stored_fingerprint, calculated_fingerprint):
        return error_result(
            "ConfirmationTampered",
            "确认清单指纹校验失败：prompt、参考媒体或其他参数在准备后可能被修改。"
            "请不要继续生成；如果确实需要修改，必须重新向用户展示全部输入并取得新确认。",
            "PermissionDenied",
        )

    expected = normalize_fingerprint(expected_fingerprint)
    if not expected:
        return error_result(
            "ConfirmationFingerprintRequired",
            "必须通过 --confirm-fingerprint 传入执行器在准备阶段保留的完整指纹。",
            "PermissionDenied",
        )
    if not hmac.compare_digest(expected, stored_fingerprint):
        return error_result(
            "ConfirmationFingerprintMismatch",
            "--confirm-fingerprint 与确认清单不一致，已拒绝发起 API 请求。",
            "PermissionDenied",
            expected_fingerprint=expected,
            actual_fingerprint=stored_fingerprint,
        )

    confirmation_id = str(manifest.get("confirmation_id") or "").strip()
    created_at = str(manifest.get("created_at") or "").strip()
    expires_at = str(manifest.get("expires_at") or "").strip()
    request_data = manifest.get("request")
    if not confirmation_id or not created_at or not expires_at or not isinstance(request_data, dict):
        return error_result("InvalidConfirmation", "确认清单缺少 confirmation_id、created_at、expires_at 或 request。")
    created_epoch = parse_expiry_epoch(created_at)
    expires_epoch = parse_expiry_epoch(expires_at)
    if created_epoch is None or expires_epoch is None or expires_epoch <= created_epoch:
        return error_result("InvalidConfirmation", "确认清单的创建时间或有效期无效。", "PermissionDenied")
    if time.time() >= expires_epoch:
        return error_result(
            "ConfirmationExpired",
            "确认清单已过期，请重新准备并向用户展示新的确认清单。",
            "PermissionDenied",
        )

    policy = manifest.get("confirmation_policy")
    if not isinstance(policy, dict):
        return error_result(
            "ConfirmationPolicyRequired",
            "确认清单缺少费用、链接有效期和产物位置说明；请重新准备并向用户展示确认清单。",
            "PermissionDenied",
        )
    if policy.get("precharge_points") != VIDEO_GENERATION_PRECHARGE_POINTS:
        return error_result(
            "ConfirmationPolicyChanged",
            "确认清单中的预扣积分与当前规则不一致，请重新准备并重新确认。",
            "PermissionDenied",
        )
    if policy.get("official_link_ttl_hours") != OFFICIAL_LINK_TTL_HOURS:
        return error_result(
            "ConfirmationPolicyChanged",
            "确认清单中的官方链接有效期与当前规则不一致，请重新准备并重新确认。",
            "PermissionDenied",
        )
    if policy.get("refund_rule") != REFUND_RULE:
        return error_result(
            "ConfirmationPolicyChanged",
            "确认清单中的返还规则与当前规则不一致，请重新准备并重新确认。",
            "PermissionDenied",
        )
    if policy.get("artifact_locations") != ["local", "cloud"]:
        return error_result(
            "ConfirmationPolicyChanged",
            "确认清单缺少本地产物或云端产物说明，请重新准备并重新确认。",
            "PermissionDenied",
        )

    agent_id = str(request_data.get("agent_id") or "").strip()
    if not agent_id:
        return error_result("InvalidConfirmation", "确认清单缺少 agent_id。")
    output_dir = str(request_data.get("output_dir") or "").strip()
    if not output_dir or not Path(output_dir).is_absolute():
        return error_result(
            "InvalidConfirmation",
            "确认清单必须固定绝对本地产物目录，不能在确认后改变输出位置。",
            "PermissionDenied",
        )
    normalized_expected_agent = str(expected_agent_id or "").strip()
    if normalized_expected_agent and agent_id != normalized_expected_agent:
        return error_result(
            "ConfirmationAgentMismatch",
            f"确认清单属于 agent_id={agent_id}，当前调用为 {normalized_expected_agent}。",
            "PermissionDenied",
        )

    images = request_data.get("images")
    videos = request_data.get("videos")
    audios = request_data.get("audios")
    if not all(isinstance(items, list) for items in (images, videos, audios)):
        return error_result(
            "InvalidConfirmation",
            "确认清单中 images、videos 和 audios 必须都是数组。",
        )
    if not isinstance(request_data.get("watermark"), bool):
        return error_result("InvalidConfirmation", "确认清单中 watermark 必须是 boolean。")
    if request_data.get("generate_audio") is not True:
        return error_result(
            "GenerateAudioRequired",
            "确认清单中 generate_audio 必须为 true；禁止无声成片或把声音推给后期配音/BGM。",
        )

    prepared = build_video_request_payload(
        prompt=request_data.get("prompt"),
        duration=request_data.get("duration"),
        image_paths=images,
        video_paths=videos,
        audio_paths=audios,
        ratio=request_data.get("ratio"),
        watermark=request_data.get("watermark"),
        generate_audio=True,
    )
    if "error" in prepared:
        return prepared
    if (
        prepared["images"] != images
        or prepared["videos"] != videos
        or prepared["audios"] != audios
    ):
        return error_result(
            "InvalidConfirmation",
            "确认清单中存在重复或未规范化的媒体 URL，已拒绝发起 API 请求。",
        )

    return {
        "success": True,
        "manifest": manifest,
        "request": request_data,
        "payload": prepared["payload"],
        "confirmation_id": confirmation_id,
        "fingerprint": stored_fingerprint,
        "confirmation_policy": policy,
    }


def load_and_validate_confirmation_manifest(path, expected_fingerprint=None,
                                            expected_agent_id=None):
    confirmation_path = Path(str(path or "")).expanduser().resolve()
    if not confirmation_path.exists() or not confirmation_path.is_file():
        return error_result(
            "ConfirmationFileNotFound",
            f"确认清单文件不存在: {confirmation_path}",
            "NotFound",
        )
    try:
        with open(confirmation_path, "r", encoding="utf-8") as file:
            manifest = json.load(file)
    except json.JSONDecodeError as error:
        return error_result("InvalidConfirmationJson", f"确认清单 JSON 无效: {error}")
    except OSError as error:
        return error_result("ConfirmationReadError", f"读取确认清单失败: {error}", "IOError")

    result = validate_confirmation_manifest(
        manifest,
        expected_fingerprint=expected_fingerprint,
        expected_agent_id=expected_agent_id,
    )
    if "error" not in result:
        result["confirmation_file"] = absolute_path_str(confirmation_path)
    return result


def guess_extension_from_url(url=""):
    parsed = urllib.parse.urlparse(str(url or "").strip())
    suffix = Path(urllib.parse.unquote(parsed.path)).suffix.lower()
    return suffix if suffix else ""


def guess_extension_from_content_type(content_type=""):
    normalized = str(content_type or "").split(";", 1)[0].strip().lower()
    if not normalized:
        return ""
    ct_map = {
        "video/mp4": ".mp4", "video/webm": ".webm", "video/quicktime": ".mov",
        "image/jpeg": ".jpg", "image/png": ".png",
    }
    return ct_map.get(normalized, "") or mimetypes.guess_extension(normalized) or ""


def local_artifact_metadata_path(local_path):
    return Path(str(local_path) + ".seedance.json")


def calculate_file_sha256(local_path):
    digest = hashlib.sha256()
    with open(local_path, "rb") as file:
        while True:
            chunk = file.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def inspect_local_video_artifact(local_path, include_hash=False):
    path = Path(str(local_path or "")).expanduser()
    if not path.is_file():
        return None
    size = path.stat().st_size
    if size <= 0:
        return None
    metadata = {
        "local_video_path": absolute_path_str(path),
        "file_size": size,
        "file_mtime_utc": utc_iso_timestamp(path.stat().st_mtime),
    }
    if include_hash:
        metadata["sha256"] = calculate_file_sha256(path)
    return metadata


def write_local_artifact_metadata(local_path, task_id=None, agent_id=None, metadata=None):
    path = Path(str(local_path)).expanduser().resolve()
    artifact_metadata = {
        "schema_version": 1,
        "task_id": str(task_id or "").strip() or None,
        "agent_id": str(agent_id or "").strip() or None,
        "local_video_path": absolute_path_str(path),
        "created_at_utc": utc_iso_timestamp(),
    }
    if isinstance(metadata, dict):
        artifact_metadata.update(metadata)
    metadata_path = local_artifact_metadata_path(path)
    atomic_write_json(metadata_path, artifact_metadata)
    try:
        metadata_path.chmod(0o600)
    except OSError:
        pass
    return absolute_path_str(metadata_path)


def persist_video_to_local(video_url, agent_id=None, output_dir=None, file_stem="seedance-video", task_id=None):
    """流式下载到临时文件，校验后原子落盘，返回本地产物元数据。"""
    if not video_url:
        return None

    target_dir = resolve_video_output_dir(agent_id=agent_id, output_dir=output_dir)
    target_dir.mkdir(parents=True, exist_ok=True)

    parsed_video_url = validate_media_url(video_url)
    validate_public_dns_target(parsed_video_url)

    request = urllib.request.Request(
        video_url,
        headers={"Accept": "*/*", "User-Agent": "codex-seedance-video-gen/1.0"},
        method="GET",
    )
    opener = urllib.request.build_opener(ValidatingMediaRedirectHandler())
    with opener.open(request, timeout=300) as response:
        final_url = response.geturl()
        final_parsed = validate_media_url(final_url)
        validate_public_dns_target(final_parsed)
        content_type = response.headers.get("Content-Type", "")
        normalized_content_type = str(content_type or "").split(";", 1)[0].strip().lower()
        if normalized_content_type in {"text/html", "application/json", "text/plain"}:
            raise ValueError(f"下载接口返回了非视频内容: {content_type}")
        declared_length = str(response.headers.get("Content-Length") or "").strip()
        if declared_length:
            try:
                if int(declared_length) > MAX_VIDEO_BYTES:
                    raise ValueError(f"视频超过本地下载上限（{MAX_VIDEO_BYTES} bytes）。")
            except ValueError as error:
                if "超过本地下载上限" in str(error):
                    raise
                raise ValueError("视频响应包含无效的 Content-Length。") from error

        extension = guess_extension_from_url(final_url) or guess_extension_from_content_type(content_type) or ".mp4"
        stem = sanitize_filename_stem(file_stem or task_id or "seedance-video")
        task_suffix = sanitize_filename_stem(task_id) if task_id else "video"
        timestamp = time.strftime("%Y%m%d-%H%M%S")
        unique = uuid.uuid4().hex[:8]
        local_path = target_dir / f"{stem}-{task_suffix}-{timestamp}-{unique}{extension}"
        temp_path = local_path.with_name(f".{local_path.name}.{uuid.uuid4().hex}.part")
        digest = hashlib.sha256()
        total_size = 0
        first_chunk = b""
        try:
            with open(temp_path, "wb") as file:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    if not first_chunk:
                        first_chunk = chunk[:16]
                    if total_size + len(chunk) > MAX_VIDEO_BYTES:
                        raise ValueError(f"视频超过本地下载上限（{MAX_VIDEO_BYTES} bytes）。")
                    file.write(chunk)
                    digest.update(chunk)
                    total_size += len(chunk)
                file.flush()
                os.fsync(file.fileno())
            if total_size <= 0:
                raise ValueError("下载到的本地产物为空文件")
            if normalized_content_type.startswith("video/") is False and extension in {".mp4", ".mov", ".m4v"}:
                if len(first_chunk) < 8 or first_chunk[4:8] != b"ftyp":
                    raise ValueError("下载内容不是有效的 MP4/MOV 文件")
            elif extension == ".webm" and first_chunk and not first_chunk.startswith(b"\x1a\x45\xdf\xa3"):
                raise ValueError("下载内容不是有效的 WebM 文件")
            os.replace(temp_path, local_path)
        except Exception:
            try:
                temp_path.unlink(missing_ok=True)
            except OSError:
                pass
            raise

    artifact = {
        "local_video_path": absolute_path_str(local_path),
        "file_size": total_size,
        "sha256": digest.hexdigest(),
        "content_type": content_type,
    }
    artifact["metadata_path"] = write_local_artifact_metadata(
        local_path,
        task_id=task_id,
        agent_id=agent_id,
        metadata=artifact,
    )
    return artifact


def reconcile_task_artifacts(agent_id=None, output_dir=None, task_id=None):
    """只做本地账本对账，不请求生成接口，也不改变远端任务状态。"""
    normalized_agent_id = str(agent_id or "").strip()
    normalized_task_id = str(task_id or "").strip()
    target_dir = resolve_video_output_dir(agent_id=agent_id, output_dir=output_dir)
    target_dir.mkdir(parents=True, exist_ok=True)
    with task_records_lock(exclusive=True):
        records_data = load_task_records_unlocked(TASK_RECORDS_PATH)
        changed = records_data.get("version") != TASK_RECORDS_VERSION
        records_data["version"] = TASK_RECORDS_VERSION
        for task_record in records_data.get("tasks", []):
            if not isinstance(task_record, dict):
                continue
            before = json.dumps(task_record, ensure_ascii=False, sort_keys=True)
            normalize_task_record_schema(task_record)
            request_time = task_record.get("request_time")
            request_epoch = parse_expiry_epoch(request_time)
            if request_epoch is not None:
                task_record.setdefault("request_time_utc", utc_iso_timestamp(request_epoch))
            expiry_epoch = parse_expiry_epoch(task_record.get("official_link_expires_at"))
            if expiry_epoch is not None:
                task_record["official_link_expires_at"] = format_utc_timestamp(expiry_epoch)
            completed_epoch = parse_expiry_epoch(task_record.get("completed_at"))
            if completed_epoch is not None:
                task_record["completed_at"] = format_utc_timestamp(completed_epoch)
                task_record["completed_at_utc"] = task_record["completed_at"]
            task_record["schema_version"] = TASK_RECORDS_VERSION
            changed = changed or before != json.dumps(task_record, ensure_ascii=False, sort_keys=True)
        for submission_record in records_data.get("submissions", []):
            if not isinstance(submission_record, dict):
                continue
            before = json.dumps(submission_record, ensure_ascii=False, sort_keys=True)
            submission_record.setdefault("schema_version", TASK_RECORDS_VERSION)
            submission_record.setdefault("attempt_no", 1)
            submission_record.setdefault("outcome_status", None)
            submission_record.setdefault("outcome_message", None)
            submission_record.setdefault("result_reported_at", None)
            changed = changed or before != json.dumps(submission_record, ensure_ascii=False, sort_keys=True)
        if changed:
            atomic_write_json(TASK_RECORDS_PATH, records_data)
    records = load_task_records().get("tasks", [])
    matched = []
    missing = []
    manual_review = []
    tracked_paths = set()

    for record in records:
        if not isinstance(record, dict):
            continue
        record_task_id = str(record.get("task_id") or "").strip()
        record_agent_id = str(record.get("agent_id") or "").strip()
        if not record_task_id or (normalized_task_id and record_task_id != normalized_task_id):
            continue
        if normalized_agent_id and record_agent_id != normalized_agent_id:
            continue
        recorded_local_path = existing_local_video_path(record)
        if recorded_local_path:
            tracked_paths.add(recorded_local_path)

        candidate = recorded_local_path
        if not candidate:
            safe_task_id = sanitize_filename_stem(record_task_id)
            filename_candidates = [
                path for path in target_dir.glob(f"*{safe_task_id}*")
                if path.is_file() and not path.name.endswith(".seedance.json") and not path.name.endswith(".part")
            ]
            metadata_candidates = []
            for metadata_path in target_dir.glob("*.seedance.json"):
                try:
                    with open(metadata_path, "r", encoding="utf-8") as file:
                        metadata = json.load(file)
                except (OSError, json.JSONDecodeError):
                    continue
                if str(metadata.get("task_id") or "").strip() != record_task_id:
                    continue
                metadata_path_value = str(metadata.get("local_video_path") or "").strip()
                if metadata_path_value:
                    metadata_candidates.append(Path(metadata_path_value).expanduser())
            candidates = filename_candidates + [
                path for path in metadata_candidates if path.is_file()
            ]
            unique_candidates = {absolute_path_str(path) for path in candidates}
            if len(unique_candidates) == 1:
                candidate = next(iter(unique_candidates))
            elif len(unique_candidates) > 1:
                manual_review.append({
                    "task_id": record_task_id,
                    "reason": "multiple_local_artifacts",
                    "candidates": sorted(unique_candidates),
                })
                update_task_record_video_url(
                    task_id=record_task_id,
                    reconciliation_status="manual_review",
                )
                continue

        if candidate:
            artifact = inspect_local_video_artifact(candidate, include_hash=True)
            if artifact:
                metadata_path = local_artifact_metadata_path(candidate)
                if not metadata_path.exists():
                    try:
                        artifact["metadata_path"] = write_local_artifact_metadata(
                            candidate,
                            task_id=record_task_id,
                            agent_id=record_agent_id,
                            metadata=artifact,
                        )
                    except OSError:
                        pass
                update_task_record_video_url(
                    task_id=record_task_id,
                    video_url=artifact["local_video_path"],
                    local_video_path=artifact["local_video_path"],
                    status=record.get("status") or "succeeded",
                    remote_status=record.get("remote_status") or record.get("status") or "succeeded",
                    local_artifact_status="downloaded",
                    reconciliation_status="matched",
                    local_artifact_error=None,
                    local_artifact_metadata=artifact,
                )
                matched.append({"task_id": record_task_id, **artifact})
                tracked_paths.add(artifact["local_video_path"])
                continue

        if record.get("local_video_path"):
            update_task_record_video_url(
                task_id=record_task_id,
                local_artifact_status="missing",
                reconciliation_status="mismatch",
                local_artifact_error={
                    "code": "LocalArtifactMissing",
                    "message": "任务记录中的本地产物路径不存在。",
                },
            )
            missing.append({"task_id": record_task_id, "local_video_path": record.get("local_video_path")})

    untracked_artifacts = []
    for suffix in (".mp4", ".mov", ".webm", ".m4v"):
        for artifact_path in target_dir.glob(f"*{suffix}"):
            resolved_path = absolute_path_str(artifact_path)
            if resolved_path in tracked_paths:
                continue
            info = inspect_local_video_artifact(artifact_path, include_hash=False)
            if info:
                untracked_artifacts.append(info)

    return {
        "success": True,
        "mode": "reconcile_artifacts",
        "api_request_sent": False,
        "output_dir": absolute_path_str(target_dir),
        "matched": matched,
        "missing": missing,
        "manual_review": manual_review,
        "untracked_artifacts": untracked_artifacts,
    }


# ---- 核心业务函数 ----

def query_video_task(task_id, agent_id=None, output_dir=None):
    """查询视频生成任务的状态和结果（单次查询）"""
    try:
        reconcile_task_artifacts(agent_id=agent_id, output_dir=output_dir, task_id=task_id)
    except Exception as reconciliation_error:
        print_progress(f"本地产物对账失败，继续查询远端任务: {reconciliation_error}", flush=True)
    existing_record = get_task_record_by_id(task_id)
    if existing_record:
        recorded_status = str(existing_record.get("status") or "").strip().lower()
        if recorded_status == "official_link_expired":
            return official_link_expired_result(task_id, record=existing_record)
        if recorded_status in TERMINAL_TASK_FAILURE_STATUSES:
            return {
                "success": False,
                "status": recorded_status,
                "task_id": str(task_id or "").strip(),
                "skipped": True,
                "message": "该任务此前已失败，跳过重复查询。",
            }
        if record_link_is_expired(existing_record):
            return official_link_expired_result(task_id, record=existing_record)
        local_path = existing_local_video_path(existing_record)
        if local_path:
            return {
                "success": True,
                "status": "succeeded",
                "task_id": str(task_id or "").strip(),
                "video_url": local_path,
                "local_video_path": local_path,
                "original_video_url": existing_record.get("official_video_url")
                or existing_record.get("original_video_url"),
                "output_dir": absolute_path_str(
                    resolve_video_output_dir(agent_id=agent_id, output_dir=output_dir)
                ),
                "from_local_artifact": True,
            }

    if not DEFAULT_API_BASE_URL:
        return error_result("ConfigError", "缺少 SEEDANCE_API_BASE_URL 环境变量。", "Configuration")
    try:
        validate_service_base_url(DEFAULT_API_BASE_URL)
    except ValueError as error:
        return error_result("ConfigError", str(error), "Configuration")
    if not API_KEY:
        return api_key_error_result()

    normalized_task_id = str(task_id or "").strip()
    if not normalized_task_id or len(normalized_task_id) > 512:
        return error_result("InvalidTaskId", "task_id 为空或过长。")
    query_url = f"{BASE_URL}/{urllib.parse.quote(normalized_task_id, safe='')}"
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {API_KEY}"
    }

    try:
        result = request_json(query_url, method="GET", headers=headers, timeout=30)
        
        api_error = extract_api_error(result)
        if api_error:
            if api_result_indicates_expired(result):
                return official_link_expired_result(task_id, record=existing_record)
            # API 明确返回终态失败状态时，将失败状态和错误原因写入任务记录
            error_status = str(result.get("status") or "").strip().lower()
            if error_status in TERMINAL_TASK_FAILURE_STATUSES:
                update_task_record_video_url(
                    task_id=task_id,
                    status=error_status,
                    error=api_error,
                )
            return {"error": api_error, "task_id": task_id}
        
        status = (result.get("status") or "").lower()

        if status == "succeeded":
            content = result.get("content", {})
            if not isinstance(content, dict):
                content = {}
            video_url = content.get("video_url")
            cloud_artifact_ref = (
                content.get("cloud_artifact_ref")
                or content.get("cloud_artifact_id")
                or result.get("cloud_artifact_ref")
                or result.get("cloud_artifact_id")
            )

            if api_result_indicates_expired(result):
                return official_link_expired_result(
                    task_id,
                    record=existing_record,
                    official_video_url=video_url,
                    cloud_artifact_ref=cloud_artifact_ref,
                )
            if not video_url:
                update_task_record_video_url(
                    task_id=task_id,
                    status="succeeded",
                    official_link_status="unavailable",
                    cloud_artifact_ref=cloud_artifact_ref,
                    error={"code": "OfficialVideoUrlMissing", "message": "API 未返回官方视频链接。"},
                )
                return {
                    "success": False,
                    "status": "artifact_unavailable",
                    "task_id": task_id,
                    "cloud_artifact_ref": cloud_artifact_ref,
                    "requires_regeneration": False,
                    "error": {
                        "code": "OfficialVideoUrlMissing",
                        "message": "任务已完成但未取得官方视频链接，请到本地产物或云端产物中查看。",
                        "type": "ArtifactUnavailable",
                    },
                }

            official_link_expires_at = infer_official_link_expiry(
                record=existing_record,
                result=result,
                official_video_url=video_url,
            )
            if parse_expiry_epoch(official_link_expires_at) is not None and time.time() >= parse_expiry_epoch(official_link_expires_at):
                return official_link_expired_result(
                    task_id,
                    record=existing_record,
                    official_video_url=video_url,
                    cloud_artifact_ref=cloud_artifact_ref,
                )

            completed_at = (
                result.get("completed_at")
                or result.get("updated_at")
                or time.strftime("%Y-%m-%d %H:%M:%S", time.localtime())
            )
            update_task_record_video_url(
                task_id=task_id,
                original_video_url=video_url,
                status="succeeded",
                official_link_status="available",
                official_link_expires_at=official_link_expires_at,
                cloud_artifact_ref=cloud_artifact_ref,
                completed_at=completed_at,
                remote_status="succeeded",
                local_artifact_status="pending",
                reconciliation_status="not_checked",
            )

            # 下载并保存到本地
            local_path = None
            artifact = None
            try:
                artifact = persist_video_to_local(
                    video_url,
                    agent_id=agent_id,
                    output_dir=output_dir,
                    file_stem="seedance-video",
                    task_id=task_id,
                )
                local_path = artifact.get("local_video_path") if isinstance(artifact, dict) else artifact
            except urllib.error.HTTPError as error:
                if is_expired_http_error(error, video_url):
                    return official_link_expired_result(
                        task_id,
                        record=existing_record,
                        official_video_url=video_url,
                        cloud_artifact_ref=cloud_artifact_ref,
                    )
                error_data = {
                    "code": f"HTTP_{error.code}",
                    "message": f"下载本地产物失败: {error.reason}",
                    "type": "ArtifactUnavailable",
                }
                update_task_record_video_url(
                    task_id=task_id,
                    original_video_url=video_url,
                    status="succeeded",
                    official_link_status="available",
                    official_link_expires_at=official_link_expires_at,
                    cloud_artifact_ref=cloud_artifact_ref,
                    completed_at=completed_at,
                    error=error_data,
                    remote_status="succeeded",
                    local_artifact_status="failed",
                    reconciliation_status="mismatch",
                    local_artifact_error=error_data,
                )
                return {
                    "success": False,
                    "status": "artifact_unavailable",
                    "task_id": task_id,
                    "original_video_url": video_url,
                    "cloud_artifact_ref": cloud_artifact_ref,
                    "requires_regeneration": False,
                    "error": error_data,
                }
            except urllib.error.URLError as error:
                error_data = {
                    "code": "NetworkError",
                    "message": str(error.reason or error),
                    "type": "URLError",
                }
                update_task_record_video_url(
                    task_id=task_id,
                    original_video_url=video_url,
                    status="succeeded",
                    official_link_status="available",
                    official_link_expires_at=official_link_expires_at,
                    cloud_artifact_ref=cloud_artifact_ref,
                    completed_at=completed_at,
                    error=error_data,
                    remote_status="succeeded",
                    local_artifact_status="failed",
                    reconciliation_status="mismatch",
                    local_artifact_error=error_data,
                )
                return {
                    "success": False,
                    "status": "query_retryable",
                    "task_id": task_id,
                    "original_video_url": video_url,
                    "cloud_artifact_ref": cloud_artifact_ref,
                    "error": error_data,
                }
            except Exception as error:
                error_data = {
                    "code": "LocalArtifactWriteError",
                    "message": f"保存本地产物失败: {error}",
                    "type": "ArtifactUnavailable",
                }
                update_task_record_video_url(
                    task_id=task_id,
                    original_video_url=video_url,
                    status="succeeded",
                    official_link_status="available",
                    official_link_expires_at=official_link_expires_at,
                    cloud_artifact_ref=cloud_artifact_ref,
                    completed_at=completed_at,
                    error=error_data,
                    remote_status="succeeded",
                    local_artifact_status="failed",
                    reconciliation_status="mismatch",
                    local_artifact_error=error_data,
                )
                return {
                    "success": False,
                    "status": "artifact_unavailable",
                    "task_id": task_id,
                    "original_video_url": video_url,
                    "cloud_artifact_ref": cloud_artifact_ref,
                    "requires_regeneration": False,
                    "error": error_data,
                }

            output_dir_abs = absolute_path_str(
                resolve_video_output_dir(agent_id=agent_id, output_dir=output_dir)
            )
            local_path_abs = absolute_path_str(local_path) if local_path else None
            response = {
                "success": True,
                "status": "succeeded",
                "last_frame_url": content.get("last_frame_url"),
                "video_url": local_path_abs,
                "original_video_url": video_url,
                "output_dir": output_dir_abs,
                "task_id": task_id,
                "official_link_status": "available",
                "official_link_expires_at": official_link_expires_at,
                "cloud_artifact_ref": cloud_artifact_ref,
            }
            if local_path_abs:
                response["local_video_path"] = local_path_abs
            update_task_record_video_url(
                task_id=task_id,
                video_url=response.get("video_url"),
                local_video_path=local_path_abs,
                original_video_url=response.get("original_video_url"),
                status="succeeded",
                official_link_status="available",
                official_link_expires_at=official_link_expires_at,
                cloud_artifact_ref=cloud_artifact_ref,
                completed_at=completed_at,
                remote_status="succeeded",
                local_artifact_status="downloaded" if local_path_abs else "failed",
                reconciliation_status="matched" if local_path_abs else "mismatch",
                local_artifact_metadata=artifact if isinstance(artifact, dict) else None,
            )
            return response
        elif status in TERMINAL_TASK_FAILURE_STATUSES:
            # 保留 API 返回的原始错误结构，并将失败状态与原因写入任务记录
            error_data = result.get("error", "Unknown error")
            update_task_record_video_url(task_id=task_id, status=status, error=error_data)
            return {"success": False, "status": status, "error": error_data, "task_id": task_id}
        else:
            # 处理中等中间状态同步写入任务记录，保证 status 字段反映最新进度
            update_task_record_video_url(task_id=task_id, status=status or "processing")
            return {"success": True, "status": status or "processing", "task_id": task_id}
    except Exception as e:
        return {"error": {"code": "UnexpectedError", "message": str(e), "type": "Exception"}, "task_id": task_id}


def add_confirmation_metadata(result, confirmation_id=None, confirmation_fingerprint=None):
    if not isinstance(result, dict):
        return result
    if confirmation_id:
        result["confirmation_id"] = confirmation_id
    fingerprint = normalize_fingerprint(confirmation_fingerprint)
    if fingerprint:
        result["confirmation_fingerprint"] = fingerprint
    return result


def build_user_result_report(result):
    if not isinstance(result, dict):
        return {
            "status": "unknown",
            "message": "任务结果格式无效；未发起自动重试。",
            "requires_new_confirmation_for_regeneration": True,
        }
    status = str(result.get("status") or "").strip().lower()
    error_data = result.get("error")
    error_code = ""
    error_message = ""
    if isinstance(error_data, dict):
        error_code = str(error_data.get("code") or "").strip()
        error_message = str(error_data.get("message") or "").strip()
    if status == "succeeded":
        local_path = result.get("local_video_path") or result.get("video_url")
        message = (
            "本次视频任务已完成。"
            f"本地产物: {local_path or '未记录'}；"
            f"官方链接有效期为 {OFFICIAL_LINK_TTL_HOURS} 小时，过期后请到本地产物或云端产物中查看。"
            "如需再次生成，必须重新准备确认清单并取得新的用户确认。"
        )
    elif status == "official_link_expired":
        message = OFFICIAL_LINK_EXPIRED_MESSAGE
    elif status in {"artifact_unavailable", "query_retryable"}:
        message = (
            "远端任务已经完成，但本地产物当前不可用；不会自动重新生成。"
            "请先到云端产物查看，如需重新生成必须重新准备确认清单并取得新的用户确认。"
        )
    elif status in TERMINAL_TASK_FAILURE_STATUSES:
        message = (
            f"本次视频任务未成功（状态: {status}，错误: {error_code or '未提供'}）；"
            "系统不会自动重新生成，如需重试必须重新准备确认清单并取得新的用户确认。"
        )
    elif error_code in {
        "ConfirmationRequired",
        "ConfirmationManifestRequired",
        "ExplicitUserConfirmationRequired",
        "ConfirmationIdMismatch",
        "ConfirmedInputMismatch",
        "ConfirmationExpired",
        "RetryRiskConfirmationRequired",
        "ConfirmationAlreadyUsed",
        "ConfirmationSubmissionConflict",
        "SubmissionInProgress",
    }:
        message = result.get("message") or error_message or "必须重新准备确认清单并取得新的用户确认。"
    elif error_code in {"SubmissionUnknown", "InvalidResponse"} or result.get("requires_submission_recovery"):
        message = (
            "本次 POST 的服务端受理状态未知；系统不会自动重新提交，避免重复扣费。"
            "请先按 submission_key 恢复查询；如需重新生成，必须重新准备确认清单并取得新的用户确认。"
        )
    elif error_code in {"SubmissionRejected", "APIError"}:
        message = (
            "本次提交未创建可确认的远端任务；系统不会自动重新提交。"
            "如需重新生成，必须重新准备确认清单并取得新的用户确认。"
        )
    elif error_code == "TimeoutError":
        message = (
            "轮询等待已超时，但远端任务仍可按 task_id 查询；系统不会创建新的 POST。"
            "如需再次生成，必须重新准备确认清单并取得新的用户确认。"
        )
    else:
        message = result.get("message") or "本次任务已返回结果；系统不会自动重新生成。"

    report_status = status or error_code or "unknown"
    report_status = {
        "submissionunknown": "submission_unknown",
        "submissionrejected": "retry_eligible",
        "confirmationalreadyused": "confirmation_already_used",
        "retryriskconfirmationrequired": "retry_confirmation_required",
    }.get(report_status.replace("_", "").replace("-", "").lower(), report_status.lower())
    return {
        "status": report_status,
        "message": message,
        "task_id": result.get("task_id"),
        "submission_key": result.get("submission_key"),
        "precharge_points": VIDEO_GENERATION_PRECHARGE_POINTS,
        "official_link_ttl_hours": OFFICIAL_LINK_TTL_HOURS,
        "requires_new_confirmation_for_regeneration": True,
        "api_request_sent": bool(result.get("api_request_sent")),
        "reported_at": utc_iso_timestamp(),
    }


def finalize_generation_result(result, confirmation_id=None, confirmation_fingerprint=None,
                               submission_key=None):
    result = add_confirmation_metadata(
        result,
        confirmation_id=confirmation_id,
        confirmation_fingerprint=confirmation_fingerprint,
    )
    if not isinstance(result, dict):
        return result
    normalized_submission_key = str(
        submission_key or result.get("submission_key") or ""
    ).strip()
    task_id = str(result.get("task_id") or "").strip()
    if not normalized_submission_key and task_id:
        normalized_submission_key = find_submission_key_for_task(task_id) or ""
    if normalized_submission_key:
        result["submission_key"] = normalized_submission_key
    report = build_user_result_report(result)
    result["result_report"] = report
    result["requires_new_confirmation_for_regeneration"] = True
    if not result.get("message"):
        result["message"] = report["message"]
    if normalized_submission_key:
        report_submission_outcome(
            normalized_submission_key,
            report["status"],
            report["message"],
            error=result.get("error"),
            task_id=task_id or None,
        )
    return result


def recover_submission(submission_key, agent_id=None, output_dir=None):
    """恢复查询提交结果；该入口只允许 GET/本地对账，绝不 POST。"""
    record = get_submission_record(submission_key, agent_id=agent_id)
    if record is None:
        return error_result(
            "SubmissionNotFound",
            "找不到对应 submission_key，无法安全恢复查询。",
            "NotFound",
            submission_key=str(submission_key or "").strip(),
            api_request_sent=False,
        )

    normalized_key = str(record.get("submission_key") or "").strip()
    task_id = str(record.get("task_id") or "").strip()
    if task_id:
        recovered = query_video_task(task_id, agent_id=agent_id, output_dir=output_dir)
        recovered["recovered_from_submission_key"] = normalized_key
        recovered["api_request_sent"] = False
        return finalize_generation_result(recovered, submission_key=normalized_key)

    state = str(record.get("state") or "submission_unknown").strip().lower()
    if state in {"retry_eligible", "rejected"}:
        status = "retry_eligible"
        message = (
            "已确认上一次请求没有可查询的远端任务；本次不会自动重新提交。"
            "如需重新生成，请重新准备确认清单并取得用户明确确认。"
        )
    else:
        status = "submission_unknown"
        message = (
            "上一次 POST 的服务端受理状态仍无法确认；本次不会自动重新提交，避免重复扣费。"
            "如需重新生成，必须先向用户说明该风险，再重新准备确认清单并取得新的用户确认。"
        )
    result = {
        "success": False,
        "status": status,
        "submission_key": normalized_key,
        "api_request_sent": False,
        "requires_submission_recovery": status == "submission_unknown",
        "requires_new_confirmation_for_regeneration": True,
        "may_duplicate_charge": status == "submission_unknown",
        "message": message,
    }
    return finalize_generation_result(result, submission_key=normalized_key)


def generate_video_task(prompt, duration=None, image_paths=None, audio_paths=None,
                        ratio="16:9", watermark=False, generate_audio=True,
                        poll_interval=60, max_wait=1800, agent_id=None, output_dir=None,
                        confirmation_id=None, confirmation_fingerprint=None,
                        video_paths=None, user_confirmation=None, confirmation_file=None,
                        confirm_retry_risk=False):
    """提交视频生成任务并阻塞等待结果（一站式调用）"""
    if not str(confirmation_file or "").strip():
        return finalize_generation_result(
            error_result(
                "ConfirmationManifestRequired",
                "每次视频生成都必须使用已向用户展示的 confirmation_file。",
                "PermissionDenied",
            ),
            confirmation_id=confirmation_id,
            confirmation_fingerprint=confirmation_fingerprint,
        )
    if not str(confirmation_id or "").strip() or len(normalize_fingerprint(confirmation_fingerprint)) != 64:
        return finalize_generation_result(
            error_result(
                "ConfirmationRequired",
                "每次视频生成都必须先完成用户确认，并由执行器内部使用对应的 confirmation_id 和完整指纹。",
                "PermissionDenied",
            ),
            confirmation_id=confirmation_id,
            confirmation_fingerprint=confirmation_fingerprint,
        )
    if not is_explicit_user_confirmation(user_confirmation):
        return finalize_generation_result(
            error_result(
                "ExplicitUserConfirmationRequired",
                "必须先取得用户明确回复“确认”或“就按这份生成”，才能发起视频生成。",
                "PermissionDenied",
            ),
            confirmation_id=confirmation_id,
            confirmation_fingerprint=confirmation_fingerprint,
        )

    ensure_result = resolve_missing_task_video_urls_before_generation(
        agent_id=agent_id,
        output_dir=output_dir,
    )
    if ensure_result.get("requires_user_confirmation"):
        return finalize_generation_result(
            ensure_result,
            confirmation_id=confirmation_id,
            confirmation_fingerprint=confirmation_fingerprint,
        )

    # 1. 提交任务
    submit_result = submit_video_task(
        prompt=prompt,
        duration=duration,
        image_paths=image_paths,
        video_paths=video_paths,
        audio_paths=audio_paths,
        ratio=ratio,
        watermark=watermark,
        generate_audio=generate_audio,
        agent_id=agent_id,
        confirmation_id=confirmation_id,
        confirmation_fingerprint=confirmation_fingerprint,
        user_confirmation=user_confirmation,
        confirmation_file=confirmation_file,
        confirm_retry_risk=confirm_retry_risk,
    )
    if "error" in submit_result:
        return finalize_generation_result(
            submit_result,
            confirmation_id=confirmation_id,
            confirmation_fingerprint=confirmation_fingerprint,
            submission_key=submit_result.get("submission_key"),
        )

    task_id = submit_result["task_id"]
    print_progress(f"任务已提交，task_id: {task_id}", flush=True)
    print_progress(f"Polling (every {poll_interval}s, max {max_wait}s)...", flush=True)

    # 2. 阻塞轮询
    elapsed = 0
    poll_count = 0
    poll_error_count = 0
    max_poll_errors = 3
    while elapsed < max_wait:
        poll_count += 1
        result = query_video_task(task_id, agent_id=agent_id, output_dir=output_dir)
        if isinstance(result, dict):
            result.setdefault("api_request_sent", True)

        # 任务已提交后，查询阶段的临时网络/解析错误不要立即视为生成失败。
        if "error" in result and "status" not in result:
            error_data = result.get("error") or {}
            error_code = error_data.get("code") if isinstance(error_data, dict) else ""
            error_type = error_data.get("type") if isinstance(error_data, dict) else ""
            if error_code in {"NetworkError", "UnexpectedError"} or error_type in {"URLError", "Exception"}:
                poll_error_count += 1
                if poll_error_count <= max_poll_errors:
                    print_progress(
                        f"Polling warning ({poll_error_count}/{max_poll_errors}): "
                        f"{error_code or error_type}; retrying...",
                        end="\r",
                        flush=True,
                    )
                    time.sleep(poll_interval)
                    elapsed += poll_interval
                    continue
            return finalize_generation_result(
                result,
                confirmation_id=confirmation_id,
                confirmation_fingerprint=confirmation_fingerprint,
                submission_key=submit_result.get("submission_key"),
            )

        poll_error_count = 0
        status = result.get("status", "")

        if status == "succeeded":
            print_progress("\nVideo generated successfully!", flush=True)
            return finalize_generation_result(
                result,
                confirmation_id=confirmation_id,
                confirmation_fingerprint=confirmation_fingerprint,
                submission_key=submit_result.get("submission_key"),
            )
        elif status in ("failed", "cancelled", "unknown"):
            return finalize_generation_result(
                result,
                confirmation_id=confirmation_id,
                confirmation_fingerprint=confirmation_fingerprint,
                submission_key=submit_result.get("submission_key"),
            )
        elif status in {"official_link_expired", "artifact_unavailable"}:
            return finalize_generation_result(
                result,
                confirmation_id=confirmation_id,
                confirmation_fingerprint=confirmation_fingerprint,
                submission_key=submit_result.get("submission_key"),
            )

        # 覆盖式进度输出
        print_progress(f"Task in progress (status={status}, poll #{poll_count}, "
                       f"{elapsed}s elapsed)...", end="\r", flush=True)
        time.sleep(poll_interval)
        elapsed += poll_interval

    return finalize_generation_result(
        {"error": {"code": "TimeoutError",
         "message": f"Polling timeout ({max_wait}s)",
         "type": "Timeout"}, "task_id": task_id, "api_request_sent": True},
        confirmation_id=confirmation_id,
        confirmation_fingerprint=confirmation_fingerprint,
        submission_key=submit_result.get("submission_key"),
    )


def print_json_result(result):
    print(json.dumps(result, ensure_ascii=False))
    return 1 if isinstance(result, dict) and result.get("error") else 0


def collect_media_for_confirmation(args, media_type):
    configs = {
        "image": {
            "plural": "images", "expected": "expected_image_count",
            "label": "图片",
        },
        "video": {
            "plural": "videos", "expected": "expected_video_count",
            "label": "视频",
        },
        "audio": {
            "plural": "audios", "expected": "expected_audio_count",
            "label": "音频",
        },
    }
    config = configs[media_type]
    plural = config["plural"]
    expected_name = config["expected"]
    expected_count = getattr(args, expected_name, None)

    if expected_count is None:
        return error_result(
            f"Expected{media_type.title()}CountRequired",
            f"准备确认清单时必须显式传入 --expected-{media_type}-count。"
            f"没有参考{config['label']}时传 0；有素材时传用户将要确认的准确数量。",
        )
    if expected_count < 0:
        return error_result(
            f"Invalid{media_type.title()}Count",
            f"--expected-{media_type}-count 不能为负数。",
        )

    cli_urls = [str(item or "").strip() for item in (getattr(args, plural, None) or [])]
    media_urls = cli_urls
    media_source = "explicit_cli" if media_urls else f"confirmed_no_{plural}"

    if len(media_urls) != expected_count:
        return error_result(
            f"{media_type.title()}CountMismatch",
            f"用户将要确认 {expected_count} 个参考{config['label']}，"
            f"但脚本实际收集到 {len(media_urls)} 个。"
            "已在 API 请求前停止；不得删减、替换素材或改用其他模式试跑。",
            **{
                expected_name: expected_count,
                f"actual_{media_type}_count": len(media_urls),
                plural: media_urls,
                f"{media_type}_source": media_source,
            },
        )
    return {
        "success": True,
        plural: media_urls,
        f"{media_type}_source": media_source,
    }


def collect_images_for_confirmation(args):
    return collect_media_for_confirmation(args, "image")


def collect_videos_for_confirmation(args):
    return collect_media_for_confirmation(args, "video")


def collect_audios_for_confirmation(args):
    return collect_media_for_confirmation(args, "audio")


def main():
    parser = argparse.ArgumentParser(description="Seedance 2.0 Video Generation")
    parser.add_argument("prompt", type=str, nargs="?", help="仅用于准备确认清单或查询历史")
    parser.add_argument(
        "--agent-id",
        type=str,
        default=DEFAULT_AGENT_ID,
        help="外部视频网关要求的路由 ID；也可通过 SEEDANCE_AGENT_ID 配置",
    )
    parser.add_argument("--output-dir", type=str, help="本地视频保存目录")
    parser.add_argument("--duration", type=int, help="视频时长（秒），支持 4-15")
    parser.add_argument("--ratio", type=str, default=None, choices=["16:9", "9:16"], help="准备阶段默认 16:9")
    parser.add_argument(
        "--image",
        action="append",
        dest="images",
        help="准备阶段的参考图片 URL，可多次使用；顺序必须与用户上传顺序一致，禁止按文件名重排",
    )
    parser.add_argument(
        "--video",
        action="append",
        dest="videos",
        help="准备阶段的参考视频 URL，可多次使用；顺序必须与用户上传顺序一致，禁止按文件名重排",
    )
    parser.add_argument(
        "--audio",
        action="append",
        dest="audios",
        help="准备阶段的参考音频 URL，可多次使用；顺序必须与用户上传顺序一致，禁止按文件名重排",
    )
    parser.add_argument("--expected-image-count", type=int, help="用户将要确认的参考图片数；没有则传 0")
    parser.add_argument("--expected-video-count", type=int, help="用户将要确认的参考视频数；没有则传 0")
    parser.add_argument("--expected-audio-count", type=int, help="用户将要确认的参考音频数；没有则传 0")
    parser.add_argument("--watermark", action="store_true", help="准备阶段设置添加水印")
    parser.add_argument("--poll-interval", type=int, default=60, help="轮询间隔（秒）")
    parser.add_argument("--max-wait", type=int, default=1800, help="最大等待时间（秒）")
    parser.add_argument("--confirm-charge", action="store_true", help="用户已确认费用规则和当前确认清单")
    parser.add_argument(
        "--confirm-retry-risk",
        action="store_true",
        help="用户已看到此前未知提交可能重复扣费的风险，并确认创建新的生成尝试",
    )
    parser.add_argument(
        "--user-confirmation",
        type=str,
        choices=sorted(EXPLICIT_USER_CONFIRMATIONS),
        help="用户明确回复的确认文本，只接受“确认”或“就按这份生成”",
    )

    mode_group = parser.add_mutually_exclusive_group()
    mode_group.add_argument("--prepare-confirmation", action="store_true", help="离线固化 prompt/参考媒体/参数，不请求 API")
    mode_group.add_argument("--confirmation-file", type=str, help="从已确认清单生成或 dry-run")
    mode_group.add_argument("--query-history", action="store_true", help="查询历史任务")
    mode_group.add_argument("--recover-submission", type=str, help="按 submission_key 恢复查询，不发起 POST")
    mode_group.add_argument("--reconcile-artifacts", action="store_true", help="只对账本地视频和任务 JSON，不请求 API")

    parser.add_argument("--confirmation-output", type=str, help="准备阶段的确认清单输出路径")
    parser.add_argument("--confirm-fingerprint", type=str, help="准备阶段返回、仅供执行器内部校验的完整 SHA-256 指纹")
    parser.add_argument("--dry-run", action="store_true", help="校验确认清单并输出实际 payload，不请求 API")

    parser.add_argument("--task-id", type=str, help="直接按 task_id 查询任务")
    parser.add_argument("--keyword", action="append", dest="keywords", help="按提示词关键词筛选历史任务")
    parser.add_argument("--time-from", type=str, help="格式 YYYY-MM-DD HH:mm:ss")
    parser.add_argument("--time-to", type=str, help="格式 YYYY-MM-DD HH:mm:ss")
    parser.add_argument(
        "--time-keyword",
        type=str,
        choices=[
            "today", "yesterday", "this-hour", "last-10-minutes",
            "今天", "昨天", "这一小时", "最近一小时", "最近10分钟", "十分钟前后",
        ],
    )
    parser.add_argument("--list-matches", type=int, default=10)

    args = parser.parse_args()

    try:
        args.agent_id = normalize_agent_id(args.agent_id)
    except ValueError as error:
        return print_json_result(error_result(
            "AgentIdRequired",
            f"{error} 必须通过 --agent-id 或 SEEDANCE_AGENT_ID 配置；不要把 Codex 任务 ID 猜作该值。",
            "Configuration",
        ))
    policy_error = adapter_policy_configuration_error()
    if policy_error:
        return print_json_result(error_result("ConfigError", policy_error, "Configuration"))

    if args.poll_interval < 1 or args.max_wait < 1:
        return print_json_result(error_result(
            "InvalidPollingParameter",
            "--poll-interval 和 --max-wait 必须是大于 0 的整数。",
        ))

    if args.reconcile_artifacts:
        if args.confirm_charge or args.confirm_fingerprint or args.user_confirmation or args.confirm_retry_risk or args.dry_run:
            return print_json_result(error_result("InvalidModeArguments", "本地对账不接受生成确认参数。"))
        return print_json_result(reconcile_task_artifacts(
            agent_id=args.agent_id,
            output_dir=args.output_dir,
            task_id=args.task_id,
        ))

    if args.recover_submission:
        if args.confirm_charge or args.confirm_fingerprint or args.user_confirmation or args.confirm_retry_risk or args.dry_run:
            return print_json_result(error_result("InvalidModeArguments", "提交恢复不接受生成确认参数。"))
        return print_json_result(recover_submission(
            args.recover_submission,
            agent_id=args.agent_id,
            output_dir=args.output_dir,
        ))

    if args.query_history:
        if args.dry_run or args.confirm_charge or args.confirm_fingerprint or args.user_confirmation or args.confirm_retry_risk:
            return print_json_result(error_result("InvalidModeArguments", "历史查询不接受生成确认参数。"))
        result = query_history_task(
            agent_id=args.agent_id,
            task_id=args.task_id,
            prompt_query=args.prompt,
            keywords=args.keywords,
            time_from=args.time_from,
            time_to=args.time_to,
            time_keyword=args.time_keyword,
            limit=args.list_matches,
            output_dir=args.output_dir,
        )
        return print_json_result(result)

    if args.prepare_confirmation:
        if args.confirm_charge or args.confirm_fingerprint or args.user_confirmation or args.confirm_retry_risk or args.dry_run:
            return print_json_result(error_result(
                "InvalidModeArguments",
                "--prepare-confirmation 是不请求 API 的准备阶段，不能携带生成确认或 dry-run 参数。",
            ))
        if args.prompt is None or args.duration is None:
            return print_json_result(error_result(
                "InvalidParameter",
                "准备确认清单时必须提供完整 prompt 和 --duration。",
            ))
        collected_images = collect_images_for_confirmation(args)
        if "error" in collected_images:
            return print_json_result(collected_images)
        collected_videos = collect_videos_for_confirmation(args)
        if "error" in collected_videos:
            return print_json_result(collected_videos)
        collected_audios = collect_audios_for_confirmation(args)
        if "error" in collected_audios:
            return print_json_result(collected_audios)
        built = build_confirmation_manifest(
            agent_id=args.agent_id,
            prompt=args.prompt,
            duration=args.duration,
            image_paths=collected_images["images"],
            video_paths=collected_videos["videos"],
            audio_paths=collected_audios["audios"],
            ratio=args.ratio or "16:9",
            watermark=args.watermark,
            generate_audio=True,
            output_dir=args.output_dir,
        )
        if "error" in built:
            return print_json_result(built)
        written = write_confirmation_manifest(
            built["manifest"],
            agent_id=args.agent_id,
            output_dir=args.output_dir,
            confirmation_output=args.confirmation_output,
        )
        if "error" in written:
            return print_json_result(written)

        manifest = built["manifest"]
        request_data = manifest["request"]
        warnings = [
            item["warning"]
            for item in (collected_images, collected_videos, collected_audios)
            if isinstance(item, dict) and item.get("warning")
        ]
        result = {
            "success": True,
            "mode": "prepare_confirmation",
            "api_request_sent": False,
            "confirmation_file": absolute_path_str(written["confirmation_file"]),
            "output_dir": absolute_path_str(
                resolve_video_output_dir(agent_id=args.agent_id, output_dir=args.output_dir)
            ),
            "confirmation_id": manifest["confirmation_id"],
            "confirmation_fingerprint": manifest["fingerprint"],
            "expires_at": manifest["expires_at"],
            "attempt_no": manifest["attempt_no"],
            "retry_context": manifest["retry_context"],
            "prompt": request_data["prompt"],
            "prompt_sha256": sha256_text(request_data["prompt"]),
            "images": request_data["images"],
            "image_count": len(request_data["images"]),
            "image_source": collected_images["image_source"],
            "videos": request_data["videos"],
            "video_count": len(request_data["videos"]),
            "video_source": collected_videos["video_source"],
            "audios": request_data["audios"],
            "audio_count": len(request_data["audios"]),
            "audio_source": collected_audios["audio_source"],
            "duration": request_data["duration"],
            "ratio": request_data["ratio"],
            "watermark": request_data["watermark"],
            "generate_audio": request_data["generate_audio"],
            "confirmation_policy": manifest["confirmation_policy"],
            "precharge_points": manifest["confirmation_policy"]["precharge_points"],
            "official_link_ttl_hours": manifest["confirmation_policy"]["official_link_ttl_hours"],
            "artifact_locations": manifest["confirmation_policy"]["artifact_locations"],
            "message": (
                "请将上述 prompt、图片/视频/音频列表、参数和服务方费用策略逐项展示给用户；"
                "确认指纹仅供执行器内部校验，不向用户展示。"
                "用户确认前禁止生成。确认表必须原样照抄本结果中的完整 URL，"
                "禁止拆分或重组 attachment id 与文件名。"
                "音频生成必须展示为「是」/true，禁止写「否」或后期配音/BGM。"
                f"服务方声明每个视频会先预扣 {VIDEO_GENERATION_PRECHARGE_POINTS} 积分；"
                f"返还规则：{REFUND_RULE}；"
                f"官方视频链接有效期为 {OFFICIAL_LINK_TTL_HOURS} 小时，链接过期后请到本地产物或云端产物中查看。"
            ),
        }
        if warnings:
            result["warnings"] = warnings
        return print_json_result(result)

    if args.confirmation_file:
        conflicts = []
        if args.prompt is not None:
            conflicts.append("prompt")
        if args.duration is not None:
            conflicts.append("--duration")
        if args.ratio is not None:
            conflicts.append("--ratio")
        if args.images:
            conflicts.append("--image")
        if args.videos:
            conflicts.append("--video")
        if args.audios:
            conflicts.append("--audio")
        if args.expected_image_count is not None:
            conflicts.append("--expected-image-count")
        if args.expected_video_count is not None:
            conflicts.append("--expected-video-count")
        if args.expected_audio_count is not None:
            conflicts.append("--expected-audio-count")
        if args.watermark:
            conflicts.append("--watermark")
        if args.confirmation_output:
            conflicts.append("--confirmation-output")
        if args.output_dir:
            conflicts.append("--output-dir")
        if conflicts:
            return print_json_result(error_result(
                "ConfirmedInputOverrideRejected",
                "正式生成或 dry-run 时禁止覆盖已确认输入: " + ", ".join(conflicts),
                "PermissionDenied",
            ))
        if args.dry_run and (args.confirm_charge or args.confirm_retry_risk or args.user_confirmation):
            return print_json_result(error_result(
                "InvalidModeArguments",
                "--dry-run 不得携带确认参数，以免将离线校验误认为已生成。",
            ))

        validated = load_and_validate_confirmation_manifest(
            args.confirmation_file,
            expected_fingerprint=args.confirm_fingerprint,
            expected_agent_id=args.agent_id,
        )
        if "error" in validated:
            return print_json_result(validated)
        retry_context = validated.get("manifest", {}).get("retry_context") or {}
        requires_retry_risk_confirmation = bool(
            retry_context.get("requires_duplicate_charge_risk_confirmation")
        )
        if requires_retry_risk_confirmation and not args.confirm_retry_risk:
            return print_json_result(error_result(
                "RetryRiskConfirmationRequired",
                "此前存在服务端受理状态未知的同参数提交；必须向用户说明可能重复扣费风险，"
                "并传入 --confirm-retry-risk 和新的用户确认后才能发起新的 POST。",
                "PermissionDenied",
            ))
        if args.confirm_retry_risk and not requires_retry_risk_confirmation:
            return print_json_result(error_result(
                "InvalidModeArguments",
                "当前确认清单没有未知提交风险，不需要 --confirm-retry-risk。",
            ))
        if args.dry_run:
            request_data = validated["request"]
            return print_json_result({
                "success": True,
                "mode": "dry_run",
                "api_request_sent": False,
                "confirmation_file": validated["confirmation_file"],
                "confirmation_id": validated["confirmation_id"],
                "confirmation_fingerprint": validated["fingerprint"],
                "prompt_sha256": sha256_text(request_data["prompt"]),
                "image_count": len(request_data["images"]),
                "video_count": len(request_data["videos"]),
                "audio_count": len(request_data["audios"]),
                "confirmation_policy": validated["confirmation_policy"],
                "request_payload": validated["payload"],
            })
        if not args.confirm_charge or not is_explicit_user_confirmation(args.user_confirmation):
            return print_json_result(error_result(
                "ConfirmationRequired",
                f"生成 Seedance 视频前必须先告知用户服务方配置：每个视频会先预扣 {VIDEO_GENERATION_PRECHARGE_POINTS} 积分；"
                f"返还规则：{REFUND_RULE}；官方视频链接有效期为 {OFFICIAL_LINK_TTL_HOURS} 小时，"
                "链接过期后请到本地产物或云端产物中查看；"
                "并让用户明确回复“确认”或“就按这份生成”，确认该清单的完整 prompt、参考媒体列表、参数和费用说明。",
                "PermissionDenied",
            ))

        request_data = validated["request"]
        result = generate_video_task(
            prompt=request_data["prompt"],
            duration=request_data["duration"],
            image_paths=request_data["images"],
            video_paths=request_data["videos"],
            audio_paths=request_data["audios"],
            ratio=request_data["ratio"],
            watermark=request_data["watermark"],
            generate_audio=request_data["generate_audio"],
            poll_interval=args.poll_interval,
            max_wait=args.max_wait,
            agent_id=request_data["agent_id"],
            output_dir=request_data["output_dir"],
            confirmation_id=validated["confirmation_id"],
            confirmation_fingerprint=validated["fingerprint"],
            user_confirmation=args.user_confirmation,
            confirmation_file=validated["confirmation_file"],
            confirm_retry_risk=args.confirm_retry_risk,
        )
        return print_json_result(result)

    if args.dry_run:
        return print_json_result(error_result(
            "ConfirmationFileRequired",
            "--dry-run 必须与 --confirmation-file 和 --confirm-fingerprint 一起使用。",
        ))
    return print_json_result(error_result(
        "ConfirmationManifestRequired",
        "为保证提示词和参考媒体不在重试中被改动，禁止直接生成。"
        "请先使用 --prepare-confirmation 生成离线确认清单；用户明确回复后，再用 --confirmation-file、"
        "--confirm-fingerprint、--confirm-charge 和 --user-confirmation 生成。",
        "PermissionDenied",
    ))


if __name__ == "__main__":
    sys.exit(main())
