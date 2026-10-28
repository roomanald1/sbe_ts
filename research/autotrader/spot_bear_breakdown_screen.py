#!/usr/bin/env python3
"""Mirror the daily breakout as a borrow-costed paper short in BTC bear regimes."""
import json
import math
import statistics
from pathlib import Path

from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "spot_bear_breakdown_results.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")


def stats(returns, entries):
    wealth = peak = 1.0
    drawdown = 0.0
    for value in returns:
        wealth *= 1 + value
        peak = max(peak, wealth)
        drawdown = min(drawdown, wealth / peak - 1)
    mean = statistics.mean(returns) if returns else 0
    stdev = statistics.stdev(returns) if len(returns) > 1 else 0
    return {
        "days": len(returns), "return_percent": 100 * (wealth - 1),
        "max_drawdown_percent": 100 * drawdown,
        "annualized_daily_sharpe": mean / stdev * math.sqrt(365) if stdev else 0,
        "entry_events": entries,
    }


def run(dates, data, lookback, side_cost, annual_borrow=0.05):
    sleeves = {symbol: 1.0 for symbol in SYMBOLS}
    short = {symbol: False for symbol in SYMBOLS}
    rows = []
    for i in range(202, len(dates) - 1):
        day, prior, next_day = dates[i], dates[i - 1], dates[i + 1]
        btc_sma = sum(data["BTCUSDT"][dates[k]]["close"] for k in range(i - 200, i)) / 200
        bear = data["BTCUSDT"][prior]["close"] <= btc_sma
        before = sum(sleeves.values())
        entries = 0
        for symbol in SYMBOLS:
            low = min(data[symbol][dates[k]]["low"] for k in range(i - lookback - 1, i - 1))
            target = bear and data[symbol][prior]["close"] < low
            old = short[symbol]
            entries += int(target and not old)
            close_to_close = data[symbol][next_day]["open"] / data[symbol][day]["open"] - 1
            cashflow = -close_to_close if target else 0.0
            borrow = annual_borrow / 365 if target else 0.0
            sleeves[symbol] *= (1 - side_cost * int(target != old)) * (1 + cashflow - borrow)
            short[symbol] = target
        rows.append({
            "date": day, "return": sum(sleeves.values()) / before - 1,
            "entries": entries,
        })
    return rows


def summarize(rows):
    periods = {
        "train_2021_2022": [row for row in rows if row["date"] < "2023-01-01"],
        "validation_2023_2024": [row for row in rows if "2023-01-01" <= row["date"] < "2025-01-01"],
        "later_2025_2026": [row for row in rows if row["date"] >= "2025-01-01"],
        **{f"year_{year}": [row for row in rows if row["date"].startswith(str(year))]
           for year in range(2021, 2027)},
        "all": rows,
    }
    return {
        key: stats([row["return"] for row in values], sum(row["entries"] for row in values))
        for key, values in periods.items()
    }


def main():
    dates, data = load()
    result = {
        "source": "retained Binance daily spot candles plus frozen 2026-10-28 REST snapshot",
        "date_range": [dates[202], dates[-2]],
        "rule": "short a coin after its prior close falls below its previous 20- or 55-day low, only while BTC prior close is below its trailing SMA200; exit when daily eligibility ends",
        "portfolio": "seven separate equal-initial-capital short sleeves, cash earns zero",
        "borrowing": "5% annual borrowing charge on each open short sleeve",
        "risk_note": "spot shorting requires actual borrow and margin; no intraday liquidation replay",
        "policies": {},
    }
    for lookback in (20, 55):
        policy = f"short{lookback}_btc200"
        result["policies"][policy] = {}
        for basis_points in (15, 30, 50):
            result["policies"][policy][str(basis_points)] = summarize(
                run(dates, data, lookback, basis_points / 10000)
            )
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({
        name: {
            part: {
                "return_percent": result["policies"][name]["15"][part]["return_percent"],
                "return_at_50bp": result["policies"][name]["50"][part]["return_percent"],
                "max_drawdown_percent": result["policies"][name]["15"][part]["max_drawdown_percent"],
            }
            for part in ("train_2021_2022", "validation_2023_2024", "later_2025_2026")
        }
        for name in result["policies"]
    }, indent=2))


if __name__ == "__main__":
    main()
