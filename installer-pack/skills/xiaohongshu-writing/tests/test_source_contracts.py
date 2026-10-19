from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"
TEXT = SKILL.read_text(encoding="utf-8")


class XiaohongshuSourceContractTests(unittest.TestCase):
    def test_personal_experience_and_credentials_are_evidence_gated(self):
        self.assertIn("只能来自用户提供且用户确认可公开的真实材料", TEXT)
        self.assertIn("不得虚构体验、专家背书、排名、对比结果或社会证明", TEXT)
        self.assertIn("仅在用户提供并确认可公开时填写", TEXT)
        self.assertNotIn("亲测有效", TEXT)
        self.assertNotIn("作为业内人士我想说", TEXT)

    def test_numeric_and_sensitive_effect_claims_require_evidence(self):
        self.assertIn("数字、比例、时间、功效与因果声明需有可核验来源及适用条件", TEXT)
        self.assertIn("健康、美容、育儿、减重等效果", TEXT)
        for fabricated_claim in ("显瘦20斤", "显白到离谱", "宝宝一觉睡到天亮", "3个月从小白到实战"):
            self.assertNotIn(fabricated_claim, TEXT)

    def test_commercial_relationships_must_be_disclosed(self):
        self.assertIn("商业合作、赠品、联盟佣金、自营产品或其他利益关系", TEXT)
        self.assertIn("不得建议隐藏合作或把广告伪装成独立体验", TEXT)
        self.assertIn("不得隐藏广告关系", TEXT)

    def test_platform_algorithm_claims_are_not_asserted_as_current_facts(self):
        self.assertIn("不代表平台内部算法或当前排序规则", TEXT)
        self.assertIn("核对官方现行资料并注明来源与日期", TEXT)
        self.assertIn("未能核实时标为待验证假设", TEXT)

    def test_user_tone_scope_and_output_are_respected(self):
        self.assertIn("用户只要标题时，仅生成标题", TEXT)
        self.assertIn("用户明确要求克制、正式或不使用 emoji 时仍强行加入表情", TEXT)
        self.assertIn("只有适合内容或用户要求时才加入 emoji", TEXT)
        self.assertIn("只有用户需要时提供相关标签", TEXT)
        self.assertIn("按用户目标决定是否", TEXT)

    def test_ai_labeling_date_and_integrity_are_accurate(self):
        self.assertIn("自2025年9月1日起施行", TEXT)
        self.assertNotIn("2026年2月新规", TEXT)
        self.assertIn("不得移除或伪造生成标识", TEXT)

    def test_declared_count_matches_the_three_four_row_groups(self):
        self.assertIn("常见的12种机械表达", TEXT)
        block = TEXT.split("### 🚫 常见的12种机械表达", 1)[1].split("### ✅ 人类化重写规则", 1)[0]
        rows = [line for line in block.splitlines() if line.startswith("|") and not line.startswith("|---") and "问题" not in line]
        self.assertEqual(len(rows), 12)

    def test_templates_do_not_imply_unverified_reader_identity_or_pressure(self):
        self.assertIn("标题：需求/产品信息 + 可核验的比较维度", TEXT)
        self.assertNotIn("不看会亏型", TEXT)
        self.assertNotIn("#程序员日常", TEXT)
        self.assertNotIn("每天加班到很晚，根本没时间运动对吧？", TEXT)
        self.assertIn("如果你最近经常加班", TEXT)
        self.assertNotIn("别再犹豫了，现在就开始！", TEXT)
        self.assertIn("不得建议隐藏合作或把广告伪装成独立体验", TEXT)

    def test_curiosity_words_must_match_real_content(self):
        self.assertIn("仅在正文确有对应事实时使用", TEXT)
        self.assertIn("不得暗示不存在的秘密、独家信息或未核实的“真相”", TEXT)


if __name__ == "__main__":
    unittest.main()
