#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Transparent fund-portfolio risk scenario calculator."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_INPUT_BYTES = 1024 * 1024
ASSET_CLASSES = {"equity", "bond", "cash", "other"}


def _required_number(data: dict[str, Any], key: str, minimum=None, maximum=None) -> float:
    if key not in data or isinstance(data[key], bool) or not isinstance(data[key], (int, float)):
        raise ValueError(f"{key} 必须是显式提供的数字。")
    value = float(data[key])
    if not math.isfinite(value):
        raise ValueError(f"{key} 必须是有限数字。")
    if minimum is not None and value < minimum:
        raise ValueError(f"{key} 不得小于 {minimum}。")
    if maximum is not None and value > maximum:
        raise ValueError(f"{key} 不得大于 {maximum}。")
    return value


def validate_portfolio_data(data: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise ValueError("组合 JSON 顶层必须是对象。")
    as_of = str(data.get("as_of") or "").strip()
    sources = data.get("sources")
    if not as_of:
        raise ValueError("缺少 as_of 数据时点。")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("sources 必须至少包含一个非空来源说明或 URL。")

    allocations_raw = data.get("allocations")
    volatilities_raw = data.get("annual_volatility")
    classes_raw = data.get("asset_classes")
    if not isinstance(allocations_raw, dict) or not allocations_raw:
        raise ValueError("allocations 必须是非空的基金代码到百分比映射。")
    if not isinstance(volatilities_raw, dict):
        raise ValueError("annual_volatility 必须包含每只基金的年化波动率。")
    if not isinstance(classes_raw, dict):
        raise ValueError("asset_classes 必须包含每只基金的资产类别。")

    allocations: dict[str, float] = {}
    volatilities: dict[str, float] = {}
    asset_classes: dict[str, str] = {}
    for raw_code, raw_weight in allocations_raw.items():
        code = str(raw_code).strip()
        if not code:
            raise ValueError("allocations 包含空基金代码。")
        allocations[code] = _required_number({"weight": raw_weight}, "weight", 0.000001, 100)
        if code not in volatilities_raw:
            raise ValueError(f"annual_volatility 缺少基金 {code}。")
        volatilities[code] = _required_number(
            {"volatility": volatilities_raw[code]}, "volatility", 0, 1000
        )
        asset_class = str(classes_raw.get(code) or "").strip().casefold()
        if asset_class not in ASSET_CLASSES:
            raise ValueError(f"asset_classes.{code} 必须是 {sorted(ASSET_CLASSES)} 之一。")
        asset_classes[code] = asset_class
    if set(volatilities_raw) != set(allocations) or set(classes_raw) != set(allocations):
        raise ValueError("annual_volatility、asset_classes 与 allocations 的基金代码必须完全一致。")
    if not math.isclose(sum(allocations.values()), 100.0, abs_tol=0.01):
        raise ValueError("allocations 百分比合计必须为 100。")

    correlation = _required_number(data, "correlation", -1, 1)
    if len(allocations) > 1 and correlation < -1 / (len(allocations) - 1):
        raise ValueError("统一相关系数会产生无效相关矩阵，请提高 correlation。")
    return {
        "as_of": as_of,
        "sources": [item.strip() for item in sources],
        "allocations": allocations,
        "annual_volatility": volatilities,
        "asset_classes": asset_classes,
        "correlation": correlation,
        "expected_return": _required_number(data, "expected_return", -100, 1000),
        "risk_free_rate": _required_number(data, "risk_free_rate", -100, 1000),
        "drawdown_multiplier": _required_number(data, "drawdown_multiplier", 0.000001, 10),
    }


def calculate_portfolio_volatility(
    allocations: dict[str, float], annual_volatility: dict[str, float], correlation: float
) -> float:
    codes = list(allocations)
    weights = [allocations[code] / 100 for code in codes]
    volatilities = [annual_volatility[code] for code in codes]
    variance = 0.0
    for i in range(len(codes)):
        for j in range(len(codes)):
            coefficient = 1.0 if i == j else correlation
            variance += coefficient * weights[i] * weights[j] * volatilities[i] * volatilities[j]
    return math.sqrt(max(variance, 0))


def calculate_allocation_structure(
    allocations: dict[str, float], asset_classes: dict[str, str]
) -> dict[str, float]:
    totals = {asset_class: 0.0 for asset_class in ASSET_CLASSES}
    for code, weight in allocations.items():
        totals[asset_classes[code]] += weight
    return totals


def assess_risk(portfolio_data: dict[str, Any]) -> dict[str, Any]:
    validated = validate_portfolio_data(portfolio_data)
    allocations = validated["allocations"]
    volatility = calculate_portfolio_volatility(
        allocations, validated["annual_volatility"], validated["correlation"]
    )
    scenario_drawdown = volatility * validated["drawdown_multiplier"]
    sharpe = None if volatility == 0 else (
        validated["expected_return"] - validated["risk_free_rate"]
    ) / volatility
    structure = calculate_allocation_structure(allocations, validated["asset_classes"])
    concentration_hhi = sum((weight / 100) ** 2 for weight in allocations.values())
    return {
        "assessed_at": datetime.now(timezone.utc).isoformat(),
        "data_as_of": validated["as_of"],
        "sources": validated["sources"],
        "assumptions": {
            "correlation": validated["correlation"],
            "drawdown_multiplier": validated["drawdown_multiplier"],
            "expected_return": validated["expected_return"],
            "risk_free_rate": validated["risk_free_rate"],
        },
        "metrics": {
            "annual_volatility": round(volatility, 2),
            "scenario_drawdown": round(scenario_drawdown, 2),
            "sharpe_ratio": None if sharpe is None else round(sharpe, 4),
            "concentration_hhi": round(concentration_hhi, 6),
            "largest_position_percent": round(max(allocations.values()), 4),
            "stock_allocation": round(structure["equity"], 4),
            "bond_allocation": round(structure["bond"], 4),
            "cash_allocation": round(structure["cash"], 4),
            "other_allocation": round(structure["other"], 4),
        },
        "methodology": (
            "组合波动率使用调用者给出的统一相关系数；场景回撤 = 波动率 × 显式倍数；"
            "Sharpe 使用显式预期收益和无风险利率。脚本不内置风险等级、适配人群、仓位或止损建议。"
        ),
        "limitations": [
            "统一相关系数不是完整相关矩阵，无法表达基金对之间的差异。",
            "场景回撤不是历史最大回撤或概率预测。",
            "预期收益、波动率和相关性变化会显著改变结果。",
        ],
    }


def format_report(assessment: dict[str, Any]) -> str:
    metrics = assessment["metrics"]
    return "\n".join(
        [
            "基金组合风险场景",
            f"数据时点：{assessment['data_as_of']}",
            "来源：" + "；".join(assessment["sources"]),
            f"年化波动率场景：{metrics['annual_volatility']}%",
            f"回撤场景：{metrics['scenario_drawdown']}%",
            f"Sharpe 场景：{metrics['sharpe_ratio']}",
            f"最大单一仓位：{metrics['largest_position_percent']}%",
            f"集中度 HHI：{metrics['concentration_hhi']}",
            "方法边界：" + assessment["methodology"],
            *["限制：" + item for item in assessment["limitations"]],
        ]
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="基金组合透明风险场景计算器")
    parser.add_argument("--portfolio", required=True, help="完整组合与场景假设 JSON")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        path = Path(args.portfolio).expanduser().resolve()
        if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
            raise ValueError("组合 JSON 不存在或超过 1 MiB。")
        assessment = assess_risk(json.loads(path.read_text(encoding="utf-8")))
        output = json.dumps(assessment, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"评估场景已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_report(assessment))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"风险场景计算失败：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
