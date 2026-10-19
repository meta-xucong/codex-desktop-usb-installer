"""Reject unsupported gender values before applying sex-specific BMR formulas."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import meal_db


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "installer-pack" / "skills" / "meal-planner" / "scripts" / "recipe_generator.py"
PROFILE = {
    "initialized": True,
    "age": 30,
    "gender": "female",
    "height_cm": 165,
    "weight_kg": 60,
    "activity_level": "moderate",
    "goal": "maintain",
    "cuisine_preference": "chinese",
    "diet_type": "regular",
    "meals_per_day": 3,
    "allergies": [],
    "disliked_foods": [],
    "cooking_condition": "full_kitchen",
    "meal_prep": True,
}


class MealDatabaseGenderValidationTests(unittest.TestCase):
    def test_bmr_rejects_missing_null_nonstring_empty_and_unsupported_gender(self):
        invalid_values = (None, 1, True, "", "   ", "other")
        for gender in invalid_values:
            with self.subTest(gender=gender):
                with self.assertRaisesRegex(ValueError, "gender.*male.*female"):
                    meal_db.calculate_bmr({**PROFILE, "gender": gender})
        with self.assertRaisesRegex(ValueError, "gender.*male.*female"):
            meal_db.calculate_bmr({key: value for key, value in PROFILE.items() if key != "gender"})

    def test_bmr_preserves_male_and_female_formula_results_and_normalizes_strings(self):
        numeric_profile = {**PROFILE, "age": 30, "height_cm": 170, "weight_kg": 70}
        self.assertEqual(meal_db.calculate_bmr({**numeric_profile, "gender": "male"}), 1618)
        self.assertEqual(meal_db.calculate_bmr({**numeric_profile, "gender": "female"}), 1452)
        self.assertEqual(meal_db.calculate_bmr({**numeric_profile, "gender": "  FEMALE  "}), 1452)

    def test_cli_null_gender_fails_concisely_without_output_or_plan(self):
        with tempfile.TemporaryDirectory(prefix="meal-gender-cli-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            data_root = root / "data"
            meal_dir = data_root / "meal_planner"
            meal_dir.mkdir(parents=True)
            profile_path = meal_dir / "profile.json"
            plans_path = meal_dir / "plans.json"
            profile_path.write_text(json.dumps({**PROFILE, "gender": None}), encoding="utf-8")
            plans_path.write_text(json.dumps({
                "active_plans": [], "completed_plans": [], "plan_templates": []
            }), encoding="utf-8")
            (meal_dir / "logs.json").write_text(json.dumps({"food_logs": []}), encoding="utf-8")
            (meal_dir / "recipes.json").write_text(json.dumps({"custom_recipes": [], "favorite_recipes": []}), encoding="utf-8")
            output = root / "meal-plan.json"
            env = os.environ.copy()
            env["CODEX_DATA_DIR"] = str(data_root)

            result = subprocess.run(
                [sys.executable, "-B", str(SCRIPT), "--days", "1", "--output", str(output)],
                cwd=ROOT, env=env, capture_output=True, text=True, check=False, timeout=30,
            )

            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("gender 必须是 male 或 female", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertFalse(output.exists())
            self.assertEqual(json.loads(plans_path.read_text(encoding="utf-8"))["active_plans"], [])


if __name__ == "__main__":
    unittest.main()
