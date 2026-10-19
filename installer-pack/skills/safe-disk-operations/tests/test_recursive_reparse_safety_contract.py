"""Focused source-contract checks for recursive path and reparse safety."""
from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class RecursiveReparseSafetyContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = " ".join(SKILL.read_text(encoding="utf-8").split())

    def test_recursive_mutations_require_read_only_tree_preflight(self):
        self.assertIn("Before any recursive delete, move, rename, copy, merge, or overwrite", self.text)
        self.assertIn("run a read-only preflight over the exact planned tree", self.text)
        self.assertIn("Enumerate descendants without following links", self.text)

    def test_target_ancestors_and_all_descendants_check_for_reparse_points(self):
        self.assertIn("the target, every descendant, and each path component", self.text)
        for kind in ("symbolic links", "junctions", "mount points", "reparse point"):
            with self.subTest(kind=kind):
                self.assertIn(kind, self.text)

    def test_physical_resolution_must_stay_inside_approved_directory(self):
        self.assertIn("physically resolved target remains inside the exact user-approved directory", self.text)
        self.assertIn("lexical prefix check alone is not sufficient", self.text)

    def test_any_link_or_uncertain_preflight_aborts_the_whole_batch(self):
        self.assertIn("stop the entire recursive batch", self.text)
        self.assertIn("Do not follow, traverse through, or remove the link and then continue", self.text)
        self.assertIn("preflight cannot reliably inspect every item", self.text)

    def test_recheck_before_and_verify_after_execution(self):
        self.assertIn("Recheck the planned tree immediately before execution", self.text)
        self.assertIn("read-only postcondition check on the exact authorized paths", self.text)
        self.assertIn("report partial failures without claiming full success", self.text)

    def test_exact_authorization_gate_remains_in_force(self):
        self.assertIn("clearly authorizes the exact operation", self.text)
        self.assertIn("If the target, scope, overwrite policy, or desired end state is ambiguous, stop and ask one concise question", self.text)

if __name__ == "__main__":
    unittest.main()
