"""Isolated regression tests for same-second meal plan and log identities."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from datetime import datetime as RealDateTime
from pathlib import Path


SCRIPT = Path(__file__).with_name("meal_db.py")
ROOT = SCRIPT.parents[4]
SPEC = importlib.util.spec_from_file_location("usb_meal_db_id_test", SCRIPT)
assert SPEC and SPEC.loader
MEAL_DB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MEAL_DB)
FIXED = RealDateTime(2026, 10, 19, 12, 0, 0)


class FrozenDateTime(RealDateTime):
    @classmethod
    def now(cls, tz=None):
        if tz is not None:
            return FIXED.replace(tzinfo=tz)
        return FIXED


class MealDatabaseIdUniquenessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(
            prefix="meal-db-id-", dir=ROOT / "tmp"
        )
        data_dir = Path(self.temp.name) / "meal_planner"
        MEAL_DB.DB_DIR = data_dir
        MEAL_DB.PROFILE_FILE = data_dir / "profile.json"
        MEAL_DB.PLANS_FILE = data_dir / "plans.json"
        MEAL_DB.LOGS_FILE = data_dir / "logs.json"
        MEAL_DB.RECIPES_FILE = data_dir / "recipes.json"
        self.original_datetime = MEAL_DB.datetime
        MEAL_DB.datetime = FrozenDateTime

    def tearDown(self):
        MEAL_DB.datetime = self.original_datetime
        self.temp.cleanup()

    def test_same_second_plans_keep_distinct_get_and_update_identity(self):
        first_id = MEAL_DB.add_plan({"name": "synthetic-one"})
        second_id = MEAL_DB.add_plan({"name": "synthetic-two"})
        self.assertNotEqual(first_id, second_id)
        self.assertEqual(len(MEAL_DB.get_plans("active")["active_plans"]), 2)
        self.assertEqual(MEAL_DB.get_plan_by_id(first_id)["name"], "synthetic-one")
        self.assertEqual(MEAL_DB.get_plan_by_id(second_id)["name"], "synthetic-two")

        self.assertTrue(MEAL_DB.update_plan(second_id, {"name": "synthetic-two-updated"}))
        self.assertEqual(MEAL_DB.get_plan_by_id(first_id)["name"], "synthetic-one")
        self.assertEqual(
            MEAL_DB.get_plan_by_id(second_id)["name"], "synthetic-two-updated"
        )

    def test_same_second_meal_logs_keep_distinct_ids_and_records(self):
        first_id = MEAL_DB.log_meal({"name": "synthetic-meal-one"})
        second_id = MEAL_DB.log_meal({"name": "synthetic-meal-two"})
        logs = MEAL_DB.get_logs()
        self.assertNotEqual(first_id, second_id)
        self.assertEqual(len(logs), 2)
        self.assertEqual({log["log_id"] for log in logs}, {first_id, second_id})
        self.assertEqual({log["name"] for log in logs}, {
            "synthetic-meal-one", "synthetic-meal-two"
        })


if __name__ == "__main__":
    unittest.main()
