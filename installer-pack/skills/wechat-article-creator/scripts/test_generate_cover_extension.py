import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from PIL import Image

import generate_cover


class CoverOutputExtensionTests(unittest.TestCase):
    def test_unsupported_extension_fails_before_parent_creation_or_render(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            parent = Path(temp_dir) / "missing" / "nested"
            output = parent / "cover.txt"
            with patch.object(generate_cover.Image, "new", wraps=Image.new) as render:
                with self.assertRaisesRegex(ValueError, r"只支持 \.jpg、\.jpeg 或 \.png"):
                    generate_cover.generate_cover(
                        title="Synthetic cover",
                        width=256,
                        height=256,
                        output_path=str(output),
                    )
            self.assertFalse(parent.exists())
            self.assertFalse(output.exists())
            render.assert_not_called()

    def test_supported_extensions_write_valid_images_case_insensitively(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            expected_formats = {"cover.jpg": "JPEG", "cover.JPEG": "JPEG", "cover.PNG": "PNG"}
            for filename, expected_format in expected_formats.items():
                with self.subTest(filename=filename):
                    output = Path(temp_dir) / filename
                    generated = generate_cover.generate_cover(
                        title="Synthetic cover",
                        width=256,
                        height=256,
                        output_path=str(output),
                    )
                    self.assertEqual(Path(generated), output)
                    with Image.open(output) as image:
                        self.assertEqual(image.format, expected_format)
                        self.assertEqual(image.size, (256, 256))


if __name__ == "__main__":
    unittest.main()
