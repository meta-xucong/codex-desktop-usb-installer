#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Render an amount allocation from an explicit, attributable policy template."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime
from pathlib import Path
from typing import Dict


MAX_TEMPLATE_BYTES = 1024 * 1024
BUCKETS = ("survival", "growth", "aggressive")


def _number(data: Dict, key: str, minimum=None, maximum=None) -> float:
    value = data.get(key)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{key} 必须是显式提供的数字。")
    normalized = float(value)
    if not math.isfinite(normalized):
        raise ValueError(f"{key} 必须是有限数字。")
    if minimum is not None and normalized < minimum:
        raise ValueError(f"{key} 不得小于 {minimum}。")
    if maximum is not None and normalized > maximum:
        raise ValueError(f"{key} 不得大于 {maximum}。")
    return normalized


def validate_policy_template(template: Dict) -> Dict:
    """Validate a user-reviewed policy; no risk profile or return is guessed."""
    if not isinstance(template, dict):
        raise ValueError("策略模板 JSON 顶层必须是对象。")
    texts = {}
    for key in ("risk_level", "description", "as_of"):
        value = str(template.get(key) or "").strip()
        if not value:
            raise ValueError(f"策略模板缺少 {key}。")
        texts[key] = value
    sources = template.get("sources")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("sources 必须至少包含一个非空的制定依据或来源。")

    allocation_raw = template.get("allocation")
    if not isinstance(allocation_raw, dict) or set(allocation_raw) != set(BUCKETS):
        raise ValueError(f"allocation 必须且只能包含 {list(BUCKETS)}。")
    allocation = {
        bucket: _number({bucket: allocation_raw[bucket]}, bucket, 0, 100)
        for bucket in BUCKETS
    }
    if not math.isclose(sum(allocation.values()), 100.0, abs_tol=0.01):
        raise ValueError("allocation 百分比合计必须为 100。")

    return {
        **texts,
        "sources": [item.strip() for item in sources],
        "allocation": allocation,
        "scenario_annual_return": _number(template, "scenario_annual_return", -100, 1000),
        "scenario_max_drawdown": _number(template, "scenario_max_drawdown", 0, 100),
        "emergency_months": _number(template, "emergency_months", 0, 120),
        "rebalance_threshold": _number(template, "rebalance_threshold", 0, 100),
    }


def generate_allocation_plan(
    total_amount: float,
    monthly_expense: float,
    period_years: float,
    template: Dict,
) -> Dict:
    """Convert explicit policy percentages to amounts and expose every assumption."""
    template = validate_policy_template(template)
    for name, value in (("total_amount", total_amount), ("monthly_expense", monthly_expense), ("period_years", period_years)):
        if not math.isfinite(value) or value <= 0:
            raise ValueError(f"{name} 必须是正数。")

    allocations = {
        bucket: {
            "ratio": template["allocation"][bucket],
            "amount": round(total_amount * template["allocation"][bucket] / 100, 2),
        }
        for bucket in BUCKETS
    }
    emergency_target = monthly_expense * template["emergency_months"]
    survival_amount = allocations["survival"]["amount"]
    return {
        "generated_at": datetime.now().isoformat(),
        "policy_as_of": template["as_of"],
        "sources": template["sources"],
        "risk_level": template["risk_level"],
        "description": template["description"],
        "total_amount": total_amount,
        "monthly_expense": monthly_expense,
        "investment_period_years": period_years,
        "scenario_annual_return": template["scenario_annual_return"],
        "scenario_max_drawdown": template["scenario_max_drawdown"],
        "allocation": allocations,
        "emergency_fund": {
            "months": template["emergency_months"],
            "target_amount": round(emergency_target, 2),
            "survival_bucket_amount": survival_amount,
            "shortfall": round(max(emergency_target - survival_amount, 0), 2),
        },
        "rebalance_threshold": template["rebalance_threshold"],
        "methodology": "所有比例、收益/回撤场景、应急月数和再平衡阈值均来自显式模板；脚本只换算金额。",
    }


def format_report(plan: Dict) -> str:
    labels = {"survival": "生存资产", "growth": "增值资产", "aggressive": "进攻资产"}
    lines = [
        "=" * 70,
        "资产配置场景换算",
        "=" * 70,
        f"风险标签：{plan['risk_level']}（由模板提供）",
        f"总资金：{plan['total_amount']:,.0f} 元；期限：{plan['investment_period_years']} 年",
        f"场景年化收益：{plan['scenario_annual_return']}%；场景最大回撤：{plan['scenario_max_drawdown']}%",
        "",
    ]
    for bucket in BUCKETS:
        item = plan["allocation"][bucket]
        lines.append(f"• {labels[bucket]}：{item['ratio']:.2f}%（{item['amount']:,.2f} 元）")
    emergency = plan["emergency_fund"]
    lines.extend(
        [
            "",
            f"应急金目标：{emergency['months']} 个月 × 月支出 = {emergency['target_amount']:,.2f} 元",
            f"生存资产相对目标缺口：{emergency['shortfall']:,.2f} 元",
            f"再平衡偏离阈值：{plan['rebalance_threshold']}%",
            "",
            f"策略时点：{plan['policy_as_of']}",
            "策略来源：" + "；".join(plan["sources"]),
            f"方法边界：{plan['methodology']}",
            "不包含具体产品推荐、交易执行或收益承诺。",
            "=" * 70,
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="显式策略模板的资产配置金额换算器")
    parser.add_argument("--template", required=True, help="用户审阅过的策略模板 JSON")
    parser.add_argument("--amount", type=float, required=True, help="总资金（元）")
    parser.add_argument("--period-years", type=float, required=True, help="投资期限（年）")
    parser.add_argument("--monthly-expense", type=float, required=True, help="月支出（元）")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="JSON 格式输出")
    args = parser.parse_args()

    try:
        template_path = Path(args.template).expanduser().resolve()
        if not template_path.is_file() or template_path.stat().st_size > MAX_TEMPLATE_BYTES:
            raise ValueError("策略模板不存在或超过 1 MiB。")
        template = json.loads(template_path.read_text(encoding="utf-8"))
        plan = generate_allocation_plan(args.amount, args.monthly_expense, args.period_years, template)
        if args.json or args.output:
            output = json.dumps(plan, ensure_ascii=False, indent=2)
            if args.output:
                output_path = Path(args.output).expanduser().resolve()
                output_path.write_text(output, encoding="utf-8")
                print(f"方案已保存到: {output_path}")
            else:
                print(output)
        else:
            print(format_report(plan))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        if args.json:
            print(json.dumps({"error": str(error)}, ensure_ascii=False))
        else:
            print(f"资产配置场景计算失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
