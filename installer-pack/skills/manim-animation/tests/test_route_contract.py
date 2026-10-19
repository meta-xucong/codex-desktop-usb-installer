from pathlib import Path
import re
import unittest

SKILL_DIR = Path(__file__).resolve().parents[1]


class ManimRouteContract(unittest.TestCase):
    def test_animation_skill_remains_explicit_only(self):
        metadata = (SKILL_DIR / "agents" / "openai.yaml").read_text(encoding="utf-8")
        self.assertRegex(metadata, r"(?m)^  allow_implicit_invocation: false\s*$")
        description = re.search(r"(?m)^description:\s*(.+)$", (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8"))
        self.assertIsNotNone(description)
        self.assertIn("mathematical or explanatory animations", description.group(1))

    def test_user_facing_prompt_names_the_skill_and_preserves_render_boundaries(self):
        metadata = (SKILL_DIR / "agents" / "openai.yaml").read_text(encoding="utf-8")
        self.assertIn('default_prompt: "Use $manim-animation to design and render this explanatory animation."', metadata)
        self.assertIn("Manim Community 0.21.0", (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8"))
        self.assertIn("先输出低成本预览", (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
