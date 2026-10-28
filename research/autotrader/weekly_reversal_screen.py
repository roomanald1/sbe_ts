#!/usr/bin/env python3
"""Fixed weekly cross-sectional reversal on seven liquid spot pairs."""
import json
import math
import statistics
from datetime import datetime
from pathlib import Path

from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "weekly_reversal_results.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")


def metrics(rows):
    equity = peak = 1.0
    dd = 0.0
    for row in rows:
        equity *= 1 + row["return"]
        peak = max(peak, equity)
        dd = min(dd, equity / peak - 1)
    rs = [row["return"] for row in rows]
    mean = statistics.mean(rs) if rs else 0
    sd = statistics.stdev(rs) if len(rs) > 1 else 0
    return dict(days=len(rows), return_percent=100 * (equity - 1),
                max_drawdown_percent=100 * dd,
                annualized_daily_sharpe=mean / sd * math.sqrt(365) if sd else 0,
                rebalance_events=sum(row["rebalance"] for row in rows),
                absolute_weight_turnover=sum(row["turnover"] for row in rows))


def run(dates, data, policy, cost):
    weights = {symbol: 0.0 for symbol in SYMBOLS}
    cash = 1.0
    rows = []
    for i in range(202, len(dates) - 1):
        date, prior, next_date = dates[i], dates[i - 1], dates[i + 1]
        rebalance = int(datetime.fromisoformat(date).weekday() == 0)
        turnover = 0.0
        if rebalance:
            btc_sma = sum(data["BTCUSDT"][dates[k]]["close"] for k in range(i - 200, i)) / 200
            btc_on = data["BTCUSDT"][prior]["close"] > btc_sma
            ranked = sorted(
                (data[symbol][prior]["close"] / data[symbol][dates[i - 8]]["close"] - 1, symbol)
                for symbol in SYMBOLS
            )
            picks = [symbol for _, symbol in (ranked[:2] if policy.startswith("bottom") else ranked[-2:])]
            if policy.endswith("_btc200") and not btc_on:
                picks = []
            target = {symbol: 0.5 if symbol in picks else 0.0 for symbol in SYMBOLS}
            turnover = sum(abs(target[symbol] - weights[symbol]) for symbol in SYMBOLS)
            weights = target
            cash = 1 - sum(weights.values())
        changes = {
            symbol: data[symbol][next_date]["open"] / data[symbol][date]["open"] - 1
            for symbol in SYMBOLS
        }
        growth = cash + sum(weights[symbol] * (1 + changes[symbol]) for symbol in SYMBOLS)
        rows.append(dict(date=date, return_=(1 - cost * turnover) * growth - 1,
                         rebalance=rebalance, turnover=turnover))
        weights = {symbol: weights[symbol] * (1 + changes[symbol]) / growth for symbol in SYMBOLS}
        cash /= growth
    return [
        {"date": row["date"], "return": row["return_"],
         "rebalance": row["rebalance"], "turnover": row["turnover"]}
        for row in rows
    ]


def summarize(rows):
    return {
        "train_2021_2022": metrics([r for r in rows if r["date"] < "2023-01-01"]),
        "validation_2023_2024": metrics([r for r in rows if "2023-01-01" <= r["date"] < "2025-01-01"]),
        "later_2025_2026": metrics([r for r in rows if r["date"] >= "2025-01-01"]),
        **{f"year_{year}": metrics([r for r in rows if r["date"].startswith(str(year))])
           for year in range(2021, 2027)},
    }


def main():
    dates, data = load()
    result = {
        "source": "retained Binance daily spot candles plus frozen 2026-10-28 REST snapshot",
        "date_range": [dates[202], dates[-2]],
        "rule": "every Monday UTC, rank prior seven-day close-to-close returns and buy the two lowest; optional BTC prior close above SMA200 cash gate",
        "execution": "signal from Sunday close, Monday open rebalance, open-to-next-open return; cash earns zero",
        "universe": list(SYMBOLS),
        "policies": {},
    }
    for policy in ("bottom_two_btc200", "bottom_two", "top_two_btc200"):
        result["policies"][policy] = {
            str(bps): summarize(run(dates, data, policy, bps / 10000))
            for bps in (15, 30, 50)
        }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({
        name: {
            period: result["policies"][name]["15"][period]["return_percent"]
            for period in ("train_2021_2022", "validation_2023_2024", "later_2025_2026")
        }
        for name in result["policies"]
    }, indent=2))


if __name__ == "__main__":
    main()
