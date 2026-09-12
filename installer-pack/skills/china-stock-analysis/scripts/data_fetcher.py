#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = ["akshare", "pandas"]
# ///
"""Fetch attributed A-share data through pinned AkShare interfaces."""

from __future__ import annotations

import argparse
import importlib
import json
import math
import os
import re
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Callable


MAX_CODES = 100
MAX_CACHE_BYTES = 20 * 1024 * 1024
DATA_TYPES = ("all", "basic", "financial", "valuation", "holder")
INDEX_CODES = {
    "hs300": "000300",
    "zz500": "000905",
    "zz1000": "000852",
    "cyb": "399006",
    "kcb": "000688",
}


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _provider() -> Any:
    try:
        return importlib.import_module("akshare")
    except ImportError as exc:
        raise RuntimeError("缺少锁定依赖 akshare；请先运行安装包运行时检查。") from exc


def _validate_code(raw_code: str) -> str:
    code = str(raw_code or "").strip()
    if not re.fullmatch(r"\d{6}", code):
        raise ValueError(f"股票代码必须是六位数字：{raw_code!r}")
    if not code.startswith(("0", "3", "4", "6", "8")):
        raise ValueError(f"无法识别股票代码所属市场：{code}")
    return code


def safe_float(value: Any) -> float | None:
    if value is None or value == "" or value == "--":
        return None
    if isinstance(value, bool):
        return None
    if isinstance(value, str):
        value = value.replace("%", "").replace(",", "").replace("亿", "").strip()
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    return parsed if math.isfinite(parsed) else None


def _records(frame: Any, *, limit: int | None = None) -> list[dict[str, Any]]:
    if frame is None or bool(getattr(frame, "empty", False)):
        return []
    selected = frame.head(limit) if limit is not None else frame
    try:
        records = selected.to_dict(orient="records")
    except Exception as exc:
        raise RuntimeError("数据接口返回值不是可识别的表格。") from exc
    if not isinstance(records, list) or not all(isinstance(row, dict) for row in records):
        raise RuntimeError("数据接口返回值不是记录数组。")
    return records


def _retry(call: Callable[[], Any], interface: str, attempts: int = 2) -> Any:
    last_error: Exception | None = None
    for attempt in range(attempts):
        try:
            return call()
        except Exception as exc:
            last_error = exc
            if attempt + 1 < attempts:
                time.sleep(0.1 * (attempt + 1))
    raise RuntimeError(f"{interface} 接口在 {attempts} 次尝试后失败：{last_error}") from last_error


def _cache_directory() -> Path:
    explicit = str(os.environ.get("CHINA_STOCK_CACHE_DIR") or "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    data_root = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data")).expanduser().resolve()
    return (data_root / "china-stock-analysis" / "cache").resolve()


def get_cache_path(code: str, data_type: str) -> Path:
    code = _validate_code(code)
    if data_type not in DATA_TYPES:
        raise ValueError(f"不支持的数据类型：{data_type}")
    directory = _cache_directory()
    directory.mkdir(parents=True, exist_ok=True)
    return directory / f"{code}_{data_type}_{datetime.now().strftime('%Y%m%d')}.json"


def load_cache(code: str, data_type: str) -> dict[str, Any] | None:
    path = get_cache_path(code, data_type)
    if not path.is_file() or path.stat().st_size > MAX_CACHE_BYTES:
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict) or payload.get("completeness", {}).get("status") != "complete":
        return None
    payload = dict(payload)
    payload["cache"] = {"hit": True, "path": str(path)}
    return payload


def save_cache(code: str, data_type: str, payload: dict[str, Any]) -> None:
    if payload.get("completeness", {}).get("status") != "complete":
        return
    path = get_cache_path(code, data_type)
    temporary = path.with_suffix(path.suffix + ".tmp")
    try:
        temporary.write_text(json.dumps(payload, ensure_ascii=False, default=str), encoding="utf-8")
        temporary.replace(path)
    except OSError as exc:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise RuntimeError(f"缓存写入失败：{exc}") from exc


def get_stock_info(code: str, provider: Any) -> dict[str, Any]:
    interface = "akshare.stock_individual_info_em"
    frame = _retry(lambda: provider.stock_individual_info_em(symbol=code), interface)
    rows = _records(frame)
    if not rows:
        raise RuntimeError(f"{interface} 没有返回记录。")
    info = {str(row.get("item", "")): row.get("value") for row in rows}
    return {
        "code": code,
        "name": str(info.get("股票简称") or ""),
        "industry": str(info.get("行业") or ""),
        "market_cap": safe_float(info.get("总市值")),
        "float_cap": safe_float(info.get("流通市值")),
        "total_shares": safe_float(info.get("总股本")),
        "float_shares": safe_float(info.get("流通股")),
        "pe_ttm": safe_float(info.get("市盈率(动态)")),
        "pb": safe_float(info.get("市净率")),
        "listing_date": str(info.get("上市时间") or ""),
        "source_interface": interface,
    }


def get_financial_data(code: str, years: int, provider: Any) -> tuple[dict[str, Any], list[dict[str, str]]]:
    max_records = min(years * 4, 40)
    configurations = (
        ("balance_sheet", "stock_balance_sheet_by_report_em"),
        ("income_statement", "stock_profit_sheet_by_report_em"),
        ("cash_flow", "stock_cash_flow_sheet_by_report_em"),
    )
    data: dict[str, Any] = {}
    errors: list[dict[str, str]] = []
    for key, method_name in configurations:
        interface = f"akshare.{method_name}"
        try:
            method = getattr(provider, method_name)
            rows = _records(_retry(lambda method=method: method(symbol=code), interface), limit=max_records)
            if not rows:
                raise RuntimeError(f"{interface} 没有返回记录。")
            data[key] = rows
            data[f"{key}_source_interface"] = interface
        except (AttributeError, RuntimeError) as exc:
            errors.append({"component": key, "error": str(exc)})
    return data, errors


def get_financial_indicators(code: str, provider: Any, limit: int = 8) -> dict[str, Any]:
    failures: list[str] = []
    for method_name in ("stock_financial_abstract", "stock_financial_analysis_indicator"):
        interface = f"akshare.{method_name}"
        try:
            method = getattr(provider, method_name)
            rows = _records(_retry(lambda method=method: method(symbol=code), interface), limit=limit)
            if rows:
                return {"records": rows, "source_interface": interface}
            failures.append(f"{interface} 没有返回记录")
        except (AttributeError, RuntimeError) as exc:
            failures.append(str(exc))
    raise RuntimeError("财务指标接口均不可用：" + "；".join(failures))


def get_valuation_data(code: str, provider: Any) -> dict[str, Any]:
    interface = "akshare.stock_a_ttm_lyr"
    frame = _retry(lambda: provider.stock_a_ttm_lyr(symbol=code), interface)
    rows = _records(frame)
    if not rows:
        raise RuntimeError(f"{interface} 没有返回记录。")
    latest = rows[-1]
    result: dict[str, Any] = {
        "latest": latest,
        "history_count": len(rows),
        "source_interface": interface,
    }
    columns = getattr(frame, "columns", [])
    for field in ("pe_ttm", "pb"):
        if field not in columns:
            continue
        values = [safe_float(row.get(field)) for row in rows]
        values = [value for value in values if value is not None]
        current = safe_float(latest.get(field))
        if values and current is not None:
            result[f"{field}_percentile"] = round(sum(value < current for value in values) / len(values) * 100, 6)
    return result


def get_holder_data(code: str, provider: Any) -> tuple[dict[str, Any], list[dict[str, str]]]:
    configurations = (
        ("top_10_holders", "stock_gdfx_top_10_em", 10),
        ("holder_count_history", "stock_zh_a_gdhs", 10),
    )
    data: dict[str, Any] = {}
    errors: list[dict[str, str]] = []
    for key, method_name, limit in configurations:
        interface = f"akshare.{method_name}"
        try:
            method = getattr(provider, method_name)
            rows = _records(_retry(lambda method=method: method(symbol=code), interface), limit=limit)
            if not rows:
                raise RuntimeError(f"{interface} 没有返回记录。")
            data[key] = rows
            data[f"{key}_source_interface"] = interface
        except (AttributeError, RuntimeError) as exc:
            errors.append({"component": key, "error": str(exc)})
    return data, errors


def get_dividend_data(code: str, provider: Any) -> dict[str, Any]:
    failures: list[str] = []
    for method_name, kwargs in (
        ("stock_dividend_cninfo", {"symbol": code}),
        ("stock_history_dividend_detail", {"symbol": code, "indicator": "分红"}),
    ):
        interface = f"akshare.{method_name}"
        try:
            method = getattr(provider, method_name)
            rows = _records(_retry(lambda method=method, kwargs=kwargs: method(**kwargs), interface))
            return {
                "dividend_history": rows,
                "dividend_count": len(rows),
                "source_interface": interface,
                "provider_returned_empty": not rows,
            }
        except (AttributeError, RuntimeError) as exc:
            failures.append(str(exc))
    raise RuntimeError("分红接口均不可用：" + "；".join(failures))


def get_price_data(code: str, days: int, provider: Any) -> dict[str, Any]:
    interface = "akshare.stock_zh_a_hist"
    end_date = datetime.now().strftime("%Y%m%d")
    start_date = (datetime.now() - timedelta(days=days)).strftime("%Y%m%d")
    frame = _retry(
        lambda: provider.stock_zh_a_hist(
            symbol=code,
            period="daily",
            start_date=start_date,
            end_date=end_date,
            adjust="qfq",
        ),
        interface,
    )
    rows = _records(frame)
    if not rows:
        raise RuntimeError(f"{interface} 没有返回记录。")
    required = {"日期", "收盘", "最高", "最低", "成交量", "成交额"}
    missing = sorted(required - set(rows[-1]))
    if missing:
        raise RuntimeError(f"{interface} 缺少字段：{', '.join(missing)}")
    latest = rows[-1]
    highs = [safe_float(row.get("最高")) for row in rows]
    lows = [safe_float(row.get("最低")) for row in rows]
    volumes = [safe_float(row.get("成交量")) for row in rows[-20:]]
    highs = [value for value in highs if value is not None]
    lows = [value for value in lows if value is not None]
    volumes = [value for value in volumes if value is not None]
    return {
        "latest_price": safe_float(latest.get("收盘")),
        "latest_date": str(latest.get("日期")),
        "price_change_percent": safe_float(latest.get("涨跌幅")),
        "volume": safe_float(latest.get("成交量")),
        "turnover": safe_float(latest.get("成交额")),
        "sample_high": max(highs) if highs else None,
        "sample_low": min(lows) if lows else None,
        "average_volume_last_20_records": sum(volumes) / len(volumes) if volumes else None,
        "price_data": rows[-30:],
        "adjustment": "qfq",
        "requested_calendar_days": days,
        "source_interface": interface,
    }


def get_index_constituents(index_name: str, provider: Any) -> list[str]:
    if index_name not in INDEX_CODES:
        raise ValueError(f"不支持的指数范围：{index_name}")
    interface = "akshare.index_stock_cons"
    frame = _retry(lambda: provider.index_stock_cons(symbol=INDEX_CODES[index_name]), interface)
    rows = _records(frame)
    codes = [str(row.get("品种代码") or "").strip() for row in rows]
    codes = [code for code in codes if re.fullmatch(r"\d{6}", code)]
    if not codes:
        raise RuntimeError(f"{interface} 没有返回有效成分股代码。")
    return codes


def get_all_a_stocks(provider: Any) -> list[str]:
    interface = "akshare.stock_zh_a_spot_em"
    rows = _records(_retry(provider.stock_zh_a_spot_em, interface))
    codes = [str(row.get("代码") or "").strip() for row in rows]
    codes = [code for code in codes if re.fullmatch(r"\d{6}", code)]
    if not codes:
        raise RuntimeError(f"{interface} 没有返回有效股票代码。")
    return codes


def _requested_components(data_type: str) -> list[str]:
    mapping = {
        "basic": ["basic_info"],
        "financial": ["financial_data", "financial_indicators"],
        "valuation": ["valuation", "price"],
        "holder": ["holder", "dividend"],
    }
    if data_type == "all":
        return [component for key in ("basic", "financial", "valuation", "holder") for component in mapping[key]]
    return mapping[data_type]


def fetch_stock_data(
    code: str,
    data_type: str = "all",
    years: int = 3,
    use_cache: bool = True,
    *,
    provider: Any | None = None,
) -> dict[str, Any]:
    code = _validate_code(code)
    if data_type not in DATA_TYPES:
        raise ValueError(f"不支持的数据类型：{data_type}")
    if not 1 <= years <= 10:
        raise ValueError("years 必须在 1–10 之间。")
    if use_cache:
        cached = load_cache(code, data_type)
        if cached is not None:
            return cached
    provider = provider or _provider()
    requested = _requested_components(data_type)
    interfaces: set[str] = set()
    errors: list[dict[str, str]] = []
    result: dict[str, Any] = {
        "schema_version": 2,
        "code": code,
        "data_type": data_type,
        "retrieved_at_utc": _utc_now(),
        "provider": "AkShare",
        "provider_version": str(getattr(provider, "__version__", "unknown")),
        "requested_components": requested,
        "cache": {"hit": False},
    }

    def capture(component: str, call: Callable[[], Any]) -> None:
        try:
            value = call()
            result[component] = value
            if isinstance(value, dict):
                for key, item in value.items():
                    if key.endswith("source_interface") and isinstance(item, str):
                        interfaces.add(item)
        except RuntimeError as exc:
            errors.append({"component": component, "error": str(exc)})

    if "basic_info" in requested:
        capture("basic_info", lambda: get_stock_info(code, provider))
    if "financial_data" in requested:
        financial_data, component_errors = get_financial_data(code, years, provider)
        if financial_data:
            result["financial_data"] = financial_data
            interfaces.update(value for key, value in financial_data.items() if key.endswith("source_interface"))
        errors.extend({"component": f"financial_data.{item['component']}", "error": item["error"]} for item in component_errors)
        if not financial_data:
            errors.append({"component": "financial_data", "error": "没有任何财务报表可用。"})
    if "financial_indicators" in requested:
        capture("financial_indicators", lambda: get_financial_indicators(code, provider))
    if "valuation" in requested:
        capture("valuation", lambda: get_valuation_data(code, provider))
    if "price" in requested:
        capture("price", lambda: get_price_data(code, 60, provider))
    if "holder" in requested:
        holder, component_errors = get_holder_data(code, provider)
        if holder:
            result["holder"] = holder
            interfaces.update(value for key, value in holder.items() if key.endswith("source_interface"))
        errors.extend({"component": f"holder.{item['component']}", "error": item["error"]} for item in component_errors)
        if not holder:
            errors.append({"component": "holder", "error": "没有任何股东数据可用。"})
    if "dividend" in requested:
        capture("dividend", lambda: get_dividend_data(code, provider))

    successful = [component for component in requested if component in result]
    status = "complete" if len(successful) == len(requested) and not errors else "partial" if successful else "failed"
    result["sources"] = [f"AkShare {interface}" for interface in sorted(interfaces)]
    result["completeness"] = {
        "status": status,
        "successful_components": successful,
        "errors": errors,
    }
    if use_cache and status == "complete":
        save_cache(code, data_type, result)
    return result


def fetch_multiple_stocks(
    codes: list[str],
    data_type: str = "basic",
    years: int = 3,
    use_cache: bool = True,
    *,
    provider: Any | None = None,
) -> dict[str, Any]:
    if not codes or len(codes) > MAX_CODES:
        raise ValueError(f"一次必须请求 1–{MAX_CODES} 只股票。")
    normalized = [_validate_code(code) for code in codes]
    if len(set(normalized)) != len(normalized):
        raise ValueError("codes 包含重复股票代码。")
    provider = provider or _provider()
    stocks = [
        fetch_stock_data(code, data_type, years, use_cache, provider=provider)
        for code in normalized
    ]
    failed = [item for item in stocks if item["completeness"]["status"] == "failed"]
    partial = [item for item in stocks if item["completeness"]["status"] == "partial"]
    status = "complete" if not failed and not partial else "partial" if len(failed) < len(stocks) else "failed"
    return {
        "schema_version": 2,
        "retrieved_at_utc": _utc_now(),
        "data_type": data_type,
        "stocks": stocks,
        "completeness": {
            "status": status,
            "complete_count": sum(item["completeness"]["status"] == "complete" for item in stocks),
            "partial_count": len(partial),
            "failed_count": len(failed),
        },
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A 股可追溯数据获取工具")
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--code", help="六位股票代码")
    target.add_argument("--codes", help="逗号分隔的多个六位股票代码")
    target.add_argument("--scope", choices=[*INDEX_CODES, "all"], help="指数范围或全部 A 股代码")
    parser.add_argument("--data-type", choices=DATA_TYPES, default="basic", help="请求的数据组件")
    parser.add_argument("--years", type=int, default=3, help="财务报表年数（1–10）")
    parser.add_argument("--no-cache", action="store_true", help="不读取或写入当日缓存")
    parser.add_argument("--allow-partial", action="store_true", help="显式允许部分组件失败时返回退出码 0")
    parser.add_argument("--output", help="输出 JSON 文件")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        provider = _provider()
        if args.code:
            result = fetch_stock_data(
                args.code,
                args.data_type,
                args.years,
                not args.no_cache,
                provider=provider,
            )
        elif args.codes:
            codes = [item.strip() for item in args.codes.split(",") if item.strip()]
            result = fetch_multiple_stocks(
                codes,
                args.data_type,
                args.years,
                not args.no_cache,
                provider=provider,
            )
        else:
            codes = get_all_a_stocks(provider) if args.scope == "all" else get_index_constituents(args.scope, provider)
            result = {
                "schema_version": 2,
                "scope": args.scope,
                "stocks": codes,
                "count": len(codes),
                "retrieved_at_utc": _utc_now(),
                "provider": "AkShare",
                "source_interface": (
                    "akshare.stock_zh_a_spot_em" if args.scope == "all" else "akshare.index_stock_cons"
                ),
                "completeness": {"status": "complete", "errors": []},
            }
        output = json.dumps(result, ensure_ascii=False, indent=2, default=str)
        if args.output:
            output_path = Path(args.output).expanduser().resolve()
            output_path.write_text(output + "\n", encoding="utf-8")
            print(f"数据结果已保存到：{output_path}")
        else:
            print(output)
        status = result.get("completeness", {}).get("status", "failed")
        return 0 if status == "complete" or (args.allow_partial and status == "partial") else 1
    except (OSError, RuntimeError, ValueError) as exc:
        print(json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
