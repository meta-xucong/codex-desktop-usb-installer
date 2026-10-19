"""PDF generation must publish only a fully saved and verified staging file."""

import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import pdf_tool


ROOT = Path(__file__).resolve().parents[4]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class GenerateAtomicWriteTests(unittest.TestCase):
    def _case(self, canvas_type):
        temporary = tempfile.TemporaryDirectory(prefix="pdf-generate-atomic-", dir=ROOT / "tmp")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        output = root / "generated.pdf"
        original = b"SYNTHETIC-EXISTING-PDF-MUST-SURVIVE"
        output.write_bytes(original)
        state = {"opened": None, "drawn_text": None, "saved": False}
        expected_root = root.resolve()
        expected_output = output.resolve()

        class Canvas:
            def __init__(self, path):
                resolved = Path(path).resolve()
                if resolved.parent != expected_root or resolved == expected_output:
                    raise AssertionError(f"canvas received an unexpected path: {resolved}")
                state["opened"] = resolved
                self.handle = resolved.open("wb")
                self.handle.write(b"%PDF-1.4\n")

            def drawString(self, _x, _y, text):
                state["drawn_text"] = text
                self.handle.write(text.encode("utf-8") + b"\n")

            def save(self):
                state["saved"] = True
                if canvas_type == "failure":
                    self.handle.write(b"%synthetic partial")
                    self.handle.flush()
                    self.handle.close()
                    raise OSError("S037 injected canvas save failure")
                self.handle.write(b"%%EOF\n")
                self.handle.flush()
                self.handle.close()

        canvas_module = type("CanvasModule", (), {"Canvas": Canvas})

        return temporary, root, output, original, state, canvas_module

    def test_save_failure_preserves_existing_bytes_and_cleans_stage(self):
        _temporary, root, output, original, state, canvas_module = self._case("failure")
        before_hash = digest(output.read_bytes())
        with patch.object(pdf_tool, "_imports", return_value=(None, None, None, canvas_module)):
            with self.assertRaisesRegex(OSError, "S037 injected canvas save failure"):
                pdf_tool.generate(str(output), "synthetic text")

        self.assertTrue(state["saved"])
        self.assertEqual(state["drawn_text"], "synthetic text")
        self.assertIsNotNone(state["opened"])
        self.assertNotEqual(state["opened"], output.resolve())
        self.assertEqual(state["opened"].parent, root.resolve())
        self.assertEqual(output.read_bytes(), original)
        self.assertEqual(digest(output.read_bytes()), before_hash)
        self.assertEqual(sorted(path.name for path in root.iterdir()), ["generated.pdf"])

    def test_success_saves_verifies_and_replaces_output(self):
        _temporary, root, output, original, state, canvas_module = self._case("success")
        with patch.object(pdf_tool, "_imports", return_value=(None, None, None, canvas_module)):
            result = pdf_tool.generate(str(output), "successful synthetic PDF")

        self.assertEqual(result, 0)
        self.assertTrue(state["saved"])
        self.assertEqual(state["drawn_text"], "successful synthetic PDF")
        self.assertEqual(state["opened"].parent, root.resolve())
        self.assertNotEqual(state["opened"], output.resolve())
        generated = output.read_bytes()
        self.assertTrue(generated.startswith(b"%PDF-"))
        self.assertIn(b"successful synthetic PDF", generated)
        self.assertTrue(generated.endswith(b"%%EOF\n"))
        self.assertNotEqual(generated, original)
        self.assertEqual(sorted(path.name for path in root.iterdir()), ["generated.pdf"])


if __name__ == "__main__":
    unittest.main()
