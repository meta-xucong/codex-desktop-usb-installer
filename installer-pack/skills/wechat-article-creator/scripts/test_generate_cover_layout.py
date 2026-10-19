import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from PIL import Image, ImageDraw, ImageFont

import generate_cover


class CoverTitleLayoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        windir = os.environ.get("WINDIR", r"C:\Windows")
        cls.font_path = os.path.join(windir, "Fonts", "msyh.ttc")
        if not os.path.isfile(cls.font_path):
            raise unittest.SkipTest("Microsoft YaHei font is unavailable")

    def test_long_cjk_title_fits_safe_area_without_subtitle_overlap(self):
        width, height = 1200, 630
        x, y = int(width * 0.1), int(height * 0.25)
        subtitle_y = int(height * 0.55)
        title = "人工智能时代的数据安全治理与企业数字化转型实践路径研究全流程指南案例分析"
        draw = ImageDraw.Draw(Image.new("RGB", (width, height)))

        wrapped, font, bounds = generate_cover._fit_title_layout(
            draw, title, self.font_path, int(width * 0.08), x, y,
            int(width * 0.8), subtitle_y - 16,
        )

        self.assertGreater(len(wrapped.splitlines()), 1)
        self.assertLess(font.size, int(width * 0.08))
        self.assertGreaterEqual(bounds[0], x)
        self.assertLessEqual(bounds[2], int(width * 0.9))
        self.assertLess(bounds[3], subtitle_y)

    def test_short_title_keeps_original_size_and_single_line(self):
        width, height = 1200, 630
        x, y = int(width * 0.1), int(height * 0.25)
        subtitle_y = int(height * 0.55)
        title = "春日读书会"
        draw = ImageDraw.Draw(Image.new("RGB", (width, height)))

        wrapped, font, bounds = generate_cover._fit_title_layout(
            draw, title, self.font_path, int(width * 0.08), x, y,
            int(width * 0.8), subtitle_y - 16,
        )

        self.assertEqual(wrapped, title)
        self.assertEqual(font.size, int(width * 0.08))
        original_font = ImageFont.truetype(self.font_path, int(width * 0.08))
        original_bounds = draw.textbbox((x, y), title, font=original_font)
        self.assertEqual(bounds, original_bounds)
        self.assertLessEqual(bounds[2], int(width * 0.9))
        self.assertLess(bounds[3], subtitle_y)

    def test_impossible_title_fails_instead_of_clipping(self):
        draw = ImageDraw.Draw(Image.new("RGB", (1200, 630)))
        with self.assertRaisesRegex(ValueError, "无法放入封面安全区域"):
            generate_cover._fit_title_layout(
            draw, "标题", self.font_path, 96, 120, 157, 960, 100,
                minimum_size=16,
            )

    def test_short_title_cover_still_generates(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            output = Path(temp_dir) / "short.png"
            generated = generate_cover.generate_cover(
                title="春日读书会",
                subtitle="数据安全与转型实践指南",
                author="研究团队",
                style="tech",
                width=1200,
                height=630,
                output_path=str(output),
            )
            self.assertEqual(Path(generated), output)
            with Image.open(output) as image:
                self.assertEqual(image.size, (1200, 630))
                self.assertEqual(image.format, "PNG")

    def test_long_subtitle_wraps_inside_safe_area(self):
        width, height = 1200, 630
        x, y = int(width * 0.1), int(height * 0.55)
        author_y = height - int(height * 0.12)
        draw = ImageDraw.Draw(Image.new("RGB", (width, height)))
        fallback = ImageFont.truetype(self.font_path, int(width * 0.04))

        wrapped, font, bounds = generate_cover._fit_auxiliary_text(
            draw, "副标题" * 100, self.font_path, fallback, int(width * 0.04),
            x, y, int(width * 0.8), author_y - 16, "副标题",
        )

        self.assertGreater(len(wrapped.splitlines()), 1)
        self.assertGreater(font.size, 0)
        self.assertGreaterEqual(bounds[0], x)
        self.assertLessEqual(bounds[2], int(width * 0.9))
        self.assertLessEqual(bounds[3], author_y - 16)

    def test_author_stays_left_of_date_and_impossible_author_is_rejected(self):
        width, height = 1200, 630
        x = int(width * 0.1)
        author_y = height - int(height * 0.12)
        date_x = width - int(width * 0.1)
        draw = ImageDraw.Draw(Image.new("RGB", (width, height)))
        author_font = ImageFont.truetype(self.font_path, int(width * 0.03))
        date_bounds = draw.textbbox(
            (date_x, author_y), "2026.10.19", font=author_font, anchor="rm"
        )

        wrapped, _, bounds = generate_cover._fit_auxiliary_text(
            draw, "作者：研究团队" * 8, self.font_path, author_font,
            int(width * 0.03), x, author_y, date_bounds[0] - x - 24,
            height - int(height * 0.05), "作者", minimum_size=12, spacing=2,
        )
        self.assertTrue(wrapped)
        self.assertLessEqual(bounds[2], date_bounds[0] - 24)
        with self.assertRaisesRegex(ValueError, "作者无法放入封面安全区域"):
            generate_cover._fit_auxiliary_text(
                draw, "作者：作者名" * 200, self.font_path, author_font,
                int(width * 0.03), x, author_y, date_bounds[0] - x - 24,
                height - int(height * 0.05), "作者", minimum_size=12, spacing=2,
            )

    def test_long_subtitle_cover_still_generates(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            output = Path(temp_dir) / "long-subtitle.png"
            generated = generate_cover.generate_cover(
                title="短标题",
                subtitle="副标题" * 100,
                author="研究团队",
                style="tech",
                width=1200,
                height=630,
                output_path=str(output),
            )
            self.assertEqual(Path(generated), output)
            with Image.open(output) as image:
                self.assertEqual(image.size, (1200, 630))

    def test_impossible_author_fails_without_creating_output(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            output = Path(temp_dir) / "impossible-author.png"
            with self.assertRaisesRegex(ValueError, "作者无法放入封面安全区域"):
                generate_cover.generate_cover(
                    title="短标题",
                    subtitle="短副标题",
                    author="作者名" * 200,
                    style="tech",
                    width=1200,
                    height=630,
                    output_path=str(output),
                )
            self.assertFalse(output.exists())

    def test_missing_candidate_fonts_use_fallback_for_short_cover(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            output = Path(temp_dir) / "fallback-font.png"
            with patch.object(generate_cover.os.path, "exists", return_value=False):
                generated = generate_cover.generate_cover(
                    title="Short title",
                    subtitle="Short subtitle",
                    author="Research team",
                    style="tech",
                    width=1200,
                    height=630,
                    output_path=str(output),
                )
            self.assertEqual(Path(generated), output)
            with Image.open(output) as image:
                self.assertEqual(image.size, (1200, 630))
                self.assertEqual(image.format, "PNG")

    def test_missing_candidate_fonts_reject_oversized_subtitle_before_output(self):
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp_dir:
            output = Path(temp_dir) / "fallback-font-too-long.png"
            with patch.object(generate_cover.os.path, "exists", return_value=False):
                width, height = 1200, 630
                x, y = int(width * 0.1), int(height * 0.55)
                author_y = height - int(height * 0.12)
                draw = ImageDraw.Draw(Image.new("RGB", (width, height)))
                fallback_font = ImageFont.load_default()
                wrapped, _, bounds = generate_cover._fit_auxiliary_text(
                    draw,
                    "A subtitle that cannot fit " * 100,
                    None,
                    fallback_font,
                    int(width * 0.04),
                    x,
                    y,
                    int(width * 0.8),
                    author_y - 16,
                    "副标题",
                )
                self.assertGreater(len(wrapped.splitlines()), 1)
                self.assertGreaterEqual(bounds[0], x)
                self.assertLessEqual(bounds[2], int(width * 0.9))
                self.assertLessEqual(bounds[3], author_y - 16)
                try:
                    generated = generate_cover.generate_cover(
                        title="Short title",
                        subtitle="A subtitle that cannot fit " * 100,
                        author="Research team",
                        style="tech",
                        width=1200,
                        height=630,
                        output_path=str(output),
                    )
                except ValueError:
                    self.assertFalse(output.exists())
                else:
                    self.assertEqual(Path(generated), output)
                    with Image.open(output) as image:
                        self.assertEqual(image.size, (1200, 630))


if __name__ == "__main__":
    unittest.main()
