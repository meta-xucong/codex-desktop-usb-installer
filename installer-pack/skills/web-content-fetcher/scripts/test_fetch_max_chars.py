import contextlib
import io
import sys
import types
import unittest
from unittest import mock

import fetch


class MaxCharsBoundaryTests(unittest.TestCase):
    def test_api_rejects_invalid_values_before_optional_imports(self):
        attempted_optional_imports = []
        real_import = __import__

        def recording_import(name, *args, **kwargs):
            if name == "html2text" or name == "scrapling.fetchers":
                attempted_optional_imports.append(name)
            return real_import(name, *args, **kwargs)

        invalid_values = (-1, 0, True, False, 1.5, "12", None)
        with mock.patch("builtins.__import__", side_effect=recording_import):
            for value in invalid_values:
                with self.subTest(value=value):
                    with self.assertRaisesRegex(ValueError, "max_chars must be a positive integer"):
                        fetch.scrapling_fetch("https://fixture.invalid/", value)

        self.assertEqual(attempted_optional_imports, [])

    def test_cli_rejects_negative_and_zero_with_argparse_error(self):
        for value in ("-1", "0"):
            with self.subTest(value=value):
                stderr = io.StringIO()
                with mock.patch.object(sys, "argv", ["fetch.py", "https://fixture.invalid/", value]):
                    with contextlib.redirect_stderr(stderr):
                        with self.assertRaises(SystemExit) as raised:
                            fetch.main()
                self.assertEqual(raised.exception.code, 2)
                self.assertIn("error: argument max_chars: max_chars must be a positive integer", stderr.getvalue())
                self.assertNotIn("Traceback", stderr.getvalue())

    def test_positive_limit_truncates_fake_extracted_body(self):
        body = "x" * 401

        class FakeElement:
            html_content = "<article>fixture</article>"

        class FakePage:
            html_content = "<html>fallback</html>"

            def css(self, selector):
                return [FakeElement()] if selector == "article" else []

        class FakeFetcher:
            @classmethod
            def configure(cls, **_kwargs):
                pass

            @classmethod
            def get(cls, _url, **_kwargs):
                return FakePage()

        class FakeHTML2Text:
            ignore_links = None
            ignore_images = None
            body_width = None

            def handle(self, _html):
                return body

        html2text_module = types.ModuleType("html2text")
        html2text_module.HTML2Text = FakeHTML2Text
        scrapling_module = types.ModuleType("scrapling")
        fetchers_module = types.ModuleType("scrapling.fetchers")
        fetchers_module.Fetcher = FakeFetcher

        with mock.patch.dict(
            sys.modules,
            {"html2text": html2text_module, "scrapling": scrapling_module, "scrapling.fetchers": fetchers_module},
        ):
            result, selector, truncated = fetch.scrapling_fetch("https://fixture.invalid/article", 37)

        self.assertEqual(result, "x" * 37)
        self.assertEqual(selector, "article")
        self.assertTrue(truncated)

    def test_fallback_reports_truncation_and_short_body_is_not_truncated(self):
        for body, expected_flag in (("y" * 401, True), ("z" * 37, False), ("short fixture", False)):
            with self.subTest(length=len(body)):
                class FakePage:
                    html_content = "<html>fallback fixture</html>"

                    def css(self, _selector):
                        return []

                class FakeFetcher:
                    @classmethod
                    def configure(cls, **_kwargs):
                        pass

                    @classmethod
                    def get(cls, _url, **_kwargs):
                        return FakePage()

                class FakeHTML2Text:
                    ignore_links = None
                    ignore_images = None
                    body_width = None

                    def handle(self, _html):
                        return body

                html2text_module = types.ModuleType("html2text")
                html2text_module.HTML2Text = FakeHTML2Text
                scrapling_module = types.ModuleType("scrapling")
                fetchers_module = types.ModuleType("scrapling.fetchers")
                fetchers_module.Fetcher = FakeFetcher
                with mock.patch.dict(sys.modules, {
                    "html2text": html2text_module,
                    "scrapling": scrapling_module,
                    "scrapling.fetchers": fetchers_module,
                }):
                    text, selector, truncated = fetch.scrapling_fetch("https://fixture.invalid/fallback", 37)

                self.assertEqual(selector, "body(fallback)")
                self.assertEqual(truncated, expected_flag)
                self.assertLessEqual(len(text), 37)
                self.assertEqual(text, body[:37])

    def test_cli_discloses_only_actual_truncation(self):
        for truncated in (True, False):
            with self.subTest(truncated=truncated):
                stdout = io.StringIO()
                with mock.patch.object(fetch, "scrapling_fetch", return_value=("x" * 37, "article", truncated)):
                    with mock.patch.object(sys, "argv", ["fetch.py", "https://fixture.invalid/", "37"]):
                        with contextlib.redirect_stdout(stdout):
                            self.assertEqual(fetch.main(), 0)

                output = stdout.getvalue()
                self.assertTrue(output.startswith("x" * 37 + "\n"))
                if truncated:
                    self.assertIn("正文已截断", output)
                    self.assertIn("上限仅适用于正文", output)
                else:
                    self.assertNotIn("截断", output)


if __name__ == "__main__":
    unittest.main()
