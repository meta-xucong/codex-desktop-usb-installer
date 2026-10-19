from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import pdf_tool


class _FakeReader:
    def __init__(self, _path: str) -> None:
        self.pages: list[object] = []


class _PartialFailureWriter:
    def add_page(self, _page: object) -> None:
        pass

    def write(self, handle) -> None:
        handle.write(b"PARTIAL-PDF")
        raise OSError("synthetic writer failure after partial bytes")


class _SuccessfulWriter:
    def add_page(self, _page: object) -> None:
        pass

    def write(self, handle) -> None:
        handle.write(b"%PDF-synthetic-merged-content")


class PdfToolMergeAtomicWriteTests(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(prefix="pdf-merge-atomic-")
        self.root = Path(self._temporary.name)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def _patch_imports(self, writer) -> patch:
        return patch.object(pdf_tool, "_imports", return_value=(_FakeReader, lambda: writer, None, None))

    def _temporary_outputs(self, destination: Path) -> list[Path]:
        return list(destination.parent.glob(f".{destination.name}.*.merge-tmp"))

    def test_partial_write_failure_preserves_existing_destination_and_cleans_temp(self) -> None:
        source = self.root / "input.pdf"
        destination = self.root / "merged.pdf"
        source.write_bytes(b"synthetic source")
        original = b"PRESERVE-OLD-OUTPUT-SENTINEL"
        destination.write_bytes(original)

        with self._patch_imports(_PartialFailureWriter()):
            with self.assertRaisesRegex(OSError, "after partial bytes"):
                pdf_tool.merge(str(destination), [str(source)])

        self.assertEqual(destination.read_bytes(), original)
        self.assertEqual(self._temporary_outputs(destination), [])

    def test_success_publishes_staged_output_and_cleans_temp_name(self) -> None:
        source = self.root / "input.pdf"
        destination = self.root / "merged.pdf"
        source.write_bytes(b"synthetic source")
        destination.write_bytes(b"old-output")

        with self._patch_imports(_SuccessfulWriter()):
            self.assertEqual(pdf_tool.merge(str(destination), [str(source)]), 0)

        self.assertEqual(destination.read_bytes(), b"%PDF-synthetic-merged-content")
        self.assertEqual(self._temporary_outputs(destination), [])


if __name__ == "__main__":
    unittest.main()
