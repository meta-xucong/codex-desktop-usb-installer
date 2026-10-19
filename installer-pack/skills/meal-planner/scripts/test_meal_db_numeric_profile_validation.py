"""Numeric profile fields must be finite positive numbers before nutrition math."""

import json
import math
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


class MealDatabaseNumericProfileValidationTests(unittest.TestCase):
    def test_bmr_rejects_invalid_values_for_each_numeric_profile_field(self):
        invalid_values = (True, False, None, "30", 0, -1, math.nan, math.inf, -math.inf)
        for field in ("age", "height_cm", "weight_kg"):
            for value in invalid_values:
                with self.subTest(field=field, value=value):
                    with self.assertRaisesRegex(ValueError, field):
                        meal_db.calculate_bmr({**PROFILE, field: value})
            with self.subTest(field=field, value="missing"):
                with self.assertRaisesRegex(ValueError, field):
                    meal_db.calculate_bmr({key: value for key, value in PROFILE.items() if key != field})

    def test_valid_numeric_profile_preserves_bmr_and_nutrition_targets(self):
        self.assertEqual(meal_db.calculate_bmr(PROFILE), 1320)
        self.assertEqual(meal_db.calculate_nutrition_targets(PROFILE), {
            "calories": 2046,
            "protein_g": 128,
            "carbs_g": 256,
            "fat_g": 57,
            "protein_pct": 25,
            "carbs_pct": 50,
            "fat_pct": 25,
            "bmr": 1320,
            "tdee": 2046,
        })

    def test_invalid_derived_bmr_calories_and_macro_denominator_fail_cleanly(self):
        with self.assertRaisesRegex(ValueError, "BMR"):
            meal_db.calculate_bmr({**PROFILE, "age": 10000})
        with self.assertRaisesRegex(ValueError, "target_calories"):
            meal_db.calculate_nutrition_targets({
                **PROFILE, "age": 100, "height_cm": 100, "weight_kg": 10, "goal": "fat_loss"
            })
        with self.assertRaisesRegex(ValueError, "total_calculated_calories"):
            meal_db.calculate_nutrition_targets({
                **PROFILE, "age": 1, "height_cm": 0.1, "weight_kg": 0.1, "gender": "male"
            })

    def test_oversized_integer_and_derived_float_overflow_fail_cleanly(self):
        with self.assertRaisesRegex(ValueError, "age"):
            meal_db.calculate_bmr({**PROFILE, "age": 10**400})
        with self.assertRaisesRegex(ValueError, "BMR"):
            meal_db.calculate_bmr({**PROFILE, "height_cm": 1e308})
        with self.assertRaisesRegex(ValueError, "TDEE"):
            meal_db.calculate_tdee({
                **PROFILE, "height_cm": 1.6e307, "activity_level": "very_high"
            })

    def test_cli_bool_profile_fails_concisely_without_output_or_plan(self):
        with tempfile.TemporaryDirectory(prefix="meal-bool-profile-cli-", dir=ROOT / "tmp") as temporary:
            root = Path(temporary)
            data_root = root / "data"
            meal_dir = data_root / "meal_planner"
            meal_dir.mkdir(parents=True)
            profile_path = meal_dir / "profile.json"
            plans_path = meal_dir / "plans.json"
            profile_path.write_text(json.dumps({
                **PROFILE, "age": True, "height_cm": False, "weight_kg": False,
            }), encoding="utf-8")
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
            self.assertIn("weight_kg", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertNotIn("ZeroDivisionError", result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertFalse(output.exists())
            self.assertEqual(json.loads(plans_path.read_text(encoding="utf-8"))["active_plans"], [])


if __name__ == "__main__":
    unittest.main()
