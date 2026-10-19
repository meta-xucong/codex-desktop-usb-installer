"""Missing optional fetch dependencies should be concise offline CLI failures."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "installer-pack" / "skills" / "web-content-fetcher" / "scripts" / "fetch.py"
RUNNER = r'''
import builtins
import runpy
import sys
import types

script, missing = sys.argv[1:3]
real_import = builtins.__import__
calls = []

class FakeHTML2Text:
    def __init__(self):
        pass

    def handle(self, _html):
        return "x" * 100

html2text = types.ModuleType("html2text")
html2text.HTML2Text = FakeHTML2Text
fetchers = types.ModuleType("scrapling.fetchers")

class FakePage:
    html_content = "<html>offline fixture</html>"

    def css(self, _selector):
        return []

class FakeFetcher:
    @classmethod
    def configure(cls, **_kwargs):
        pass

    @classmethod
    def get(cls, url, **_kwargs):
        assert url == "https://fixture.invalid/article"
        return FakePage()

fetchers.Fetcher = FakeFetcher

def isolated_import(name, *args, **kwargs):
    if missing == "html2text" and name == "html2text":
        raise ModuleNotFoundError("injected missing html2text", name="html2text")
    if missing == "scrapling" and name == "html2text":
        return html2text
    if missing == "scrapling" and name in {"scrapling", "scrapling.fetchers"}:
        raise ModuleNotFoundError("injected missing scrapling", name="scrapling")
    if missing == "success" and name == "html2text":
        return html2text
    if missing == "success" and name in {"scrapling", "scrapling.fetchers"}:
        return fetchers
    return real_import(name, *args, **kwargs)

builtins.__import__ = isolated_import
max_chars = "37" if missing == "success" else "12000"
sys.argv = [script, "https://fixture.invalid/article", max_chars]
runpy.run_path(script, run_name="__main__")
'''


class MissingDependencyCliTests(unittest.TestCase):
    def _run_with_missing(self, root: Path, dependency: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.update({"TEMP": str(root), "TMP": str(root), "PYTHONDONTWRITEBYTECODE": "1"})
        return subprocess.run(
            [sys.executable, "-B", "-c", RUNNER, str(SCRIPT), dependency],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )

    def test_missing_html2text_is_actionable_without_traceback_or_network(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fetch-missing-html2text-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            result = self._run_with_missing(root, "html2text")

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("html2text", result.stderr)
            self.assertIn("Codex native web reading", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(list(root.iterdir()), [])

    def test_missing_scrapling_is_actionable_without_traceback_or_network(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fetch-missing-scrapling-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            result = self._run_with_missing(root, "scrapling")

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("scrapling", result.stderr)
            self.assertIn("Codex native web reading", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(list(root.iterdir()), [])

    def test_normal_fake_extraction_preserves_cli_max_chars(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fetch-success-offline-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            result = self._run_with_missing(root, "success")

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout,
                "x" * 37 + "\n\n[正文已截断：原始正文超过 37 个字符；max_chars 上限仅适用于正文，不包含此提示。]\n",
            )
            self.assertEqual(result.stderr, "")
            self.assertEqual(list(root.iterdir()), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
