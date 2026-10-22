#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Validate and format an evidence-backed policy scenario.

This entrypoint deliberately contains no built-in policy-to-market lookup table,
sector call, asset-allocation advice, or stop-loss percentage.
"""

from __future__ import annotations

import argparse
import json
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


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} 必须是数组。")
    result = [_text(item, f"{field}[]") for item in value]
    if not result:
        raise ValueError(f"{field} 至少包含一项。")
    return result


def load_policy_input(path_value: str) -> dict[str, Any]:
    path = Path(path_value).expanduser().resolve()
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError("政策输入 JSON 不存在或超过 1 MiB。")
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise ValueError("政策输入 JSON 顶层必须是对象。")
    return raw


def validate_policy_input(raw: dict[str, Any]) -> dict[str, Any]:
    hypotheses_raw = raw.get("transmission_hypotheses")
    if not isinstance(hypotheses_raw, list) or not hypotheses_raw:
        raise ValueError("transmission_hypotheses 必须至少包含一项。")
    hypotheses = []
    for index, item in enumerate(hypotheses_raw):
        if not isinstance(item, dict):
            raise ValueError(f"transmission_hypotheses[{index}] 必须是对象。")
        hypotheses.append(
            {
                "statement": _text(item.get("statement"), f"transmission_hypotheses[{index}].statement"),
                "basis": _text(item.get("basis"), f"transmission_hypotheses[{index}].basis"),
                "confidence": _text(item.get("confidence"), f"transmission_hypotheses[{index}].confidence"),
            }
        )

    scenarios_raw = raw.get("scenario_implications")
    if not isinstance(scenarios_raw, list) or not scenarios_raw:
        raise ValueError("scenario_implications 必须至少包含一项。")
    scenarios = []
    for index, item in enumerate(scenarios_raw):
        if not isinstance(item, dict):
            raise ValueError(f"scenario_implications[{index}] 必须是对象。")
        scenarios.append(
            {
                "scenario": _text(item.get("scenario"), f"scenario_implications[{index}].scenario"),
                "implication": _text(item.get("implication"), f"scenario_implications[{index}].implication"),
                "monitor": _string_list(item.get("monitor"), f"scenario_implications[{index}].monitor"),
            }
        )

    return {
        "data_as_of": _text(raw.get("as_of"), "as_of"),
        "sources": _string_list(raw.get("sources"), "sources"),
        "policy_type": _text(raw.get("policy_type"), "policy_type"),
        "action": _text(raw.get("action"), "action"),
        "context": _text(raw.get("context"), "context"),
        "evidence": _string_list(raw.get("evidence"), "evidence"),
        "transmission_hypotheses": hypotheses,
        "scenario_implications": scenarios,
        "risks": _string_list(raw.get("risks"), "risks"),
    }


def generate_policy_analysis(raw: dict[str, Any]) -> dict[str, Any]:
    validated = validate_policy_input(raw)
    return {
        "analyzed_at": datetime.now(timezone.utc).isoformat(),
        **validated,
        "methodology": (
            "本脚本只校验并格式化调用者依据政策原文形成的证据、传导假设和条件场景；"
            "不内置政策影响、板块受益、配置比例或交易指令。"
        ),
        "limitations": [
            "传导关系是待验证假设，不是确定因果结论。",
            "结果不构成个性化投资建议，执行前需核验最新政策、市场状态与用户约束。",
        ],
    }


def format_report(report: dict[str, Any]) -> str:
    lines = [
        f"政策情景分析：{report['policy_type']} / {report['action']}",
        f"数据时点：{report['data_as_of']}",
        "来源：" + "；".join(report["sources"]),
        f"背景：{report['context']}",
        "证据：",
        *[f"- {item}" for item in report["evidence"]],
        "传导假设：",
    ]
    lines.extend(
        f"- [{item['confidence']}] {item['statement']}（依据：{item['basis']}）"
        for item in report["transmission_hypotheses"]
    )
    lines.append("条件场景：")
    lines.extend(
        f"- {item['scenario']}：{item['implication']}；监测：{'、'.join(item['monitor'])}"
        for item in report["scenario_implications"]
    )
    lines.extend(["风险：", *[f"- {item}" for item in report["risks"]], "方法边界：" + report["methodology"]])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="来源明确的政策情景输入校验与格式化")
    parser.add_argument("--input", required=True, help="完整、带来源和时点的政策情景 JSON")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        report = generate_policy_analysis(load_policy_input(args.input))
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
