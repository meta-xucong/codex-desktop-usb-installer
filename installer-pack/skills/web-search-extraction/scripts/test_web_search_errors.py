"""Offline security and contract tests for the optional web-search adapter."""

import io
import json
import os
from unittest import TestCase
from unittest.mock import patch
from urllib.error import HTTPError, URLError

import web_search


API_KEY = "SYNTHETIC_ONLY_NOT_A_REAL_KEY_abc123"


class FakeResponse:
    def __init__(self, body):
        self.body = body

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def read(self, limit=-1):
        return self.body[:limit]


class WebSearchErrorTests(TestCase):
    def call_with(self, urlopen_side_effect=None, urlopen_return=None, **env_overrides):
        env = {
            "DASHSCOPE_API_KEY": API_KEY,
            "DASHSCOPE_MODEL": "synthetic-offline-model",
            "DASHSCOPE_BASE_URL": web_search.DEFAULT_BASE_URL,
        }
        env.update(env_overrides)
        with patch.dict(os.environ, env, clear=True):
            with patch.object(
                web_search.urllib.request,
                "urlopen",
                side_effect=urlopen_side_effect,
                return_value=urlopen_return,
            ) as mocked_urlopen:
                result = web_search.web_search("synthetic offline query")
        mocked_urlopen.assert_called_once()
        return result

    def test_missing_api_key_stops_before_network(self):
        env = {
            "DASHSCOPE_API_KEY": "",
            "DASHSCOPE_MODEL": "synthetic-offline-model",
            "DASHSCOPE_BASE_URL": web_search.DEFAULT_BASE_URL,
        }
        with patch.dict(os.environ, env, clear=True):
            with patch.object(web_search.urllib.request, "urlopen") as mocked_urlopen:
                result = web_search.web_search("synthetic offline query")
        self.assertEqual(
            result,
            {"error": "缺少 DASHSCOPE_API_KEY；请通过当前用户的环境变量显式配置凭据。"},
        )
        mocked_urlopen.assert_not_called()

    def test_missing_model_stops_before_network(self):
        env = {
            "DASHSCOPE_API_KEY": API_KEY,
            "DASHSCOPE_MODEL": "",
            "DASHSCOPE_BASE_URL": web_search.DEFAULT_BASE_URL,
        }
        with patch.dict(os.environ, env, clear=True):
            with patch.object(web_search.urllib.request, "urlopen") as mocked_urlopen:
                result = web_search.web_search("synthetic offline query")
        self.assertEqual(
            result,
            {"error": "缺少 DASHSCOPE_MODEL；请显式配置服务当前支持的模型，不使用猜测默认值。"},
        )
        mocked_urlopen.assert_not_called()

    def test_boolean_max_words_stops_before_network(self):
        env = {
            "DASHSCOPE_API_KEY": API_KEY,
            "DASHSCOPE_MODEL": "synthetic-offline-model",
            "DASHSCOPE_BASE_URL": web_search.DEFAULT_BASE_URL,
        }
        with patch.dict(os.environ, env, clear=True):
            with patch.object(web_search.urllib.request, "urlopen") as mocked_urlopen:
                result = web_search.web_search("synthetic offline query", max_words=True)
        self.assertEqual(result, {"error": "max_words 必须是 1 到 5000 的整数。"})
        mocked_urlopen.assert_not_called()

    def test_http_error_body_echo_does_not_disclose_api_key(self):
        echoed = f"upstream reflected Authorization: Bearer {API_KEY}"
        error = HTTPError(
            "https://example.invalid/", 401, "Unauthorized", {}, io.BytesIO(echoed.encode())
        )
        result = self.call_with(urlopen_side_effect=error)
        self.assertEqual(result, {"error": "HTTP 401: 上游错误正文已隐藏。"})
        self.assertNotIn(API_KEY, json.dumps(result))
        self.assertNotIn("Authorization", json.dumps(result))

    def test_url_error_reason_is_not_reflected(self):
        result = self.call_with(urlopen_side_effect=URLError(f"failure echoed {API_KEY}"))
        self.assertEqual(result, {"error": "网络请求失败：无法连接上游服务。"})
        self.assertNotIn(API_KEY, json.dumps(result))

    def test_success_response_contract_is_preserved(self):
        body = {"choices": [{"message": {"content": "Synthetic result with source: https://example.invalid"}}]}
        result = self.call_with(urlopen_return=FakeResponse(json.dumps(body).encode("utf-8")))
        self.assertEqual(result, {"success": True, "content": body["choices"][0]["message"]["content"]})

    def test_oversized_response_fails_without_echoing_body(self):
        with patch.object(web_search, "MAX_RESPONSE_BYTES", 16):
            result = self.call_with(urlopen_return=FakeResponse(b"SYNTHETIC_BODY_" * 10))
        self.assertEqual(result, {"error": "接口响应超过 16 bytes，已停止解析。"})
        self.assertNotIn(API_KEY, json.dumps(result))

    def test_invalid_utf8_response_fails_with_stable_error(self):
        result = self.call_with(urlopen_return=FakeResponse(b"\xff"))
        self.assertEqual(result, {"error": "请求或响应解析失败；上游错误详情已隐藏。"})
        self.assertNotIn(API_KEY, json.dumps(result))

    def test_excessively_nested_json_fails_with_stable_error(self):
        body = b"[" * 10000 + b"0" + b"]" * 10000
        self.assertLess(len(body), web_search.MAX_RESPONSE_BYTES)
        result = self.call_with(urlopen_return=FakeResponse(body))
        self.assertEqual(result, {"error": "请求或响应解析失败；上游错误详情已隐藏。"})

    def test_integer_digit_limit_fails_with_stable_error(self):
        body = b'{"choices":[],"unused":' + b"9" * 5000 + b"}"
        self.assertLess(len(body), web_search.MAX_RESPONSE_BYTES)
        result = self.call_with(urlopen_return=FakeResponse(body))
        self.assertEqual(result, {"error": "请求或响应解析失败；上游错误详情已隐藏。"})

    def test_empty_response_fails_with_stable_message(self):
        result = self.call_with(urlopen_return=FakeResponse(b"{}"))
        self.assertEqual(result, {"error": "接口返回为空：未找到 message.content"})

    def test_malformed_json_shapes_fail_with_stable_error(self):
        malformed_bodies = (
            [],
            "synthetic",
            17,
            {"choices": "synthetic"},
            {"choices": ["synthetic"]},
            {"choices": [{"message": "synthetic"}]},
            {"choices": [{"message": {"content": ["synthetic"]}}]},
        )
        for body in malformed_bodies:
            with self.subTest(body_type=type(body).__name__, body=body):
                result = self.call_with(
                    urlopen_return=FakeResponse(json.dumps(body).encode("utf-8"))
                )
                self.assertEqual(
                    result,
                    {"error": "接口返回格式无效：choices/message/content 类型不符合要求。"},
                )

    def test_json_escaped_secret_is_redacted_by_error_helper(self):
        unusual_key = 'SYNTHETIC/"\\KEY'
        escaped_key = json.dumps(unusual_key, ensure_ascii=False)[1:-1]
        result = web_search._error_result(f"upstream reflected {escaped_key}", unusual_key)
        self.assertNotIn(unusual_key, result["error"])
        self.assertNotIn(escaped_key, result["error"])
        self.assertIn("[REDACTED]", result["error"])


if __name__ == "__main__":
    import unittest

    unittest.main()
