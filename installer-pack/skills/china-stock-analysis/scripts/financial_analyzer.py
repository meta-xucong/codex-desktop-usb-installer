#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Summarize attributed A-share financial fields without ratings or rankings."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_INPUT_BYTES = 20 * 1024 * 1024
METRIC_GROUPS = {
    "profitability": {
        "roe_percent": ("净资产收益率", "加权净资产收益率"),
        "roa_percent": ("总资产报酬率",),
        "gross_margin_percent": ("销售毛利率",),
        "net_margin_percent": ("销售净利率",),
    },
    "solvency": {
        "debt_ratio_percent": ("资产负债率",),
        "current_ratio": ("流动比率",),
        "quick_ratio": ("速动比率",),
        "equity_multiplier": ("权益乘数",),
    },
    "operations": {
        "receivables_turnover": ("应收账款周转率",),
        "receivables_days": ("应收账款周转天数",),
        "inventory_turnover": ("存货周转率",),
        "inventory_days": ("存货周转天数",),
        "asset_turnover": ("总资产周转率",),
    },
    "growth": {
        "revenue_growth_percent": ("主营业务收入增长率", "营业收入增长率"),
        "net_profit_growth_percent": ("净利润增长率",),
        "receivables_growth_percent": ("应收账款增长率",),
        "inventory_growth_percent": ("存货增长率",),
    },
}
PERIOD_FIELDS = ("日期", "报告期", "报告日", "REPORT_DATE", "截止日期")


def _safe_float(value: Any) -> float | None:
    if value is None or value == "" or value == "--" or isinstance(value, bool):
        return None
    if isinstance(value, str):
        value = value.replace("%", "").replace(",", "").strip()
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    return parsed if math.isfinite(parsed) else None


def _first_number(row: dict[str, Any], aliases: tuple[str, ...]) -> float | None:
    for alias in aliases:
        value = _safe_float(row.get(alias))
        if value is not None:
            return value
    return None


def _period(row: dict[str, Any], index: int) -> str:
    for field in PERIOD_FIELDS:
        value = str(row.get(field) or "").strip()
        if value:
            return value
    return f"record-{index + 1}"


def _indicator_records(stock_data: dict[str, Any]) -> list[dict[str, Any]]:
    raw = stock_data.get("financial_indicators")
    if isinstance(raw, dict):
        raw = raw.get("records")
    if not isinstance(raw, list) or not raw or not all(isinstance(item, dict) for item in raw):
        raise ValueError("financial_indicators 必须包含至少一条指标记录。")
    return raw


def _validate_provenance(stock_data: dict[str, Any]) -> tuple[str, list[str]]:
    retrieved_at = str(stock_data.get("retrieved_at_utc") or stock_data.get("data_as_of") or "").strip()
    if not retrieved_at:
        raise ValueError("输入缺少 retrieved_at_utc/data_as_of。")
    sources = stock_data.get("sources")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("输入 sources 必须至少包含一个非空来源。")
    completeness = stock_data.get("completeness")
    if isinstance(completeness, dict) and completeness.get("status") == "failed":
        raise ValueError("输入数据完整性状态为 failed。")
    return retrieved_at, [item.strip() for item in sources]


def _series(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    output: list[dict[str, Any]] = []
    for index, row in enumerate(records):
        metrics: dict[str, float | None] = {}
        for group in METRIC_GROUPS.values():
            for name, aliases in group.items():
                metrics[name] = _first_number(row, aliases)
        output.append({"period": _period(row, index), "metrics": metrics})
    return output


def _latest_groups(series: list[dict[str, Any]]) -> dict[str, Any]:
    latest = series[0]
    groups: dict[str, Any] = {"period": latest["period"]}
    for group_name, definitions in METRIC_GROUPS.items():
        groups[group_name] = {name: latest["metrics"].get(name) for name in definitions}
    return groups


def _period_changes(series: list[dict[str, Any]]) -> dict[str, Any]:
    if len(series) < 2:
        return {"available": False, "reason": "至少需要两条指标记录。"}
    newest = series[0]
    previous = series[1]
    changes: dict[str, float | None] = {}
    for metric, current in newest["metrics"].items():
        prior = previous["metrics"].get(metric)
        changes[metric] = round(current - prior, 6) if current is not None and prior is not None else None
    return {
        "available": True,
        "newest_period": newest["period"],
        "comparison_period": previous["period"],
        "newest_minus_comparison": changes,
        "notice": "百分比指标为百分点差，其余指标为原单位差；记录顺序沿用输入。",
    }


def _find_statement_number(rows: list[dict[str, Any]], aliases: tuple[str, ...]) -> float | None:
    if not rows:
        return None
    return _first_number(rows[0], aliases)


def _diagnostics(stock_data: dict[str, Any], series: list[dict[str, Any]]) -> dict[str, Any]:
    latest = series[0]["metrics"]
    previous = series[1]["metrics"] if len(series) > 1 else {}
    revenue_growth = latest.get("revenue_growth_percent")
    receivables_growth = latest.get("receivables_growth_percent")
    inventory_growth = latest.get("inventory_growth_percent")
    current_margin = latest.get("gross_margin_percent")
    previous_margin = previous.get("gross_margin_percent")

    financial_data = stock_data.get("financial_data")
    if not isinstance(financial_data, dict):
        financial_data = {}
    cash_flow = financial_data.get("cash_flow") if isinstance(financial_data.get("cash_flow"), list) else []
    income = financial_data.get("income_statement") if isinstance(financial_data.get("income_statement"), list) else []
    operating_cash_flow = _find_statement_number(
        cash_flow,
        ("经营活动产生的现金流量净额", "经营活动现金流量净额"),
    )
    net_profit = _find_statement_number(income, ("净利润", "归属于母公司股东的净利润"))

    def difference(left: float | None, right: float | None) -> float | None:
        return round(left - right, 6) if left is not None and right is not None else None

    return {
        "receivables_growth_minus_revenue_growth_percentage_points": difference(receivables_growth, revenue_growth),
        "inventory_growth_minus_revenue_growth_percentage_points": difference(inventory_growth, revenue_growth),
        "gross_margin_change_percentage_points": difference(current_margin, previous_margin),
        "operating_cash_flow_to_net_profit": (
            round(operating_cash_flow / net_profit, 6)
            if operating_cash_flow is not None and net_profit not in (None, 0)
            else None
        ),
        "notice": "这些是机械差值/比值；脚本不使用固定阈值将其标记为异常或风险等级。",
    }


def _dupont(latest: dict[str, Any]) -> dict[str, Any]:
    net_margin = latest["metrics"].get("net_margin_percent")
    asset_turnover = latest["metrics"].get("asset_turnover")
    equity_multiplier = latest["metrics"].get("equity_multiplier")
    calculated = (
        net_margin * asset_turnover * equity_multiplier
        if net_margin is not None and asset_turnover is not None and equity_multiplier is not None
        else None
    )
    result = {
        "reported_roe_percent": latest["metrics"].get("roe_percent"),
        "net_margin_percent": net_margin,
        "asset_turnover": asset_turnover,
        "equity_multiplier": equity_multiplier,
        "calculated_roe_percent": round(calculated, 6) if calculated is not None else None,
        "notice": "杜邦乘积仅在三个分量均存在时计算，不对驱动因素评级。",
    }
    return result


def analyze_stock(stock_data: dict[str, Any], level: str = "standard") -> dict[str, Any]:
    if level not in {"summary", "standard", "deep"}:
        raise ValueError("level 只接受 summary/standard/deep。")
    if not isinstance(stock_data, dict):
        raise ValueError("股票输入必须是 JSON 对象。")
    retrieved_at, sources = _validate_provenance(stock_data)
    records = _indicator_records(stock_data)
    series = _series(records)
    basic = stock_data.get("basic_info") if isinstance(stock_data.get("basic_info"), dict) else {}
    result: dict[str, Any] = {
        "code": str(stock_data.get("code") or ""),
        "name": str(basic.get("name") or ""),
        "analyzed_at_utc": datetime.now(timezone.utc).isoformat(),
        "input_retrieved_at": retrieved_at,
        "sources": sources,
        "input_completeness": stock_data.get("completeness"),
        "level": level,
        "latest": _latest_groups(series),
        "methodology": (
            "仅整理输入字段并计算相邻记录差值、可复算比值；不内置行业阈值、综合评分、"
            "风险评级、股票排名、目标价或投资建议。记录顺序和财务口径必须由调用者核验。"
        ),
    }
    if level in {"standard", "deep"}:
        result["period_changes"] = _period_changes(series)
        result["diagnostics"] = _diagnostics(stock_data, series)
        result["dupont"] = _dupont(series[0])
    if level == "deep":
        result["indicator_series"] = series
        result["raw_financial_data"] = stock_data.get("financial_data")
    return result


def compare_stocks(stocks_data: list[dict[str, Any]], level: str = "summary") -> dict[str, Any]:
    if not isinstance(stocks_data, list) or not stocks_data:
        raise ValueError("comparison 模式要求非空 stocks 数组。")
    rows = [analyze_stock(stock, level=level) for stock in stocks_data]
    return {
        "compared_at_utc": datetime.now(timezone.utc).isoformat(),
        "stocks": rows,
        "methodology": "并列展示同一字段，不计算综合分或排名；调用者需核验统计期、币种、单位与会计口径可比。",
    }


def _load_input(path_value: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError("输入 JSON 不存在或超过 20 MiB。")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("输入 JSON 顶层必须是对象。")
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A 股财务字段与机械诊断分析器")
    parser.add_argument("--input", required=True, help="带来源和抓取时点的输入 JSON")
    parser.add_argument("--level", choices=["summary", "standard", "deep"], default="standard")
    parser.add_argument("--mode", choices=["single", "comparison"], default="single")
    parser.add_argument("--output", help="输出 JSON 文件")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        data = _load_input(args.input)
        if args.mode == "single":
            result = analyze_stock(data, level=args.level)
        else:
            stocks = data.get("stocks")
            if not isinstance(stocks, list):
                raise ValueError("comparison 模式要求输入顶层包含 stocks 数组。")
            result = compare_stocks(stocks, level=args.level)
        output = json.dumps(result, ensure_ascii=False, indent=2, default=str)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"分析结果已保存到：{output_path}")
        else:
            print(output)
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
