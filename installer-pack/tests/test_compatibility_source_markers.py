import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = Path(__file__).resolve().parents[1] / "audit-codex-compatibility.py"
SPEC = importlib.util.spec_from_file_location("compat_audit", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class ArgparseOptionMarkerTests(unittest.TestCase):
    def test_single_line_option_is_detected(self):
        self.assertTrue(MODULE.has_argparse_option('parser.add_argument("--allow-partial", action="store_true")', "--allow-partial"))

    def test_multiline_option_and_interstitial_comment_are_detected(self):
        source = 'parser.add_argument(\n    # public opt-in\n    "--allow-partial",\n    action="store_true",\n)'
        self.assertTrue(MODULE.has_argparse_option(source, "--allow-partial"))

    def test_parenthesized_receiver_is_detected(self):
        source = '(\n  parser\n).add_argument(\n  "--allow-partial"\n)'
        self.assertTrue(MODULE.has_argparse_option(source, "--allow-partial"))

    def test_adjacent_string_literals_are_detected(self):
        source = 'parser.add_argument("--allow-" "partial")'
        self.assertTrue(MODULE.has_argparse_option(source, "--allow-partial"))

    def test_comments_and_docstrings_do_not_count_as_declarations(self):
        source = '''# parser.add_argument("--allow-partial")
example = "--allow-partial"
doc = """
parser.add_argument("--allow-partial")
"""
'''
        self.assertFalse(MODULE.has_argparse_option(source, "--allow-partial"))


if __name__ == "__main__":
    unittest.main()
