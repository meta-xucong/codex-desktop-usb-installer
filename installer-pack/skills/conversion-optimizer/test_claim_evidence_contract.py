"""Focused source-contract regression tests for Conversion Optimizer claims."""

from pathlib import Path
import unittest


SKILL = Path(__file__).with_name("SKILL.md")


class ClaimEvidenceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")

    def test_customer_facing_claims_require_verifiable_user_facts(self):
        self.assertIn("用户提供且可核验的事实", self.text)
        self.assertIn("用户确认的正式条款与实时条件", self.text)
        self.assertIn("[待核实：具体事实]", self.text)
        self.assertIn("或先向用户询问", self.text)
        self.assertIn("不得自行补齐", self.text)

    def test_overview_does_not_guarantee_conversion_improvement(self):
        self.assertIn("通过诊断与实验寻找提升转化率的机会；实际效果需经数据验证", self.text)
        self.assertNotIn("系统提升转化率", self.text)

    def test_aida_proof_and_roi_objection_require_evidence(self):
        self.assertIn("经授权且可核验的案例  经授权且可核验的社会证明", self.text)
        self.assertIn("基于用户实际数据的价值/ROI 对比", self.text)
        self.assertIn("无数据时仅作为明确待验证假设", self.text)
        self.assertIn("目前没有数据时，只能把该对比作为待验证假设", self.text)
        self.assertNotIn("如果你算一下 ROI", self.text)

    def test_time_saving_objection_requires_evidence(self):
        self.assertNotIn("正是因为忙，才更需要这个帮你省时间", self.text)
        self.assertIn("仅引用有证据支持的时间/效率效果", self.text)
        self.assertIn("无证据时列为待验证假设并删除结果暗示", self.text)
        self.assertIn("目前没有数据时，这只是假设，不能承诺省时效果", self.text)

    def test_refund_and_scarcity_are_conditional(self):
        self.assertIn("不得承诺退款期限、无理由退款或“零风险”", self.text)
        self.assertIn("真实截止时间、可用库存/名额", self.text)
        self.assertIn("真实实时数据来源", self.text)
        self.assertIn("否则删除稀缺表达", self.text)
        self.assertNotIn("7 天无理由退款", self.text)
        self.assertNotIn("今晚 12 点截止", self.text)
        self.assertIn("赠品", self.text)
        self.assertIn("真实存在且已确认", self.text)
        self.assertIn("总价值计算须有可核验的定价依据", self.text)

    def test_generic_thresholds_and_outcome_guarantees_are_not_default_copy(self):
        for unsupported_claim in (
            "> 10%", "> 50%", "> 30%", "> 20%", "> 15%",
            "实现月入过万", "薪资 30% 的增长", "10000+ 职场人的选择",
            "抢占早鸟名额，立省 200 元", "获得免费试用", "立即开始省钱",
        ):
            with self.subTest(unsupported_claim=unsupported_claim):
                self.assertNotIn(unsupported_claim, self.text)

    def test_numeric_example_is_explicitly_synthetic(self):
        marker = "【合成示例，仅演示计算格式；不是行业基准、客户数据或对外文案】"
        example_start = self.text.index(marker)
        example_end = self.text.index("【销售话术】", example_start)
        example = self.text[example_start:example_end]
        self.assertIn("假设某个虚构漏斗的数据为", example)
        self.assertIn("合成留资率", example)
        self.assertIn("合成激活率", example)
        self.assertIn("合成成交率", example)

    def test_useful_cro_framework_remains(self):
        for useful_section in (
            "转化诊断", "AIDA 模型升级版", "Landing Page 黄金结构",
            "CTA 设计原则", "A/B 测试计划", "数据说话",
        ):
            with self.subTest(section=useful_section):
                self.assertIn(useful_section, self.text)


if __name__ == "__main__":
    unittest.main()
