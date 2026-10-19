from __future__ import annotations

import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
REQUIRED = ("删除、移动或覆盖", "源路径", "目标路径", "取得用户授权", "同名目标", "不覆盖", "清单中列出的")


def instructions(agent: str) -> str:
    payload = tomllib.loads((ROOT / "agents" / f"{agent}.toml").read_text(encoding="utf-8"))
    return payload["developer_instructions"]


class AgentFileOperationBoundaryTests(unittest.TestCase):
    def test_office_agent_scopes_file_cleanup_and_collision_policy(self):
        text = instructions("office")
        for phrase in REQUIRED:
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, text)
        self.assertIn("只读清点、分类和建议可以直接进行", text)
        self.assertIn("视为已授权，无需重复确认", text)

    def test_general_agent_requires_path_specific_authorization(self):
        text = instructions("general")
        for phrase in REQUIRED:
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, text)

    def test_agent_role_scope_text_remains_intact(self):
        office = instructions("office")
        general = instructions("general")
        self.assertIn("“整理文件/表格/邮件/会议”", office)
        self.assertIn("视为已授权，无需重复确认", general)
        self.assertIn("文件保存到用户指定位置", general)


if __name__ == "__main__":
    unittest.main(verbosity=2)
