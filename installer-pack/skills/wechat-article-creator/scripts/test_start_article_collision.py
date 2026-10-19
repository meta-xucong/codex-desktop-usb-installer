"""Timestamp collisions must never overwrite an existing article draft."""

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import start_article


ROOT = Path(__file__).resolve().parents[4]
REAL_PATH_OPEN = Path.open


class PartialWriter:
    """Write a prefix then simulate a disk failure after exclusive creation."""

    def __init__(self, handle):
        self.handle = handle

    def __enter__(self):
        return self

    def write(self, text):
        self.handle.write(text[:32])
        self.handle.flush()
        raise OSError("injected disk write failure")

    def __exit__(self, exc_type, exc, traceback):
        self.handle.close()
        return False


def partial_write_open(path, mode="r", *args, **kwargs):
    handle = REAL_PATH_OPEN(path, mode, *args, **kwargs)
    if mode == "x":
        return PartialWriter(handle)
    return handle


class StartArticleCollisionTests(unittest.TestCase):
    def _create_first_draft(self, temporary, fixed_datetime):
        with patch.object(start_article, "resolve_article_data_dir", return_value=Path(temporary)), \
             patch.object(start_article, "datetime", fixed_datetime), \
             contextlib.redirect_stdout(io.StringIO()):
            first = Path(start_article.create_article("同标题", "首篇主题"))
        sentinel = b"USER_EDIT_SENTINEL\r\n"
        first.write_bytes(sentinel)
        return first, sentinel

    def test_fixed_clock_same_title_uses_distinct_exclusive_paths(self):
        with tempfile.TemporaryDirectory(prefix="article-draft-collision-", dir=ROOT / "tmp") as temporary:
            timestamp = "20261019_123456_123456"
            fixed_datetime = Mock()
            fixed_datetime.now.return_value.strftime.return_value = timestamp

            with patch.object(start_article, "resolve_article_data_dir", return_value=Path(temporary)), \
                 patch.object(start_article, "datetime", fixed_datetime):
                first = Path(start_article.create_article("同标题", "首篇主题"))
                sentinel = b"USER_EDIT_SENTINEL\r\n"
                first.write_bytes(sentinel)
                second = Path(start_article.create_article("同标题", "第二篇主题"))

            self.assertNotEqual(first, second)
            self.assertEqual(first.read_bytes(), sentinel)
            self.assertEqual(second.name, f"{timestamp}_同标题_2.md")
            second_text = second.read_text(encoding="utf-8")
            self.assertIn("# 同标题", second_text)
            self.assertIn("**主题：** 第二篇主题", second_text)
            self.assertIn("## 发布前检查清单", second_text)

    def test_write_failure_removes_only_partial_new_draft(self):
        with tempfile.TemporaryDirectory(prefix="article-draft-write-failure-", dir=ROOT / "tmp") as temporary:
            fixed_datetime = Mock()
            fixed_datetime.now.return_value.strftime.return_value = "20261019_123456_123456"
            first, sentinel = self._create_first_draft(temporary, fixed_datetime)
            with patch.object(start_article, "resolve_article_data_dir", return_value=Path(temporary)), \
                 patch.object(start_article, "datetime", fixed_datetime), \
                 patch.object(Path, "open", new=partial_write_open):
                with self.assertRaisesRegex(OSError, "injected disk write failure"):
                    start_article.create_article("同标题", "失败主题")

            self.assertEqual(first.read_bytes(), sentinel)
            self.assertEqual(list(first.parent.glob("*.md")), [first])

    def test_cli_write_failure_is_concise_and_nonzero(self):
        with tempfile.TemporaryDirectory(prefix="article-draft-cli-failure-", dir=ROOT / "tmp") as temporary:
            fixed_datetime = Mock()
            fixed_datetime.now.return_value.strftime.return_value = "20261019_123456_123456"
            first, sentinel = self._create_first_draft(temporary, fixed_datetime)
            stdout = io.StringIO()
            stderr = io.StringIO()
            with patch.object(start_article, "resolve_article_data_dir", return_value=Path(temporary)), \
                 patch.object(start_article, "datetime", fixed_datetime), \
                 patch.object(Path, "open", new=partial_write_open), \
                 patch.object(sys, "argv", ["start_article.py", "同标题", "--topic", "失败主题"]), \
                 contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exit_code = start_article.main()

            self.assertEqual(exit_code, 1)
            self.assertIn("Error: failed to create article draft: injected disk write failure", stderr.getvalue())
            self.assertNotIn("Traceback", stderr.getvalue())
            self.assertEqual(stdout.getvalue(), "")
            self.assertEqual(first.read_bytes(), sentinel)
            self.assertEqual(list(first.parent.glob("*.md")), [first])


if __name__ == "__main__":
    unittest.main()
