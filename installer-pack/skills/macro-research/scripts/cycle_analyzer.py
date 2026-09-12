#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Transparent economic-cycle quadrant calculator.

The script does not fetch current data and does not embed GDP/CPI thresholds,
asset allocations, transition probabilities, or trading advice. Current data
and a reviewed interpretation policy must be supplied explicitly.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MAX_INPUT_BYTES = 1024 * 1024
PHASE_KEYS = (
    "growth_up_inflation_down",
    "growth_up_inflation_up",
    "growth_down_inflation_up",
    "growth_down_inflation_down",
)


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


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} 必须是数组。")
    result = [_text(item, f"{field}[]") for item in value]
    if not result:
        raise ValueError(f"{field} 至少包含一项。")
    return result


def load_json(path_value: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError("规则 JSON 不存在或超过 1 MiB。")
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("规则 JSON 顶层必须是对象。")
    return data


def validate_cycle_rules(raw: dict[str, Any]) -> dict[str, Any]:
    """Validate a reviewed interpretation policy without adding defaults."""
    rules_as_of = _text(raw.get("as_of"), "rules.as_of")
    sources = _string_list(raw.get("sources"), "rules.sources")
    phase_policies_raw = raw.get("phase_policies")
    if not isinstance(phase_policies_raw, dict):
        raise ValueError("rules.phase_policies 必须是对象。")

    missing = [key for key in PHASE_KEYS if key not in phase_policies_raw]
    if missing:
        raise ValueError(f"rules.phase_policies 缺少阶段：{', '.join(missing)}。")

    phase_policies: dict[str, Any] = {}
    for key in PHASE_KEYS:
        raw_policy = phase_policies_raw[key]
        if not isinstance(raw_policy, dict):
            raise ValueError(f"rules.phase_policies.{key} 必须是对象。")
        policy: dict[str, Any] = {
            "label": _text(raw_policy.get("label"), f"{key}.label"),
            "description": _text(raw_policy.get("description"), f"{key}.description"),
            "characteristics": _string_list(raw_policy.get("characteristics"), f"{key}.characteristics"),
        }

        allocation_raw = raw_policy.get("allocation")
        if allocation_raw is not None:
            if not isinstance(allocation_raw, dict) or not allocation_raw:
                raise ValueError(f"{key}.allocation 必须是非空对象。")
            allocation = {
                _text(asset, f"{key}.allocation key"): _number(weight, f"{key}.allocation.{asset}")
                for asset, weight in allocation_raw.items()
            }
            if any(weight < 0 for weight in allocation.values()):
                raise ValueError(f"{key}.allocation 不允许负权重。")
            if not math.isclose(sum(allocation.values()), 100.0, abs_tol=0.01):
                raise ValueError(f"{key}.allocation 权重必须合计 100。")
            policy["allocation"] = allocation
            policy["allocation_basis"] = _text(
                raw_policy.get("allocation_basis"), f"{key}.allocation_basis"
            )

        if raw_policy.get("monitor") is not None:
            policy["monitor"] = _string_list(raw_policy["monitor"], f"{key}.monitor")

        transition_raw = raw_policy.get("transition")
        if transition_raw is not None:
            if not isinstance(transition_raw, dict):
                raise ValueError(f"{key}.transition 必须是对象。")
            policy["transition"] = {
                "next_phase": _text(transition_raw.get("next_phase"), f"{key}.transition.next_phase"),
                "assessment": _text(transition_raw.get("assessment"), f"{key}.transition.assessment"),
                "basis": _text(transition_raw.get("basis"), f"{key}.transition.basis"),
            }
        phase_policies[key] = policy

    return {"as_of": rules_as_of, "sources": sources, "phase_policies": phase_policies}


def identify_cycle_phase(
    gdp_growth: float,
    inflation: float,
    gdp_trend: float,
    inflation_trend: float,
) -> dict[str, Any]:
    """Classify supplied observations relative to supplied trend values."""
    values = {
        "gdp_growth": _number(gdp_growth, "gdp_growth"),
        "inflation": _number(inflation, "inflation"),
        "gdp_trend": _number(gdp_trend, "gdp_trend"),
        "inflation_trend": _number(inflation_trend, "inflation_trend"),
    }
    growth_up = values["gdp_growth"] >= values["gdp_trend"]
    inflation_up = values["inflation"] >= values["inflation_trend"]
    if growth_up and not inflation_up:
        phase_key = "growth_up_inflation_down"
    elif growth_up and inflation_up:
        phase_key = "growth_up_inflation_up"
    elif not growth_up and inflation_up:
        phase_key = "growth_down_inflation_up"
    else:
        phase_key = "growth_down_inflation_down"
    return {
        **values,
        "growth_relative_to_trend": "at_or_above" if growth_up else "below",
        "inflation_relative_to_trend": "at_or_above" if inflation_up else "below",
        "phase_key": phase_key,
    }


def generate_cycle_analysis(
    gdp_growth: float,
    inflation: float,
    gdp_trend: float,
    inflation_trend: float,
    data_as_of: str,
    sources: list[str],
    rules: dict[str, Any],
) -> dict[str, Any]:
    data_as_of = _text(data_as_of, "data_as_of")
    sources = _string_list(sources, "sources")
    validated_rules = validate_cycle_rules(rules)
    observation = identify_cycle_phase(gdp_growth, inflation, gdp_trend, inflation_trend)
    phase_policy = validated_rules["phase_policies"][observation["phase_key"]]
    return {
        "analyzed_at": datetime.now(timezone.utc).isoformat(),
        "data_as_of": data_as_of,
        "sources": sources,
        "rules_as_of": validated_rules["as_of"],
        "rule_sources": validated_rules["sources"],
        "observation": observation,
        "phase": phase_policy,
        "methodology": (
            "把调用者提供的增长与通胀观测值分别同调用者提供的趋势值比较，"
            "再按经审阅规则映射到四象限；配置与转向文字来自规则文件，不是脚本预测。"
        ),
        "limitations": [
            "仅使用四个输入数值，未自动纳入 PMI、就业、信贷、政策或市场价格。",
            "阶段标签和任何配置比例都是规则场景，不构成实时投资建议或收益承诺。",
        ],
    }


def format_report(report: dict[str, Any]) -> str:
    observation = report["observation"]
    phase = report["phase"]
    lines = [
        "经济周期四象限场景",
        f"数据时点：{report['data_as_of']}",
        "数据来源：" + "；".join(report["sources"]),
        f"规则时点：{report['rules_as_of']}",
        "规则来源：" + "；".join(report["rule_sources"]),
        f"阶段：{phase['label']}",
        f"说明：{phase['description']}",
        (
            f"输入：增长 {observation['gdp_growth']}% / 趋势 {observation['gdp_trend']}%；"
            f"通胀 {observation['inflation']}% / 趋势 {observation['inflation_trend']}%"
        ),
        "特征：" + "；".join(phase["characteristics"]),
    ]
    if "allocation" in phase:
        allocations = "，".join(f"{asset} {weight:g}%" for asset, weight in phase["allocation"].items())
        lines.extend([f"规则场景配置：{allocations}", f"配置依据：{phase['allocation_basis']}"])
    if "monitor" in phase:
        lines.append("待监测：" + "；".join(phase["monitor"]))
    if "transition" in phase:
        transition = phase["transition"]
        lines.append(
            f"规则中的转向场景：{transition['next_phase']}；{transition['assessment']}；依据：{transition['basis']}"
        )
    lines.extend(["方法边界：" + report["methodology"], *["限制：" + item for item in report["limitations"]]])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="经济周期四象限透明场景计算器")
    parser.add_argument("--gdp-growth", type=float, required=True, help="已核验的增长指标（%）")
    parser.add_argument("--inflation", type=float, required=True, help="已核验的通胀指标（%）")
    parser.add_argument("--gdp-trend", type=float, required=True, help="同口径增长趋势/基准（%）")
    parser.add_argument("--inflation-trend", type=float, required=True, help="同口径通胀趋势/基准（%）")
    parser.add_argument("--data-as-of", required=True, help="观测数据时点")
    parser.add_argument("--source", action="append", required=True, help="观测数据来源，可多次使用")
    parser.add_argument("--rules", required=True, help="经用户审阅、带来源的四象限规则 JSON")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        report = generate_cycle_analysis(
            args.gdp_growth,
            args.inflation,
            args.gdp_trend,
            args.inflation_trend,
            args.data_as_of,
            args.source,
            load_json(args.rules),
        )
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
            print(f"错误: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
