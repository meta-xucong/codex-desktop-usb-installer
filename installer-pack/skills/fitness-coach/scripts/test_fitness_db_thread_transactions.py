#!/usr/bin/env python3
"""Thread-level transaction tests for Fitness Coach database mutators."""

from __future__ import annotations

import importlib.util
import os
import tempfile
import threading
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SOURCE = Path(__file__).with_name("fitness_db.py")


class FitnessDatabaseThreadTransactionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="fitness-thread-tx-", dir=ROOT / "tmp")
        self.previous_data_dir = os.environ.get("CODEX_DATA_DIR")
        isolated_data_dir = Path(self.temp_dir.name) / "isolated-data"
        os.environ["CODEX_DATA_DIR"] = str(isolated_data_dir)
        spec = importlib.util.spec_from_file_location(f"fitness_db_thread_test_{id(self)}", SOURCE)
        self.db = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.db)
        self.db.ensure_db_files()

    def tearDown(self) -> None:
        if self.previous_data_dir is None:
            os.environ.pop("CODEX_DATA_DIR", None)
        else:
            os.environ["CODEX_DATA_DIR"] = self.previous_data_dir
        self.temp_dir.cleanup()

    def run_concurrently(self, operation, values):
        # Synchronize before entering the database operation. A barrier after a
        # transaction's read would deadlock once that transaction lock is held.
        start = threading.Barrier(len(values) + 1)
        results = [None] * len(values)
        failures = []

        def worker(index, value):
            try:
                start.wait(timeout=10)
                results[index] = operation(value)
            except BaseException as exc:  # surface worker errors in the test thread
                failures.append(exc)

        threads = [
            threading.Thread(target=worker, args=(index, value), daemon=True)
            for index, value in enumerate(values)
        ]
        for thread in threads:
            thread.start()
        start.wait(timeout=10)
        for thread in threads:
            thread.join(timeout=20)
        self.assertFalse(any(thread.is_alive() for thread in threads), "worker thread did not finish")
        if failures:
            error = failures[0]
            if getattr(error, "winerror", None) == 5:
                self.fail(f"Windows ERROR_ACCESS_DENIED during atomic replace (storage failure, not a lost-update result): {error}")
            raise error
        return results

    def test_concurrent_completions_preserve_both_programs(self) -> None:
        first_id = self.db.add_program({"name": "synthetic completed one", "workout_days": []})
        second_id = self.db.add_program({"name": "synthetic completed two", "workout_days": []})

        results = self.run_concurrently(self.db.complete_program, [first_id, second_id])

        self.assertEqual(results, [True, True])
        programs = self.db.get_programs()
        self.assertEqual(programs["active_programs"], [])
        completed_ids = {program["id"] for program in programs["completed_programs"]}
        self.assertEqual(completed_ids, {first_id, second_id})

    def test_concurrent_active_adds_preserve_both_and_one_active_status(self) -> None:
        new_programs = [
            {"name": "synthetic active one", "workout_days": []},
            {"name": "synthetic active two", "workout_days": []},
        ]
        ids = self.run_concurrently(
            lambda program: self.db.add_program(program, status="active"), new_programs
        )

        programs = self.db.get_programs()["active_programs"]
        self.assertEqual({program["id"] for program in programs}, set(ids))
        self.assertEqual(len(programs), 2)
        self.assertEqual(sum(program["status"] == "active" for program in programs), 1)
        self.assertEqual(sum(program["status"] == "paused" for program in programs), 1)

    def test_normal_program_add_remains_compatible(self) -> None:
        program_id = self.db.add_program({"name": "synthetic normal add", "workout_days": []})

        stored = self.db.get_program_by_id(program_id)
        self.assertIsNotNone(stored)
        self.assertEqual(stored["name"], "synthetic normal add")
        self.assertEqual(stored["status"], "active")
        self.assertTrue(stored["created_at"])


if __name__ == "__main__":
    unittest.main()
