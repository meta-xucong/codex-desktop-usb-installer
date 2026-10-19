#!/usr/bin/env python3
"""CLI regression tests for reference image/output path aliases."""

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
import upload_reference  # noqa: E402


class UploadReferenceOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(
            dir=Path(__file__).resolve().parents[4] / "tmp"
        )
        self.root = Path(self.temp_dir.name)
        self.input_path = self.root / "reference.jpg"
        self.original = b"\xff\xd8\xffsynthetic reference"
        self.input_path.write_bytes(self.original)

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _digest(self) -> str:
        return hashlib.sha256(self.input_path.read_bytes()).hexdigest()

    def _run(self, output_path: Path) -> tuple[int, str]:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            result = upload_reference.main(
                ["--input", str(self.input_path), "--output", str(output_path)]
            )
        return result, stderr.getvalue()

    def test_rejects_same_input_before_upload_or_source_modification(self) -> None:
        before = self._digest()
        with patch.object(upload_reference, "upload_and_verify") as upload:
            result, stderr = self._run(self.input_path)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        upload.assert_not_called()
        self.assertEqual(self._digest(), before)

    def test_rejects_normalized_input_alias_before_upload(self) -> None:
        before = self._digest()
        alias = self.root / "missing" / ".." / "reference.jpg"
        with patch.object(upload_reference, "upload_and_verify") as upload:
            result, stderr = self._run(alias)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        upload.assert_not_called()
        self.assertEqual(self._digest(), before)

    def test_allows_distinct_output_and_preserves_image(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        with patch.object(
            upload_reference,
            "upload_and_verify",
            return_value={"verified": True, "reference_image_url": "https://example.invalid/ref"},
        ) as upload:
            result, stderr = self._run(output_path)
        self.assertEqual(result, 0, stderr)
        upload.assert_called_once()
        self.assertTrue(output_path.is_file())
        self.assertTrue(json.loads(output_path.read_text(encoding="utf-8"))["verified"])
        self.assertEqual(self._digest(), before)

    def test_rejects_samefile_alias(self) -> None:
        output_path = self.root / "other-name.json"
        output_path.write_text("{}", encoding="utf-8")
        before = self._digest()
        with patch.object(upload_reference.os.path, "samefile", return_value=True):
            with patch.object(upload_reference, "upload_and_verify") as upload:
                result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("must not overwrite", stderr)
        upload.assert_not_called()
        self.assertEqual(self._digest(), before)

    def test_fails_closed_when_identity_check_is_denied(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        with patch.object(upload_reference.os.path, "samefile", side_effect=PermissionError("denied")):
            with patch.object(upload_reference, "upload_and_verify") as upload:
                result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("cannot verify", stderr)
        upload.assert_not_called()
        self.assertFalse(output_path.exists())
        self.assertEqual(self._digest(), before)

    def test_fails_closed_when_path_resolution_detects_symlink_loop(self) -> None:
        output_path = self.root / "report.json"
        before = self._digest()
        with patch.object(Path, "resolve", side_effect=RuntimeError("symlink loop")):
            with patch.object(upload_reference, "upload_and_verify") as upload:
                result, stderr = self._run(output_path)
        self.assertEqual(result, 2)
        self.assertIn("cannot verify", stderr)
        upload.assert_not_called()
        self.assertFalse(output_path.exists())
        self.assertEqual(self._digest(), before)


if __name__ == "__main__":
    unittest.main()
