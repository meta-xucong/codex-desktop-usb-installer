#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Calculate a user-reviewed venture evaluation rubric.

Text length, industry labels, market size, team background, and competitive
labels are never converted into scores automatically.
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


def _text(value: Any, field: str) -> str:
    normalized = str(value or "").strip()
    if not normalized:
        raise ValueError(f"{field} 必须是非空文本。")
    return normalized


def _number(value: Any, field: str, minimum: float | None = None, maximum: float | None = None) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{field} 必须是有限数值。")
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{field} 必须是有限数值。") from exc
    if not math.isfinite(number):
        raise ValueError(f"{field} 必须是有限数值。")
    if minimum is not None and number < minimum:
        raise ValueError(f"{field} 不得小于 {minimum}。")
    if maximum is not None and number > maximum:
        raise ValueError(f"{field} 不得大于 {maximum}。")
    return number


def _string_list(value: Any, field: str, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} 必须是数组。")
    result = [_text(item, f"{field}[]") for item in value]
    if not allow_empty and not result:
        raise ValueError(f"{field} 至少包含一项。")
    return result


def load_input(path_value: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError("评估输入 JSON 不存在或超过 1 MiB。")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("评估输入 JSON 顶层必须是对象。")
    return value


def validate_business_input(raw: dict[str, Any]) -> dict[str, Any]:
    sources = _string_list(raw.get("sources"), "sources")
    evidence = _string_list(raw.get("evidence"), "evidence")

    market_raw = raw.get("market")
    if not isinstance(market_raw, dict):
        raise ValueError("market 必须是对象。")
    tam = _number(market_raw.get("tam"), "market.tam", 0)
    sam = _number(market_raw.get("sam"), "market.sam", 0)
    som = _number(market_raw.get("som"), "market.som", 0)
    if not som <= sam <= tam:
        raise ValueError("市场口径必须满足 0 ≤ SOM ≤ SAM ≤ TAM。")
    market = {
        "tam": tam,
        "sam": sam,
        "som": som,
        "currency": _text(market_raw.get("currency"), "market.currency"),
        "as_of": _text(market_raw.get("as_of"), "market.as_of"),
        "sources": _string_list(market_raw.get("sources"), "market.sources"),
        "basis": _text(market_raw.get("basis"), "market.basis"),
    }

    criteria_raw = raw.get("criteria")
    if not isinstance(criteria_raw, list) or not criteria_raw:
        raise ValueError("criteria 必须至少包含一项。")
    criteria = []
    identifiers = set()
    for index, item in enumerate(criteria_raw):
        if not isinstance(item, dict):
            raise ValueError(f"criteria[{index}] 必须是对象。")
        identifier = _text(item.get("id"), f"criteria[{index}].id")
        if identifier in identifiers:
            raise ValueError(f"criteria.id 重复：{identifier}。")
        identifiers.add(identifier)
        criteria.append(
            {
                "id": identifier,
                "label": _text(item.get("label"), f"criteria[{index}].label"),
                "weight": _number(item.get("weight"), f"criteria[{index}].weight", 0, 1),
                "score": _number(item.get("score"), f"criteria[{index}].score", 0, 10),
                "basis": _text(item.get("basis"), f"criteria[{index}].basis"),
                "risks": _string_list(item.get("risks"), f"criteria[{index}].risks", allow_empty=True),
            }
        )
    if not math.isclose(sum(item["weight"] for item in criteria), 1.0, abs_tol=0.0001):
        raise ValueError("criteria 权重必须合计 1。")

    return {
        "project_name": _text(raw.get("project_name"), "project_name"),
        "industry": _text(raw.get("industry"), "industry"),
        "inputs_as_of": _text(raw.get("as_of"), "as_of"),
        "sources": sources,
        "evidence": evidence,
        "market": market,
        "criteria": criteria,
    }


def generate_evaluation(raw: dict[str, Any]) -> dict[str, Any]:
    validated = validate_business_input(raw)
    contributions = []
    for item in validated["criteria"]:
        contribution = item["weight"] * item["score"]
        contributions.append({**item, "weighted_contribution": round(contribution, 4)})
    weighted_score = sum(item["weighted_contribution"] for item in contributions)
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        **{key: value for key, value in validated.items() if key != "criteria"},
        "criteria": contributions,
        "weighted_score_out_of_10": round(weighted_score, 4),
        "methodology": (
            "加权分仅等于用户审阅的 criterion score × weight 之和；脚本不根据文字长度、行业、市场规模或"
            "团队描述生成分数，也不把分数映射为投/不投建议。"
        ),
        "limitations": [
            "分数和权重是主观政策输入，必须与原始证据一起阅读。",
            "TAM/SAM/SOM 只保留调用者口径，不据金额大小自动评价机会质量。",
            "结果不是法律、财务或投资意见。",
        ],
    }


def format_report(report: dict[str, Any]) -> str:
    lines = [
        f"创业项目评估场景：{report['project_name']}",
        f"行业：{report['industry']}",
        f"输入时点：{report['inputs_as_of']}",
        "来源：" + "；".join(report["sources"]),
        f"用户审阅加权分：{report['weighted_score_out_of_10']}/10",
        "评估项：",
    ]
    lines.extend(
        f"- {item['label']}：score={item['score']}，weight={item['weight']}，"
        f"contribution={item['weighted_contribution']}；依据：{item['basis']}"
        for item in report["criteria"]
    )
    lines.extend(["方法边界：" + report["methodology"], *["限制：" + item for item in report["limitations"]]])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="用户审阅评分表的创业项目评估计算器")
    parser.add_argument("--input", required=True, help="带来源、证据、市场口径和评分依据的 JSON")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        report = generate_evaluation(load_input(args.input))
        output = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"评估场景已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_report(report))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"评估失败：{exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
