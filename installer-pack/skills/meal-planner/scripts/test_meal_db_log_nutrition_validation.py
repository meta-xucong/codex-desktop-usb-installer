"""Regression coverage for validated, atomic food-log nutrient input."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import math
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("meal_db.py")
ROOT = SCRIPT.parents[4]
SPEC = importlib.util.spec_from_file_location("usb_meal_db_log_validation_test", SCRIPT)
assert SPEC and SPEC.loader
MEAL_DB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MEAL_DB)


class MealLogNutritionValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meal-log-validation-", dir=ROOT / "tmp")
        data_dir = Path(self.temp.name) / "meal_planner"
        MEAL_DB.DB_DIR = data_dir
        MEAL_DB.PROFILE_FILE = data_dir / "profile.json"
        MEAL_DB.PLANS_FILE = data_dir / "plans.json"
        MEAL_DB.LOGS_FILE = data_dir / "logs.json"
        MEAL_DB.RECIPES_FILE = data_dir / "recipes.json"
        MEAL_DB.ensure_db_files()
        MEAL_DB.save_json(MEAL_DB.LOGS_FILE, {"food_logs": [{"name": "sentinel"}]})

    def tearDown(self):
        self.temp.cleanup()

    def test_invalid_later_food_rejected_without_partial_write_or_caller_mutation(self):
        cases = [
            ("calories", -1),
            ("protein", -0.1),
            ("carbs", -2),
            ("fat", -0.01),
            ("calories", True),
            ("protein", math.nan),
            ("carbs", math.inf),
            ("fat", 10**400),
        ]
        for nutrient, invalid_value in cases:
            with self.subTest(nutrient=nutrient, value=repr(invalid_value)):
                meal = {
                    "name": "invalid synthetic dinner",
                    "foods": [
                        {"name": "valid first item", "calories": 10, "protein": 1, "carbs": 1, "fat": 1},
                        {"name": "invalid later item", "calories": 0, "protein": 0, "carbs": 0, "fat": 0,
                         nutrient: invalid_value},
                    ],
                }
                original = copy.deepcopy(meal)
                before = MEAL_DB.LOGS_FILE.read_bytes()
                before_hash = hashlib.sha256(before).hexdigest()
                with self.assertRaisesRegex(ValueError, rf"foods\[1\]\.{nutrient}"):
                    MEAL_DB.log_meal(meal)
                self.assertEqual(meal, original)
                self.assertEqual(hashlib.sha256(MEAL_DB.LOGS_FILE.read_bytes()).hexdigest(), before_hash)
                self.assertEqual(json.loads(MEAL_DB.LOGS_FILE.read_text(encoding="utf-8")), {"food_logs": [{"name": "sentinel"}]})

    def test_zero_and_positive_values_are_accepted(self):
        meal = {"name": "valid synthetic meal", "foods": [
            {"name": "zero", "calories": 0, "protein": 0, "carbs": 0, "fat": 0},
            {"name": "positive", "calories": 120, "protein": 8, "carbs": 15, "fat": 3},
        ]}
        log_id = MEAL_DB.log_meal(meal)
        self.assertEqual(meal["log_id"], log_id)
        self.assertTrue(meal.get("logged_at"))
        saved = json.loads(MEAL_DB.LOGS_FILE.read_text(encoding="utf-8"))["food_logs"]
        self.assertEqual(len(saved), 2)
        self.assertEqual(saved[-1]["log_id"], log_id)


if __name__ == "__main__":
    unittest.main()
