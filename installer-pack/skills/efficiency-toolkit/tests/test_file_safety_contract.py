"""Focused source-contract checks for safe file-organization guidance."""

from pathlib import Path
import unittest


SKILL_MD = Path(__file__).resolve().parents[1] / "SKILL.md"


class FileSafetyContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.skill = SKILL_MD.read_text(encoding="utf-8")

    def test_read_only_inventory_and_preview_come_before_changes(self):
        inventory = self.skill.index("**只读盘点**")
        preview = self.skill.index("**预览与授权**")
        self.assertLess(inventory, preview)
        self.assertIn("不先重命名、移动、删除或覆盖", self.skill)
        self.assertIn("未授权的项目只给建议，不执行", self.skill)

    def test_mutations_require_exact_paths_and_user_authorization(self):
        for operation in ("删除", "移动", "重命名", "覆盖"):
            with self.subTest(operation=operation):
                self.assertIn(operation, self.skill)
        self.assertIn("明确源路径、目标路径和操作", self.skill)
        self.assertIn("并取得用户授权", self.skill)
        self.assertIn("授权范围以外不操作", self.skill)

    def test_duplicate_candidates_require_hash_or_content_verification(self):
        self.assertIn("文件名相同只能作为候选线索", self.skill)
        self.assertIn("比较文件哈希", self.skill)
        self.assertIn("必要时再检查内容", self.skill)
        self.assertIn("不因名称相似或相同直接删除", self.skill)

    def test_conflicts_are_skipped_without_overwrite(self):
        self.assertIn("默认跳过该项并报告冲突", self.skill)
        self.assertIn("不覆盖", self.skill)

    def test_non_file_organization_capabilities_are_preserved(self):
        for heading in ("## 笔记管理规范", "## 日程规划方法", "## 密码管理规范"):
            with self.subTest(heading=heading):
                self.assertIn(heading, self.skill)


if __name__ == "__main__":
    unittest.main()
