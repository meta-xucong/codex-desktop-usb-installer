#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""
定投计算器

Usage:
    python sip_calculator.py --monthly 3000 --years 5 --expected-return 8
    python sip_calculator.py --monthly 5000 --years 10 --strategy "智能定投"
"""

import argparse
import json
import math
import sys
from datetime import datetime
from typing import Dict, List


def calculate_regular_sip(
    monthly_amount: float,
    years: int,
    annual_return: float
) -> Dict:
    """按调用者给出的年化收益假设计算普通定投场景。"""
    if not math.isfinite(monthly_amount) or monthly_amount <= 0:
        raise ValueError("monthly_amount 必须是正数。")
    if not isinstance(years, int) or not 1 <= years <= 100:
        raise ValueError("years 必须是 1 到 100 的整数。")
    if not math.isfinite(annual_return) or not -100 < annual_return <= 1000:
        raise ValueError("annual_return 必须大于 -100% 且不超过 1000%。")
    months = years * 12
    monthly_rate = annual_return / 100 / 12
    
    # 累计投入
    total_invested = monthly_amount * months
    
    # 未来价值（年金终值公式）
    if monthly_rate == 0:
        future_value = total_invested
    else:
        future_value = monthly_amount * ((1 + monthly_rate) ** months - 1) / monthly_rate
    
    # 总收益
    total_return = future_value - total_invested
    return_rate = (total_return / total_invested) * 100 if total_invested > 0 else 0
    
    # 生成明细
    details = []
    accumulated = 0
    for month in range(1, months + 1):
        accumulated = accumulated * (1 + monthly_rate) + monthly_amount
        if month % 12 == 0:
            details.append({
                "year": month // 12,
                "invested": monthly_amount * month,
                "value": accumulated,
                "return": accumulated - monthly_amount * month
            })
    
    return {
        "strategy": "普通定投",
        "monthly_amount": monthly_amount,
        "years": years,
        "total_months": months,
        "annual_return": annual_return,
        "scenario_notice": "年化收益率是调用者显式提供的场景假设，不是历史数据或收益预测。",
        "total_invested": total_invested,
        "future_value": future_value,
        "total_return": total_return,
        "return_rate": return_rate,
        "details": details
    }


def calculate_smart_sip(
    monthly_amount: float,
    years: int,
    base_return: float,
    smart_return: float,
) -> Dict:
    """比较两个由调用者显式给出的收益率场景。"""
    regular_result = calculate_regular_sip(monthly_amount, years, base_return)
    enhanced_result = calculate_regular_sip(monthly_amount, years, smart_return)
    
    return {
        "strategy": "智能定投",
        "description": "调用者定义的智能定投收益率场景，与普通定投场景比较",
        "monthly_amount": monthly_amount,
        "years": years,
        "base_annual_return": base_return,
        "smart_scenario_annual_return": smart_return,
        "total_invested": regular_result["total_invested"],
        "future_value": enhanced_result["future_value"],
        "total_return": enhanced_result["total_return"],
        "return_rate": enhanced_result["return_rate"],
        "advantage_vs_regular": enhanced_result["future_value"] - regular_result["future_value"],
        "advantage_rate": ((enhanced_result["future_value"] / regular_result["future_value"]) - 1) * 100,
        "scenario_notice": "两个收益率均为显式场景假设；脚本不假设智能定投必然提高收益。",
    }


def format_report(result: Dict) -> str:
    """格式化报告"""
    lines = [
        "=" * 60,
        "定投计划报告",
        "=" * 60,
        "",
        f"【策略】{result['strategy']}",
        f"【每月定投】{result['monthly_amount']:,.0f} 元",
        f"【定投期限】{result['years']} 年（{result['total_months']}期）",
        "",
        "-" * 60,
        "收益率假设下的场景测算",
        "-" * 60,
        f"累计投入：{result['total_invested']:,.0f} 元",
        f"场景期末市值：{result['future_value']:,.0f} 元",
        f"场景收益：{result['total_return']:+,.0f} 元（{result['return_rate']:+.1f}%）",
    ]
    
    if "advantage_vs_regular" in result:
        lines.extend([
            "",
            f"相对普通场景差额：{result['advantage_vs_regular']:,.0f} 元",
            f"相对场景差异：{result['advantage_rate']:.1f}%",
        ])
    
    lines.extend([
        "",
        "-" * 60,
        "模型边界",
        "-" * 60,
        f"• {result['scenario_notice']}",
        "• 未计入申赎费、税费、通胀、收益波动和定投时点差异。",
        "• 不能据此断言智能定投优于普通定投，也不构成投资建议。",
        "=" * 60,
    ])
    
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="定投计算器")
    parser.add_argument("--monthly", type=float, required=True, help="每月定投金额")
    parser.add_argument("--years", type=int, required=True, help="定投年限")
    parser.add_argument("--expected-return", type=float, required=True, help="普通定投场景年化收益率假设（%）")
    parser.add_argument("--smart-expected-return", type=float, help="智能定投场景年化收益率假设（%）")
    parser.add_argument("--strategy", type=str, default="普通定投", 
                       choices=["普通定投", "智能定投"],
                       help="定投策略")
    parser.add_argument("--output", type=str, help="输出JSON文件")
    parser.add_argument("--json", action="store_true", help="JSON格式输出")
    
    args = parser.parse_args()
    
    try:
        if args.strategy == "智能定投":
            if args.smart_expected_return is None:
                raise ValueError("智能定投比较必须显式提供 --smart-expected-return。")
            result = calculate_smart_sip(args.monthly, args.years, args.expected_return, args.smart_expected_return)
        else:
            result = calculate_regular_sip(args.monthly, args.years, args.expected_return)

        if args.json or args.output:
            output = json.dumps(result, ensure_ascii=False, indent=2)
            if args.output:
                with open(args.output, 'w', encoding='utf-8') as f:
                    f.write(output)
                print(f"报告已保存到: {args.output}")
            else:
                print(output)
        else:
            print(format_report(result))
        return 0
    except (OSError, ValueError) as error:
        if args.json:
            print(json.dumps({"error": str(error)}, ensure_ascii=False))
        else:
            print(f"定投场景计算失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
