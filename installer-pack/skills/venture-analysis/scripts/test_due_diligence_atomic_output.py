from __future__ import annotations

import hashlib
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import due_diligence


class _PartialWriteThenFail:
    def __init__(self, wrapped):
        self.wrapped = wrapped

    def __enter__(self):
        self.wrapped.__enter__()
        return self

    def __exit__(self, exc_type, exc, traceback):
        return self.wrapped.__exit__(exc_type, exc, traceback)

    def write(self, text):
        self.wrapped.write(text[:37])
        self.wrapped.flush()
        raise OSError("synthetic disk full after partial checklist write")

    def flush(self):
        return self.wrapped.flush()

    def fileno(self):
        return self.wrapped.fileno()


class DueDiligenceAtomicOutputTests(unittest.TestCase):
    def setUp(self):
        repo_tmp = Path(__file__).resolve().parents[3] / "tmp"
        self.temp_dir = tempfile.TemporaryDirectory(prefix="s047-dd-output-", dir=repo_tmp)
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)
        self.base_args = [
            "--stage", "天使轮",
            "--jurisdiction", "Synthetic jurisdiction",
            "--industry", "Synthetic industry",
            "--transaction-structure", "Synthetic equity",
        ]

    def test_partial_staged_write_preserves_destination_and_cleans_temp(self):
        target = self.root / "checklist.md"
        original = b"PRESERVE-EXISTING-CHECKLIST-SENTINEL"
        target.write_bytes(original)
        before_sha = hashlib.sha256(original).hexdigest()
        real_fdopen = os.fdopen

        def partial_fdopen(*args, **kwargs):
            return _PartialWriteThenFail(real_fdopen(*args, **kwargs))

        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(due_diligence.os, "fdopen", side_effect=partial_fdopen), \
                patch("sys.stdout", stdout), patch("sys.stderr", stderr):
            result = due_diligence.main([*self.base_args, "--output", str(target)])

        self.assertEqual(result, 1)
        self.assertEqual(target.read_bytes(), original)
        self.assertEqual(hashlib.sha256(target.read_bytes()).hexdigest(), before_sha)
        self.assertEqual(list(self.root.glob(".checklist.md.*.tmp")), [])
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(
            stderr.getvalue(),
            "清单生成失败：synthetic disk full after partial checklist write\n",
        )

    def test_successful_markdown_output(self):
        target = self.root / "checklist.md"
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch("sys.stdout", stdout), patch("sys.stderr", stderr):
            result = due_diligence.main([*self.base_args, "--output", str(target)])

        self.assertEqual(result, 0)
        self.assertTrue(target.read_text(encoding="utf-8").startswith("# 天使轮尽职调查起点清单\n"))
        self.assertIn("## 使用前必须定制", target.read_text(encoding="utf-8"))
        self.assertIn(f"尽调起点清单已保存到：{target.resolve()}", stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(list(self.root.glob(".checklist.md.*.tmp")), [])

    def test_successful_json_output(self):
        target = self.root / "checklist.json"
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch("sys.stdout", stdout), patch("sys.stderr", stderr):
            result = due_diligence.main([*self.base_args, "--json", "--output", str(target)])

        self.assertEqual(result, 0)
        parsed = json.loads(target.read_text(encoding="utf-8"))
        self.assertEqual(parsed["context"]["stage"], "天使轮")
        self.assertGreater(parsed["summary"]["item_count"], 0)
        self.assertIn(f"尽调起点清单已保存到：{target.resolve()}", stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(list(self.root.glob(".checklist.json.*.tmp")), [])


if __name__ == "__main__":
    unittest.main()
