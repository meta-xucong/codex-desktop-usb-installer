#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Mechanical portfolio target-difference and transaction-cost calculator."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_INPUT_BYTES = 1024 * 1024


def validate_allocations(raw: Any, label: str) -> dict[str, float]:
    if not isinstance(raw, dict) or not raw:
        raise ValueError(f"{label} 必须是非空的资产到百分比 JSON 对象。")
    normalized = {}
    for raw_code, raw_weight in raw.items():
        code = str(raw_code).strip()
        if not code:
            raise ValueError(f"{label} 包含空资产代码。")
        if isinstance(raw_weight, bool):
            raise ValueError(f"{label}.{code} 必须是有限数值。")
        try:
            weight = float(raw_weight)
        except (TypeError, ValueError) as exc:
            raise ValueError(f"{label}.{code} 必须是有限数值。") from exc
        if not math.isfinite(weight) or weight < 0 or weight > 100:
            raise ValueError(f"{label}.{code} 必须在 0–100 之间。")
        normalized[code] = weight
    if not math.isclose(sum(normalized.values()), 100.0, abs_tol=0.01):
        raise ValueError(f"{label} 权重必须合计 100。")
    return normalized


def calculate_rebalance_plan(
    current: dict[str, float],
    target: dict[str, float],
    total_value: float,
    threshold: float,
    transaction_cost_rate: float,
) -> dict[str, Any]:
    current = validate_allocations(current, "current")
    target = validate_allocations(target, "target")
    for value, field, lower, upper in (
        (total_value, "total_value", 0, None),
        (threshold, "threshold", 0, 100),
        (transaction_cost_rate, "transaction_cost_rate", 0, 100),
    ):
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
            raise ValueError(f"{field} 必须是有限数值。")
        below_lower_bound = float(value) <= lower if field == "total_value" else float(value) < lower
        if below_lower_bound:
            raise ValueError(f"{field} 超出允许范围。")
        if upper is not None and float(value) > upper:
            raise ValueError(f"{field} 超出允许范围。")

    differences = []
    total_increase = 0.0
    total_decrease = 0.0
    for code in sorted(set(current) | set(target)):
        current_weight = current.get(code, 0.0)
        target_weight = target.get(code, 0.0)
        difference_points = target_weight - current_weight
        amount_difference = total_value * difference_points / 100
        included = abs(difference_points) >= threshold and not math.isclose(difference_points, 0, abs_tol=1e-12)
        if included and amount_difference > 0:
            total_increase += amount_difference
        elif included and amount_difference < 0:
            total_decrease += -amount_difference
        differences.append(
            {
                "asset": code,
                "current_percent": current_weight,
                "target_percent": target_weight,
                "difference_percentage_points": round(difference_points, 6),
                "amount_difference": round(amount_difference, 2),
                "included_by_threshold": included,
                "direction": "increase" if difference_points > 0 else "decrease" if difference_points < 0 else "unchanged",
            }
        )
    estimated_cost = total_decrease * transaction_cost_rate / 100
    return {
        "calculated_at": datetime.now(timezone.utc).isoformat(),
        "portfolio_value": float(total_value),
        "threshold_percentage_points": float(threshold),
        "transaction_cost_rate": float(transaction_cost_rate),
        "differences": differences,
        "summary": {
            "total_buy": round(total_increase, 2),
            "total_sell": round(total_decrease, 2),
            "estimated_cost": round(estimated_cost, 2),
            "unfunded_increase_after_estimated_sell_cost": round(
                max(0.0, total_increase - max(0.0, total_decrease - estimated_cost)), 2
            ),
        },
        "methodology": (
            "仅计算当前权重到用户给定目标权重的差额；阈值和卖出交易成本率均为显式输入。"
            "结果不下单，不考虑税、买入费用、滑点、最小交易单位或账户限制。"
        ),
    }


def _load_json(path_value: str, label: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError(f"{label} JSON 不存在或超过 1 MiB。")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{label} JSON 顶层必须是对象。")
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="组合目标差额计算器")
    parser.add_argument("--current", required=True, help="当前配置 JSON 文件")
    parser.add_argument("--target", required=True, help="用户审阅的目标配置 JSON 文件")
    parser.add_argument("--value", type=float, required=True, help="组合总价值")
    parser.add_argument("--threshold", type=float, required=True, help="纳入计算的最小偏离百分点")
    parser.add_argument("--transaction-cost-rate", type=float, required=True, help="卖出交易成本率（%）")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        plan = calculate_rebalance_plan(
            _load_json(args.current, "current"),
            _load_json(args.target, "target"),
            args.value,
            args.threshold,
            args.transaction_cost_rate,
        )
        output = json.dumps(plan, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"差额计算已保存到: {output_path}")
        else:
            print(output)
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        payload = json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False)
        print(payload if args.json else f"再平衡差额计算失败：{exc}", file=sys.stdout if args.json else sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
