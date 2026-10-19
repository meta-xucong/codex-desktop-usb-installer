"""Offline regression tests for finite fund-return filters."""

from __future__ import annotations

import json
import unittest
from decimal import Decimal

import fund_screener


class FakeFrame:
    empty = False

    def iterrows(self):
        yield 0, {
            "基金代码": "000001",
            "基金简称": "Synthetic Fund",
            "基金类型": "混合型",
            "单位净值": 1.2,
            "累计净值": 1.8,
            "近1年": 20.0,
            "近2年": 15.0,
            "近3年": 30.0,
            "近5年": 40.0,
            "今年来": 5.0,
            "成立来": 80.0,
        }


class FakeProvider:
    def __init__(self):
        self.calls = 0

    def fund_open_fund_rank_em(self):
        self.calls += 1
        return FakeFrame()


class FundScreenerFiniteThresholdTests(unittest.TestCase):
    def test_nonfinite_thresholds_fail_before_provider_call(self):
        for field in ("min_return_1y", "min_return_3y"):
            for value in (float("nan"), float("inf"), float("-inf"), True, "10", Decimal("1.5")):
                with self.subTest(field=field, value=value):
                    provider = FakeProvider()
                    with self.assertRaisesRegex(ValueError, "有限数值"):
                        fund_screener.screen_funds(provider=provider, **{field: value})
                    self.assertEqual(provider.calls, 0)

    def test_finite_thresholds_filter_as_requested_and_serialize_as_strict_json(self):
        provider = FakeProvider()

        passing = fund_screener.screen_funds(min_return_1y=10.0, provider=provider)
        self.assertEqual(passing["matched_count_before_limit"], 1)
        self.assertEqual([fund["code"] for fund in passing["funds"]], ["000001"])
        self.assertEqual(json.loads(json.dumps(passing, allow_nan=False))["funds"][0]["code"], "000001")

        excluded = fund_screener.screen_funds(min_return_3y=31.0, provider=provider)
        self.assertEqual(excluded["matched_count_before_limit"], 0)
        self.assertEqual(excluded["funds"], [])
        self.assertEqual(provider.calls, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
