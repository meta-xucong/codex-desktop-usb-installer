"""Offline source-contract tests for the video prompt generator."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL = (ROOT / "SKILL.md").read_text(encoding="utf-8")
EXAMPLE = (ROOT / "examples.md").read_text(encoding="utf-8")


class VideoPromptSourceContracts(unittest.TestCase):
    def test_storyboard_intervals_cover_example_duration_without_gaps(self):
        output = EXAMPLE.split("### 生成的提示词", 1)[1]
        # Parse the explicit MM:SS interval annotations rather than narration alone.
        intervals = [(int(a) * 60 + int(b), int(c) * 60 + int(d)) for a, b, c, d in re.findall(
            r"\[(\d{2}):(\d{2})-(\d{2}):(\d{2})\)", output
        )]
        self.assertEqual(intervals, [(0, 3), (3, 6), (6, 10)])
        self.assertEqual(intervals[0][0], 0)
        self.assertEqual(intervals[-1][1], 10)
        self.assertTrue(all(start < end for start, end in intervals))
        self.assertTrue(all(left[1] == right[0] for left, right in zip(intervals, intervals[1:])))
        self.assertEqual(sum(end - start for start, end in intervals), 10)

    def test_general_rule_requires_contiguous_positive_intervals_and_single_shot_fallback(self):
        self.assertIn("区间长度为正且合计恰为 T", SKILL)
        self.assertIn("首镜从 0 开始、末镜结束于总时长", SKILL)
        self.assertIn("不机械凑足3-5镜头", SKILL)
        self.assertIn("改为一个连续镜头或减少镜头并说明", SKILL)
        self.assertNotIn("默认使用分镜脚本模式", SKILL)

    def test_resolution_and_frame_rate_are_capability_gated(self):
        self.assertIn("具体分辨率等规格须由端点能力确认", SKILL)
        self.assertNotIn("直接用于Runway、Pika、Midjourney Video", SKILL)
        self.assertNotIn("降低帧率，捕捉动作细节", SKILL)
        self.assertIn("不得承诺输出规格", SKILL)
        self.assertIn("单个提示词中不得出现相互冲突的数值规格", SKILL)
        self.assertNotRegex(EXAMPLE, r"(?i)\b(?:8k|\d+\s*fps)\b")

    def test_people_brands_and_protected_style_requests_have_boundaries(self):
        output = EXAMPLE.split("### 生成的提示词", 1)[1]
        self.assertIn("不复刻特定导演的独特风格", output)
        self.assertNotIn("黑泽明式", output)
        self.assertIn("不把模仿特定艺术家、导演或工作室（无论是否在世）", SKILL)
        self.assertIn("虚构/合成", SKILL)
        self.assertIn("不复刻受保护的角色、商标或品牌资产", SKILL)
        self.assertIn("Logo/品牌标识只有在用户提供并确认有权使用时才纳入", SKILL)
        self.assertIn("不得推断私人人物身份或补造事实事件", SKILL)

    def test_no_duplicate_sections_or_corrupt_orphan_rows(self):
        for text in (SKILL, EXAMPLE):
            headings = re.findall(r"^#{2,3} .+$", text, re.M)
            self.assertEqual(len(headings), len(set(headings)))
            self.assertNotRegex(text, r"(?m)^建立场景/铺垫 \|")
            self.assertTrue(all(line.startswith("|") for line in text.splitlines() if " | " in line))

    def test_every_storyboard_template_shot_has_time_range(self):
        template = SKILL.split("### 分镜脚本模式输出格式", 1)[1].split("【整合提示词】", 1)[0]
        self.assertEqual(template.count("▌ 时间范围"), 3)
        self.assertIn("[起始-结束时间；时长]", template)


if __name__ == "__main__":
    unittest.main()
