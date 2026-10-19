#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Attributed portfolio performance arithmetic without embedded advice."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_INPUT_BYTES = 5 * 1024 * 1024


def _ensure_finite_values(value: Any, path: str = "report") -> None:
    """Reject non-finite values derived anywhere in the performance report."""
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError(f"{path} 计算结果超出有限数值范围。")
    elif isinstance(value, dict):
        for key, item in value.items():
            _ensure_finite_values(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            _ensure_finite_values(item, f"{path}[{index}]")


def calculate_annualized_return(initial: float, current: float, years: float) -> float:
    if initial <= 0 or current <= 0 or years <= 0:
        raise ValueError("年化收益计算要求正的初值、终值和期间。")
    return (pow(current / initial, 1 / years) - 1) * 100


def calculate_max_drawdown(values: list[float]) -> float:
    peak = values[0]
    maximum = 0.0
    for value in values:
        peak = max(peak, value)
        maximum = max(maximum, (peak - value) / peak * 100)
    return maximum


def calculate_volatility(monthly_returns_percent: list[float]) -> float | None:
    if len(monthly_returns_percent) < 2:
        return None
    mean = sum(monthly_returns_percent) / len(monthly_returns_percent)
    population_variance = sum((value - mean) ** 2 for value in monthly_returns_percent) / len(
        monthly_returns_percent
    )
    return math.sqrt(population_variance) * math.sqrt(12)


def _finite(value: Any, field: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{field} 必须是有限数值。")
    try:
        number = float(value)
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(f"{field} 必须是有限数值。") from exc
    if not math.isfinite(number):
        raise ValueError(f"{field} 必须是有限数值。")
    return number


def track_portfolio(
    initial_value: float,
    current_value: float,
    history: list[dict[str, Any]],
    months: int,
    risk_free_rate: float,
    benchmarks: dict[str, float],
    data_as_of: str,
    sources: list[str],
) -> dict[str, Any]:
    initial_value = _finite(initial_value, "initial_value")
    current_value = _finite(current_value, "current_value")
    risk_free_rate = _finite(risk_free_rate, "risk_free_rate")
    if initial_value <= 0 or current_value <= 0:
        raise ValueError("initial_value 和 current_value 必须为正。")
    if isinstance(months, bool) or not isinstance(months, int) or not 1 <= months <= 1200:
        raise ValueError("months 必须是 1–1200 的整数。")
    if risk_free_rate <= -100:
        raise ValueError("risk_free_rate 必须大于 -100%。")
    if not str(data_as_of or "").strip():
        raise ValueError("缺少 data_as_of。")
    normalized_sources = [str(item).strip() for item in sources or [] if str(item).strip()]
    if not normalized_sources:
        raise ValueError("sources 必须至少包含一个非空来源说明或 URL。")
    if not isinstance(history, list) or len(history) != months + 1:
        raise ValueError("按月 history 必须恰好包含 months + 1 个净值点。")

    values = []
    for index, item in enumerate(history):
        if not isinstance(item, dict) or "value" not in item:
            raise ValueError(f"history[{index}] 必须包含 value。")
        value = _finite(item["value"], f"history[{index}].value")
        if value <= 0:
            raise ValueError(f"history[{index}].value 必须为正。")
        values.append(value)
    if not math.isclose(values[0], initial_value, rel_tol=1e-9, abs_tol=1e-9):
        raise ValueError("history 首值必须与 initial_value 一致。")
    if not math.isclose(values[-1], current_value, rel_tol=1e-9, abs_tol=1e-9):
        raise ValueError("history 末值必须与 current_value 一致。")

    if not isinstance(benchmarks, dict):
        raise ValueError("benchmarks 必须是名称到同期年化收益率的对象。")
    normalized_benchmarks = {}
    for raw_name, raw_return in benchmarks.items():
        name = str(raw_name).strip()
        if not name:
            raise ValueError("benchmarks 包含空名称。")
        if name in normalized_benchmarks:
            raise ValueError(f"benchmarks 名称去除首尾空白后重复：{name}")
        normalized_benchmarks[name] = _finite(raw_return, f"benchmarks.{name}")

    monthly_returns = [(values[index] / values[index - 1] - 1) * 100 for index in range(1, len(values))]
    years = months / 12
    annualized_return = calculate_annualized_return(initial_value, current_value, years)
    volatility = calculate_volatility(monthly_returns)
    maximum_drawdown = calculate_max_drawdown(values)
    sharpe = None if volatility in (None, 0) else (annualized_return - risk_free_rate) / volatility
    calmar = None if maximum_drawdown == 0 else annualized_return / maximum_drawdown
    comparisons = {
        name: {
            "benchmark_return": value,
            "portfolio_annualized_return": round(annualized_return, 4),
            "difference_percentage_points": round(annualized_return - value, 4),
        }
        for name, value in normalized_benchmarks.items()
    }
    report = {
        "calculated_at": datetime.now(timezone.utc).isoformat(),
        "data_as_of": str(data_as_of).strip(),
        "sources": normalized_sources,
        "period_months": months,
        "initial_value": initial_value,
        "current_value": current_value,
        "total_return_percent": round((current_value / initial_value - 1) * 100, 4),
        "annualized_return_percent": round(annualized_return, 4),
        "annualized_population_volatility_percent": None if volatility is None else round(volatility, 4),
        "max_drawdown_percent": round(maximum_drawdown, 4),
        "risk_free_rate": risk_free_rate,
        "sharpe_ratio": None if sharpe is None else round(sharpe, 4),
        "calmar_ratio": None if calmar is None else round(calmar, 4),
        "monthly_returns_percent": [round(value, 4) for value in monthly_returns],
        "benchmark_comparison": comparisons,
        "methodology": (
            "假设 history 是无期间申赎的月末净值序列；波动率使用月收益总体标准差 × sqrt(12)。"
            "脚本不内置基准收益、评分、调仓、止盈或止损建议。"
        ),
    }
    _ensure_finite_values(report)
    return report


def _load_json(path_value: str, label: str) -> Any:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError(f"{label}不存在或超过 5 MiB。")
    return json.loads(path.read_text(encoding="utf-8"))


def _reject_output_overwrite(output_path: Path, input_paths: list[str | Path]) -> None:
    """Reject replacing any JSON source read by this command, including aliases."""
    for input_path in input_paths:
        try:
            resolved_output = output_path.expanduser().resolve()
            resolved_input = Path(input_path).expanduser().resolve()
            if resolved_output == resolved_input:
                raise ValueError("--output must differ from input JSON files.")
            # Inputs were successfully read before this guard. An absent output is
            # safe; any other metadata error means identity cannot be established.
            resolved_input.stat()
            try:
                resolved_output.stat()
            except FileNotFoundError:
                continue
            if resolved_output.samefile(resolved_input):
                raise ValueError("--output must differ from input JSON files.")
        except ValueError:
            raise
        except OSError as exc:
            raise ValueError("Cannot verify --output is distinct from input JSON files.") from exc


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="组合历史表现透明计算器")
    parser.add_argument("--initial", type=float, required=True, help="期初组合价值")
    parser.add_argument("--current", type=float, required=True, help="期末组合价值")
    parser.add_argument("--history", required=True, help="按月历史净值 JSON 数组")
    parser.add_argument("--months", type=int, required=True, help="区间月数")
    parser.add_argument("--risk-free-rate", type=float, required=True, help="同期年化无风险利率（%）")
    parser.add_argument("--benchmarks", help="可选：已核验的同期基准年化收益率 JSON 对象")
    parser.add_argument("--as-of", required=True, help="数据时点")
    parser.add_argument("--source", action="append", required=True, help="数据来源说明或 URL，可多次使用")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        history = _load_json(args.history, "history JSON")
        benchmarks = _load_json(args.benchmarks, "benchmarks JSON") if args.benchmarks else {}
        report = track_portfolio(
            args.initial,
            args.current,
            history,
            args.months,
            args.risk_free_rate,
            benchmarks,
            args.as_of,
            args.source,
        )
        output = json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            _reject_output_overwrite(
                output_path,
                [args.history, *([args.benchmarks] if args.benchmarks else [])],
            )
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"追踪报告已保存到: {output_path}")
        else:
            print(output)
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        payload = json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False)
        print(payload if args.json else f"追踪计算失败：{exc}", file=sys.stdout if args.json else sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
