"""Offline regressions for malformed nested cache metadata."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import data_fetcher


ROOT = Path(__file__).resolve().parents[4]


class FakeFrame:
    empty = False

    def to_dict(self, orient):
        if orient != "records":
            raise ValueError(orient)
        return [{"item": "股票简称", "value": "Synthetic Co"}]


class FakeProvider:
    __version__ = "synthetic"

    def __init__(self):
        self.calls = 0

    def stock_individual_info_em(self, **kwargs):
        self.calls += 1
        return FakeFrame()


class MalformedCacheShapeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="malformed-stock-cache-", dir=ROOT / "tmp")
        self.cache_dir = Path(self.temporary.name)
        self.env_patch = patch.dict("os.environ", {"CHINA_STOCK_CACHE_DIR": str(self.cache_dir)})
        self.env_patch.start()
        self.addCleanup(self._cleanup)

    def _cleanup(self):
        self.env_patch.stop()
        self.temporary.cleanup()

    def test_load_cache_treats_non_object_completeness_as_cache_miss(self):
        path = data_fetcher.get_cache_path("000001", "basic")
        for value in (None, [], "complete", 7):
            with self.subTest(completeness=value):
                path.write_text(json.dumps({"completeness": value}), encoding="utf-8")
                self.assertIsNone(data_fetcher.load_cache("000001", "basic"))

    def test_fetch_recovers_from_null_completeness_and_replaces_bad_cache(self):
        path = data_fetcher.get_cache_path("000002", "basic")
        path.write_text('{"code":"000002","completeness":null}', encoding="utf-8")
        provider = FakeProvider()

        result = data_fetcher.fetch_stock_data("000002", "basic", provider=provider)

        self.assertEqual(provider.calls, 1)
        self.assertFalse(result["cache"]["hit"])
        self.assertEqual(result["completeness"]["status"], "complete")
        self.assertEqual(result["basic_info"]["name"], "Synthetic Co")
        saved = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(saved["completeness"]["status"], "complete")

    def test_save_cache_ignores_malformed_completeness(self):
        path = data_fetcher.get_cache_path("000003", "basic")
        for value in (None, [], "complete"):
            with self.subTest(completeness=value):
                data_fetcher.save_cache("000003", "basic", {"completeness": value})
                self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
