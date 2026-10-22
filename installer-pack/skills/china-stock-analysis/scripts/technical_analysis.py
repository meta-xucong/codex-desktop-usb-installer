#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = ["akshare"]
# ///
"""
A股技术分析器

计算K线技术指标，识别技术形态

Usage:
    python technical_analysis.py --code 000001 --period daily
"""

import argparse
import json
import re
import sys
from datetime import datetime, timedelta
from typing import Dict, List, Tuple


def calculate_ma(prices: List[float], period: int) -> List[float]:
    """计算移动平均线"""
    if len(prices) < period:
        return []
    
    ma = []
    for i in range(len(prices)):
        if i < period - 1:
            ma.append(None)
        else:
            ma.append(sum(prices[i-period+1:i+1]) / period)
    return ma


def calculate_ema(prices: List[float], period: int) -> List[float]:
    """计算指数移动平均线"""
    if len(prices) < period:
        return []
    
    multiplier = 2 / (period + 1)
    ema = [sum(prices[:period]) / period]  # 初始值用SMA
    
    for i in range(period, len(prices)):
        ema.append((prices[i] - ema[-1]) * multiplier + ema[-1])
    
    # 补齐前面的None
    return [None] * (period - 1) + ema


def calculate_macd(prices: List[float]) -> Dict:
    """计算MACD指标"""
    ema12 = calculate_ema(prices, 12)
    ema26 = calculate_ema(prices, 26)
    
    # DIF = EMA12 - EMA26
    dif = []
    for i in range(len(prices)):
        if ema12[i] is None or ema26[i] is None:
            dif.append(None)
        else:
            dif.append(ema12[i] - ema26[i])
    
    # DEA = EMA(DIF, 9)
    valid_dif = [d for d in dif if d is not None]
    dea_values = calculate_ema(valid_dif, 9) if len(valid_dif) >= 9 else []
    
    # 补齐dea
    dea = [None] * (len(prices) - len(dea_values)) + dea_values if dea_values else [None] * len(prices)
    
    # MACD = 2 * (DIF - DEA)
    macd = []
    for i in range(len(prices)):
        if dif[i] is not None and len(dea) > i and dea[i] is not None:
            macd.append(2 * (dif[i] - dea[i]))
        else:
            macd.append(None)
    
    return {
        "dif": dif[-1] if dif and dif[-1] is not None else 0,
        "dea": dea[-1] if dea and dea[-1] is not None else 0,
        "macd": macd[-1] if macd and macd[-1] is not None else 0,
        "signal": "金叉" if dif[-1] > dea[-1] and dif[-2] <= dea[-2] else \
                 "死叉" if dif[-1] < dea[-1] and dif[-2] >= dea[-2] else "无"
    }


def calculate_rsi(prices: List[float], period: int = 14) -> float:
    """计算RSI指标"""
    if len(prices) < period + 1:
        return 50.0
    
    gains = []
    losses = []
    
    for i in range(1, len(prices)):
        change = prices[i] - prices[i-1]
        if change > 0:
            gains.append(change)
            losses.append(0)
        else:
            gains.append(0)
            losses.append(abs(change))
    
    if len(gains) < period:
        return 50.0
    
    avg_gain = sum(gains[-period:]) / period
    avg_loss = sum(losses[-period:]) / period
    
    if avg_loss == 0:
        return 100.0
    
    rs = avg_gain / avg_loss
    rsi = 100 - (100 / (1 + rs))
    
    return round(rsi, 2)


def calculate_kdj(highs: List[float], lows: List[float], closes: List[float], 
                  n: int = 9, m1: int = 3, m2: int = 3) -> Dict:
    """计算KDJ指标"""
    if len(closes) < n:
        return {"k": 50, "d": 50, "j": 50}
    
    # RSV
    rsv_list = []
    for i in range(n - 1, len(closes)):
        period_high = max(highs[i-n+1:i+1])
        period_low = min(lows[i-n+1:i+1])
        if period_high == period_low:
            rsv = 50
        else:
            rsv = 100 * (closes[i] - period_low) / (period_high - period_low)
        rsv_list.append(rsv)
    
    # K, D, J
    k = 50
    d = 50
    
    for rsv in rsv_list:
        k = (2 * k + rsv) / 3
        d = (2 * d + k) / 3
    
    j = 3 * k - 2 * d
    
    return {
        "k": round(k, 2),
        "d": round(d, 2),
        "j": round(j, 2),
        "signal": "金叉" if k > d and k <= 20 else \
                 "死叉" if k < d and k >= 80 else "无"
    }


def calculate_bollinger(prices: List[float], period: int = 20, std_dev: int = 2) -> Dict:
    """计算布林带"""
    if len(prices) < period:
        return {"upper": 0, "middle": 0, "lower": 0, "position": "unknown"}
    
    recent_prices = prices[-period:]
    middle = sum(recent_prices) / period
    variance = sum((p - middle) ** 2 for p in recent_prices) / period
    std = variance ** 0.5
    
    upper = middle + std_dev * std
    lower = middle - std_dev * std
    
    current_price = prices[-1]
    
    if current_price > upper:
        position = "上轨上方"
    elif current_price < lower:
        position = "下轨下方"
    else:
        position = "中轨附近"
    
    return {
        "upper": round(upper, 2),
        "middle": round(middle, 2),
        "lower": round(lower, 2),
        "position": position,
        "bandwidth": round((upper - lower) / middle * 100, 2)
    }


def identify_trend(prices: List[float], ma5: List[float], ma20: List[float]) -> Dict:
    """识别趋势"""
    if len(prices) < 20 or not ma5[-1] or not ma20[-1]:
        return {"trend": "unknown", "strength": 0}
    
    # 短期vs长期均线
    short_above_long = ma5[-1] > ma20[-1]
    
    # 价格vs均线
    price_above_ma20 = prices[-1] > ma20[-1]
    
    # 均线方向
    ma5_rising = ma5[-1] > ma5[-5] if len(ma5) >= 5 and ma5[-5] else False
    ma20_rising = ma20[-1] > ma20[-10] if len(ma20) >= 10 and ma20[-10] else False
    
    if short_above_long and price_above_ma20 and ma5_rising and ma20_rising:
        trend = "上升趋势"
        strength = 3
    elif not short_above_long and not price_above_ma20 and not ma5_rising and not ma20_rising:
        trend = "下降趋势"
        strength = -3
    elif short_above_long and price_above_ma20:
        trend = "偏多震荡"
        strength = 1
    elif not short_above_long and not price_above_ma20:
        trend = "偏空震荡"
        strength = -1
    else:
        trend = "横盘整理"
        strength = 0
    
    return {"trend": trend, "strength": strength}


def identify_support_resistance(prices: List[float], highs: List[float], 
                                 lows: List[float], period: int = 20) -> Dict:
    """识别支撑阻力位"""
    if len(prices) < period:
        return {"support": 0, "resistance": 0}
    
    recent_highs = highs[-period:]
    recent_lows = lows[-period:]
    
    resistance = max(recent_highs)
    support = min(recent_lows)
    
    current = prices[-1]
    
    # 计算距离
    dist_to_support = (current - support) / current * 100 if support > 0 else 0
    dist_to_resistance = (resistance - current) / current * 100 if resistance > 0 else 0
    
    return {
        "support": round(support, 2),
        "resistance": round(resistance, 2),
        "dist_to_support": round(dist_to_support, 2),
        "dist_to_resistance": round(dist_to_resistance, 2),
        "window": period,
        "notice": "support/resistance 仅表示窗口最低/最高价，不是交易触发位。"
    }


def generate_technical_analysis(code: str, prices: List[float], 
                                highs: List[float], lows: List[float],
                                volumes: List[float], period: str) -> Dict:
    """生成技术分析报告"""
    
    # 计算指标
    ma5 = calculate_ma(prices, 5)
    ma10 = calculate_ma(prices, 10)
    ma20 = calculate_ma(prices, 20)
    ma60 = calculate_ma(prices, 60)
    
    macd = calculate_macd(prices)
    rsi = calculate_rsi(prices)
    kdj = calculate_kdj(highs, lows, prices)
    boll = calculate_bollinger(prices)
    trend = identify_trend(prices, ma5, ma20)
    sr = identify_support_resistance(prices, highs, lows)
    
    return {
        "analyzed_at": datetime.now().isoformat(),
        "code": code,
        "period": period,
        "current_price": prices[-1] if prices else 0,
        "moving_averages": {
            "ma5": round(ma5[-1], 2) if ma5 and ma5[-1] else None,
            "ma10": round(ma10[-1], 2) if ma10 and ma10[-1] else None,
            "ma20": round(ma20[-1], 2) if ma20 and ma20[-1] else None,
            "ma60": round(ma60[-1], 2) if ma60 and ma60[-1] else None
        },
        "indicators": {
            "macd": macd,
            "rsi": rsi,
            "kdj": kdj,
            "bollinger": boll
        },
        "trend": trend,
        "support_resistance": sr,
        "rule_observations": generate_rule_observations(macd, rsi, kdj, trend, boll),
        "methodology": "指标使用固定公开算式；规则观察只描述当前样本条件，不评分、不预测价格、不生成买卖或止损建议。"
    }


def generate_rule_observations(macd: Dict, rsi: float, kdj: Dict, trend: Dict, boll: Dict) -> Dict:
    """Expose rule conditions without translating them into an action or forecast."""
    return {
        "macd_latest_cross": macd.get("signal", "无"),
        "rsi_below_30": rsi < 30,
        "rsi_above_70": rsi > 70,
        "kdj_latest_cross": kdj.get("signal", "无"),
        "moving_average_rule": trend.get("trend", "unknown"),
        "bollinger_position": boll.get("position", "unknown"),
        "thresholds": {"rsi_low": 30, "rsi_high": 70},
        "notice": "这些是规则条件是否成立的记录，不代表未来方向或交易信号。",
    }


def format_report(report: Dict) -> str:
    """格式化中性的技术指标报告。"""
    
    ma = report["moving_averages"]
    ind = report["indicators"]
    lines = [
        "=" * 60,
        f"技术指标报告 - {report['code']}",
        "=" * 60,
        f"【当前价格】{report['current_price']:.2f}",
        f"【均线规则状态】{report['trend']['trend']}",
        "",
        "【均线系统】",
        f"• MA5: {ma['ma5']:.2f}" if ma['ma5'] else "• MA5: N/A",
        f"• MA10: {ma['ma10']:.2f}" if ma['ma10'] else "• MA10: N/A",
        f"• MA20: {ma['ma20']:.2f}" if ma['ma20'] else "• MA20: N/A",
        f"• MA60: {ma['ma60']:.2f}" if ma['ma60'] else "• MA60: N/A",
        "",
        "【技术指标】",
        f"• MACD: DIF={ind['macd']['dif']:.2f}, DEA={ind['macd']['dea']:.2f}, "
        f"MACD={ind['macd']['macd']:.2f} ({ind['macd']['signal']})",
        f"• RSI: {ind['rsi']:.2f}",
        f"• KDJ: K={ind['kdj']['k']:.2f}, D={ind['kdj']['d']:.2f}, J={ind['kdj']['j']:.2f}",
        f"• 布林带: 上轨{ind['bollinger']['upper']}, 中轨{ind['bollinger']['middle']}, "
        f"下轨{ind['bollinger']['lower']} ({ind['bollinger']['position']})",
        "",
        "【支撑阻力】",
        f"• 支撑位: {report['support_resistance']['support']:.2f} "
        f"(距离{report['support_resistance']['dist_to_support']:.1f}%)",
        f"• 阻力位: {report['support_resistance']['resistance']:.2f} "
        f"(距离{report['support_resistance']['dist_to_resistance']:.1f}%)",
        "",
        "【规则条件】",
    ]
    for key, value in report["rule_observations"].items():
        if key != "notice":
            lines.append(f"• {key}: {value}")
    lines.extend([
        "",
        f"规则边界：{report['rule_observations']['notice']}",
        f"方法边界：{report['methodology']}",
        "",
        "=" * 60,
        f"数据来源：{report.get('data_source', '未记录')}",
        f"数据时点：{report.get('data_as_of', '未记录')}",
        f"复权方式：{report.get('price_adjustment', '未记录')}；样本数：{report.get('observations', 0)}",
        f"分析时间：{report['analyzed_at']}",
        "=" * 60
    ])
    
    return "\n".join(lines)


def fetch_market_data(
    code: str,
    period: str,
    days: int,
) -> Tuple[List[float], List[float], List[float], List[float], str]:
    """从 AkShare 获取真实 K 线；缺少依赖或数据时明确失败。"""
    if not re.fullmatch(r"\d{6}", str(code or "")):
        raise ValueError("股票代码必须是 6 位数字。")
    if period not in {"daily", "weekly", "monthly"}:
        raise ValueError("period 必须是 daily、weekly 或 monthly。")
    if not isinstance(days, int) or not 60 <= days <= 2000:
        raise ValueError("days 必须是 60 到 2000 的整数，以保证指标有足够样本。")
    try:
        import akshare as ak
    except ImportError as error:
        raise RuntimeError("缺少 akshare；请先完成该 Skill 的 Python 运行时安装。") from error

    calendar_multiplier = {"daily": 3, "weekly": 10, "monthly": 40}[period]
    end_date = datetime.now().strftime("%Y%m%d")
    start_date = (datetime.now() - timedelta(days=max(days * calendar_multiplier, 180))).strftime("%Y%m%d")
    frame = ak.stock_zh_a_hist(
        symbol=code,
        period=period,
        start_date=start_date,
        end_date=end_date,
        adjust="qfq",
    )
    if frame is None or frame.empty:
        raise RuntimeError(f"AkShare 未返回 {code} 的 {period} K 线数据。")
    required_columns = {"日期", "收盘", "最高", "最低", "成交量"}
    missing_columns = required_columns.difference(frame.columns)
    if missing_columns:
        raise RuntimeError(f"AkShare K 线缺少字段：{sorted(missing_columns)}")
    frame = frame.tail(days)
    if len(frame) < 60:
        raise RuntimeError(f"有效 K 线只有 {len(frame)} 条，至少需要 60 条。")

    prices = [float(value) for value in frame["收盘"].tolist()]
    highs = [float(value) for value in frame["最高"].tolist()]
    lows = [float(value) for value in frame["最低"].tolist()]
    volumes = [float(value) for value in frame["成交量"].tolist()]
    as_of = str(frame["日期"].tolist()[-1])
    return prices, highs, lows, volumes, as_of


def main() -> int:
    parser = argparse.ArgumentParser(description="A股技术分析器")
    parser.add_argument("--code", type=str, required=True,
                       help="股票代码")
    parser.add_argument("--period", type=str, default="daily",
                       choices=["daily", "weekly", "monthly"],
                       help="分析周期")
    parser.add_argument("--days", type=int, default=100,
                       help="分析天数")
    parser.add_argument("--output", type=str, help="输出JSON文件")
    parser.add_argument("--json", action="store_true", help="JSON格式输出")
    
    args = parser.parse_args()
    
    try:
        prices, highs, lows, volumes, data_as_of = fetch_market_data(args.code, args.period, args.days)
        
        # 生成分析
        report = generate_technical_analysis(
            args.code, prices, highs, lows, volumes, args.period
        )
        report["data_source"] = "AkShare stock_zh_a_hist"
        report["data_as_of"] = data_as_of
        report["price_adjustment"] = "qfq"
        report["observations"] = len(prices)
        
        if args.json or args.output:
            output = json.dumps(report, ensure_ascii=False, indent=2)
            if args.output:
                with open(args.output, 'w', encoding='utf-8') as f:
                    f.write(output)
                print(f"报告已保存到: {args.output}")
            else:
                print(output)
        else:
            print(format_report(report))
        return 0
    except Exception as e:
        error = {"error": str(e), "code": args.code, "period": args.period}
        if args.json:
            print(json.dumps(error, ensure_ascii=False))
        else:
            print(f"分析失败: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
