#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = ["akshare", "pandas"]
# ///
"""Fail-closed A-share spot-field screener.

Only fields exposed by ``stock_zh_a_spot_em`` are accepted. Financial-statement
filters such as ROE, debt ratio, and dividend yield are intentionally absent.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


INDEX_CODE_MAP = {
    "hs300": "000300",
    "zz500": "000905",
    "zz1000": "000852",
    "cyb": "399006",
    "kcb": "000688",
}
SORT_COLUMNS = {"pe": "市盈率-动态", "pb": "市净率", "market_cap": "总市值"}


def _load_dependencies():
    try:
        import akshare as ak  # type: ignore
        import pandas as pd  # type: ignore
    except ImportError as exc:
        raise RuntimeError("缺少 akshare/pandas；请先按运行时清单安装锁定依赖。") from exc
    return ak, pd


def _parse_scope(scope: str) -> tuple[str, list[str]]:
    scope = str(scope or "").strip().lower()
    if scope == "all" or scope in INDEX_CODE_MAP:
        return scope, []
    if scope.startswith("custom:"):
        codes = [item.strip() for item in scope.removeprefix("custom:").split(",") if item.strip()]
        if not codes or any(not re.fullmatch(r"\d{6}", code) for code in codes):
            raise ValueError("custom scope 必须包含一个或多个六位股票代码。")
        return "custom", codes
    raise ValueError("scope 只接受 all/hs300/zz500/zz1000/cyb/kcb/custom:代码列表。")


def _finite_optional(value: float | None, field: str) -> float | None:
    if value is None:
        return None
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"{field} 必须是有限数值。")
    return number


def screen_stocks(
    scope: str,
    filters: dict[str, float | None],
    sort_by: str,
    top_n: int,
    provider=None,
    pandas_module=None,
) -> dict[str, Any]:
    if sort_by not in SORT_COLUMNS:
        raise ValueError("sort_by 只接受 pe、pb、market_cap。")
    if not 1 <= top_n <= 500:
        raise ValueError("top 必须在 1–500 之间。")
    filters = {key: _finite_optional(value, key) for key, value in filters.items() if value is not None}
    normalized_scope, custom_codes = _parse_scope(scope)
    if provider is None or pandas_module is None:
        loaded_provider, loaded_pandas = _load_dependencies()
        provider = provider or loaded_provider
        pandas_module = pandas_module or loaded_pandas

    source_interfaces = ["akshare.stock_zh_a_spot_em"]
    try:
        frame = provider.stock_zh_a_spot_em()
        if frame is None or getattr(frame, "empty", True):
            raise RuntimeError("stock_zh_a_spot_em 没有返回数据。")
        if normalized_scope in INDEX_CODE_MAP:
            constituents = provider.index_stock_cons(symbol=INDEX_CODE_MAP[normalized_scope])
            if constituents is None or getattr(constituents, "empty", True) or "品种代码" not in constituents.columns:
                raise RuntimeError("指数成分接口没有返回可验证的品种代码。")
            source_interfaces.append(f"akshare.index_stock_cons({INDEX_CODE_MAP[normalized_scope]})")
            codes = {str(code).zfill(6) for code in constituents["品种代码"].tolist()}
            frame = frame[frame["代码"].astype(str).str.zfill(6).isin(codes)]
        elif normalized_scope == "custom":
            frame = frame[frame["代码"].astype(str).str.zfill(6).isin(set(custom_codes))]
    except RuntimeError:
        raise
    except Exception as exc:
        raise RuntimeError(f"A 股筛选数据接口失败：{exc}") from exc

    column_contract = {
        "pe_min": "市盈率-动态",
        "pe_max": "市盈率-动态",
        "pb_min": "市净率",
        "pb_max": "市净率",
        "market_cap_min": "总市值",
        "market_cap_max": "总市值",
    }
    required_columns = {"代码", "名称", SORT_COLUMNS[sort_by]}
    required_columns.update(column_contract[key] for key in filters)
    missing = sorted(required_columns.difference(set(frame.columns)))
    if missing:
        raise RuntimeError(f"接口缺少请求所需字段：{', '.join(missing)}。筛选条件未执行。")

    numeric_cache: dict[str, Any] = {}
    for key, value in filters.items():
        column = column_contract[key]
        numeric = numeric_cache.setdefault(column, pandas_module.to_numeric(frame[column], errors="coerce"))
        comparison_value = value * 100_000_000 if key.startswith("market_cap_") else value
        frame = frame[numeric >= comparison_value] if key.endswith("_min") else frame[numeric <= comparison_value]
        numeric_cache.clear()

    sort_column = SORT_COLUMNS[sort_by]
    frame = frame.assign(_sort_value=pandas_module.to_numeric(frame[sort_column], errors="coerce"))
    frame = frame.dropna(subset=["_sort_value"])
    frame = frame.sort_values("_sort_value", ascending=sort_by in {"pe", "pb"}).head(top_n)

    results = []
    for _, row in frame.iterrows():
        market_cap = pandas_module.to_numeric(row.get("总市值"), errors="coerce")
        results.append(
            {
                "code": str(row.get("代码", "")).zfill(6),
                "name": str(row.get("名称", "")),
                "latest_price": row.get("最新价"),
                "change_percent": row.get("涨跌幅"),
                "pe_dynamic": row.get("市盈率-动态"),
                "pb": row.get("市净率"),
                "market_cap_100m": None
                if pandas_module.isna(market_cap)
                else round(float(market_cap) / 100_000_000, 2),
            }
        )

    return {
        "fetched_at": datetime.now(timezone.utc).isoformat(),
        "source_interfaces": source_interfaces,
        "scope": scope,
        "filters": filters,
        "sort_by": sort_by,
        "count": len(results),
        "results": results,
        "notice": "仅按行情接口实际字段筛选；没有内置综合分数，财务指标需从报表数据另行核验。",
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A 股实时行情字段筛选器")
    parser.add_argument("--scope", required=True, help="all/hs300/zz500/zz1000/cyb/kcb/custom:代码列表")
    parser.add_argument("--pe-max", type=float, help="最大动态 PE")
    parser.add_argument("--pe-min", type=float, help="最小动态 PE")
    parser.add_argument("--pb-max", type=float, help="最大 PB")
    parser.add_argument("--pb-min", type=float, help="最小 PB")
    parser.add_argument("--market-cap-min", type=float, help="最小总市值（亿元）")
    parser.add_argument("--market-cap-max", type=float, help="最大总市值（亿元）")
    parser.add_argument("--sort-by", required=True, choices=sorted(SORT_COLUMNS), help="排序字段")
    parser.add_argument("--top", type=int, default=50, help="返回前 N 只（1–500）")
    parser.add_argument("--output", help="输出 JSON 文件")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    filters = {
        "pe_max": args.pe_max,
        "pe_min": args.pe_min,
        "pb_max": args.pb_max,
        "pb_min": args.pb_min,
        "market_cap_min": args.market_cap_min,
        "market_cap_max": args.market_cap_max,
    }
    try:
        report = screen_stocks(args.scope, filters, args.sort_by, args.top)
        output = json.dumps(report, ensure_ascii=False, indent=2, default=str)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"筛选结果已保存到: {output_path}")
        else:
            print(output)
        return 0
    except (OSError, RuntimeError, ValueError) as exc:
        print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
