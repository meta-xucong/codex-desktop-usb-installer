"""Focused contract checks for current password safety guidance."""
from pathlib import Path
import unittest

SKILL = Path(__file__).resolve().parents[1] / "SKILL.md"

class PasswordGuidanceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")

    def test_each_account_requires_a_distinct_password(self):
        self.assertIn("每个账号使用不同的密码", self.text)
        self.assertIn("不要跨网站或服务复用密码", self.text)
        self.assertIn("同样使用各不相同的密码", self.text)
        self.assertNotIn("可适度复用", self.text)

    def test_password_manager_and_mfa_are_recommended(self):
        self.assertIn("密码管理器生成并保存长且随机的唯一密码", self.text)
        self.assertIn("开启多因素认证（MFA）", self.text)

    def test_rotation_is_tied_to_exposure_or_policy(self):
        self.assertIn("发现泄露、可疑登录或服务通知凭据暴露时，立即更换", self.text)
        self.assertIn("不要求在没有泄露迹象时按固定周期机械轮换", self.text)
        self.assertNotIn("定期更换核心密码", self.text)

    def test_single_factor_length_is_currently_qualified(self):
        self.assertIn("单因素密码给出的最低长度为 15 个字符", self.text)
        self.assertIn("优先遵循当前服务要求", self.text)

if __name__ == "__main__":
    unittest.main()
