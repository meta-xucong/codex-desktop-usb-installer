"""Offline shopping-list filtering using a plan's frozen restrictions."""

from __future__ import annotations

import sys
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import meal_db  # noqa: E402

SCRIPT = Path(__file__).with_name("meal_db.py")
ROOT = SCRIPT.parents[4]


def synthetic_plan(snapshot, foods):
    return {
        "id": "synthetic-shopping-plan",
        "restriction_snapshot": snapshot,
        "days": [{"meals": [{"foods": foods}]}],
    }


def listed_items(shopping_list):
    return [item for category in shopping_list.values() for item in category]


class ShoppingListRestrictionTests(unittest.TestCase):
    def _shopping_list(self, plan):
        with patch.object(meal_db, "get_plan_by_id", return_value=plan):
            return meal_db.generate_shopping_list("synthetic-shopping-plan", days=7)

    def test_chinese_and_english_peanut_aliases_are_excluded_and_safe_items_aggregate(self):
        plan = synthetic_plan(
            {"allergies": ["peanut"], "disliked_foods": []},
            [
                {"name": "花生酱", "amount": 20, "unit": "g"},
                {"name": "Peanut Butter", "amount": 15, "unit": "g"},
                {"name": "糙米饭", "amount": 100, "unit": "g"},
                {"name": "糙米饭", "amount": 50, "unit": "g"},
            ],
        )

        items = listed_items(self._shopping_list(plan))
        names = {item["name"] for item in items}
        self.assertNotIn("花生酱", names)
        self.assertNotIn("Peanut Butter", names)
        self.assertEqual(names, {"糙米饭"})
        self.assertEqual(items[0]["amount"], 150)

    def test_plan_snapshot_takes_precedence_over_current_profile(self):
        plan = synthetic_plan(
            {"allergies": [], "disliked_foods": []},
            [{"name": "Peanut Butter", "amount": 10, "unit": "g"}],
        )
        with patch.object(meal_db, "get_plan_by_id", return_value=plan), \
             patch.object(meal_db, "get_profile", side_effect=AssertionError("current profile must not be consulted")):
            items = listed_items(meal_db.generate_shopping_list("synthetic-shopping-plan"))
        self.assertEqual([item["name"] for item in items], ["Peanut Butter"])

    def test_missing_or_malformed_snapshot_fails_closed(self):
        invalid_snapshots = (
            None,
            {"allergies": ["peanut"]},
            {"allergies": "peanut", "disliked_foods": []},
            {"allergies": [""], "disliked_foods": []},
            {"allergies": [None], "disliked_foods": []},
            {"allergies": [7], "disliked_foods": []},
        )
        for snapshot in invalid_snapshots:
            with self.subTest(snapshot=snapshot):
                with self.assertRaisesRegex(ValueError, "restriction_snapshot"):
                    self._shopping_list(synthetic_plan(snapshot, [{"name": "rice", "amount": 1, "unit": "g"}]))

        plan = synthetic_plan({"allergies": [], "disliked_foods": []}, [])
        del plan["restriction_snapshot"]
        with self.assertRaisesRegex(ValueError, "restriction_snapshot"):
            self._shopping_list(plan)

    def test_restricted_plan_without_ingredient_name_fails_closed(self):
        plan = synthetic_plan(
            {"allergies": ["peanut"], "disliked_foods": []},
            [{"amount": 1, "unit": "g"}],
        )
        with self.assertRaisesRegex(ValueError, "name must be a non-empty string"):
            self._shopping_list(plan)

        plan["days"][0]["meals"][0]["foods"] = [None]
        with self.assertRaisesRegex(ValueError, "foods\\[0\\] must be a mapping"):
            self._shopping_list(plan)

    def test_unavailable_canonical_filter_fails_closed_for_restricted_plan(self):
        plan = synthetic_plan(
            {"allergies": ["peanut"], "disliked_foods": []},
            [{"name": "花生酱", "amount": 10, "unit": "g"}],
        )
        with patch.dict(sys.modules, {"recipe_generator": None}):
            with self.assertRaisesRegex(ValueError, "cannot safely apply"):
                self._shopping_list(plan)

    def test_cli_reports_missing_snapshot_without_traceback_or_list_output(self):
        with tempfile.TemporaryDirectory(prefix="meal-shopping-cli-", dir=ROOT / "tmp") as temporary:
            data_dir = Path(temporary) / "data"
            meal_dir = data_dir / "meal_planner"
            meal_dir.mkdir(parents=True)
            (meal_dir / "plans.json").write_text(json.dumps({
                "active_plans": [{
                    "id": "legacy-plan",
                    "days": [{"meals": [{"foods": [{"name": "rice", "amount": 1, "unit": "g"}]}]}],
                }],
                "completed_plans": [],
                "plan_templates": [],
            }), encoding="utf-8")
            env = os.environ.copy()
            env["CODEX_DATA_DIR"] = str(data_dir)
            env["TEMP"] = temporary
            env["TMP"] = temporary
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "shopping_list", "legacy-plan"],
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=30,
            )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("restriction_snapshot", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_no_restrictions_preserve_normal_items_and_quantity_aggregation(self):
        plan = synthetic_plan(
            {"allergies": [], "disliked_foods": []},
            [
                {"name": "糙米饭", "amount": 100, "unit": "g"},
                {"name": "糙米饭", "amount": 50, "unit": "g"},
                {"name": "香蕉", "amount": 2, "unit": "个"},
            ],
        )
        items = listed_items(self._shopping_list(plan))
        by_name = {item["name"]: item for item in items}
        self.assertEqual(set(by_name), {"糙米饭", "香蕉"})
        self.assertEqual(by_name["糙米饭"]["amount"], 150)
        self.assertEqual(by_name["香蕉"]["amount"], 2)

    def test_malformed_nested_shapes_and_amounts_are_rejected_before_aggregation(self):
        base_food = {"name": "rice", "amount": 10, "unit": "g"}
        malformed_plans = (
            ({"days": None}, "plan.days must be a list"),
            ({"days": [None]}, r"plan.days\[0\] must be a mapping"),
            ({"days": [{"meals": None}]}, r"plan.days\[0\]\.meals must be a list"),
            ({"days": [{"meals": [None]}]}, r"plan.days\[0\]\.meals\[0\] must be a mapping"),
            ({"days": [{"meals": [{"foods": None}]}]}, r"plan.days\[0\]\.meals\[0\]\.foods must be a list"),
        )
        for changes, message in malformed_plans:
            with self.subTest(changes=changes):
                plan = synthetic_plan({"allergies": [], "disliked_foods": []}, [base_food])
                plan.update(changes)
                with self.assertRaisesRegex(ValueError, message):
                    self._shopping_list(plan)

        invalid_amounts = (-1, True, "10", float("nan"), float("inf"), float("-inf"), 10**400)
        for amount in invalid_amounts:
            with self.subTest(amount=repr(amount)):
                plan = synthetic_plan(
                    {"allergies": [], "disliked_foods": []},
                    [{"name": "rice", "amount": amount, "unit": "g"}],
                )
                with self.assertRaisesRegex(ValueError, r"plan.days\[0\].*\.amount must be a finite non-negative number"):
                    self._shopping_list(plan)

    def test_validates_all_days_before_aggregating_selected_days(self):
        plan = synthetic_plan({"allergies": [], "disliked_foods": []}, [])
        plan["days"].append({"meals": [{"foods": [{"name": "rice", "amount": -1, "unit": "g"}]}]})
        with patch.object(meal_db, "get_plan_by_id", return_value=plan):
            with self.assertRaisesRegex(ValueError, "plan.days\\[1\\].*amount"):
                meal_db.generate_shopping_list("synthetic-shopping-plan", days=1)

    def test_aggregate_amount_overflow_is_rejected(self):
        plan = synthetic_plan(
            {"allergies": [], "disliked_foods": []},
            [
                {"name": "rice", "amount": 1e308, "unit": "g"},
                {"name": "rice", "amount": 1e308, "unit": "g"},
            ],
        )
        with self.assertRaisesRegex(ValueError, "overflowed its finite range"):
            self._shopping_list(plan)

    def test_cli_reports_malformed_days_meals_and_amounts_without_list_or_traceback(self):
        valid_snapshot = {"allergies": [], "disliked_foods": []}
        cases = (
            (synthetic_plan(valid_snapshot, []), "plan.days must be a list", "days-null"),
            (synthetic_plan(valid_snapshot, []), "plan.days[0].meals[0] must be a mapping", "meal-null"),
            (synthetic_plan(valid_snapshot, []), "amount must be a finite non-negative number", "amount-invalid"),
        )
        cases[0][0]["days"] = None
        cases[1][0]["days"] = [{"meals": [None]}]
        cases[2][0]["days"] = [{"meals": [{"foods": [{"name": "rice", "amount": "many", "unit": "g"}]}]}]

        for plan, expected_error, label in cases:
            with self.subTest(case=label), tempfile.TemporaryDirectory(
                prefix=f"meal-shopping-cli-{label}-", dir=ROOT / "tmp"
            ) as temporary:
                data_dir = Path(temporary) / "data"
                meal_dir = data_dir / "meal_planner"
                meal_dir.mkdir(parents=True)
                plan["id"] = "malformed-plan"
                (meal_dir / "plans.json").write_text(json.dumps({
                    "active_plans": [plan], "completed_plans": [], "plan_templates": [],
                }), encoding="utf-8")
                env = os.environ.copy()
                env["CODEX_DATA_DIR"] = str(data_dir)
                env["TEMP"] = temporary
                env["TMP"] = temporary
                result = subprocess.run(
                    [sys.executable, str(SCRIPT), "shopping_list", "malformed-plan"],
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=30,
                )
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn(expected_error, result.stderr)
                self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
