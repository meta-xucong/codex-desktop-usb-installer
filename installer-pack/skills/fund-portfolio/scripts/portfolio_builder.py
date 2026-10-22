#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Render amounts from an attributed, caller-reviewed fund allocation scenario."""

import argparse
import json
import math
import sys
from datetime import datetime
from pathlib import Path
from typing import Dict, List


MAX_MODEL_BYTES = 1024 * 1024


def validate_allocation_model(model: Dict) -> Dict:
    """Require an attributable, user-reviewed allocation scenario."""
    if not isinstance(model, dict):
        raise ValueError("配置模型 JSON 顶层必须是对象。")
    required_text = {}
    for key in ("risk_level", "description", "as_of"):
        value = str(model.get(key) or "").strip()
        if not value:
            raise ValueError(f"配置模型缺少 {key}。")
        required_text[key] = value
    sources = model.get("sources")
    if not isinstance(sources, list) or not sources or not all(isinstance(item, str) and item.strip() for item in sources):
        raise ValueError("配置模型 sources 必须至少包含一个非空来源或制定依据。")
    allocation = model.get("allocation")
    if not isinstance(allocation, dict) or not allocation:
        raise ValueError("配置模型 allocation 必须是非空类别到百分比映射。")
    normalized_allocation = {}
    for raw_name, raw_ratio in allocation.items():
        name = str(raw_name).strip()
        if not name or isinstance(raw_ratio, bool) or not isinstance(raw_ratio, (int, float)):
            raise ValueError("allocation 包含无效类别或比例。")
        ratio = float(raw_ratio)
        if not math.isfinite(ratio) or not 0 <= ratio <= 100:
            raise ValueError(f"allocation.{name} 必须在 0 到 100 之间。")
        normalized_allocation[name] = ratio
    if not math.isclose(sum(normalized_allocation.values()), 100.0, abs_tol=0.01):
        raise ValueError("allocation 比例合计必须为 100。")

    expected_return = model.get("scenario_annual_return")
    max_drawdown = model.get("scenario_max_drawdown")
    for key, value in (("scenario_annual_return", expected_return), ("scenario_max_drawdown", max_drawdown)):
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
            raise ValueError(f"配置模型 {key} 必须显式提供有限数字。")
    notes = model.get("fund_type_notes", {})
    if not isinstance(notes, dict):
        raise ValueError("fund_type_notes 必须是对象。")
    return {
        **required_text,
        "sources": [item.strip() for item in sources],
        "allocation": normalized_allocation,
        "scenario_annual_return": float(expected_return),
        "scenario_max_drawdown": float(max_drawdown),
        "fund_type_notes": notes,
    }


def generate_portfolio(model: Dict, amount: float, period_years: int) -> Dict:
    """按显式配置模型计算各类别金额。"""
    model = validate_allocation_model(model)
    if not math.isfinite(amount) or amount <= 0:
        raise ValueError("amount 必须是正数。")
    if not isinstance(period_years, int) or not 1 <= period_years <= 100:
        raise ValueError("period_years 必须是 1 到 100 的整数。")
    
    portfolio = {
        "generated_at": datetime.now().isoformat(),
        "risk_level": model["risk_level"],
        "description": model["description"],
        "model_as_of": model["as_of"],
        "sources": model["sources"],
        "total_amount": amount,
        "investment_period": f"{period_years}年",
        "scenario_annual_return": model["scenario_annual_return"],
        "scenario_max_drawdown": model["scenario_max_drawdown"],
        "methodology": "配置比例与收益/回撤均来自调用者提供的场景模型；脚本只换算金额。",
        "allocation": {},
        "fund_type_notes": {},
        "limitations": [
            "执行前需由用户重新核验模型来源、数据时点、费用和自身约束。",
            "具体基金选择、交易时点与再平衡规则不由本金额换算器决定。",
            "场景收益和回撤不是预测或承诺。",
        ],
    }
    
    # 计算各类基金金额
    for fund_type, ratio in model["allocation"].items():
        fund_amount = amount * ratio / 100
        portfolio["allocation"][fund_type] = {
            "ratio": ratio,
            "amount": fund_amount
        }
        
        notes = model["fund_type_notes"].get(fund_type, [])
        portfolio["fund_type_notes"][fund_type] = notes if isinstance(notes, list) else [str(notes)]
    
    return portfolio


def format_report(portfolio: Dict) -> str:
    """格式化报告"""
    lines = [
        "=" * 60,
        "基金配置金额换算场景",
        "=" * 60,
        "",
        f"【风险等级】{portfolio['risk_level']}",
        f"【投资金额】{portfolio['total_amount']:,.0f} 元",
        f"【投资期限】{portfolio['investment_period']}",
        f"【风险描述】{portfolio['description']}",
        "",
        "-" * 60,
        "用户模型中的收益与回撤场景",
        "-" * 60,
        f"场景年化收益：{portfolio['scenario_annual_return']}%",
        f"场景最大回撤：{portfolio['scenario_max_drawdown']}%",
        "",
        "-" * 60,
        "配置金额换算",
        "-" * 60,
        ""
    ]
    
    for fund_type, data in portfolio['allocation'].items():
        lines.append(f"■ {fund_type}: {data['ratio']}% ({data['amount']:,.0f}元)")
        notes = portfolio['fund_type_notes'].get(fund_type, [])
        if notes:
            lines.append(f"  用户模型备注：{'、'.join(str(item) for item in notes)}")
        lines.append("")
    
    lines.extend([
        "-" * 60,
        "限制与核验项",
        "-" * 60
    ])
    
    for i, limitation in enumerate(portfolio['limitations'], 1):
        lines.append(f"{i}. {limitation}")
    
    lines.extend([
        "",
        "=" * 60,
        f"模型时点：{portfolio['model_as_of']}",
        "模型来源：" + "；".join(portfolio["sources"]),
        f"方法边界：{portfolio['methodology']}",
        "=" * 60
    ])
    
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="基金配置金额换算器")
    parser.add_argument("--model", required=True, help="用户审阅过的配置模型 JSON")
    parser.add_argument("--amount", type=float, required=True,
                       help="投资金额（元）")
    parser.add_argument("--period-years", type=int, required=True, help="投资期限（年）")
    parser.add_argument("--output", type=str, help="输出JSON文件")
    parser.add_argument("--json", action="store_true", help="JSON格式输出")
    
    args = parser.parse_args()
    
    try:
        model_path = Path(args.model).expanduser().resolve()
        if not model_path.is_file() or model_path.stat().st_size > MAX_MODEL_BYTES:
            raise ValueError("配置模型不存在或超过 1 MiB。")
        model = json.loads(model_path.read_text(encoding="utf-8"))
        portfolio = generate_portfolio(model, args.amount, args.period_years)

        if args.json or args.output:
            output = json.dumps(portfolio, ensure_ascii=False, indent=2)
            if args.output:
                with open(args.output, 'w', encoding='utf-8') as f:
                    f.write(output)
                print(f"方案已保存到: {args.output}")
            else:
                print(output)
        else:
            print(format_report(portfolio))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        if args.json:
            print(json.dumps({"error": str(error)}, ensure_ascii=False))
        else:
            print(f"组合场景计算失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
