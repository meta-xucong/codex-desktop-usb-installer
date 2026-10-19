from pathlib import Path
import re
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class MeetingSecretarySourceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")
        cls.compact = " ".join(cls.text.split())

    def test_agenda_timing_example_sums_to_declared_totals(self):
        block = re.search(r"议题\s+计划时间\s+实际用时\s+状态\s*\n(.*?)\n```", self.text, re.S)
        self.assertIsNotNone(block, "agenda timing code block exists")
        rows = []
        for line in block.group(1).splitlines():
            m = re.search(r"(\d+)分钟\s+(\d+)分钟", line)
            if m:
                rows.append((int(m.group(1)), int(m.group(2))))
        self.assertGreaterEqual(len(rows), 2)
        details, totals = rows[:-1], rows[-1]
        self.assertEqual(totals, (sum(x[0] for x in details), sum(x[1] for x in details)))
        self.assertIn("示例计时记录", self.text)

    def test_invite_and_email_are_drafts_with_action_specific_authorization(self):
        checklist = self.text.split("## 会议准备清单", 1)[1]
        for line in checklist.splitlines():
            if line.startswith("- [ ]") and any(k in line for k in ("发送", "邀请")):
                self.assertTrue("草稿" in line or "可选" in line, line)
                self.assertIn("授权", line)
        self.assertIn("必须由用户在当前任务中明确要求", self.compact)

    def test_sync_persistence_and_feedback_have_authorization_gates(self):
        checklist = self.text.split("## 会议准备清单", 1)[1]
        lines = [x for x in checklist.splitlines() if x.startswith("- [ ]") and any(k in x for k in ("同步", "写入", "保存", "收集个人信息", "跟进关键待办"))]
        self.assertTrue(lines)
        for line in lines:
            self.assertTrue("可选" in line or "草稿" in line, line)
            self.assertIn("授权", line)

    def test_global_rule_requires_current_task_authorization(self):
        self.assertIn("执行前必须取得用户对本次具体外部操作的明确授权", self.compact)
        self.assertIn("不代表授权执行这些清单项", self.compact)

if __name__ == "__main__":
    unittest.main()
