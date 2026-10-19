from __future__ import annotations

import hashlib
import json
import math
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import travel_db as db


ROOT = Path(__file__).resolve().parents[4]


class TravelExpenseValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", self.root),
            patch.object(db, "PREFERENCES_FILE", self.root / "preferences.json"),
            patch.object(db, "TRIPS_FILE", self.root / "trips.json"),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()
        self.seed = {
            "current_trips": [{
                "id": "synthetic-trip",
                "budget": {"total": 1000, "spent": 0},
                "expenses": [],
            }],
            "past_trips": [],
            "trip_ideas": [],
        }
        db.TRIPS_FILE.write_text(json.dumps(self.seed, indent=2) + "\n", encoding="utf-8")
        self.original_bytes = db.TRIPS_FILE.read_bytes()

    def _cleanup(self) -> None:
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def _digest(self) -> str:
        return hashlib.sha256(db.TRIPS_FILE.read_bytes()).hexdigest()

    def test_negative_boolean_nonfinite_nonnumeric_and_missing_amount_fail_before_save(self) -> None:
        invalid = (-250, True, False, math.inf, -math.inf, math.nan, "250", None, 10**1000)
        original_digest = self._digest()
        for amount in invalid:
            expense = {"category": "food", "amount": amount, "description": "synthetic"}
            original_expense = dict(expense)
            with self.subTest(amount=repr(amount)), self.assertRaisesRegex(ValueError, "expense.amount"):
                db.add_expense("synthetic-trip", expense)
            self.assertEqual(expense, original_expense)
            self.assertEqual(db.TRIPS_FILE.read_bytes(), self.original_bytes)
            self.assertEqual(self._digest(), original_digest)

        with self.assertRaisesRegex(ValueError, "expense.amount"):
            db.add_expense("synthetic-trip", {"category": "food"})
        self.assertEqual(db.TRIPS_FILE.read_bytes(), self.original_bytes)

    def test_zero_and_positive_expenses_are_persisted_and_counted(self) -> None:
        zero = {"category": "other", "amount": 0}
        self.assertTrue(db.add_expense("synthetic-trip", zero))
        self.assertNotIn("id", zero)
        self.assertNotIn("date", zero)
        self.assertTrue(db.add_expense("synthetic-trip", {"category": "food", "amount": 250.5}))
        summary = db.get_budget_summary("synthetic-trip")
        self.assertEqual(summary["spent"], 250.5)
        self.assertEqual(summary["remaining"], 749.5)
        self.assertEqual(summary["by_category"]["food"], 250.5)

    def test_cumulative_overflow_is_rejected_without_database_mutation(self) -> None:
        self.assertTrue(db.add_expense("synthetic-trip", {"category": "other", "amount": 1e308}))
        before = db.TRIPS_FILE.read_bytes()
        digest = self._digest()
        second = {"category": "other", "amount": 1e308}
        with self.assertRaisesRegex(ValueError, "累计支出超出有限数值范围"):
            db.add_expense("synthetic-trip", second)
        self.assertEqual(second, {"category": "other", "amount": 1e308})
        self.assertEqual(db.TRIPS_FILE.read_bytes(), before)
        self.assertEqual(self._digest(), digest)
        self.assertNotIn(b"Infinity", before)
        self.assertEqual(db.get_budget_summary("synthetic-trip")["spent"], 1e308)

    def test_summary_rejects_preexisting_nonfinite_expense_total(self) -> None:
        invalid = {
            "current_trips": [{
                "id": "synthetic-trip",
                "budget": {"total": 1000},
                "expenses": [
                    {"category": "other", "amount": 1e308},
                    {"category": "other", "amount": 1e308},
                ],
            }],
            "past_trips": [],
            "trip_ideas": [],
        }
        db.TRIPS_FILE.write_text(json.dumps(invalid), encoding="utf-8")
        before = db.TRIPS_FILE.read_bytes()
        with self.assertRaisesRegex(ValueError, "累计支出超出有限数值范围"):
            db.get_budget_summary("synthetic-trip")
        self.assertEqual(db.TRIPS_FILE.read_bytes(), before)

    def test_summary_rejects_legacy_negative_rows_even_if_categories_would_cancel(self) -> None:
        legacy = {
            "current_trips": [{
                "id": "synthetic-trip",
                "budget": {"total": 1000},
                "expenses": [
                    {"category": "food", "amount": 1e308},
                    {"category": "other", "amount": -1e308},
                    {"category": "food", "amount": 1e308},
                    {"category": "other", "amount": -1e308},
                ],
            }],
            "past_trips": [],
            "trip_ideas": [],
        }
        db.TRIPS_FILE.write_text(json.dumps(legacy), encoding="utf-8")
        before = db.TRIPS_FILE.read_bytes()
        with self.assertRaisesRegex(ValueError, r"expenses\[1\]\.amount"):
            db.get_budget_summary("synthetic-trip")
        self.assertEqual(db.TRIPS_FILE.read_bytes(), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
