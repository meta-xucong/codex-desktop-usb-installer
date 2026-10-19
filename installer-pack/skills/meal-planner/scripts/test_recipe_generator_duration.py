"""Meal plan durations must be positive integers before generation or persistence."""

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import recipe_generator


ROOT = Path(__file__).resolve().parents[4]
SCRIPT = Path(recipe_generator.__file__).resolve()
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


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class RecipeGeneratorDurationTests(unittest.TestCase):
    def test_duration_helper_rejects_nonpositive_and_noninteger_values(self):
        for value in (-1, 0, True, 1.5, "3"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "正整数"):
                    recipe_generator._validate_duration_days(value)
        self.assertEqual(recipe_generator._validate_duration_days(1), 1)

    def test_generator_rejects_invalid_duration_before_calculating_targets(self):
        with patch.object(recipe_generator, "calculate_nutrition_targets") as calculate:
            with self.assertRaisesRegex(ValueError, "正整数"):
                recipe_generator.generate_weekly_plan(PROFILE, {"duration_days": -1})
        calculate.assert_not_called()

    def _isolated_cli_case(self, days, output_name):
        temporary = tempfile.TemporaryDirectory(prefix="meal-duration-cli-", dir=ROOT / "tmp")
        root = Path(temporary.name)
        data_root = root / "data"
        meal_data = data_root / "meal_planner"
        meal_data.mkdir(parents=True)
        profile_path = meal_data / "profile.json"
        plans_path = meal_data / "plans.json"
        profile_path.write_text(json.dumps(PROFILE), encoding="utf-8")
        plans_path.write_text(json.dumps({
            "active_plans": [], "completed_plans": [], "plan_templates": []
        }), encoding="utf-8")
        logs_path = meal_data / "logs.json"
        recipes_path = meal_data / "recipes.json"
        logs_path.write_text(json.dumps({"meal_logs": []}), encoding="utf-8")
        recipes_path.write_text(json.dumps({"recipes": []}), encoding="utf-8")
        output = root / output_name
        before_profile = profile_path.read_bytes()
        before_plans = plans_path.read_bytes()
        env = os.environ.copy()
        env["CODEX_DATA_DIR"] = str(data_root)
        result = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--days", str(days), "--output", str(output)],
            cwd=ROOT, env=env, capture_output=True, text=True, check=False, timeout=30,
        )
        return temporary, profile_path, plans_path, output, before_profile, before_plans, result

    def test_cli_zero_and_negative_days_fail_without_output_or_database_changes(self):
        for days in (0, -1):
            with self.subTest(days=days):
                temporary, profile, plans, output, before_profile, before_plans, result = self._isolated_cli_case(
                    days, f"invalid-{days}.json"
                )
                try:
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertIn("duration_days 必须是正整数", result.stderr)
                    self.assertFalse(output.exists())
                    self.assertEqual(profile.read_bytes(), before_profile)
                    self.assertEqual(plans.read_bytes(), before_plans)
                    self.assertEqual(json.loads(plans.read_text(encoding="utf-8"))["active_plans"], [])
                finally:
                    temporary.cleanup()

    def test_cli_positive_duration_creates_a_plan(self):
        temporary, profile, plans, output, before_profile, before_plans, result = self._isolated_cli_case(
            1, "valid-plan.json"
        )
        try:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            generated = json.loads(output.read_text(encoding="utf-8"))
            stored = json.loads(plans.read_text(encoding="utf-8"))
            self.assertEqual(generated["duration_days"], 1)
            self.assertEqual(len(generated["days"]), 1)
            self.assertEqual(len(stored["active_plans"]), 1)
            self.assertEqual(stored["active_plans"][0]["duration_days"], 1)
            self.assertEqual(profile.read_bytes(), before_profile)
            self.assertNotEqual(digest(plans), hashlib.sha256(before_plans).hexdigest())
        finally:
            temporary.cleanup()


if __name__ == "__main__":
    unittest.main()
