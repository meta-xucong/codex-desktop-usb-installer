from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class PainpointEvidenceBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")
        cls.compact = " ".join(cls.text.split())

    def test_five_answers_are_not_treated_as_verified_evidence(self):
        self.assertIn("五项信息补齐只是可以开始分析的条件，不代表这些信息已被验证", self.compact)
        self.assertIn("已观察事实 / 来源支持的推断 / 假设待验证 / 未知", self.compact)

    def test_claims_require_traceable_source_time_and_sample_scope(self):
        self.assertIn("保留来源、采集时间、样本范围与分母", self.compact)
        self.assertIn("不得从用户猜测或少量非代表性反馈推成总体结论", self.compact)
        self.assertIn("不编造数字、来源或结论", self.compact)

    def test_opportunity_map_marks_each_material_claim(self):
        for phrase in ("影响人群规模：【数值与总体/样本范围、来源、时间；无证据则未知】", "付费意愿强度：【行为/交易证据与样本范围；无证据则未知】", "当前解决方案的不足：【来源和观察范围；无证据则未知】", "竞品忽略的机会点：【竞品样本、来源和观察时间；无证据则未知】", "差异化切入点：【依据、来源和证据状态；无证据则标为假设或未知】"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, self.text)

    def test_assumption_only_example_does_not_claim_scale_willingness_or_gap(self):
        example = self.text.split("【条件性分析（只基于上方假设，不代表真实结论）】", 1)[1].split("【待验证方向】", 1)[0]
        for phrase in ("五项输入均为假设待验证", "影响人群规模：未知", "付费意愿强度：未知", "竞品缺陷/竞争空白：未知", "不能将它认定为已验证 S/A 级痛点"):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, example)

    def test_evidence_gate_does_not_expand_unrelated_editing_tasks(self):
        self.assertIn("只完成该任务", self.compact)
        self.assertIn("不转入痛点诊断", self.compact)

if __name__ == "__main__":
    unittest.main()
