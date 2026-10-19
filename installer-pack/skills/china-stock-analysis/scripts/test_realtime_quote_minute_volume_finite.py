"""Minute quote volume must be finite and nonnegative before int conversion."""

from __future__ import annotations

import contextlib
import io
import json
import unittest
from unittest.mock import patch

import realtime_quote


def payload_for(volume_tokens: list[str]) -> bytes:
    rows = []
    for index, token in enumerate(volume_tokens):
        minute = 30 + index
        rows.append(
            "{" +
            f'"day":"2026-10-19 09:{minute:02d}:00",' +
            '"open":"10","high":"10","low":"10","close":"10",' +
            f'"volume":{token},"amount":"100"' +
            "}"
        )
    return ("[" + ",".join(rows) + "]").encode("utf-8")


class FakeResponse:
    def __init__(self, body: bytes):
        self.body = body

    def read(self) -> bytes:
        return self.body


def fake_opener_for(body: bytes):
    def opener(_request, timeout=10):
        return FakeResponse(body)
    return opener


class MinuteQuoteVolumeFiniteTests(unittest.TestCase):
    def test_fake_provider_rejects_overflow_nonfinite_and_negative_volume(self):
        for token in ("1e10000", "Infinity", "-1"):
            with self.subTest(volume=token):
                with self.assertRaisesRegex(RuntimeError, "成交量"):
                    realtime_quote.fetch_minute_data_sina(
                        "sh600000", opener=fake_opener_for(payload_for([token]))
                    )

    def test_zero_and_positive_volumes_remain_valid(self):
        rows = realtime_quote.fetch_minute_data_sina(
            "sh600000", opener=fake_opener_for(payload_for(["5", "0"]))
        )
        self.assertEqual([row["volume_shares"] for row in rows], [5, 0])

    def test_cli_returns_error_envelope_for_overflowed_provider_volume(self):
        original_fetch_minute = realtime_quote.fetch_minute_data_sina
        cache = {
            "sh600000": {
                "code": "600000", "name": "Synthetic", "exchange": "SH",
                "price": 10.0, "open": 10.0, "pre_close": 10.0, "high": 10.0,
                "low": 10.0, "bid_1": 10.0, "ask_1": 10.0, "volume_shares": 0,
                "amount_cny": 0.0, "change_amount": 0.0, "change_percent": 0.0,
                "market_timestamp": "2026-10-19 09:30:00", "retrieved_at_utc": "synthetic",
                "source_name": realtime_quote.SOURCE_NAME,
                "source_interface": realtime_quote.REALTIME_SOURCE_INTERFACE,
            }
        }
        fake_minute = lambda symbol: original_fetch_minute(
            symbol, opener=fake_opener_for(payload_for(["1e10000"]))
        )
        stdout = io.StringIO()
        stderr = io.StringIO()
        with patch.object(realtime_quote, "fetch_realtime_sina", return_value=cache), \
             patch.object(realtime_quote, "fetch_minute_data_sina", side_effect=fake_minute), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = realtime_quote.main(["600000", "--minute", "--json"])

        envelope = json.loads(stdout.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertFalse(envelope["success"])
        self.assertEqual(envelope["results"], [])
        self.assertEqual(len(envelope["errors"]), 1)
        self.assertEqual(envelope["errors"][0]["code"], "600000")
        self.assertIn("成交量", envelope["errors"][0]["error"])
        self.assertNotIn("Traceback", stdout.getvalue() + stderr.getvalue())

    def test_cli_preserves_normal_minute_bucket_envelope(self):
        original_fetch_minute = realtime_quote.fetch_minute_data_sina
        cache = {
            "sh600000": {
                "code": "600000", "name": "Synthetic", "exchange": "SH",
                "price": 10.0, "open": 10.0, "pre_close": 10.0, "high": 10.0,
                "low": 10.0, "bid_1": 10.0, "ask_1": 10.0, "volume_shares": 5,
                "amount_cny": 50.0, "change_amount": 0.0, "change_percent": 0.0,
                "market_timestamp": "2026-10-19 09:30:00", "retrieved_at_utc": "synthetic",
                "source_name": realtime_quote.SOURCE_NAME,
                "source_interface": realtime_quote.REALTIME_SOURCE_INTERFACE,
            }
        }
        fake_minute = lambda symbol: original_fetch_minute(
            symbol, opener=fake_opener_for(payload_for(["5", "0"]))
        )
        stdout = io.StringIO()
        with patch.object(realtime_quote, "fetch_realtime_sina", return_value=cache), \
             patch.object(realtime_quote, "fetch_minute_data_sina", side_effect=fake_minute), \
             contextlib.redirect_stdout(stdout):
            exit_code = realtime_quote.main(["600000", "--minute", "--json"])

        envelope = json.loads(stdout.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertTrue(envelope["success"])
        self.assertEqual(len(envelope["results"]), 1)
        analysis = envelope["results"][0]["minute_volume"]
        self.assertEqual(analysis["bar_count"], 1)
        self.assertEqual(analysis["total_volume_shares"], 5)
        self.assertEqual(analysis["distribution"]["open_30min"]["volume_shares"], 5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
