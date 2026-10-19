"""Focused regression tests for meal-plan allergy and dislike filtering."""

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import recipe_generator


class RecipeGeneratorRestrictionTests(unittest.TestCase):
    def test_nuts_allergy_filters_recipes_and_generated_advice(self):
        profile = {
            "age": 30,
            "gender": "female",
            "height_cm": 165,
            "weight_kg": 60,
            "activity_level": "moderate",
            "goal": "maintain",
            "cuisine_preference": "chinese",
            "diet_type": "regular",
            "meals_per_day": 4,
            "allergies": ["坚果"],
            "disliked_foods": [],
            "cooking_condition": "full_kitchen",
            "meal_prep": True,
        }

        plan = recipe_generator.generate_weekly_plan(profile, {"duration_days": 3})

        inspected_text = []
        for day in plan["days"]:
            for meal in day["meals"]:
                inspected_text.append(meal["name"])
                for food in meal["foods"]:
                    inspected_text.append(food["name"])
        for suggestion in plan["meal_prep_suggestions"]:
            for item in suggestion["items"]:
                inspected_text.append(item)
            inspected_text.append(suggestion["method"])
        for tip in plan["cooking_tips"]:
            inspected_text.append(tip)

        # Keep the regression oracle independent from the production matcher.
        for text in inspected_text:
            for forbidden in ("坚果", "混合坚果", "杏仁", "核桃", "腰果", "花生", "花生油", "花生酱"):
                self.assertNotIn(forbidden, text)

        self.assertEqual(plan["restriction_snapshot"]["allergies"], ["坚果"])
        self.assertEqual(plan["restriction_snapshot"]["disliked_foods"], [])
        self.assertIn("交叉接触", plan["allergy_safety_notice"])
        self.assertIn("配料", plan["allergy_safety_notice"])

    def test_disliked_food_english_alias_filters_chinese_ingredient(self):
        recipes = [
            {"name": "香菜汤", "foods": [{"name": "芫荽", "amount": 10}]},
            {"name": "番茄汤", "foods": [{"name": "番茄", "amount": 100}]},
        ]

        safe = recipe_generator.filter_recipes_by_diet(
            recipes, "regular", disliked_foods=["cilantro"]
        )

        self.assertEqual([recipe["name"] for recipe in safe], ["番茄汤"])

    def test_category_with_no_safe_recipe_raises_instead_of_relaxing(self):
        database = copy.deepcopy(recipe_generator.RECIPE_DATABASE)
        database["breakfast"]["chinese"] = [
            {
                "name": "花生酱吐司",
                "foods": [{"name": "花生酱", "amount": 20, "calories": 100}],
            }
        ]
        profile = {
            "cuisine_preference": "chinese",
            "diet_type": "regular",
            "allergies": ["peanut"],
            "disliked_foods": [],
        }

        with patch.object(recipe_generator, "RECIPE_DATABASE", database):
            with self.assertRaisesRegex(ValueError, "早餐.*peanut.*未放宽限制"):
                recipe_generator.generate_daily_meals(
                    profile, {"calories": 2000, "protein_g": 100}, day_number=1
                )

    def test_cli_rejects_unsafe_plan_without_writing_or_relaxing(self):
        repo_root = Path(__file__).resolve().parents[4]
        temp_root = repo_root / "tmp"
        temp_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="meal-restriction-cli-", dir=temp_root) as isolated:
            isolated_path = Path(isolated)
            data_root = isolated_path / "data"
            meal_data = data_root / "meal_planner"
            meal_data.mkdir(parents=True)
            output_path = isolated_path / "plan.json"

            # Derive synthetic restrictions from every configured Chinese breakfast
            # ingredient so the CLI cannot find any safe breakfast recipe.
            allergies = [
                food["name"]
                for recipe in recipe_generator.RECIPE_DATABASE["breakfast"]["chinese"]
                for food in recipe["foods"]
            ]
            profile = {
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
                "allergies": allergies,
                "disliked_foods": [],
            }
            (meal_data / "profile.json").write_text(json.dumps(profile), encoding="utf-8")
            (meal_data / "plans.json").write_text(json.dumps({
                "active_plans": [], "completed_plans": [], "plan_templates": []
            }), encoding="utf-8")

            env = os.environ.copy()
            env["CODEX_DATA_DIR"] = str(data_root)
            script_path = Path(recipe_generator.__file__).resolve()
            result = subprocess.run(
                [sys.executable, str(script_path), "--days", "1", "--output", str(output_path)],
                cwd=str(repo_root),
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("未放宽限制", result.stderr)
            self.assertFalse(output_path.exists())
            plans = json.loads((meal_data / "plans.json").read_text(encoding="utf-8"))
            self.assertEqual(plans["active_plans"], [])


if __name__ == "__main__":
    unittest.main()
