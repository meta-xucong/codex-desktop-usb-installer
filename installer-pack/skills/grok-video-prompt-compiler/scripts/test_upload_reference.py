#!/usr/bin/env python3
"""Offline tests for the fixed reference upload flow."""

from __future__ import annotations

import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from upload_reference import UploadError, detect_mime, inspect_local, parse_uguu_response  # noqa: E402


def test_detects_supported_signatures() -> None:
    assert detect_mime(b"\xff\xd8\xff" + b"x") == "image/jpeg"
    assert detect_mime(b"\x89PNG\r\n\x1a\n" + b"x") == "image/png"
    assert detect_mime(b"GIF89a" + b"x") == "image/gif"
    assert detect_mime(b"RIFF1234WEBP" + b"x") == "image/webp"


def test_rejects_unknown_signature() -> None:
    try:
        detect_mime(b"not-an-image")
    except UploadError:
        pass
    else:
        raise AssertionError("unknown signature should fail")


def test_inspection_reports_hash_and_mime() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "reference.jpg"
        path.write_bytes(b"\xff\xd8\xffsample")
        report = inspect_local(path)
        assert report["mime"] == "image/jpeg"
        assert report["bytes"] == len(b"\xff\xd8\xffsample")
        assert len(report["sha256"]) == 64


def test_parses_only_one_https_uguu_url() -> None:
    payload = {"success": True, "files": [{"url": "https://n.uguu.se/example.jpg"}]}
    assert parse_uguu_response(payload).startswith("https://")
    for invalid in [
        {"success": False, "files": []},
        {"success": True, "files": []},
        {"success": True, "files": [{"url": "http://example.com/a.jpg"}]},
        {"success": True, "files": [{"url": "https://example.com/a.jpg?x=1"}]},
    ]:
        try:
            parse_uguu_response(invalid)
        except UploadError:
            pass
        else:
            raise AssertionError("invalid Uguu response should fail")


def test_upload_and_remote_verification_use_bundled_requests_offline() -> None:
    import upload_reference

    image = b"\xff\xd8\xffsample"
    local = {"filename": "ref.jpg", "data": image, "mime": "image/jpeg", "sha256": upload_reference.sha256(image)}
    events: list[tuple[str, dict[str, object]]] = []

    class FakeResponse:
        status_code = 200
        headers = {"content-type": "image/jpeg; charset=binary"}

        def json(self):
            return {"success": True, "files": [{"url": "https://n.uguu.se/ref.jpg"}]}

        def iter_content(self, chunk_size):
            assert chunk_size == 65536
            yield image

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

    class FakeSession:
        trust_env = True

        def post(self, url, **kwargs):
            events.append(("POST", {"url": url, "trust_env": self.trust_env, **kwargs}))
            return FakeResponse()

        def get(self, url, **kwargs):
            events.append(("GET", {"url": url, "trust_env": self.trust_env, **kwargs}))
            return FakeResponse()

    requests_stub = types.SimpleNamespace(Session=FakeSession, RequestException=Exception)
    with mock.patch.dict(sys.modules, {"requests": requests_stub}):
        uploaded_url = upload_reference.upload_to_uguu(local)
        verified = upload_reference.verify_remote(uploaded_url, local)

    assert uploaded_url == "https://n.uguu.se/ref.jpg"
    assert verified["verified"] is True
    assert verified["remote_sha256"] == local["sha256"]
    assert len(events) == 2
    method, upload = events[0]
    assert method == "POST" and upload["url"] == upload_reference.UPLOAD_ENDPOINT
    assert upload["trust_env"] is False and upload["allow_redirects"] is True
    assert upload["timeout"] == 90.0 and "files" in upload
    method, verify = events[1]
    assert method == "GET" and verify["trust_env"] is False and verify["stream"] is True
    assert verify["allow_redirects"] is True and verify["timeout"] == 60.0


if __name__ == "__main__":
    tests = [
        test_detects_supported_signatures,
        test_rejects_unknown_signature,
        test_inspection_reports_hash_and_mime,
        test_parses_only_one_https_uguu_url,
        test_upload_and_remote_verification_use_bundled_requests_offline,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    print(f"{len(tests)} tests passed")
