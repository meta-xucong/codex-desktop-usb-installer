from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pandas as pd

SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))
import technical_analysis  # noqa: E402


class TechnicalAnalysisNonfiniteInputTests(unittest.TestCase):
    def _finite_inputs(self):
        prices = [100.0 + index for index in range(60)]
        return prices, [value + 1 for value in prices], [value - 1 for value in prices], [1000.0] * 60

    def test_direct_generator_rejects_nonfinite_values_in_each_series(self) -> None:
        for series_index, field in enumerate(("收盘价", "最高价", "最低价", "成交量")):
            for invalid in (float("nan"), float("inf"), float("-inf")):
                with self.subTest(field=field, invalid=invalid):
                    inputs = [list(values) for values in self._finite_inputs()]
                    inputs[series_index][17] = invalid
                    with self.assertRaisesRegex(ValueError, rf"{field}第18条K线必须是有限数值"):
                        technical_analysis.generate_technical_analysis(
                            "000001", *inputs, "daily"
                        )

    def test_market_fetch_rejects_nonfinite_provider_row_with_field_and_position(self) -> None:
        prices, highs, lows, volumes = self._finite_inputs()
        prices[10] = float("nan")
        frame = pd.DataFrame({
            "日期": [f"2026-08-{(index % 28) + 1:02d}" for index in range(60)],
            "收盘": prices,
            "最高": highs,
            "最低": lows,
            "成交量": volumes,
        })
        fake_akshare = SimpleNamespace(stock_zh_a_hist=lambda **_kwargs: frame)

        with patch.dict(sys.modules, {"akshare": fake_akshare}):
            with self.assertRaisesRegex(ValueError, "收盘价第11条K线必须是有限数值"):
                technical_analysis.fetch_market_data("000001", "daily", 60)

    def test_cli_invalid_source_fails_in_json_without_creating_or_overwriting_output(self) -> None:
        temp_root = Path.cwd() / "tmp"
        temp_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="technical-nonfinite-", dir=temp_root) as temporary:
            root = Path(temporary)
            new_output = root / "new-report.json"
            existing_output = root / "existing-report.json"
            existing_output.write_bytes(b"keep existing report\n")
            existing_before = existing_output.read_bytes()
            prices, highs, lows, volumes = self._finite_inputs()
            prices[-1] = float("nan")

            for output_path, existed in ((new_output, False), (existing_output, True)):
                with self.subTest(existing=existed):
                    stdout = io.StringIO()
                    with patch.object(
                        technical_analysis,
                        "fetch_market_data",
                        return_value=(prices, highs, lows, volumes, "2026-10-19"),
                    ):
                        with patch.object(sys, "argv", [
                            "technical_analysis.py", "--code", "000001", "--json",
                            "--output", str(output_path),
                        ]):
                            with patch.object(sys, "stdout", stdout):
                                exit_code = technical_analysis.main()

                    self.assertEqual(exit_code, 1)
                    error = json.loads(stdout.getvalue())
                    self.assertIn("收盘价第60条K线必须是有限数值", error["error"])
                    if existed:
                        self.assertEqual(output_path.read_bytes(), existing_before)
                    else:
                        self.assertFalse(output_path.exists())

    def test_finite_series_remain_reportable_as_strict_json(self) -> None:
        report = technical_analysis.generate_technical_analysis(
            "000001", *self._finite_inputs(), "daily"
        )

        encoded = json.dumps(report, ensure_ascii=False, allow_nan=False)
        self.assertEqual(json.loads(encoded)["code"], "000001")


if __name__ == "__main__":
    unittest.main(verbosity=2)
