"""Offline source-contract checks for image-prompt-generator SKILL.md."""
import re
import unittest
from pathlib import Path

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"
TEXT = SKILL.read_text(encoding="utf-8")


class ImagePromptGeneratorSourceContracts(unittest.TestCase):
    def test_style_catalog_count_and_core_aliases_resolve(self):
        block = TEXT.split("### 完整风格库", 1)[1].split("### 风格详细定义", 1)[0]
        ids = re.findall(r"^\| `([^`]+)` \|", block, re.M)
        self.assertEqual(len(ids), 22)
        self.assertEqual(len(set(ids)), 22)
        for style in ("vector", "minimal", "blueprint", "sketch", "editorial", "cute"):
            self.assertIn(style, ids)
        for alias, target in (("minimal-flat", "minimal"), ("sci-fi", "blueprint"), ("hand-drawn", "sketch")):
            self.assertIn(f"`{alias}` → `{target}`", TEXT)
        self.assertIn("`scene` 属于内容类型", TEXT)

    def test_compatibility_matrix_header_and_rows_have_same_width(self):
        block = TEXT.split("## Type × Style 兼容性矩阵", 1)[1].split("## 工作流程", 1)[0]
        rows = [line for line in block.splitlines() if line.startswith("|")]
        widths = [len([cell for cell in line.strip().strip("|").split("|")]) for line in rows]
        self.assertTrue(rows)
        self.assertEqual(len(set(widths)), 1)
        header = [cell.strip() for cell in rows[0].strip().strip("|").split("|")]
        self.assertEqual(header[0], "")
        expected_types = {"infographic", "scene", "flowchart", "comparison", "framework", "timeline"}
        actual_types = {row.strip().strip("|").split("|")[0].strip() for row in rows[2:]}
        self.assertEqual(actual_types, expected_types)
        self.assertTrue(all(len(row.strip().strip("|").split("|")) == len(header) for row in rows[2:]))
        self.assertIn("仅列常用风格组合", block)

    def test_all_documented_aspect_ratios_are_available_in_prompt_template(self):
        ratios = set(re.findall(r"\| (?:portrait-3-4|square|portrait-2-3|landscape-16-9) \| ([0-9:]+) \|", TEXT))
        self.assertEqual(ratios, {"3:4", "1:1", "2:3", "16:9"})
        template = TEXT.split("### 通用结构", 1)[1].split("## Style Section", 1)[0]
        for ratio in ratios:
            self.assertIn(ratio, template)

    def test_labels_require_source_evidence_and_protect_brand_claims(self):
        self.assertIn("仅使用用户提供或已核实来源支持", TEXT)
        self.assertIn("不得编造数据、引用、出处或事实", TEXT)
        self.assertIn("品牌、商标、Logo、产品功效与背书", TEXT)
        self.assertIn("授权使用时纳入", TEXT)

    def test_reference_input_is_capability_gated(self):
        self.assertIn("不得假定 `$image-2`、统一媒体 MCP 或 Codex 工具接受 `--ref`", TEXT)
        self.assertIn("当前生成能力明确支持且接口说明确认参数", TEXT)
        self.assertIn("不声称传入了参考图", TEXT)

    def test_style_requests_use_generic_traits_instead_of_brand_replication(self):
        self.assertIn("改用高层视觉属性", TEXT)
        self.assertIn("不声称官方关联或授权", TEXT)
        fantasy_row = next(line for line in TEXT.splitlines() if line.startswith("| `fantasy-animation`"))
        self.assertNotIn("吉卜力", fantasy_row)
        self.assertNotIn("迪士尼", fantasy_row)

    def test_files_are_only_written_on_explicit_request_and_never_overwritten(self):
        self.assertIn("默认只在对话中交付", TEXT)
        self.assertIn("只有用户明确要求保存/创建文件时", TEXT)
        self.assertIn("不得覆盖现有文件", TEXT)
        self.assertNotIn("交付提示词文件并说明限制", TEXT)
        self.assertIn("没有可用的默认生成能力时，在对话中交付提示词", TEXT)
        self.assertNotIn("粘贴内容 → 保存到 `source.md`", TEXT)

    def test_generation_failure_does_not_switch_tools_implicitly(self):
        self.assertIn("不可用或失败则说明限制并在对话中交付提示词", TEXT)
        self.assertIn("不自动切换到其他工具、脚本或直连 API", TEXT)
        self.assertIn("未指定其他生成方式时，调用 `$image-2`", TEXT)
        self.assertIn("只有用户明确指定其他方式时才切换", TEXT)

    def test_clarification_is_only_for_material_missing_information(self):
        self.assertIn("优先采用内容分析得出的推荐值", TEXT)
        self.assertIn("不得为简单提示词任务强制提问", TEXT)
        self.assertNotIn("使用 AskUserQuestion 确认", TEXT)


if __name__ == "__main__":
    unittest.main()
