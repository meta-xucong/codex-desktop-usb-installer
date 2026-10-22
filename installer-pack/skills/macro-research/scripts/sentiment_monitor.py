#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Attributed market-sentiment arithmetic with explicit scoring weights."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


INDICATOR_KEYS = (
    "market_momentum",
    "stock_price_strength",
    "stock_price_breadth",
    "put_call_ratio",
    "market_volatility",
    "safe_haven_demand",
)
MAX_INPUT_BYTES = 1024 * 1024


def _required_number(data: dict[str, Any], key: str, section: str, minimum=None, maximum=None) -> float:
    if key not in data or isinstance(data[key], bool) or not isinstance(data[key], (int, float)):
        raise ValueError(f"{section}.{key} 必须是显式提供的数字。")
    value = float(data[key])
    if not math.isfinite(value):
        raise ValueError(f"{section}.{key} 必须是有限数字。")
    if minimum is not None and value < minimum:
        raise ValueError(f"{section}.{key} 不得小于 {minimum}。")
    if maximum is not None and value > maximum:
        raise ValueError(f"{section}.{key} 不得大于 {maximum}。")
    return value


def validate_sentiment_data(data: dict[str, Any]) -> dict[str, Any]:
    """Require complete real inputs and a reviewed weighting policy."""
    if not isinstance(data, dict):
        raise ValueError("输入 JSON 顶层必须是对象。")
    as_of = str(data.get("as_of") or "").strip()
    if not as_of:
        raise ValueError("缺少 data.as_of 数据时点。")
    sources = data.get("sources")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("data.sources 必须至少包含一个非空来源说明或 URL。")

    sections = {}
    for section in ("indicators", "weights", "breadth", "volume", "fund_flow"):
        value = data.get(section)
        if not isinstance(value, dict):
            raise ValueError(f"缺少完整的 data.{section} 对象。")
        sections[section] = value
    if set(sections["indicators"]) != set(INDICATOR_KEYS):
        raise ValueError(f"indicators 必须且只能包含 {list(INDICATOR_KEYS)}。")
    if set(sections["weights"]) != set(INDICATOR_KEYS):
        raise ValueError(f"weights 必须且只能包含 {list(INDICATOR_KEYS)}。")

    indicators = {
        key: _required_number(sections["indicators"], key, "indicators", 0, 100)
        for key in INDICATOR_KEYS
    }
    weights = {
        key: _required_number(sections["weights"], key, "weights", 0, 1)
        for key in INDICATOR_KEYS
    }
    if not math.isclose(sum(weights.values()), 1.0, abs_tol=0.000001):
        raise ValueError("weights 必须合计 1。")

    advancing = _required_number(sections["breadth"], "advancing_stocks", "breadth", 0)
    declining = _required_number(sections["breadth"], "declining_stocks", "breadth", 0)
    if not advancing.is_integer() or not declining.is_integer():
        raise ValueError("breadth 的上涨/下跌家数必须是整数。")
    if advancing + declining == 0:
        raise ValueError("breadth 上涨和下跌家数不能同时为 0。")
    volume = {
        "current_volume": _required_number(sections["volume"], "current_volume", "volume", 0),
        "avg_volume": _required_number(sections["volume"], "avg_volume", "volume", 0),
        "price_change": _required_number(sections["volume"], "price_change", "volume"),
    }
    if volume["avg_volume"] == 0:
        raise ValueError("volume.avg_volume 必须大于 0。")
    fund_flow = {
        key: _required_number(sections["fund_flow"], key, "fund_flow")
        for key in ("northbound_flow", "main_force_flow", "retail_flow")
    }
    return {
        "as_of": as_of,
        "sources": [item.strip() for item in sources],
        "indicators": indicators,
        "weights": weights,
        "breadth": {"advancing_stocks": int(advancing), "declining_stocks": int(declining)},
        "volume": volume,
        "fund_flow": fund_flow,
    }


def calculate_weighted_index(indicators: dict[str, float], weights: dict[str, float]) -> dict[str, Any]:
    contributions = {key: indicators[key] * weights[key] for key in INDICATOR_KEYS}
    return {
        "value": round(sum(contributions.values()), 4),
        "scale": [0, 100],
        "components": indicators,
        "weights": weights,
        "contributions": {key: round(value, 4) for key, value in contributions.items()},
        "notice": "指标归一化方法与权重均由调用者负责；脚本不把分数映射为贪婪/恐惧或买卖动作。",
    }


def analyze_market_breadth(data: dict[str, float]) -> dict[str, Any]:
    advancing = data["advancing_stocks"]
    declining = data["declining_stocks"]
    total = advancing + declining
    return {
        "advancing_stocks": advancing,
        "declining_stocks": declining,
        "advance_decline_ratio": None if declining == 0 else round(advancing / declining, 4),
        "advancing_share_percent": round(advancing / total * 100, 4),
    }


def analyze_volume(data: dict[str, float]) -> dict[str, Any]:
    return {
        "current_volume": data["current_volume"],
        "average_volume": data["avg_volume"],
        "volume_ratio": round(data["current_volume"] / data["avg_volume"], 4),
        "price_change": data["price_change"],
    }


def analyze_fund_flow(data: dict[str, float]) -> dict[str, Any]:
    total = data["northbound_flow"] + data["main_force_flow"] + data["retail_flow"]
    return {**data, "total_flow": round(total, 4)}


def generate_sentiment_report(index_name: str, data: dict[str, Any]) -> dict[str, Any]:
    validated = validate_sentiment_data(data)
    index_name = str(index_name or "").strip()
    if not index_name:
        raise ValueError("index_name 必须是非空文本。")
    return {
        "calculated_at": datetime.now(timezone.utc).isoformat(),
        "data_as_of": validated["as_of"],
        "sources": validated["sources"],
        "index": index_name,
        "weighted_index": calculate_weighted_index(validated["indicators"], validated["weights"]),
        "market_breadth": analyze_market_breadth(validated["breadth"]),
        "volume": analyze_volume(validated["volume"]),
        "fund_flow": analyze_fund_flow(validated["fund_flow"]),
        "methodology": "只计算显式权重的合成值及广度、量比、资金流算术；不生成仓位、止损或交易建议。",
    }


def format_report(report: dict[str, Any]) -> str:
    breadth = report["market_breadth"]
    volume = report["volume"]
    flow = report["fund_flow"]
    return "\n".join(
        [
            f"市场情绪输入汇总：{report['index']}",
            f"数据时点：{report['data_as_of']}",
            "来源：" + "；".join(report["sources"]),
            f"显式权重合成值：{report['weighted_index']['value']}/100",
            f"上涨占比：{breadth['advancing_share_percent']}%",
            f"涨跌家数比：{breadth['advance_decline_ratio']}",
            f"量比：{volume['volume_ratio']}；价格变化：{volume['price_change']}",
            f"三类资金流合计：{flow['total_flow']}",
            "方法边界：" + report["methodology"],
        ]
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="显式权重的市场情绪算术汇总器")
    parser.add_argument("--index", required=True, help="市场或指数标签")
    parser.add_argument("--input", required=True, help="完整、带来源/时点/权重的 JSON")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        path = Path(args.input).expanduser().resolve()
        if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
            raise ValueError("输入 JSON 不存在或超过 1 MiB。")
        report = generate_sentiment_report(args.index, json.loads(path.read_text(encoding="utf-8")))
        output = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"报告已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_report(report))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"情绪汇总失败：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
