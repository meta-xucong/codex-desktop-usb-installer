from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class OfferDesignerPricingAndOutputContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")
        cls.compact = " ".join(cls.text.split())
        template = cls.text.split("### STEP 4: 输出结构", 1)[1].split("## Best Practices", 1)[0]
        cls.step4 = " ".join(template.split())

    def test_markup_and_gross_margin_are_distinguished(self):
        self.assertIn("成本加成率 = (售价 - 成本) / 成本", self.compact)
        self.assertIn("售价 = 成本 × (1 + 成本加成率)", self.compact)
        self.assertIn("毛利率 = (售价 - 成本) / 售价", self.compact)
        self.assertIn("售价 = 成本 / (1 - 目标毛利率)", self.compact)
        self.assertIn("目标毛利率必须小于 100%", self.compact)

    def test_example_arithmetic_is_correct_and_clearly_hypothetical(self):
        cost, markup = 100, 0.20
        markup_price = cost * (1 + markup)
        gross_margin = (markup_price - cost) / markup_price
        target_gm = 0.20
        gross_margin_price = cost / (1 - target_gm)
        self.assertEqual(markup_price, 120)
        self.assertAlmostEqual(gross_margin, 1/6)
        self.assertEqual(gross_margin_price, 125)
        self.assertIn("纯算术示例", self.compact)
        self.assertIn("不是市场价格建议", self.compact)

    def test_optional_offer_components_require_confirmed_delivery_or_terms(self):
        for phrase in ("仅列商家确认真实存在或明确规划", "退款/试用条款和履约条件", "可核验证据的真实容量、库存或截止条件"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, self.step4)
        self.assertIn("否则省略", self.step4)

    def test_social_proof_requires_authorized_verifiable_evidence(self):
        self.assertIn("社会证明（仅使用已获授权且可核验的案例/评价；否则省略）", self.step4)

if __name__ == "__main__":
    unittest.main()
