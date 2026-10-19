import json
import tempfile
import unittest
from datetime import datetime as RealDateTime, timedelta
from pathlib import Path
from unittest.mock import patch

import reflection_db as db


class FrozenDateTime(RealDateTime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 10, 19, 12, 0, 0, tzinfo=tz)


class ReflectionDatabaseIdTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(dir=Path.cwd())
        root = Path(self.temp_dir.name)
        self.patches = [
            patch.object(db, "DB_DIR", root),
            patch.object(db, "PROFILE_FILE", root / "profile.json"),
            patch.object(db, "REFLECTIONS_FILE", root / "reflections.json"),
            patch.object(db, "HABITS_FILE", root / "habits.json"),
            patch.object(db, "GOALS_FILE", root / "goals.json"),
            patch.object(db, "READING_FILE", root / "reading.json"),
            patch.object(db, "WELLNESS_FILE", root / "wellness.json"),
            patch.object(db, "datetime", FrozenDateTime),
        ]
        for patcher in self.patches:
            patcher.start()
        self.addCleanup(self._cleanup)
        db.ensure_db_files()

    def _cleanup(self):
        for patcher in reversed(self.patches):
            patcher.stop()
        self.temp_dir.cleanup()

    def assert_unique_ids(self, identifiers):
        self.assertEqual(len(identifiers), len(set(identifiers)))
        self.assertTrue(all(len(identifier) == 32 for identifier in identifiers))

    def test_ids_are_unique_for_all_record_collections_at_same_timestamp(self):
        habit_ids = [db.add_habit({"name": name}) for name in ("Read", "Walk")]
        goal_ids = [db.add_goal({"title": title}) for title in ("Goal A", "Goal B")]
        plan_ids = [db.add_reading_plan({"title": title}) for title in ("Book A", "Book B")]
        progress_ids = [
            db.log_reading_progress({"date": "2026-10-19", "pages_read": pages})
            for pages in (5, 8)
        ]
        wellness_ids = [
            db.log_mental_wellness({"date": "2026-10-19", "mood": mood})
            for mood in ("calm", "focused")
        ]

        self.assert_unique_ids(habit_ids)
        self.assert_unique_ids(goal_ids)
        self.assert_unique_ids(plan_ids)
        self.assert_unique_ids(progress_ids)
        self.assert_unique_ids(wellness_ids)
        self.assertEqual({h["habit_id"] for h in db.get_habits()}, set(habit_ids))
        self.assertEqual({g["goal_id"] for g in db.get_goals()}, set(goal_ids))
        self.assertEqual({p["plan_id"] for p in db.get_reading_plans()}, set(plan_ids))

        self.assertTrue(db.update_goal_progress(goal_ids[1], "updated", 65))
        goals_by_id = {goal["goal_id"]: goal for goal in db.get_goals()}
        self.assertNotIn("progress_text", goals_by_id[goal_ids[0]])
        self.assertEqual(goals_by_id[goal_ids[1]]["progress_text"], "updated")
        self.assertEqual(goals_by_id[goal_ids[1]]["progress_percentage"], 65)

        reading = json.loads(db.READING_FILE.read_text(encoding="utf-8"))
        self.assertEqual({entry["log_id"] for entry in reading["reading_logs"]}, set(progress_ids))
        self.assertEqual(db.get_reading_stats(days=30)["total_pages"], 13)
        self.assertEqual(
            {entry["entry_id"] for entry in db.get_wellness_entries()}, set(wellness_ids)
        )

    def test_reflections_upsert_by_date_but_distinct_dates_get_distinct_ids(self):
        first_date = "2026-10-18"
        second_date = "2026-10-19"
        first_id = db.log_reflection({"date": first_date, "mood": "calm"})
        second_id = db.log_reflection({"date": second_date, "mood": "focused"})
        updated_id = db.log_reflection({"date": first_date, "mood": "happy"})

        self.assertNotEqual(first_id, second_id)
        self.assertEqual(updated_id, first_id)
        reflections = db.get_reflections()
        self.assertEqual(len(reflections), 2)
        self.assertEqual({item["reflection_id"] for item in reflections}, {first_id, second_id})
        self.assertEqual(db.get_reflection_by_date(first_date)["mood"], "happy")

    def test_habit_logs_are_unique_and_upsert_by_habit_and_date(self):
        first_habit = db.add_habit({"name": "Read"})
        second_habit = db.add_habit({"name": "Walk"})
        today = "2026-10-19"
        yesterday = (FrozenDateTime.now() - timedelta(days=1)).strftime("%Y-%m-%d")

        first_log = db.log_habit(first_habit, today, True, "done")
        other_habit_log = db.log_habit(second_habit, today, False, "rest")
        other_date_log = db.log_habit(first_habit, yesterday, False, "missed")
        self.assertEqual(db.get_habit_stats(first_habit, days=30)["total_completions"], 1)
        self.assertEqual(db.get_habit_stats(second_habit, days=30)["total_completions"], 0)
        updated_log = db.log_habit(first_habit, today, False, "updated")

        self.assert_unique_ids([first_log, other_habit_log, other_date_log])
        self.assertEqual(updated_log, first_log)
        self.assertEqual(len(db.get_habit_logs(first_habit, days=30)), 2)
        logs = db.get_habit_logs(days=30)
        self.assertEqual(len(logs), 3)
        self.assertEqual(
            sum(1 for item in logs if item["habit_id"] == first_habit and item["date"] == today),
            1,
        )
        self.assertEqual(db.get_habit_stats(first_habit, days=30)["habit_name"], "Read")
        self.assertEqual(db.get_habit_stats(first_habit, days=30)["total_completions"], 0)
        self.assertEqual(db.get_habit_stats(second_habit, days=30)["habit_name"], "Walk")
        self.assertEqual(db.get_habit_stats(second_habit, days=30)["total_completions"], 0)
        first_today = next(
            item for item in logs if item["habit_id"] == first_habit and item["date"] == today
        )
        self.assertEqual(first_today["note"], "updated")


if __name__ == "__main__":
    unittest.main()
