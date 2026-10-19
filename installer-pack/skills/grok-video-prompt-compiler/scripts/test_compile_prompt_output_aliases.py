#!/usr/bin/env python3
"""CLI regression tests for SceneSpec/output path alias protection."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import compile_prompt  # noqa: E402


class CompilePromptOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "scene.json"
        self.original = json.dumps(
            compile_prompt.example_spec("complex-outfit"), ensure_ascii=False, indent=2
        ).encode("utf-8")
        self.input_path.write_bytes(self.original)

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _digest(self) -> str:
        return hashlib.sha256(self.input_path.read_bytes()).hexdigest()

    def _run(self, output_path: Path) -> tuple[int, str]:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            result = compile_prompt.main(
                ["--input", str(self.input_path), "--output", str(output_path)]
            )
        return result, stderr.getvalue()

    def test_rejects_same_input_and_output_before_modifying_source(self) -> None:
        before = self._digest()
        result, stderr = self._run(self.input_path)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        self.assertEqual(self._digest(), before)

    def test_rejects_normalized_input_alias_before_modifying_source(self) -> None:
        before = self._digest()
        alias = self.root / "subdir" / ".." / "scene.json"
        result, stderr = self._run(alias)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        self.assertEqual(self._digest(), before)

    def test_allows_distinct_output_and_preserves_source(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        result, stderr = self._run(output_path)
        self.assertEqual(result, 0, stderr)
        self.assertTrue(output_path.is_file())
        self.assertIn("source_spec", json.loads(output_path.read_text(encoding="utf-8")))
        self.assertEqual(self._digest(), before)

    def test_rejects_samefile_alias(self) -> None:
        output_path = self.root / "different-name.json"
        output_path.write_text("{}", encoding="utf-8")
        before = self._digest()
        with patch.object(compile_prompt.os.path, "samefile", return_value=True):
            result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        self.assertEqual(self._digest(), before)

    def test_fails_closed_when_identity_check_is_denied(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        with patch.object(compile_prompt.os.path, "samefile", side_effect=PermissionError("denied")):
            result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output_path.exists())
        self.assertEqual(self._digest(), before)

    def test_fails_closed_when_path_resolution_detects_symlink_loop(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        with patch.object(Path, "resolve", side_effect=RuntimeError("symlink loop")):
            result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("cannot verify", stderr)
        self.assertFalse(output_path.exists())
        self.assertEqual(self._digest(), before)


if __name__ == "__main__":
    unittest.main()
