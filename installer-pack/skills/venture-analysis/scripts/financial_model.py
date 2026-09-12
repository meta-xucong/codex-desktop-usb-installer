#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Transparent venture financial scenario model.

Every business input and liquidity buffer is explicit. The script does not
label a model healthy, recommend financing rounds, or infer investment action.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from typing import Any


def _finite(value: Any, field: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{field} 必须是有限数值。")
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{field} 必须是有限数值。") from exc
    if not math.isfinite(number):
        raise ValueError(f"{field} 必须是有限数值。")
    return number


def calculate_unit_economics(
    monthly_arpu: float,
    cac_per_new_user: float,
    gross_margin: float,
    monthly_churn_rate: float,
) -> dict[str, Any]:
    monthly_churn = monthly_churn_rate / 100
    if not 0 < monthly_churn <= 1:
        raise ValueError("monthly_churn_rate 必须大于 0 且不超过 100。")
    monthly_gross_profit_per_user = monthly_arpu * gross_margin / 100
    ltv = monthly_gross_profit_per_user / monthly_churn
    ratio = None if cac_per_new_user == 0 else ltv / cac_per_new_user
    payback = None if monthly_gross_profit_per_user == 0 else cac_per_new_user / monthly_gross_profit_per_user
    return {
        "monthly_arpu": monthly_arpu,
        "cac_per_new_user": cac_per_new_user,
        "gross_margin": gross_margin,
        "monthly_churn_rate": monthly_churn_rate,
        "simple_gross_profit_ltv": round(ltv, 2),
        "ltv_to_cac": None if ratio is None else round(ratio, 4),
        "simple_payback_months": None if payback is None else round(payback, 2),
        "methodology": "LTV = 月 ARPU × 毛利率 ÷ 月流失率；未建模 cohort、折现、扩张收入或回收失败。",
    }


def project_financials(
    years: int,
    initial_active_users: int,
    annual_net_user_growth_rate: float,
    monthly_arpu: float,
    cac_per_new_user: float,
    gross_margin: float,
    monthly_churn_rate: float,
    monthly_fixed_costs: float,
    monthly_other_cash_outflow: float,
) -> list[dict[str, Any]]:
    projections = []
    opening_users = float(initial_active_users)
    annual_retention = (1 - monthly_churn_rate / 100) ** 12
    for year in range(1, years + 1):
        ending_users = max(0.0, opening_users * (1 + annual_net_user_growth_rate / 100))
        retained_users = opening_users * annual_retention
        gross_new_users = max(0.0, ending_users - retained_users)
        average_active_users = (opening_users + ending_users) / 2
        revenue = average_active_users * monthly_arpu * 12
        cost_of_revenue = revenue * (1 - gross_margin / 100)
        acquisition_cost = gross_new_users * cac_per_new_user
        fixed_costs = monthly_fixed_costs * 12
        other_cash_outflow = monthly_other_cash_outflow * 12
        operating_profit = revenue - cost_of_revenue - acquisition_cost - fixed_costs
        scenario_cash_flow = operating_profit - other_cash_outflow
        projections.append(
            {
                "year": year,
                "opening_active_users": round(opening_users, 2),
                "retained_active_users_before_acquisition": round(retained_users, 2),
                "gross_new_users_required": round(gross_new_users, 2),
                "ending_active_users": round(ending_users, 2),
                "average_active_users": round(average_active_users, 2),
                "revenue": round(revenue, 2),
                "cost_of_revenue": round(cost_of_revenue, 2),
                "acquisition_cost": round(acquisition_cost, 2),
                "fixed_costs": round(fixed_costs, 2),
                "other_cash_outflow": round(other_cash_outflow, 2),
                "operating_profit": round(operating_profit, 2),
                "scenario_cash_flow": round(scenario_cash_flow, 2),
            }
        )
        opening_users = ending_users
    return projections


def calculate_liquidity(
    projections: list[dict[str, Any]],
    initial_cash: float,
    monthly_fixed_costs: float,
    monthly_other_cash_outflow: float,
    liquidity_buffer_months: float,
) -> dict[str, Any]:
    cash = initial_cash
    minimum_cash = initial_cash
    cash_path = []
    for projection in projections:
        cash += projection["scenario_cash_flow"]
        minimum_cash = min(minimum_cash, cash)
        cash_path.append({"year": projection["year"], "ending_cash": round(cash, 2)})
    buffer_amount = (monthly_fixed_costs + monthly_other_cash_outflow) * liquidity_buffer_months
    additional_funding_scenario = max(0.0, buffer_amount - minimum_cash)
    return {
        "initial_cash": initial_cash,
        "minimum_cash": round(minimum_cash, 2),
        "liquidity_buffer_months": liquidity_buffer_months,
        "liquidity_buffer_amount": round(buffer_amount, 2),
        "additional_funding_scenario": round(additional_funding_scenario, 2),
        "cash_path": cash_path,
        "methodology": "资金缺口场景 = 显式流动性缓冲金额 − 模型期内最低现金（下限为 0）；不推荐融资轮次或条款。",
    }


def generate_model(args: SimpleNamespace) -> dict[str, Any]:
    years = args.years
    initial_users = args.initial_active_users
    if not isinstance(years, int) or not 1 <= years <= 30:
        raise ValueError("years 必须是 1–30 的整数。")
    if not isinstance(initial_users, int) or initial_users < 0:
        raise ValueError("initial_active_users 必须是非负整数。")

    values = {
        "annual_net_user_growth_rate": _finite(args.annual_net_user_growth_rate, "annual_net_user_growth_rate"),
        "monthly_arpu": _finite(args.monthly_arpu, "monthly_arpu"),
        "cac_per_new_user": _finite(args.cac_per_new_user, "cac_per_new_user"),
        "gross_margin": _finite(args.gross_margin, "gross_margin"),
        "monthly_churn_rate": _finite(args.monthly_churn_rate, "monthly_churn_rate"),
        "monthly_fixed_costs": _finite(args.monthly_fixed_costs, "monthly_fixed_costs"),
        "monthly_other_cash_outflow": _finite(args.monthly_other_cash_outflow, "monthly_other_cash_outflow"),
        "initial_cash": _finite(args.initial_cash, "initial_cash"),
        "liquidity_buffer_months": _finite(args.liquidity_buffer_months, "liquidity_buffer_months"),
    }
    if values["annual_net_user_growth_rate"] <= -100:
        raise ValueError("annual_net_user_growth_rate 必须大于 -100%。")
    if any(values[key] < 0 for key in (
        "monthly_arpu",
        "cac_per_new_user",
        "monthly_fixed_costs",
        "monthly_other_cash_outflow",
        "initial_cash",
        "liquidity_buffer_months",
    )):
        raise ValueError("收入、成本、现金和缓冲月份不得为负。")
    if not 0 <= values["gross_margin"] <= 100:
        raise ValueError("gross_margin 必须在 0–100 之间。")
    if not 0 < values["monthly_churn_rate"] <= 100:
        raise ValueError("monthly_churn_rate 必须大于 0 且不超过 100。")
    as_of = str(args.as_of or "").strip()
    sources = [str(item).strip() for item in (args.source or []) if str(item).strip()]
    if not as_of or not sources:
        raise ValueError("必须提供非空 --as-of 和至少一个 --source。")

    unit_economics = calculate_unit_economics(
        values["monthly_arpu"],
        values["cac_per_new_user"],
        values["gross_margin"],
        values["monthly_churn_rate"],
    )
    projections = project_financials(
        years,
        initial_users,
        values["annual_net_user_growth_rate"],
        values["monthly_arpu"],
        values["cac_per_new_user"],
        values["gross_margin"],
        values["monthly_churn_rate"],
        values["monthly_fixed_costs"],
        values["monthly_other_cash_outflow"],
    )
    break_even_year = next(
        (item["year"] for item in projections if item["operating_profit"] >= 0),
        None,
    )
    liquidity = calculate_liquidity(
        projections,
        values["initial_cash"],
        values["monthly_fixed_costs"],
        values["monthly_other_cash_outflow"],
        values["liquidity_buffer_months"],
    )
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "inputs_as_of": as_of,
        "sources": sources,
        "assumptions": {
            "projection_years": years,
            "initial_active_users": initial_users,
            **values,
        },
        "unit_economics": unit_economics,
        "projections": projections,
        "first_nonnegative_operating_profit_year": break_even_year,
        "liquidity": liquidity,
        "methodology": (
            "年度用户目标按显式年净增长率计算；月流失率转换为年留存，获客数为达到年末目标所需的补充用户。"
            "收入按年初/年末平均活跃用户计算。所有结果都是恒定参数场景，不是财务预测承诺。"
        ),
        "limitations": [
            "未建模月度季节性、价格变化、税、营运资本、资本开支、融资稀释或用户 cohort 差异。",
            "流动性缓冲只覆盖固定成本和显式其他现金流出，不代表法定或专业建议。",
            "脚本不使用 LTV/CAC 或回本期阈值判断健康度，也不推荐融资轮次。",
        ],
    }


def format_report(model: dict[str, Any]) -> str:
    unit = model["unit_economics"]
    liquidity = model["liquidity"]
    lines = [
        "创业财务场景",
        f"输入时点：{model['inputs_as_of']}",
        "来源：" + "；".join(model["sources"]),
        f"简单毛利 LTV：{unit['simple_gross_profit_ltv']}",
        f"LTV/CAC：{unit['ltv_to_cac']}",
        f"简单回本月数：{unit['simple_payback_months']}",
        f"首个经营利润非负年份：{model['first_nonnegative_operating_profit_year']}",
        f"显式缓冲下的追加资金场景：{liquidity['additional_funding_scenario']}",
        "年度场景：",
    ]
    lines.extend(
        f"- 第{item['year']}年：期末活跃用户 {item['ending_active_users']}，收入 {item['revenue']}，"
        f"经营利润 {item['operating_profit']}，场景现金流 {item['scenario_cash_flow']}"
        for item in model["projections"]
    )
    lines.extend(["方法边界：" + model["methodology"], *["限制：" + item for item in model["limitations"]]])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="显式输入的创业财务场景模型")
    parser.add_argument("--years", type=int, required=True, help="场景年数（1–30）")
    parser.add_argument("--initial-active-users", type=int, required=True, help="初始活跃用户数")
    parser.add_argument("--annual-net-user-growth-rate", type=float, required=True, help="年净活跃用户增长率（%）")
    parser.add_argument("--monthly-arpu", type=float, required=True, help="每活跃用户月收入")
    parser.add_argument("--cac-per-new-user", type=float, required=True, help="每新增活跃用户获客成本")
    parser.add_argument("--gross-margin", type=float, required=True, help="毛利率（%）")
    parser.add_argument("--monthly-churn-rate", type=float, required=True, help="月流失率（%）")
    parser.add_argument("--monthly-fixed-costs", type=float, required=True, help="月固定成本")
    parser.add_argument("--monthly-other-cash-outflow", type=float, required=True, help="未计入经营利润的其他月现金流出")
    parser.add_argument("--initial-cash", type=float, required=True, help="初始现金")
    parser.add_argument("--liquidity-buffer-months", type=float, required=True, help="流动性缓冲月数场景")
    parser.add_argument("--as-of", required=True, help="输入数据或假设时点")
    parser.add_argument("--source", action="append", required=True, help="数据来源或假设依据，可多次使用")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        model = generate_model(args)
        output = json.dumps(model, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"模型已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_report(model))
        return 0
    except (OSError, ValueError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"财务场景建模失败：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
