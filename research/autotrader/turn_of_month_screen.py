#!/usr/bin/env python3
"""Predeclared last-day plus first-three-days crypto turn-of-month screen."""
import calendar
import json
import math
import statistics
from datetime import date
from pathlib import Path

from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "turn_of_month_results.json"


def stats(values, entries):
    wealth = peak = 1.0
    dd = 0.0
    for value in values:
        wealth *= 1 + value
        peak = max(peak, wealth)
        dd = min(dd, wealth / peak - 1)
    mean = statistics.mean(values) if values else 0
    sd = statistics.stdev(values) if len(values) > 1 else 0
    return dict(days=len(values), return_percent=100 * (wealth - 1),
                max_drawdown_percent=100 * dd,
                annualized_daily_sharpe=mean / sd * math.sqrt(365) if sd else 0,
                entry_events=entries)


def run(dates, data, symbols, gate, cost):
    wealth = {symbol: 1.0 for symbol in symbols}
    active = {symbol: False for symbol in symbols}
    rows = []
    for i in range(202, len(dates) - 1):
        day = date.fromisoformat(dates[i])
        tom = day.day <= 3 or day.day == calendar.monthrange(day.year, day.month)[1]
        btc_sma = sum(data["BTCUSDT"][dates[k]]["close"] for k in range(i - 200, i)) / 200
        risk_on = data["BTCUSDT"][dates[i - 1]]["close"] > btc_sma
        before = sum(wealth.values())
        entries = 0
        for symbol in symbols:
            target = tom and (risk_on or not gate)
            entries += int(target and not active[symbol])
            r = data[symbol][dates[i + 1]]["open"] / data[symbol][dates[i]]["open"] - 1
            wealth[symbol] *= (1 - cost * int(target != active[symbol])) * (1 + r if target else 1)
            active[symbol] = target
        rows.append(dict(date=dates[i], return_=(sum(wealth.values()) / before - 1), entries=entries))
    return rows


def summarize(rows):
    return {
        "train_2021_2022": stats([r["return_"] for r in rows if r["date"] < "2023-01-01"], sum(r["entries"] for r in rows if r["date"] < "2023-01-01")),
        "validation_2023_2024": stats([r["return_"] for r in rows if "2023-01-01" <= r["date"] < "2025-01-01"], sum(r["entries"] for r in rows if "2023-01-01" <= r["date"] < "2025-01-01")),
        "later_2025_2026": stats([r["return_"] for r in rows if r["date"] >= "2025-01-01"], sum(r["entries"] for r in rows if r["date"] >= "2025-01-01")),
        **{f"year_{year}": stats([r["return_"] for r in rows if r["date"].startswith(str(year))], sum(r["entries"] for r in rows if r["date"].startswith(str(year))))
           for year in range(2021, 2027)},
    }


def main():
    dates, data = load()
    policies = {
        "btc_tom": (("BTCUSDT",), False),
        "btc_tom_btc200": (("BTCUSDT",), True),
        "eth_tom": (("ETHUSDT",), False),
        "btc_eth_tom_btc200": (("BTCUSDT", "ETHUSDT"), True),
    }
    result = {
        "source": "retained Binance daily spot candles plus frozen 2026-10-28 REST snapshot",
        "date_range": [dates[202], dates[-2]],
        "rule": "long on last UTC calendar day and first three UTC calendar days of each month; optional BTC prior close above SMA200 gate",
        "execution": "calendar known in advance, trade at current open, open-to-next-open return; 15/30/50 bps per side",
        "policies": {
            name: {str(bps): summarize(run(dates, data, symbols, gate, bps / 10000))
                   for bps in (15, 30, 50)}
            for name, (symbols, gate) in policies.items()
        },
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({
        name: {period: result["policies"][name]["15"][period]["return_percent"]
               for period in ("train_2021_2022", "validation_2023_2024", "later_2025_2026")}
        for name in policies
    }, indent=2))


if __name__ == "__main__":
    main()
