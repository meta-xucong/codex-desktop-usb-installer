from __future__ import annotations

import re
import unittest
from pathlib import Path

import pptx
from pptx.dml.color import RGBColor


SKILL_DIR = Path(__file__).resolve().parents[1]


class RgbColorDocumentationTests(unittest.TestCase):
    def test_documents_only_use_exported_rgbcolor_symbol(self):
        documents = list(SKILL_DIR.glob("*.md"))
        self.assertTrue(documents)
        stale_uses = []
        for path in documents:
            contents = path.read_text(encoding="utf-8")
            if re.search(r"\bRgbColor\b", contents):
                stale_uses.append(path.name)
        self.assertEqual(stale_uses, [])

    def test_bundled_python_pptx_102_import_and_color_construction(self):
        self.assertEqual(pptx.__version__, "1.0.2")
        color = RGBColor(0x1F, 0x77, 0xB4)
        self.assertEqual((color[0], color[1], color[2]), (0x1F, 0x77, 0xB4))
        self.assertEqual(str(color), "1F77B4")


if __name__ == "__main__":
    unittest.main()
