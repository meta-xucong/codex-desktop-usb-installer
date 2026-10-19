from __future__ import annotations

import contextlib
import hashlib
import io
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock


SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))


class FakePage:
    def extract_text(self):
        return "COMPLETE SYNTHETIC EXTRACTION TEXT"


class FakeDocument:
    pages = [FakePage()]

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False


def install_dependency_stubs():
    pdfplumber = types.ModuleType("pdfplumber")
    pdfplumber.open = lambda _path: FakeDocument()
    pypdf = types.ModuleType("pypdf")
    pypdf.PdfReader = object
    pypdf.PdfWriter = object
    reportlab = types.ModuleType("reportlab")
    pdfgen = types.ModuleType("reportlab.pdfgen")
    canvas_module = types.ModuleType("reportlab.pdfgen.canvas")
    canvas_module.Canvas = object
    pdfgen.canvas = canvas_module
    reportlab.pdfgen = pdfgen
    return mock.patch.dict(sys.modules, {
        "pdfplumber": pdfplumber,
        "pypdf": pypdf,
        "reportlab": reportlab,
        "reportlab.pdfgen": pdfgen,
        "reportlab.pdfgen.canvas": canvas_module,
    })


class PartialWriter:
    def __init__(self, wrapped):
        self.wrapped = wrapped

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return self.wrapped.__exit__(*args)

    def write(self, _text):
        self.wrapped.write("PARTIAL-")
        raise OSError("synthetic disk-write failure after prefix")

    def flush(self):
        return self.wrapped.flush()

    def fileno(self):
        return self.wrapped.fileno()


class PdfExtractAtomicOutputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="pdf-extract-atomic-")
        self.root = Path(self.temp.name)
        self.source = self.root / "synthetic-input.pdf"
        self.source.write_bytes(b"synthetic pdf placeholder")
        self.destination = self.root / "extracted.txt"
        with install_dependency_stubs():
            import pdf_tool
        self.pdf_tool = pdf_tool

    def tearDown(self):
        self.temp.cleanup()

    def test_failed_staged_write_preserves_destination_and_removes_temp(self):
        previous = b"PREVIOUS-REPORT-SENTINEL"
        self.destination.write_bytes(previous)
        before = hashlib.sha256(previous).hexdigest()
        real_fdopen = os.fdopen

        def injected_fdopen(descriptor, *args, **kwargs):
            return PartialWriter(real_fdopen(descriptor, *args, **kwargs))

        stdout, stderr = io.StringIO(), io.StringIO()
        with install_dependency_stubs(), mock.patch.object(self.pdf_tool.os, "fdopen", injected_fdopen):
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = self.pdf_tool.main(["extract", str(self.source), "--output", str(self.destination)])

        self.assertEqual(code, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("synthetic disk-write failure", stderr.getvalue())
        self.assertEqual(hashlib.sha256(self.destination.read_bytes()).hexdigest(), before)
        self.assertEqual(list(self.root.glob(".extracted.txt.*.extract-tmp")), [])

    def test_fdopen_failure_closes_descriptor_cleans_stage_and_preserves_original_error(self):
        previous = b"PREVIOUS-REPORT-SENTINEL"
        self.destination.write_bytes(previous)
        real_close = os.close
        real_unlink = Path.unlink
        closed_descriptors = []

        def injected_fdopen(_descriptor, *_args, **_kwargs):
            raise OSError("synthetic fdopen ownership failure")

        def close_then_raise(descriptor):
            closed_descriptors.append(descriptor)
            real_close(descriptor)
            raise OSError("synthetic close cleanup failure")

        def unlink_then_raise(path, *args, **kwargs):
            result = real_unlink(path, *args, **kwargs)
            if path.name.endswith(".extract-tmp"):
                raise OSError("synthetic unlink cleanup failure")
            return result

        stdout, stderr = io.StringIO(), io.StringIO()
        with install_dependency_stubs(), \
                mock.patch.object(self.pdf_tool.os, "fdopen", injected_fdopen), \
                mock.patch.object(self.pdf_tool.os, "close", close_then_raise), \
                mock.patch.object(Path, "unlink", unlink_then_raise):
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = self.pdf_tool.main(["extract", str(self.source), "--output", str(self.destination)])

        self.assertEqual(code, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("synthetic fdopen ownership failure", stderr.getvalue())
        self.assertNotIn("cleanup failure", stderr.getvalue())
        self.assertEqual(self.destination.read_bytes(), previous)
        self.assertEqual(len(closed_descriptors), 1)
        with self.assertRaises(OSError):
            os.fstat(closed_descriptors[0])
        self.assertEqual(list(self.root.glob(".extracted.txt.*.extract-tmp")), [])

    def test_success_publishes_complete_text_and_removes_temp(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with install_dependency_stubs():
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = self.pdf_tool.main(["extract", str(self.source), "--output", str(self.destination)])

        self.assertEqual(code, 0)
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(self.destination.read_text(encoding="utf-8"), "COMPLETE SYNTHETIC EXTRACTION TEXT")
        self.assertEqual(list(self.root.glob(".extracted.txt.*.extract-tmp")), [])

    def test_omitting_output_still_prints_extracted_text(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with install_dependency_stubs():
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = self.pdf_tool.main(["extract", str(self.source)])

        self.assertEqual(code, 0)
        self.assertEqual(stdout.getvalue(), "COMPLETE SYNTHETIC EXTRACTION TEXT\n")
        self.assertEqual(stderr.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
