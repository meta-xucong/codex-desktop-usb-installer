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
        url = "https://n.uguu.se/ref.jpg"
        history = []
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


def test_remote_verification_requires_https_for_final_url_and_every_redirect_hop() -> None:
    import upload_reference

    image = b"\xff\xd8\xffsynthetic-image"
    local = {"sha256": upload_reference.sha256(image)}

    class FakeHop:
        def __init__(self, url: str):
            self.url = url

    class FakeResponse:
        status_code = 200
        headers = {"content-type": "image/jpeg"}

        def __init__(self, final_url: str, history: list[FakeHop] | None = None,
                     status_code: int = 200, mime: str = "image/jpeg", data: bytes = image):
            self.url = final_url
            self.history = history or []
            self.status_code = status_code
            self.headers = {"content-type": mime}
            self.data = data

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def iter_content(self, _chunk_size):
            yield self.data

    class FakeRequestException(Exception):
        pass

    class FakeSession:
        trust_env = True

        def __init__(self, response):
            self.response = response

        def get(self, _url, **kwargs):
            assert kwargs["allow_redirects"] is True
            return self.response

    requests_stub = types.SimpleNamespace(Session=None, RequestException=FakeRequestException)
    cases = (
        (FakeResponse("http://mirror.invalid/final.jpg"), "final response URL"),
        (FakeResponse("https://mirror.invalid/final.jpg", [FakeHop("https://n.uguu.se/start.jpg"),
                                                           FakeHop("http://mirror.invalid/downgrade.jpg")]),
         "redirect hop 2"),
    )
    for response, message in cases:
        requests_stub.Session = lambda response=response: FakeSession(response)
        with mock.patch.dict(sys.modules, {"requests": requests_stub}):
            try:
                upload_reference.verify_remote("https://n.uguu.se/start.jpg", local)
            except UploadError as error:
                assert message in str(error)
            else:
                raise AssertionError("HTTP redirect destination must never be verified")


def test_remote_verification_keeps_https_status_mime_size_and_hash_checks() -> None:
    import upload_reference

    image = b"\xff\xd8\xffsynthetic-image"

    class FakeResponse:
        def __init__(self, *, status=200, mime="image/jpeg", data=image):
            self.status_code = status
            self.url = "https://n.uguu.se/ref.jpg"
            self.history = [type("Hop", (), {"url": "https://n.uguu.se/start.jpg"})()]
            self.headers = {"content-type": mime}
            self.data = data

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def iter_content(self, _chunk_size):
            yield self.data

    class FakeRequestException(Exception):
        pass

    class FakeSession:
        trust_env = True

        def __init__(self, response):
            self.response = response

        def get(self, _url, **_kwargs):
            return self.response

    requests_stub = types.SimpleNamespace(Session=None, RequestException=FakeRequestException)

    def check(response, local, message):
        requests_stub.Session = lambda: FakeSession(response)
        with mock.patch.dict(sys.modules, {"requests": requests_stub}):
            try:
                return upload_reference.verify_remote("https://n.uguu.se/ref.jpg", local)
            except UploadError as error:
                if message is None:
                    raise
                assert message in str(error)
                return None
        raise AssertionError("expected remote verification failure")

    local = {"sha256": upload_reference.sha256(image)}
    verified = check(FakeResponse(), local, None)
    assert verified["verified"] is True
    assert verified["remote_sha256"] == local["sha256"]

    check(FakeResponse(status=404), local, "HTTP 404")
    check(FakeResponse(mime="text/html"), local, "unsupported MIME")
    with mock.patch.object(upload_reference, "MAX_BYTES", 5):
        check(FakeResponse(), local, "exceeds 5 bytes")
    check(FakeResponse(data=b"\xff\xd8\xffdifferent"), local, "SHA-256 differs")


if __name__ == "__main__":
    tests = [
        test_detects_supported_signatures,
        test_rejects_unknown_signature,
        test_inspection_reports_hash_and_mime,
        test_parses_only_one_https_uguu_url,
        test_upload_and_remote_verification_use_bundled_requests_offline,
        test_remote_verification_requires_https_for_final_url_and_every_redirect_hop,
        test_remote_verification_keeps_https_status_mime_size_and_hash_checks,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    print(f"{len(tests)} tests passed")
