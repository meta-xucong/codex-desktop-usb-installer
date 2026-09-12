#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = ["akshare", "pandas"]
# ///
"""Filter the fields actually returned by AkShare's open-fund ranking API."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SOURCE_INTERFACE = "akshare.fund_open_fund_rank_em"


def safe_float(value: Any) -> float | None:
    if value is None or value == "" or value == "--":
        return None
    try:
        number = float(str(value).replace("%", "").replace(",", ""))
    except (TypeError, ValueError):
        return None
    return number if math.isfinite(number) else None


def _load_akshare():
    try:
        import akshare as ak  # type: ignore
    except ImportError as exc:
        raise RuntimeError("缺少 akshare；请先按运行时清单安装锁定依赖。") from exc
    return ak


def screen_funds(
    fund_type: str | None = None,
    min_return_1y: float | None = None,
    min_return_3y: float | None = None,
    top_n: int = 20,
    provider=None,
) -> dict[str, Any]:
    """Fetch once and apply only supported filters; provider is injectable for tests."""
    if not 1 <= top_n <= 500:
        raise ValueError("top 必须在 1–500 之间。")
    ak = provider or _load_akshare()
    try:
        frame = ak.fund_open_fund_rank_em()
    except Exception as exc:
        raise RuntimeError(f"基金排行接口失败：{exc}") from exc
    if frame is None or getattr(frame, "empty", True):
        raise RuntimeError("基金排行接口没有返回可验证的数据。")

    funds = []
    for _, row in frame.iterrows():
        fund = {
            "code": str(row.get("基金代码", "")).strip(),
            "name": str(row.get("基金简称", "")).strip(),
            "type": str(row.get("基金类型", "")).strip(),
            "nav": safe_float(row.get("单位净值")),
            "accumulated_nav": safe_float(row.get("累计净值")),
            "return_1y": safe_float(row.get("近1年")),
            "return_2y": safe_float(row.get("近2年")),
            "return_3y": safe_float(row.get("近3年")),
            "return_5y": safe_float(row.get("近5年")),
            "return_this_year": safe_float(row.get("今年来")),
            "return_since_inception": safe_float(row.get("成立来")),
        }
        if fund_type and fund_type not in fund["type"]:
            continue
        if min_return_1y is not None and (
            fund["return_1y"] is None or fund["return_1y"] < min_return_1y
        ):
            continue
        if min_return_3y is not None and (
            fund["return_3y"] is None or fund["return_3y"] < min_return_3y
        ):
            continue
        funds.append(fund)

    funds.sort(
        key=lambda item: item["return_1y"] if item["return_1y"] is not None else float("-inf"),
        reverse=True,
    )
    return {
        "fetched_at": datetime.now(timezone.utc).isoformat(),
        "source_interface": SOURCE_INTERFACE,
        "filters": {
            "fund_type": fund_type,
            "min_return_1y": min_return_1y,
            "min_return_3y": min_return_3y,
            "top": top_n,
        },
        "funds": funds[:top_n],
        "matched_count_before_limit": len(funds),
        "notice": "只按接口实际返回的类型和区间收益字段筛选；结果需与带日期的基金公告或可信来源交叉核验。",
    }


def format_output(report: dict[str, Any]) -> str:
    funds = report["funds"]
    lines = [
        "基金筛选结果",
        f"抓取时间：{report['fetched_at']}",
        f"数据接口：{report['source_interface']}",
        f"匹配 {report['matched_count_before_limit']}，展示 {len(funds)}",
    ]
    for fund in funds:
        r1y = "N/A" if fund["return_1y"] is None else f"{fund['return_1y']:.2f}%"
        r3y = "N/A" if fund["return_3y"] is None else f"{fund['return_3y']:.2f}%"
        lines.append(f"- {fund['code']} {fund['name']} | {fund['type']} | 近1年 {r1y} | 近3年 {r3y}")
    lines.append(report["notice"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="基金公开排行字段筛选器")
    parser.add_argument("--type", help="基金类型包含文本")
    parser.add_argument("--min-return-1y", type=float, help="近 1 年最低区间收益率（%）")
    parser.add_argument("--min-return-3y", type=float, help="近 3 年最低区间收益率（%）")
    parser.add_argument("--top", type=int, default=20, help="返回前 N 个结果（1–500）")
    parser.add_argument("--output", help="输出 JSON 文件")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        report = screen_funds(args.type, args.min_return_1y, args.min_return_3y, args.top)
        output = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"结果已保存到: {output_path}")
        elif args.json:
            print(output)
        else:
            print(format_output(report))
        return 0
    except (OSError, RuntimeError, ValueError) as exc:
        if args.json:
            print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False))
        else:
            print(f"错误: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
