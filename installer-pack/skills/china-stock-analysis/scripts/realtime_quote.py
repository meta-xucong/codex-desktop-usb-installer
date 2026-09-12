#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Fetch A-share quotes from Sina and report observations without trading inference."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
import urllib.request
from datetime import datetime, timezone
from typing import Any, Callable


SOURCE_NAME = "Sina Finance quote interface"
REALTIME_SOURCE_INTERFACE = "https://hq.sinajs.cn/list=<symbols>"
MINUTE_SOURCE_INTERFACE = "CN_MarketDataService.getKLineData"
MAX_CODES = 100


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def get_sina_symbol(raw_code: str) -> str:
    """Normalize an explicit six-digit A-share code to the provider symbol."""
    value = str(raw_code or "").strip().upper()
    match = re.fullmatch(r"(?:(SH|SZ|BJ))?(\d{6})(?:\.(SH|SZ|BJ))?", value)
    if not match:
        raise ValueError(f"股票代码必须是六位数字或带 SH/SZ/BJ 后缀：{raw_code!r}")
    prefix, code, suffix = match.groups()
    explicit_exchange = prefix or suffix
    inferred_exchange = (
        "SH"
        if code.startswith("6")
        else "SZ"
        if code.startswith(("0", "3"))
        else "BJ"
        if code.startswith(("4", "8"))
        else None
    )
    if inferred_exchange is None:
        raise ValueError(f"无法从代码识别沪、深或北交所：{code}")
    if explicit_exchange and explicit_exchange != inferred_exchange:
        raise ValueError(f"股票代码与交易所前后缀不一致：{raw_code!r}")
    return inferred_exchange.lower() + code


def _number(
    value: str,
    field: str,
    *,
    integer: bool = False,
    allow_empty: bool = True,
) -> float | int | None:
    if value == "" and allow_empty:
        return None
    try:
        parsed = float(value)
    except (TypeError, ValueError) as exc:
        raise RuntimeError(f"实时行情字段 {field} 不是数值。") from exc
    if not math.isfinite(parsed):
        raise RuntimeError(f"实时行情字段 {field} 不是有限数值。")
    return int(parsed) if integer else parsed


def fetch_realtime_sina(
    symbols: list[str],
    *,
    opener: Callable[..., Any] = urllib.request.urlopen,
) -> dict[str, dict[str, Any]]:
    """Fetch a batch and return only rows that are structurally valid."""
    if not symbols or len(symbols) > MAX_CODES:
        raise ValueError(f"一次必须请求 1–{MAX_CODES} 只股票。")
    if any(not re.fullmatch(r"(?:sh|sz|bj)\d{6}", symbol) for symbol in symbols):
        raise ValueError("symbols 包含未规范化代码。")

    endpoint = "https://hq.sinajs.cn/list=" + ",".join(symbols)
    request = urllib.request.Request(
        endpoint,
        headers={
            "Referer": "https://finance.sina.com.cn",
            "User-Agent": "codex-starter-kit/1.2",
        },
    )
    try:
        response = opener(request, timeout=10)
        payload = response.read().decode("gbk")
    except Exception as exc:
        raise RuntimeError(f"新浪实时行情接口失败：{exc}") from exc

    retrieved_at = _utc_now()
    result: dict[str, dict[str, Any]] = {}
    for line in payload.splitlines():
        match = re.match(r'^var\s+hq_str_((?:sh|sz|bj)\d{6})="([^"]*)"', line.strip())
        if not match or not match.group(2):
            continue
        symbol, data_text = match.groups()
        fields = data_text.split(",")
        if len(fields) < 32:
            continue
        try:
            pre_close = _number(fields[2], "pre_close")
            price = _number(fields[3], "price")
            if price is None or price <= 0:
                continue
            change_amount = price - pre_close if pre_close is not None and pre_close > 0 else None
            change_percent = change_amount / pre_close * 100 if change_amount is not None else None
            market_date = fields[30].strip()
            market_time = fields[31].strip()
            result[symbol] = {
                "code": symbol[2:],
                "exchange": symbol[:2].upper(),
                "name": fields[0].strip(),
                "price": price,
                "open": _number(fields[1], "open"),
                "pre_close": pre_close,
                "high": _number(fields[4], "high"),
                "low": _number(fields[5], "low"),
                "bid_1": _number(fields[6], "bid_1"),
                "ask_1": _number(fields[7], "ask_1"),
                "volume_shares": _number(fields[8], "volume", integer=True, allow_empty=False),
                "amount_cny": _number(fields[9], "amount", allow_empty=False),
                "change_amount": round(change_amount, 4) if change_amount is not None else None,
                "change_percent": round(change_percent, 4) if change_percent is not None else None,
                "market_timestamp": " ".join(part for part in (market_date, market_time) if part),
                "retrieved_at_utc": retrieved_at,
                "source_name": SOURCE_NAME,
                "source_interface": REALTIME_SOURCE_INTERFACE,
            }
        except RuntimeError:
            continue
    return result


def fetch_minute_data_sina(
    symbol: str,
    count: int = 250,
    *,
    opener: Callable[..., Any] = urllib.request.urlopen,
) -> list[dict[str, Any]]:
    """Fetch one-minute bars from the provider interface used by this adapter."""
    if not re.fullmatch(r"(?:sh|sz|bj)\d{6}", symbol):
        raise ValueError("symbol 不是规范化的新浪股票代码。")
    if not 1 <= count <= 2000:
        raise ValueError("count 必须在 1–2000 之间。")
    endpoint = (
        "https://quotes.sina.cn/cn/api/jsonp_v2.php/var%20_=/"
        "CN_MarketDataService.getKLineData"
        f"?symbol={symbol}&scale=1&ma=no&datalen={count}"
    )
    request = urllib.request.Request(
        endpoint,
        headers={"Referer": "https://finance.sina.com.cn", "User-Agent": "codex-starter-kit/1.2"},
    )
    try:
        response = opener(request, timeout=10)
        text = response.read().decode("utf-8")
        start = text.find("[")
        end = text.rfind("]")
        if start < 0 or end < start:
            raise ValueError("响应中没有 JSON 数组。")
        raw_rows = json.loads(text[start : end + 1])
    except Exception as exc:
        raise RuntimeError(f"新浪分时接口失败：{exc}") from exc
    if not isinstance(raw_rows, list) or not raw_rows:
        raise RuntimeError("新浪分时接口没有返回记录。")

    rows: list[dict[str, Any]] = []
    for raw in raw_rows:
        try:
            row = {
                "time": str(raw["day"]),
                "open": float(raw["open"]),
                "high": float(raw["high"]),
                "low": float(raw["low"]),
                "close": float(raw["close"]),
                "volume_shares": int(float(raw["volume"])),
                "amount_cny": float(raw["amount"]),
            }
        except (KeyError, TypeError, ValueError) as exc:
            raise RuntimeError("新浪分时接口返回了不完整记录。") from exc
        if not all(
            math.isfinite(float(row[key]))
            for key in ("open", "high", "low", "close", "volume_shares", "amount_cny")
        ):
            raise RuntimeError("新浪分时接口返回了非有限数值。")
        rows.append(row)
    return rows


def analyze_minute_volume(minute_data: list[dict[str, Any]]) -> dict[str, Any]:
    """Calculate volume distributions; do not infer participant identity or intent."""
    trading_rows = [
        row
        for row in minute_data
        if row.get("volume_shares", 0) > 0
        and "09:25" <= str(row.get("time", ""))[-8:-3] <= "15:00"
    ]
    if not trading_rows:
        raise ValueError("没有有效交易时段的分时数据。")
    total_volume = sum(int(row["volume_shares"]) for row in trading_rows)
    if total_volume <= 0:
        raise ValueError("分时总成交量必须大于零。")

    def period_volume(start: str, end: str) -> int:
        return sum(
            int(row["volume_shares"])
            for row in trading_rows
            if start <= str(row["time"])[-8:-3] < end
        )

    periods = {
        "open_30min": period_volume("09:30", "10:00"),
        "mid_am": period_volume("10:00", "11:30"),
        "mid_pm": period_volume("13:00", "14:30"),
        "close_30min": period_volume("14:30", "15:01"),
    }
    distribution = {
        key: {"volume_shares": volume, "percent": round(volume / total_volume * 100, 4)}
        for key, volume in periods.items()
    }
    top_rows = sorted(trading_rows, key=lambda row: int(row["volume_shares"]), reverse=True)[:10]
    return {
        "bar_count": len(trading_rows),
        "total_volume_shares": total_volume,
        "total_amount_cny": round(sum(float(row["amount_cny"]) for row in trading_rows), 4),
        "distribution": distribution,
        "highest_volume_bars": [
            {
                "time": row["time"],
                "close": row["close"],
                "volume_shares": row["volume_shares"],
                "amount_cny": row["amount_cny"],
            }
            for row in top_rows
        ],
        "source_interface": MINUTE_SOURCE_INTERFACE,
        "methodology": "仅汇总成交量分布，不推断主力身份、意图、抢筹、出货或买卖信号。",
    }


def analyze_stock(
    code: str,
    with_minute: bool = False,
    realtime_cache: dict[str, dict[str, Any]] | None = None,
) -> dict[str, Any]:
    symbol = get_sina_symbol(code)
    cache = realtime_cache if realtime_cache is not None else fetch_realtime_sina([symbol])
    realtime = cache.get(symbol)
    if realtime is None:
        raise RuntimeError(f"实时接口没有返回 {code} 的有效行情。")
    result: dict[str, Any] = {
        "code": realtime["code"],
        "name": realtime["name"],
        "realtime": realtime,
    }
    if with_minute:
        minute_rows = fetch_minute_data_sina(symbol)
        result["minute_volume"] = analyze_minute_volume(minute_rows)
        result["minute_data_as_of"] = minute_rows[-1]["time"]
    return result


def _format_number(value: Any, decimals: int = 2) -> str:
    return "N/A" if value is None else f"{float(value):,.{decimals}f}"


def format_realtime(data: dict[str, Any]) -> str:
    return "\n".join(
        [
            "=" * 60,
            f"股票：{data['name']} ({data['exchange']}{data['code']})",
            "=" * 60,
            f"现价：{_format_number(data['price'])}  涨跌幅：{_format_number(data['change_percent'], 4)}%",
            f"今开：{_format_number(data['open'])}  最高：{_format_number(data['high'])}  最低：{_format_number(data['low'])}",
            f"昨收：{_format_number(data['pre_close'])}",
            f"成交量：{data['volume_shares']:,} 股  成交额：{_format_number(data['amount_cny'])} 元",
            f"行情时点：{data['market_timestamp'] or '接口未提供'}",
            f"抓取时点（UTC）：{data['retrieved_at_utc']}",
            f"来源：{data['source_name']}；{data['source_interface']}",
        ]
    )


def format_minute_analysis(analysis: dict[str, Any]) -> str:
    lines = [
        "",
        "【分时成交量分布】",
        f"有效记录：{analysis['bar_count']}；总成交量：{analysis['total_volume_shares']:,} 股",
    ]
    for key, value in analysis["distribution"].items():
        lines.append(f"{key}: {value['volume_shares']:,} 股 ({value['percent']}%)")
    lines.append(f"方法边界：{analysis['methodology']}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A 股实时行情与分时成交量观察")
    parser.add_argument("codes", nargs="+", help="六位股票代码，可带 SH/SZ/BJ 前后缀")
    parser.add_argument("--minute", "-m", action="store_true", help="包含一分钟成交量分布")
    parser.add_argument("--json", "-j", action="store_true", help="输出 JSON 包络")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if len(args.codes) > MAX_CODES:
            raise ValueError(f"一次最多查询 {MAX_CODES} 只股票。")
        symbols = [get_sina_symbol(code) for code in args.codes]
        realtime_cache = fetch_realtime_sina(symbols)
    except (ValueError, RuntimeError) as exc:
        error = {"success": False, "results": [], "errors": [{"error": str(exc)}]}
        print(
            json.dumps(error, ensure_ascii=False, indent=2) if args.json else f"行情获取失败：{exc}",
            file=sys.stdout if args.json else sys.stderr,
        )
        return 1

    results: list[dict[str, Any]] = []
    errors: list[dict[str, str]] = []
    for code in args.codes:
        try:
            results.append(analyze_stock(code, with_minute=args.minute, realtime_cache=realtime_cache))
        except (ValueError, RuntimeError) as exc:
            errors.append({"code": code, "error": str(exc)})

    envelope = {
        "success": not errors,
        "retrieved_at_utc": _utc_now(),
        "source_name": SOURCE_NAME,
        "results": results,
        "errors": errors,
        "methodology": "行情仅为接口观测，需核验交易状态与时间；脚本不生成交易结论。",
    }
    if args.json:
        print(json.dumps(envelope, ensure_ascii=False, indent=2))
    else:
        for result in results:
            print(format_realtime(result["realtime"]))
            if "minute_volume" in result:
                print(format_minute_analysis(result["minute_volume"]))
            print()
        for error in errors:
            print(f"{error['code']}：{error['error']}", file=sys.stderr)
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
