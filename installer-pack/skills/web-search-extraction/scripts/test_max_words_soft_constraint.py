"""Offline contract tests for the adapter's model-prompt-only word limit."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import unittest
from unittest import mock


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_PATH = SKILL_DIR / "scripts" / "web_search.py"
SKILL_PATH = SKILL_DIR / "SKILL.md"


def load_adapter():
    spec = importlib.util.spec_from_file_location("web_search_adapter", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load adapter from {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeResponse:
    def __init__(self, body: bytes):
        self.body = body

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, _limit: int) -> bytes:
        return self.body


class MaxWordsSoftConstraintTests(unittest.TestCase):
    def test_skill_describes_prompt_only_soft_limit_and_overflow_handling(self):
        text = SKILL_PATH.read_text(encoding="utf-8")
        self.assertIn("只会把长度要求写入发给模型的自然语言提示", text)
        self.assertIn("是软约束", text)
        self.assertIn("模型可能返回超长内容", text)
        self.assertIn("不保证严格字数上限", text)
        self.assertIn("重新请求缩短", text)
        self.assertIn("交由用户审核", text)

    def test_overlong_upstream_content_is_returned_unchanged(self):
        adapter = load_adapter()
        content = "模型返回的内容明显超过两字上限。"
        import json

        upstream = json.dumps(
            {"choices": [{"message": {"content": content}}]}, ensure_ascii=False
        ).encode("utf-8")
        with mock.patch.dict(
            os.environ,
            {"DASHSCOPE_API_KEY": "offline-test-key", "DASHSCOPE_MODEL": "offline-model"},
            clear=False,
        ), mock.patch.object(
            adapter.urllib.request, "urlopen", return_value=FakeResponse(upstream)
        ):
            result = adapter.web_search("offline test", max_words=2)

        self.assertEqual(result, {"success": True, "content": content})
        self.assertGreater(len(content), 2)


if __name__ == "__main__":
    unittest.main()
