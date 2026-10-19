import unittest
import base64
import struct
import zlib

import fetch


class LazyImageRepairTests(unittest.TestCase):
    @staticmethod
    def one_pixel_png_data_uri():
        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        png = (
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00\xff\xff\xff\x00"))
            + chunk(b"IEND", b"")
        )
        return "data:image/png;base64," + base64.b64encode(png).decode("ascii")

    def test_placeholder_src_is_replaced_without_duplicate_attribute(self):
        source = '<img alt="cover" data-src="https://fixture.invalid/a.png" src="placeholder">'
        fixed = fetch.fix_lazy_images(source)
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)
        self.assertNotIn("data-src=", fixed)

    def test_missing_src_is_added_from_lazy_attribute(self):
        fixed = fetch.fix_lazy_images("<img alt='cover' data-src='https://fixture.invalid/a.png'>")
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)
        self.assertNotIn("data-src=", fixed)

    def test_existing_real_src_is_preserved_without_duplicate(self):
        source = '<img src="https://fixture.invalid/current.png" data-src="https://fixture.invalid/lazy.png">'
        fixed = fetch.fix_lazy_images(source)
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/current.png"', fixed)
        self.assertNotIn("data-src=", fixed)

    def test_empty_src_is_replaced(self):
        fixed = fetch.fix_lazy_images('<img src="" data-src="https://fixture.invalid/a.png">')
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)

    def test_unquoted_placeholder_and_lazy_url_are_repaired(self):
        fixed = fetch.fix_lazy_images("<img alt=cover src=placeholder data-src=https://fixture.invalid/a.png>")
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)
        self.assertNotIn("data-src=", fixed)

    def test_common_one_pixel_gif_placeholder_is_replaced(self):
        pixel = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"
        fixed = fetch.fix_lazy_images(f'<img alt="cover" src="{pixel}" data-src="https://fixture.invalid/a.png">')
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)
        self.assertNotIn(pixel, fixed)

    def test_common_one_pixel_png_placeholder_is_replaced(self):
        pixel = self.one_pixel_png_data_uri()
        fixed = fetch.fix_lazy_images(f'<img alt="cover" src="{pixel}" data-src="https://fixture.invalid/a.png">')
        self.assertEqual(fixed.count("src="), 1)
        self.assertIn('src="https://fixture.invalid/a.png"', fixed)
        self.assertNotIn(pixel, fixed)


if __name__ == "__main__":
    unittest.main()
