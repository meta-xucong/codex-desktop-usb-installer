from __future__ import annotations

import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import pdf_tool


class PdfToolOutputAliasTests(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(prefix="pdf-output-alias-")
        self.root = Path(self._temporary.name)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _sha256(self, path: Path) -> str:
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def test_extract_rejects_same_path_without_changing_source(self) -> None:
        source = self.root / "sample.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        before = self._sha256(source)

        with self.assertRaisesRegex(ValueError, "output path must differ"):
            pdf_tool.extract(str(source), str(source))

        self.assertEqual(self._sha256(source), before)
        self.assertEqual(pdf_tool.info(str(source)), 0)

    def test_extract_cli_fails_without_overwriting_source(self) -> None:
        source = self.root / "sample.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        before = self._sha256(source)
        env = os.environ.copy()
        env["TEMP"] = str(self.root)
        env["TMP"] = str(self.root)

        result = subprocess.run(
            [sys.executable, "-B", str(Path(pdf_tool.__file__).resolve()),
             "extract", str(source), "--output", str(source)],
            capture_output=True, text=True, check=False, env=env,
        )

        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("output path must differ", result.stderr)
        self.assertEqual(self._sha256(source), before)

    def test_extract_rejects_normalized_path_alias_and_preserves_separate_output(self) -> None:
        source = self.root / "sample.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        alias = self.root / "nested" / ".." / "sample.pdf"
        before = self._sha256(source)

        with self.assertRaisesRegex(ValueError, "output path must differ"):
            pdf_tool.extract(str(source), str(alias))

        self.assertEqual(self._sha256(source), before)
        output = self.root / "extract.txt"
        self.assertEqual(pdf_tool.extract(str(source), str(output)), 0)
        self.assertEqual(output.read_text(encoding="utf-8").strip(), "preserve-me")
        self.assertEqual(self._sha256(source), before)

    def test_merge_rejects_input_as_output_without_changing_source(self) -> None:
        source = self.root / "sample.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        before = self._sha256(source)

        with self.assertRaisesRegex(ValueError, "output path must differ"):
            pdf_tool.merge(str(source), [str(source)])

        self.assertEqual(self._sha256(source), before)
        self.assertEqual(pdf_tool.info(str(source)), 0)

    def test_merge_rejects_hardlink_alias_when_supported(self) -> None:
        source = self.root / "sample.pdf"
        alias = self.root / "hardlink.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        try:
            alias.hardlink_to(source)
        except OSError as exc:
            self.skipTest(f"hard links are unavailable in this environment: {exc}")
        before = self._sha256(source)

        with self.assertRaisesRegex(ValueError, "output path must differ"):
            pdf_tool.merge(str(alias), [str(source)])

        self.assertEqual(self._sha256(source), before)

    def test_extract_rejects_filesystem_samefile_alias(self) -> None:
        source = self.root / "sample.pdf"
        destination = self.root / "alias.pdf"
        pdf_tool.generate(str(source), "preserve-me")
        destination.write_bytes(b"separate path used to exercise samefile check")
        before = self._sha256(source)

        with patch("pdf_tool.os.path.samefile", return_value=True):
            with self.assertRaisesRegex(ValueError, "output path must differ"):
                pdf_tool.extract(str(source), str(destination))

        self.assertEqual(self._sha256(source), before)
        self.assertEqual(destination.read_bytes(), b"separate path used to exercise samefile check")


if __name__ == "__main__":
    unittest.main()
