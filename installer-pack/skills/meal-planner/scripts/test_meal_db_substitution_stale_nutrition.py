"""Regression coverage for ingredient substitutions without nutrition data."""

from __future__ import annotations

import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("meal_db.py")
ROOT = SCRIPT.parents[4]
SPEC = importlib.util.spec_from_file_location("usb_meal_db_stale_substitution_test", SCRIPT)
assert SPEC and SPEC.loader
MEAL_DB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MEAL_DB)


class MealSubstitutionStaleNutritionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meal-substitution-stale-", dir=ROOT / "tmp")
        data_dir = Path(self.temp.name) / "meal_planner"
        MEAL_DB.DB_DIR = data_dir
        MEAL_DB.PROFILE_FILE = data_dir / "profile.json"
        MEAL_DB.PLANS_FILE = data_dir / "plans.json"
        MEAL_DB.LOGS_FILE = data_dir / "logs.json"
        MEAL_DB.RECIPES_FILE = data_dir / "recipes.json"
        MEAL_DB.ensure_db_files()
        self.plan = {
            "id": "synthetic-plan",
            "name": "preserve this plan metadata",
            "status": "active",
            "days": [{
                "day": 1,
                "daily_totals": {"calories": 250, "protein": 20, "carbs": 0, "fat": 18},
                "targets_comparison": {"calories_pct": 10},
                "meals": [
                    {
                        "name": "fish meal",
                        "totals": {"calories": 250, "protein": 20, "carbs": 0, "fat": 18},
                        "foods": [{
                            "name": "三文鱼", "amount": 120, "unit": "g",
                            "calories": 250, "protein": 20, "carbs": 0, "fat": 18,
                        }],
                    },
                    {
                        "name": "unaffected meal",
                        "totals": {"calories": 100, "protein": 5, "carbs": 10, "fat": 2},
                        "foods": [{"name": "rice", "calories": 100, "protein": 5, "carbs": 10, "fat": 2}],
                    },
                ],
            }],
        }
        MEAL_DB.save_json(MEAL_DB.PLANS_FILE, {
            "active_plans": [self.plan], "completed_plans": [], "plan_templates": [],
        })

    def tearDown(self):
        self.temp.cleanup()

    def test_replacement_invalidates_food_meal_day_and_plan_nutrition(self):
        self.assertTrue(MEAL_DB.substitute_ingredient("synthetic-plan", "三文鱼", "鳕鱼", "lower_fat"))
        plan = MEAL_DB.get_plan_by_id("synthetic-plan")
        day = plan["days"][0]
        meal = day["meals"][0]
        food = meal["foods"][0]

        self.assertEqual(plan["name"], "preserve this plan metadata")
        self.assertEqual(food["name"], "鳕鱼")
        self.assertEqual(food["nutrition_status"], "needs_recalculation")
        self.assertFalse({"calories", "protein", "carbs", "fat"} & food.keys())
        self.assertEqual(meal["nutrition_status"], "needs_recalculation")
        self.assertNotIn("totals", meal)
        self.assertEqual(day["nutrition_status"], "needs_recalculation")
        self.assertNotIn("daily_totals", day)
        self.assertNotIn("targets_comparison", day)
        self.assertEqual(plan["nutrition_status"], "needs_recalculation")

        # An unrelated meal and its fields remain intact.
        self.assertEqual(day["meals"][1]["totals"]["calories"], 100)
        self.assertEqual(day["meals"][1]["foods"][0]["name"], "rice")

    def test_missing_ingredient_returns_false_without_writing(self):
        before = MEAL_DB.PLANS_FILE.read_bytes()
        before_hash = hashlib.sha256(before).hexdigest()
        self.assertFalse(MEAL_DB.substitute_ingredient("synthetic-plan", "not present", "cod"))
        self.assertEqual(hashlib.sha256(MEAL_DB.PLANS_FILE.read_bytes()).hexdigest(), before_hash)


if __name__ == "__main__":
    unittest.main()
