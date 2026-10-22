#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""A-share valuation scenario calculator with explicit assumptions.

No discount rate, growth rate, dividend, free cash flow, safety margin, or
"fair" multiple is inferred by this script. It performs arithmetic only on
attributed data and a separately reviewed assumptions file.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_DATA_BYTES = 10 * 1024 * 1024
MAX_ASSUMPTION_BYTES = 1024 * 1024
METHODS = {"dcf", "ddm", "relative"}


def _text(value: Any, field: str) -> str:
    normalized = str(value or "").strip()
    if not normalized:
        raise ValueError(f"{field} 必须是非空文本。")
    return normalized


def _number(value: Any, field: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{field} 必须是有限数值。")
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{field} 必须是有限数值。") from exc
    if not math.isfinite(number):
        raise ValueError(f"{field} 必须是有限数值。")
    return number


def _optional_number(value: Any, field: str) -> float | None:
    if value in (None, "", "--"):
        return None
    return _number(value, field)


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} 必须是数组。")
    result = [_text(item, f"{field}[]") for item in value]
    if not result:
        raise ValueError(f"{field} 至少包含一项。")
    return result


def load_json(path_value: str, max_bytes: int, label: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > max_bytes:
        raise ValueError(f"{label}不存在或超过大小限制。")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{label}顶层必须是对象。")
    return value


def parse_methods(value: str) -> list[str]:
    normalized = _text(value, "methods").lower()
    if normalized == "all":
        return ["dcf", "ddm", "relative"]
    methods = [item.strip() for item in normalized.split(",") if item.strip()]
    if not methods or len(methods) != len(set(methods)) or any(item not in METHODS for item in methods):
        raise ValueError("methods 只接受 dcf、ddm、relative 的不重复逗号列表，或 all。")
    return methods


def validate_valuation_assumptions(raw: dict[str, Any], methods: list[str]) -> dict[str, Any]:
    result: dict[str, Any] = {
        "as_of": _text(raw.get("as_of"), "assumptions.as_of"),
        "sources": _string_list(raw.get("sources"), "assumptions.sources"),
    }

    if "dcf" in methods:
        dcf_raw = raw.get("dcf")
        if not isinstance(dcf_raw, dict):
            raise ValueError("选择 dcf 时 assumptions.dcf 必须是对象。")
        forecast_years_value = _number(dcf_raw.get("forecast_years"), "dcf.forecast_years")
        forecast_years = int(forecast_years_value)
        if forecast_years_value != forecast_years or not 1 <= forecast_years <= 30:
            raise ValueError("dcf.forecast_years 必须是 1–30 的整数。")
        discount_rate = _number(dcf_raw.get("discount_rate"), "dcf.discount_rate")
        terminal_growth = _number(dcf_raw.get("terminal_growth"), "dcf.terminal_growth")
        if discount_rate <= terminal_growth:
            raise ValueError("dcf.discount_rate 必须大于 dcf.terminal_growth。")
        total_shares = _number(dcf_raw.get("total_shares"), "dcf.total_shares")
        if total_shares <= 0:
            raise ValueError("dcf.total_shares 必须大于 0。")
        result["dcf"] = {
            "base_free_cash_flow": _number(dcf_raw.get("base_free_cash_flow"), "dcf.base_free_cash_flow"),
            "cash_flow_growth": _number(dcf_raw.get("cash_flow_growth"), "dcf.cash_flow_growth"),
            "discount_rate": discount_rate,
            "terminal_growth": terminal_growth,
            "forecast_years": forecast_years,
            "total_shares": total_shares,
            "basis": _text(dcf_raw.get("basis"), "dcf.basis"),
        }

    if "ddm" in methods:
        ddm_raw = raw.get("ddm")
        if not isinstance(ddm_raw, dict):
            raise ValueError("选择 ddm 时 assumptions.ddm 必须是对象。")
        required_return = _number(ddm_raw.get("required_return"), "ddm.required_return")
        dividend_growth = _number(ddm_raw.get("dividend_growth"), "ddm.dividend_growth")
        if required_return <= dividend_growth:
            raise ValueError("ddm.required_return 必须大于 ddm.dividend_growth。")
        current_dividend = _number(
            ddm_raw.get("current_dividend_per_share"), "ddm.current_dividend_per_share"
        )
        if current_dividend < 0:
            raise ValueError("ddm.current_dividend_per_share 不允许为负。")
        result["ddm"] = {
            "current_dividend_per_share": current_dividend,
            "dividend_growth": dividend_growth,
            "required_return": required_return,
            "basis": _text(ddm_raw.get("basis"), "ddm.basis"),
        }

    if raw.get("margin_of_safety") is not None:
        margin = _number(raw["margin_of_safety"], "margin_of_safety")
        if not 0 <= margin < 100:
            raise ValueError("margin_of_safety 必须在 [0, 100) 范围内。")
        result["margin_of_safety"] = margin
    return result


def dcf_valuation(parameters: dict[str, Any]) -> dict[str, Any]:
    base_fcf = parameters["base_free_cash_flow"]
    growth = parameters["cash_flow_growth"] / 100
    discount = parameters["discount_rate"] / 100
    terminal_growth = parameters["terminal_growth"] / 100
    future_cash_flows = []
    present_value = 0.0
    for year in range(1, parameters["forecast_years"] + 1):
        cash_flow = base_fcf * (1 + growth) ** year
        discounted = cash_flow / (1 + discount) ** year
        present_value += discounted
        future_cash_flows.append(
            {"year": year, "free_cash_flow": round(cash_flow, 2), "present_value": round(discounted, 2)}
        )
    terminal_fcf = base_fcf * (1 + growth) ** parameters["forecast_years"] * (1 + terminal_growth)
    terminal_value = terminal_fcf / (discount - terminal_growth)
    terminal_present_value = terminal_value / (1 + discount) ** parameters["forecast_years"]
    enterprise_scenario_value = present_value + terminal_present_value
    return {
        "method": "dcf_scenario",
        "parameters": parameters,
        "future_cash_flows": future_cash_flows,
        "forecast_present_value": round(present_value, 2),
        "terminal_present_value": round(terminal_present_value, 2),
        "enterprise_scenario_value": round(enterprise_scenario_value, 2),
        "per_share_scenario_value": round(enterprise_scenario_value / parameters["total_shares"], 4),
        "notice": "未自动调整净债务、少数股东权益或非经营资产；调用者应在 base_free_cash_flow/basis 中明确口径。",
    }


def ddm_valuation(parameters: dict[str, Any]) -> dict[str, Any]:
    dividend = parameters["current_dividend_per_share"]
    growth = parameters["dividend_growth"] / 100
    required_return = parameters["required_return"] / 100
    next_dividend = dividend * (1 + growth)
    value = next_dividend / (required_return - growth)
    return {
        "method": "gordon_ddm_scenario",
        "parameters": parameters,
        "next_dividend_per_share": round(next_dividend, 4),
        "per_share_scenario_value": round(value, 4),
        "notice": "Gordon 模型假设股息永久按固定速率增长；仅适用于用户确认该假设可用的场景。",
    }


def _historical_band(percentile: float | None) -> str | None:
    if percentile is None:
        return None
    if not 0 <= percentile <= 100:
        raise ValueError("历史分位数必须在 0–100 范围内。")
    if percentile < 25:
        return "0–25 percentile"
    if percentile < 50:
        return "25–50 percentile"
    if percentile < 75:
        return "50–75 percentile"
    return "75–100 percentile"


def relative_valuation(data: dict[str, Any]) -> dict[str, Any]:
    basic = data.get("basic_info") if isinstance(data.get("basic_info"), dict) else {}
    valuation = data.get("valuation") if isinstance(data.get("valuation"), dict) else {}
    latest = valuation.get("latest") if isinstance(valuation.get("latest"), dict) else {}
    pe = _optional_number(basic.get("pe_ttm", latest.get("pe_ttm", latest.get("pe"))), "PE_TTM")
    pb = _optional_number(basic.get("pb", latest.get("pb")), "PB")
    pe_percentile = _optional_number(
        valuation.get("pe_ttm_percentile", valuation.get("pe_percentile")), "PE percentile"
    )
    pb_percentile = _optional_number(valuation.get("pb_percentile"), "PB percentile")
    if all(value is None for value in (pe, pb, pe_percentile, pb_percentile)):
        raise ValueError("relative 方法需要 PE/PB 或其历史分位数。")
    return {
        "method": "relative_observation",
        "PE_TTM": pe,
        "PB": pb,
        "PE_historical_percentile": pe_percentile,
        "PB_historical_percentile": pb_percentile,
        "PE_historical_band": _historical_band(pe_percentile),
        "PB_historical_band": _historical_band(pb_percentile),
        "notice": "分位数只描述该输入样本中的历史位置；脚本不由分位数反推所谓合理 PE 或目标价。",
    }


def calculate_valuation(
    data: dict[str, Any],
    assumptions_raw: dict[str, Any],
    methods: list[str],
    data_as_of: str,
    data_sources: list[str],
) -> dict[str, Any]:
    data_as_of = _text(data_as_of, "data_as_of")
    data_sources = _string_list(data_sources, "data_sources")
    assumptions = validate_valuation_assumptions(assumptions_raw, methods)
    results: dict[str, Any] = {}
    if "dcf" in methods:
        results["dcf"] = dcf_valuation(assumptions["dcf"])
    if "ddm" in methods:
        results["ddm"] = ddm_valuation(assumptions["ddm"])
    if "relative" in methods:
        results["relative"] = relative_valuation(data)

    margin = assumptions.get("margin_of_safety")
    if margin is not None:
        for result in results.values():
            per_share = result.get("per_share_scenario_value")
            if per_share is not None:
                result["margin_of_safety_scenario"] = {
                    "margin_percent": margin,
                    "threshold_price": round(per_share * (1 - margin / 100), 4),
                    "notice": "这是按显式场景值机械折减的阈值，不是买入建议。",
                }

    price = data.get("price") if isinstance(data.get("price"), dict) else {}
    current_price = _optional_number(price.get("latest_price"), "price.latest_price")
    return {
        "code": str(data.get("code") or "").strip(),
        "name": str((data.get("basic_info") or {}).get("name") or "").strip()
        if isinstance(data.get("basic_info"), dict)
        else "",
        "calculated_at": datetime.now(timezone.utc).isoformat(),
        "data_as_of": data_as_of,
        "data_sources": data_sources,
        "assumptions_as_of": assumptions["as_of"],
        "assumption_sources": assumptions["sources"],
        "current_price_observation": current_price,
        "methods": results,
        "methodology": "只执行显式 DCF/DDM 场景算术和相对估值位置描述，不平均不同方法，也不输出高估/低估或交易结论。",
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="显式假设的 A 股估值场景计算器")
    parser.add_argument("--input", required=True, help="股票数据 JSON")
    parser.add_argument("--assumptions", required=True, help="经审阅、带来源的估值假设 JSON")
    parser.add_argument("--methods", required=True, help="dcf,ddm,relative 的逗号列表，或 all")
    parser.add_argument("--data-as-of", required=True, help="股票数据时点")
    parser.add_argument("--data-source", action="append", required=True, help="股票数据来源，可多次使用")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        methods = parse_methods(args.methods)
        data = load_json(args.input, MAX_DATA_BYTES, "股票数据 JSON")
        assumptions = load_json(args.assumptions, MAX_ASSUMPTION_BYTES, "估值假设 JSON")
        report = calculate_valuation(data, assumptions, methods, args.data_as_of, args.data_source)
        output = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"估值场景已保存到: {output_path}")
        else:
            print(output)
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        payload = json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False)
        print(payload if args.json else f"错误: {exc}", file=sys.stdout if args.json else sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
