"""Historical weekly reviews must use the requested habit-log period."""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime as RealDateTime
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SOURCE = Path(__file__).with_name("reflection_db.py")


class FrozenDateTime(RealDateTime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 10, 19, 12, 0, 0, tzinfo=tz)


class WeeklyReviewHabitRangeTests(unittest.TestCase):
    def setUp(self):
        self.temp_root = tempfile.TemporaryDirectory(
            prefix="reflection-weekly-habits-", dir=ROOT / "tmp"
        )
        self.previous_data_dir = os.environ.get("CODEX_DATA_DIR")
        os.environ["CODEX_DATA_DIR"] = self.temp_root.name
        spec = importlib.util.spec_from_file_location(
            f"reflection_db_weekly_{id(self)}", SOURCE
        )
        self.db = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.db)
        self.db.datetime = FrozenDateTime
        self.habit_id = self.db.add_habit({"name": "Walk", "frequency": "daily"})

    def tearDown(self):
        if self.previous_data_dir is None:
            os.environ.pop("CODEX_DATA_DIR", None)
        else:
            os.environ["CODEX_DATA_DIR"] = self.previous_data_dir
        self.temp_root.cleanup()

    def test_historical_review_ignores_logs_after_its_week(self):
        self.db.log_habit(self.habit_id, "2026-10-05", True)
        self.db.log_habit(self.habit_id, "2026-10-12", False)

        report = self.db.generate_weekly_review("2026-10-05")
        habit = report["habits_summary"][0]

        self.assertEqual([report["week_start"], report["week_end"]], ["2026-10-05", "2026-10-11"])
        self.assertEqual([row["date"] for row in habit["recent_logs"]], ["2026-10-05"])
        self.assertEqual(habit["period_days"], 7)
        self.assertEqual(habit["completion_rate"], 100.0)
        self.assertEqual(habit["total_completions"], 1)
        self.assertEqual(habit["current_streak"], 0)

    def test_cli_historical_week_uses_matching_habit_logs(self):
        self.db.log_habit(self.habit_id, "2026-10-05", True)
        self.db.log_habit(self.habit_id, "2026-10-12", False)

        result = subprocess.run(
            [sys.executable, str(SOURCE), "weekly", "2026-10-05"],
            cwd=ROOT,
            env=os.environ.copy(),
            text=True,
            capture_output=True,
            check=True,
        )
        report = json.loads(result.stdout)

        self.assertEqual(report["week_end"], "2026-10-11")
        self.assertEqual(
            [row["date"] for row in report["habits_summary"][0]["recent_logs"]],
            ["2026-10-05"],
        )

    def test_cli_rejects_noncanonical_week_start_without_traceback(self):
        result = subprocess.run(
            [sys.executable, str(SOURCE), "weekly", "2026-1-5"],
            cwd=ROOT,
            env=os.environ.copy(),
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("week_start must use zero-padded YYYY-MM-DD format.", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_historical_current_streak_uses_week_end_not_today_or_future_logs(self):
        self.db.log_habit(self.habit_id, "2026-10-10", True)
        self.db.log_habit(self.habit_id, "2026-10-11", True)
        self.db.log_habit(self.habit_id, "2026-10-12", True)

        report = self.db.generate_weekly_review("2026-10-05")
        habit = report["habits_summary"][0]

        self.assertEqual([row["date"] for row in habit["recent_logs"]], ["2026-10-10", "2026-10-11"])
        self.assertEqual(habit["current_streak"], 2)

    def test_default_relative_window_remains_compatible(self):
        self.db.log_habit(self.habit_id, "2026-10-05", True)
        self.db.log_habit(self.habit_id, "2026-10-12", False)

        self.assertEqual(
            [row["date"] for row in self.db.get_habit_logs(self.habit_id, days=7)],
            ["2026-10-12"],
        )

    def test_explicit_range_rejects_reversed_or_invalid_dates(self):
        with self.assertRaises(ValueError):
            self.db.get_habit_logs(start_date="2026-10-12", end_date="2026-10-05")
        with self.assertRaises(ValueError):
            self.db.get_habit_logs(start_date="not-a-date", end_date="2026-10-11")
        with self.assertRaises(ValueError):
            self.db.get_habit_logs(start_date="2026-1-5", end_date="2026-01-11")
        with self.assertRaises(ValueError):
            self.db.generate_weekly_review("2026-1-5")

    def test_log_and_query_reject_noncanonical_stored_dates(self):
        before = self.db.HABITS_FILE.read_bytes()
        with self.assertRaises(ValueError):
            self.db.log_habit(self.habit_id, "2026-1-5", True)
        self.assertEqual(self.db.HABITS_FILE.read_bytes(), before)

        data = json.loads(before)
        data["habit_logs"] = [{
            "habit_id": self.habit_id, "date": "2026-1-5", "completed": True,
        }]
        self.db.HABITS_FILE.write_text(json.dumps(data), encoding="utf-8")
        with self.assertRaises(ValueError):
            self.db.get_habit_logs(self.habit_id, start_date="2026-01-01", end_date="2026-01-31")

    def test_empty_explicit_range_still_measures_streak_as_of_end_date(self):
        self.db.log_habit(self.habit_id, "2026-10-10", True)

        stats = self.db.get_habit_stats(
            self.habit_id, start_date="2026-10-11", end_date="2026-10-11"
        )

        self.assertEqual(stats["completion_rate"], 0)
        self.assertEqual(stats["total_completions"], 0)
        self.assertEqual(stats["current_streak"], 1)


if __name__ == "__main__":
    unittest.main()
