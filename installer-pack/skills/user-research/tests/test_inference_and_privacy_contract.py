from pathlib import Path
import unittest


SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"
TEXT = SKILL.read_text(encoding="utf-8")


class UserResearchInferenceContractTests(unittest.TestCase):
    def test_population_estimates_require_sampling_and_uncertainty(self):
        self.assertIn("覆盖充分的抽样框", TEXT)
        self.assertIn("概率抽样", TEXT)
        self.assertIn("不能通过事后加权把便利样本变成概率样本", TEXT)
        self.assertIn("不确定性区间", TEXT)
        self.assertIn("不能据此估计总体比例", TEXT)

    def test_nonprobability_results_stay_within_sample(self):
        self.assertIn("便利抽样", TEXT)
        self.assertIn("写明 `n/N`", TEXT)
        self.assertIn("不得改写成目标人群占比", TEXT)
        self.assertIn("样本内观察到的发生频率", TEXT)

    def test_validation_claims_are_method_and_evidence_bound(self):
        self.assertIn("验证方法、样本/数据范围", TEXT)
        self.assertIn("自述的意愿不能写成已观察的购买或使用行为", TEXT)
        for field in ("验证方法：", "样本/数据范围：", "实际观察到的证据：", "适用范围与限制："):
            self.assertIn(field, TEXT)

    def test_data_handling_requires_minimization_and_bounded_reuse(self):
        self.assertIn("目的限定和最少必要原则", TEXT)
        self.assertIn("公开可访问不代表可以不受限制地复用", TEXT)


if __name__ == "__main__":
    unittest.main()
