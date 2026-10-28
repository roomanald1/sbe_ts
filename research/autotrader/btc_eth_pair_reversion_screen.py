#!/usr/bin/env python3
"""Predeclared BTC/ETH relative-value mean-reversion screen."""
import json
import math
import statistics
from pathlib import Path

from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "btc_eth_pair_reversion_results.json"


def summary(rows):
    equity = peak = 1.0
    drawdown = 0.0
    for row in rows:
        equity *= 1 + row["return"]
        peak = max(peak, equity)
        drawdown = min(drawdown, equity / peak - 1)
    returns = [row["return"] for row in rows]
    mean = statistics.mean(returns) if returns else 0
    stdev = statistics.stdev(returns) if len(returns) > 1 else 0
    return {
        "days": len(rows),
        "return_percent": (equity - 1) * 100,
        "max_drawdown_percent": drawdown * 100,
        "annualized_daily_sharpe": mean / stdev * math.sqrt(365) if stdev else 0,
        "entries": sum(row["entry"] for row in rows),
        "exits": sum(row["exit"] for row in rows),
        "mean_absolute_z_at_entry": statistics.mean(
            abs(row["z"]) for row in rows if row["entry"]
        ) if any(row["entry"] for row in rows) else None,
    }


def run(dates, data, cost, borrow_annual=0.05):
    ratio = [
        math.log(data["ETHUSDT"][date]["close"] / data["BTCUSDT"][date]["close"])
        for date in dates
    ]
    side = 0
    age = 0
    rows = []
    for i in range(202, len(dates) - 1):
        date, next_date = dates[i], dates[i + 1]
        prior_z_window = ratio[i - 61:i - 1]
        sd = statistics.pstdev(prior_z_window)
        z = (ratio[i - 1] - statistics.mean(prior_z_window)) / sd if sd > 0 else 0
        old_side = side
        if side == 0:
            if z >= 2:
                side = -1
            elif z <= -2:
                side = 1
        elif (side == 1 and z >= 0) or (side == -1 and z <= 0) or age >= 10:
            side = 0
        if side == 0:
            age = 0
        elif old_side == 0:
            age = 1
        else:
            age += 1
        eth_return = data["ETHUSDT"][next_date]["open"] / data["ETHUSDT"][date]["open"] - 1
        btc_return = data["BTCUSDT"][next_date]["open"] / data["BTCUSDT"][date]["open"] - 1
        gross = side * 0.5 * (eth_return - btc_return)
        # Each long/short pair has one unit of gross exposure on entry or exit.
        turnover = abs(side - old_side)
        daily_borrow = 0.5 * borrow_annual / 365 if side else 0
        net = gross - cost * turnover - daily_borrow
        rows.append({
            "date": date, "return": net, "entry": int(old_side == 0 and side != 0),
            "exit": int(old_side != 0 and side == 0), "z": z,
        })
    return rows


def main():
    dates, data = load()
    result = {
        "rule": "BTC/ETH log price ratio z-score vs previous 60 closes; enter opposite extreme at |z|>=2, exit on mean crossing or after 10 days",
        "position": "50% long one coin and 50% short the other, unleveraged gross notional relative to capital",
        "timing": "previous closed daily candle signal, next open execution",
        "short_borrow": "5% annual rate on 50% short notional, charged daily while held",
        "source": "retained Binance daily spot candles and frozen 2026-10-28 REST snapshot",
        "date_range": [dates[202], dates[-2]],
        "scenarios": {},
    }
    for basis_points in (15, 30, 50):
        rows = run(dates, data, basis_points / 10000)
        result["scenarios"][str(basis_points)] = {
            "train_2021_2022": summary([r for r in rows if r["date"] < "2023-01-01"]),
            "validation_2023_2024": summary([r for r in rows if "2023-01-01" <= r["date"] < "2025-01-01"]),
            "later_2025_2026": summary([r for r in rows if r["date"] >= "2025-01-01"]),
            "all": summary(rows),
        }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["scenarios"], indent=2))


if __name__ == "__main__":
    main()
