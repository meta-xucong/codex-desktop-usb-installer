from pathlib import Path
import json
import unittest

ROOT = Path(__file__).resolve().parents[1]
SKILL = (ROOT / "SKILL.md").read_text(encoding="utf-8")
UI = (ROOT / "agents" / "openai.yaml").read_text(encoding="utf-8")
PACK_SKILLS = ROOT.parents[1] / "manifest" / "skill-audit.json"


class WordDocxSourceContractTests(unittest.TestCase):
    def test_ui_visual_claim_is_conditional(self):
        self.assertIn("结构校验", UI)
        self.assertIn("structurally verify", UI)
        self.assertIn("when a renderer is available", UI)
        self.assertIn("inspect rendered pages", UI)

    def test_output_path_and_no_overwrite_rules_are_explicit(self):
        self.assertIn("用户给出输出位置时使用该路径", SKILL)
        self.assertIn("已有目标不得覆盖", SKILL)
        self.assertIn("默认另存为新文件，不修改源文件", SKILL)

    def test_locked_runtime_and_package_dependency_are_named(self):
        self.assertIn("Python 3.12", SKILL)
        self.assertIn("python-docx", SKILL)
        self.assertIn("依赖缺失或导入失败时报告具体缺项", SKILL)
        manifest = json.loads(PACK_SKILLS.read_text(encoding="utf-8"))
        entry = next(x for x in manifest["skills"] if x["id"] == "word-docx")
        dependencies = {d.get("id"): d for d in entry["dependencies"]}
        self.assertEqual(dependencies["python.package.python-docx"]["lockedVersion"], "1.2.0")
        self.assertEqual(dependencies["runtime.python312"]["lockedVersion"], "3.12.10")

    def test_visual_review_requires_rendered_pages_and_reports_unavailable_renderer(self):
        self.assertIn("确实生成 PDF/页面图并检查所有相关页面", SKILL)
        self.assertIn("渲染器缺失或失败时", SKILL)
        self.assertIn("尚未验证", SKILL)
        self.assertIn("结构检查不等于页面视觉验收", SKILL)

    def test_macro_enabled_documents_are_not_claimed_as_handled(self):
        self.assertIn(".docm", SKILL)
        self.assertIn("不运行、启用或移除宏", SKILL)


if __name__ == "__main__":
    unittest.main()

