#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Explicit-assumption venture valuation scenario calculator."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_ASSUMPTIONS_BYTES = 1024 * 1024
SCORE_KEYS = ("team", "product", "market", "competition", "timing")


def _number(value: Any, name: str, minimum=None, maximum=None) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} 必须是显式提供的数字。")
    normalized = float(value)
    if not math.isfinite(normalized):
        raise ValueError(f"{name} 必须是有限数字。")
    if minimum is not None and normalized < minimum:
        raise ValueError(f"{name} 不得小于 {minimum}。")
    if maximum is not None and normalized > maximum:
        raise ValueError(f"{name} 不得大于 {maximum}。")
    return normalized


def _range(value: Any, name: str, minimum: float = 0) -> tuple[float, float]:
    if not isinstance(value, list) or len(value) != 2:
        raise ValueError(f"{name} 必须是 [low, high]。")
    low = _number(value[0], f"{name}[0]", minimum)
    high = _number(value[1], f"{name}[1]", minimum)
    if low > high:
        raise ValueError(f"{name} 的 low 不得大于 high。")
    return low, high


def validate_valuation_assumptions(data: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise ValueError("估值假设 JSON 顶层必须是对象。")
    as_of = str(data.get("as_of") or "").strip()
    sources = data.get("sources")
    if not as_of:
        raise ValueError("估值假设缺少 as_of。")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("估值假设 sources 必须至少包含一个非空来源。")
    weights_raw = data.get("scorecard_weights")
    if not isinstance(weights_raw, dict) or set(weights_raw) != set(SCORE_KEYS):
        raise ValueError(f"scorecard_weights 必须且只能包含 {list(SCORE_KEYS)}。")
    weights = {key: _number(weights_raw[key], f"scorecard_weights.{key}", 0, 1) for key in SCORE_KEYS}
    if not math.isclose(sum(weights.values()), 1.0, abs_tol=0.000001):
        raise ValueError("scorecard_weights 合计必须为 1。")
    return {
        "as_of": as_of,
        "sources": [item.strip() for item in sources],
        "comparable_valuation_range": _range(
            data.get("comparable_valuation_range"), "comparable_valuation_range"
        ),
        "revenue_multiple_range": _range(data.get("revenue_multiple_range"), "revenue_multiple_range"),
        "scorecard_base_valuation": _number(
            data.get("scorecard_base_valuation"), "scorecard_base_valuation", 0
        ),
        "scorecard_weights": weights,
        "scorecard_factor_range": _range(data.get("scorecard_factor_range"), "scorecard_factor_range"),
        "range_factor": _number(data.get("range_factor"), "range_factor", 0, 1),
    }


def comparable_company_scenario(
    industry: str, stage: str, revenue: float, assumptions: dict[str, Any]
) -> dict[str, Any]:
    comparable_low, comparable_high = assumptions["comparable_valuation_range"]
    multiple_low, multiple_high = assumptions["revenue_multiple_range"]
    revenue_range = (revenue * multiple_low / 10_000, revenue * multiple_high / 10_000)
    return {
        "method": "comparable_scenario",
        "unit": "万元",
        "industry_label": industry,
        "stage_label": stage,
        "provided_comparable_range": [round(comparable_low, 4), round(comparable_high, 4)],
        "revenue_multiple_implied_range": [round(revenue_range[0], 4), round(revenue_range[1], 4)],
        "range_envelope": [
            round(min(comparable_low, revenue_range[0]), 4),
            round(max(comparable_high, revenue_range[1]), 4),
        ],
        "notice": "并列展示可比区间与收入倍数区间；脚本不判断哪个更可靠。",
    }


def venture_capital_scenario(
    revenue: float,
    growth_rate: float,
    target_return: float,
    exit_multiple: float,
    years: int,
    range_factor: float,
) -> dict[str, Any]:
    future_revenue = revenue * pow(1 + growth_rate / 100, years)
    exit_valuation = future_revenue * exit_multiple
    present_value = exit_valuation / pow(1 + target_return / 100, years)
    return {
        "method": "venture_capital_scenario",
        "unit": "万元",
        "future_revenue": round(future_revenue, 2),
        "exit_valuation": round(exit_valuation / 10_000, 4),
        "present_value": round(present_value / 10_000, 4),
        "range": [
            round(present_value / 10_000 * (1 - range_factor), 4),
            round(present_value / 10_000 * (1 + range_factor), 4),
        ],
        "assumptions": {
            "annual_growth_rate": growth_rate,
            "target_annual_return": target_return,
            "exit_multiple": exit_multiple,
            "years": years,
            "range_factor": range_factor,
        },
    }


def scorecard_scenario(scores: dict[str, int], assumptions: dict[str, Any]) -> dict[str, Any]:
    weighted_score = sum(scores[key] * assumptions["scorecard_weights"][key] for key in SCORE_KEYS)
    factor_low, factor_high = assumptions["scorecard_factor_range"]
    factor = factor_low + weighted_score / 10 * (factor_high - factor_low)
    value = assumptions["scorecard_base_valuation"] * factor
    range_factor = assumptions["range_factor"]
    return {
        "method": "scorecard_scenario",
        "unit": "万元",
        "scores": scores,
        "weights": assumptions["scorecard_weights"],
        "weighted_score": round(weighted_score, 4),
        "factor_range": [factor_low, factor_high],
        "mapped_factor": round(factor, 6),
        "base_valuation": assumptions["scorecard_base_valuation"],
        "value": round(value, 4),
        "range": [round(value * (1 - range_factor), 4), round(value * (1 + range_factor), 4)],
    }


def generate_valuation_report(args, assumptions: dict[str, Any]) -> dict[str, Any]:
    assumptions = validate_valuation_assumptions(assumptions)
    scores = {}
    for name in SCORE_KEYS:
        score = getattr(args, f"{name}_score")
        if not isinstance(score, int) or not 0 <= score <= 10:
            raise ValueError(f"{name}_score 必须是 0–10 的整数。")
        scores[name] = score
    revenue = _number(args.revenue, "revenue", 0)
    growth_rate = _number(args.growth_rate, "growth_rate", -99.999, 10_000)
    target_return = _number(args.target_return, "target_return", 0.000001, 10_000)
    exit_multiple = _number(args.exit_multiple, "exit_multiple", 0.000001, 10_000)
    if not isinstance(args.years, int) or not 1 <= args.years <= 100:
        raise ValueError("years 必须是 1–100 的整数。")
    stage = str(args.stage or "").strip()
    industry = str(args.industry or "").strip()
    if not stage or not industry:
        raise ValueError("stage 和 industry 必须是非空文本。")

    methods = [
        comparable_company_scenario(industry, stage, revenue, assumptions),
        venture_capital_scenario(
            revenue, growth_rate, target_return, exit_multiple, args.years, assumptions["range_factor"]
        ),
        scorecard_scenario(scores, assumptions),
    ]
    ranges = []
    for method in methods:
        range_value = method.get("range") or method.get("range_envelope")
        ranges.append(range_value)
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "assumptions_as_of": assumptions["as_of"],
        "sources": assumptions["sources"],
        "project_stage": stage,
        "industry": industry,
        "methods": methods,
        "method_range_envelope": [
            round(min(item[0] for item in ranges), 4),
            round(max(item[1] for item in ranges), 4),
        ],
        "methodology": (
            "所有区间、倍数、Scorecard 基准/权重/映射因子和退出假设均显式提供。"
            "脚本只展示各方法及总包络，不平均方法、不命名公允值、不生成谈判或投资建议。"
        ),
        "limitations": [
            "各方法可能使用重叠证据，不能视为独立估计。",
            "收入增长、退出倍数、目标回报和 Scorecard 分数变化会显著改变结果。",
            "未建模融资稀释、优先权、债务、税或交易条款。",
        ],
    }


def format_report(report: dict[str, Any]) -> str:
    lines = [
        "创业估值场景",
        f"假设时点：{report['assumptions_as_of']}",
        "来源：" + "；".join(report["sources"]),
        f"方法总包络（万元）：{report['method_range_envelope'][0]}–{report['method_range_envelope'][1]}",
        "各方法：",
    ]
    lines.extend(f"- {item['method']}：{json.dumps(item, ensure_ascii=False, sort_keys=True)}" for item in report["methods"])
    lines.extend(["方法边界：" + report["methodology"], *["限制：" + item for item in report["limitations"]]])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="显式假设的创业估值场景计算器")
    parser.add_argument("--assumptions", required=True, help="带时点与来源的估值假设 JSON")
    parser.add_argument("--stage", required=True, help="融资阶段标签")
    parser.add_argument("--industry", required=True, help="行业标签")
    parser.add_argument("--revenue", type=float, required=True, help="年收入（元）")
    parser.add_argument("--growth-rate", type=float, required=True, help="年增长率场景（%）")
    for key in SCORE_KEYS:
        parser.add_argument(f"--{key}-score", type=int, required=True, help=f"{key} 用户审阅评分（0–10）")
    parser.add_argument("--target-return", type=float, required=True, help="目标年化回报率场景（%）")
    parser.add_argument("--exit-multiple", type=float, required=True, help="退出收入倍数场景")
    parser.add_argument("--years", type=int, required=True, help="退出年限场景")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        path = Path(args.assumptions).expanduser().resolve()
        if not path.is_file() or path.stat().st_size > MAX_ASSUMPTIONS_BYTES:
            raise ValueError("估值假设文件不存在或超过 1 MiB。")
        report = generate_valuation_report(args, json.loads(path.read_text(encoding="utf-8")))
        output = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"估值场景已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_report(report))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"估值场景分析失败：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
