"""Focused mathematical contract tests for the Market Sizing skill."""
from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class MarketFormulaContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = " ".join(SKILL.read_text(encoding="utf-8").replace("**", "").split())

    def test_bottom_up_sam_sums_serviceable_segment_revenue(self):
        self.assertIn("SAM = Σ (Eligible Customers in Each Serviceable, Mutually Exclusive Segment × Annual Revenue per Customer)", self.text)
        self.assertIn("do not scale TAM by the count of segments", self.text)
        # Synthetic counterexample: unequal segment values must remain weighted.
        segment_revenues = {"A": 99, "B": 1}
        self.assertEqual(sum(segment_revenues.values()), 100)
        self.assertEqual(segment_revenues["A"], 99)
        self.assertEqual(segment_revenues["B"], 1)
        self.assertNotEqual(100 * (1 / 2), segment_revenues["A"])

    def test_top_down_filtering_uses_joint_or_conditional_shares(self):
        self.assertIn("Joint Share Meeting All Geographic and Segment Filters", self.text)
        self.assertIn("directly evidenced share that satisfies all filters", self.text)
        self.assertIn("conditional on the population remaining after the earlier filter", self.text)
        self.assertIn("Do not multiply independent-looking geographic and segment percentages", self.text)

    def test_annual_arpu_is_not_multiplied_by_purchase_frequency(self):
        self.assertIn("TAM = Total Addressable Users × Annual ARPU", self.text)
        self.assertIn("TAM = Total Addressable Users × Average Spend per Purchase × Purchases per User per Year", self.text)
        self.assertIn("Do not multiply annual ARPU by purchase frequency", self.text)
        users, annual_arpu = 10_000, 100
        self.assertEqual(users * annual_arpu, 1_000_000)
        self.assertEqual(users * 100 * 2, 2_000_000)
        self.assertNotIn("TAM = Total Users × ARPU × Purchase Frequency per Year", self.text)

    def test_consumer_formula_markdown_fences_are_balanced(self):
        raw = SKILL.read_text(encoding="utf-8")
        fences = [line for line in raw.splitlines() if line.strip() == "```"]
        self.assertEqual(len(fences) % 2, 0)
        consumer = raw.split("### Consumer Markets", 1)[1].split("### B2B Services", 1)[0]
        self.assertNotIn("```", consumer)
        self.assertIn("`TAM = Total Addressable Users × Annual ARPU`", consumer)
        self.assertIn("`TAM = Total Addressable Users × Average Spend per Purchase × Purchases per User per Year`", consumer)

if __name__ == "__main__":
    unittest.main()
