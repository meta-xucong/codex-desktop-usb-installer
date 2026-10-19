#!/usr/bin/env python3
"""Upload and verify a local I2V reference image using the fixed Uguu flow."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


MAX_BYTES = 8 * 1024 * 1024
ALLOWED_MIME = {"image/jpeg", "image/png", "image/webp", "image/gif"}
UPLOAD_ENDPOINT = "https://uguu.se/upload.php"


class UploadError(ValueError):
    pass


def _reject_output_overwrite(output_path: Path, input_path: Path) -> None:
    """Prevent the JSON report from replacing the reference image."""
    try:
        resolved_output = output_path.resolve()
        resolved_input = input_path.resolve()
    except (OSError, RuntimeError) as exc:
        raise UploadError(f"cannot verify output/input paths safely: {exc}") from exc
    if resolved_output == resolved_input:
        raise UploadError("output path must not overwrite the input reference image")
    try:
        if os.path.samefile(resolved_output, resolved_input):
            raise UploadError("output path must not overwrite the input reference image")
    except FileNotFoundError:
        # The output may not exist yet; direct aliases were checked above.
        pass
    except OSError as exc:
        raise UploadError(f"cannot verify output/input file identity safely: {exc}") from exc


def detect_mime(data: bytes) -> str:
    if data.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if data.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    if len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    raise UploadError("unsupported image signature; expected JPEG, PNG, WebP, or GIF")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def inspect_local(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise UploadError(f"input file does not exist: {path}")
    data = path.read_bytes()
    if not data:
        raise UploadError("input image is empty")
    if len(data) > MAX_BYTES:
        raise UploadError(f"input image exceeds {MAX_BYTES} bytes: {len(data)}")
    mime = detect_mime(data)
    return {
        "path": str(path.resolve()),
        "filename": path.name,
        "mime": mime,
        "bytes": len(data),
        "sha256": sha256(data),
        "data": data,
    }


def _content_type(value: str | None) -> str:
    return (value or "").split(";", 1)[0].strip().lower()


def _require_absolute_https_url(value: Any, label: str) -> None:
    if not isinstance(value, str):
        raise UploadError(f"reference {label} must be an absolute HTTPS URL")
    parsed = urlparse(value)
    if parsed.scheme.lower() != "https" or not parsed.netloc:
        raise UploadError(f"reference {label} must be an absolute HTTPS URL")


def parse_uguu_response(payload: Any) -> str:
    if not isinstance(payload, dict) or payload.get("success") is not True:
        raise UploadError("Uguu response did not report success")
    files = payload.get("files")
    if not isinstance(files, list) or len(files) != 1 or not isinstance(files[0], dict):
        raise UploadError("Uguu response must contain exactly one uploaded file")
    url = files[0].get("url")
    if not isinstance(url, str) or not url.startswith("https://"):
        raise UploadError("Uguu response did not contain an HTTPS image URL")
    parsed = urlparse(url)
    if not parsed.netloc or parsed.query or parsed.fragment:
        raise UploadError("Uguu returned an invalid image URL")
    return url


def upload_to_uguu(local: dict[str, Any], timeout: float = 90.0) -> str:
    try:
        import requests
    except ImportError as exc:
        raise UploadError("the bundled Python runtime must include requests") from exc
    headers = {"User-Agent": "Codex-Grok-Video-Prompt-Compiler/1.0"}
    session = requests.Session()
    session.trust_env = False
    try:
        response = session.post(
            UPLOAD_ENDPOINT,
            headers=headers,
            timeout=timeout,
            allow_redirects=True,
            files={"files[]": (local["filename"], local["data"], local["mime"])},
        )
    except requests.RequestException as exc:
        raise UploadError(f"Uguu upload request failed: {type(exc).__name__}") from exc
    if response.status_code < 200 or response.status_code >= 300:
        raise UploadError(f"Uguu upload failed with HTTP {response.status_code}")
    try:
        payload = response.json()
    except ValueError as exc:
        raise UploadError("Uguu upload returned non-JSON response") from exc
    return parse_uguu_response(payload)


def verify_remote(url: str, local: dict[str, Any], timeout: float = 60.0) -> dict[str, Any]:
    parsed = urlparse(url)
    if parsed.scheme != "https" or not parsed.netloc:
        raise UploadError("reference URL must be a valid HTTPS URL")
    try:
        import requests
    except ImportError as exc:
        raise UploadError("the bundled Python runtime must include requests") from exc
    headers = {"User-Agent": "Codex-Grok-Video-Prompt-Compiler/1.0"}
    chunks: list[bytes] = []
    total = 0
    session = requests.Session()
    session.trust_env = False
    try:
        with session.get(url, headers=headers, timeout=timeout, allow_redirects=True, stream=True) as response:
            history = getattr(response, "history", None)
            if not isinstance(history, (list, tuple)):
                raise UploadError("reference URL response has invalid redirect history")
            for index, hop in enumerate(history, start=1):
                _require_absolute_https_url(getattr(hop, "url", None), f"redirect hop {index}")
            _require_absolute_https_url(getattr(response, "url", None), "final response URL")
            if response.status_code < 200 or response.status_code >= 300:
                raise UploadError(f"reference URL verification failed with HTTP {response.status_code}")
            mime = _content_type(response.headers.get("content-type"))
            if mime not in ALLOWED_MIME:
                raise UploadError(f"reference URL returned unsupported MIME: {mime or 'missing'}")
            for chunk in response.iter_content(65536):
                if not chunk:
                    continue
                total += len(chunk)
                if total > MAX_BYTES:
                    raise UploadError(f"reference URL exceeds {MAX_BYTES} bytes")
                chunks.append(chunk)
    except UploadError:
        raise
    except requests.RequestException as exc:
        raise UploadError(f"reference URL request failed: {type(exc).__name__}") from exc
    remote_data = b"".join(chunks)
    remote_hash = sha256(remote_data)
    if remote_hash != local["sha256"]:
        raise UploadError("remote image SHA-256 differs from local file")
    return {
        "verified": True,
        "remote_mime": mime,
        "remote_bytes": len(remote_data),
        "remote_sha256": remote_hash,
    }


def upload_and_verify(path: Path) -> dict[str, Any]:
    local = inspect_local(path)
    url = upload_to_uguu(local)
    remote = verify_remote(url, local)
    local.pop("data", None)
    return {"reference_image_url": url, **local, **remote}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.output:
            _reject_output_overwrite(args.output, args.input)
        report = upload_and_verify(args.input)
        encoded = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            args.output.write_text(encoded + "\n", encoding="utf-8")
        else:
            print(encoded)
        return 0
    except (OSError, UploadError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
