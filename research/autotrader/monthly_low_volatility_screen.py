#!/usr/bin/env python3
"""Fixed monthly low-volatility spot screen on the retained seven-pair daily data."""
import json
import math
import statistics
from pathlib import Path

from spot_trailing_breakout_screen import load

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "monthly_low_volatility_results.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")


def metrics(rows):
    wealth = peak = 1.0
    max_drawdown = 0.0
    for row in rows:
        wealth *= 1 + row["return"]
        peak = max(peak, wealth)
        max_drawdown = min(max_drawdown, wealth / peak - 1)
    values = [row["return"] for row in rows]
    mean = statistics.mean(values) if values else 0
    stdev = statistics.stdev(values) if len(values) > 1 else 0
    return {
        "days": len(rows),
        "return_percent": 100 * (wealth - 1),
        "max_drawdown_percent": 100 * max_drawdown,
        "annualized_daily_sharpe": mean / stdev * math.sqrt(365) if stdev else 0,
        "rebalance_events": sum(row["rebalanced"] for row in rows),
        "absolute_weight_turnover": sum(row["turnover"] for row in rows),
        "mean_risky_weight": statistics.mean(row["risky_weight"] for row in rows) if rows else 0,
    }


def run(dates, data, policy, side_cost):
    weights = {symbol: 0.0 for symbol in SYMBOLS}
    cash_weight = 1.0
    equity = 1.0
    rows = []
    for i in range(202, len(dates) - 1):
        day, prior, next_day = dates[i], dates[i - 1], dates[i + 1]
        rebalanced = 0
        turnover = 0.0
        if day.endswith("-01"):
            btc_closes = [data["BTCUSDT"][dates[k]]["close"] for k in range(i - 200, i)]
            btc_risk_on = data["BTCUSDT"][prior]["close"] > sum(btc_closes) / 200
            ranked = []
            for symbol in SYMBOLS:
                closes = [data[symbol][dates[k]]["close"] for k in range(i - 61, i)]
                log_returns = [math.log(closes[j] / closes[j - 1]) for j in range(1, len(closes))]
                ranked.append((statistics.pstdev(log_returns), symbol))
            ranked.sort()
            if policy == "low_two_btc200_own200":
                ranked = [
                    (vol, symbol) for vol, symbol in ranked
                    if data[symbol][prior]["close"] > sum(
                        data[symbol][dates[k]]["close"] for k in range(i - 200, i)
                    ) / 200
                ]
                picks = [symbol for _, symbol in ranked[:2]]
            elif policy.startswith("low_two"):
                picks = [symbol for _, symbol in ranked[:2]]
            elif policy.startswith("high_two"):
                picks = [symbol for _, symbol in ranked[-2:]]
            elif policy.startswith("btc_eth_equal"):
                picks = ["BTCUSDT", "ETHUSDT"]
            elif policy.startswith("equal_seven"):
                picks = list(SYMBOLS)
            else:
                raise ValueError(policy)
            if policy.endswith("_btc200") and not btc_risk_on:
                picks = []
            target = {symbol: 1 / len(picks) if symbol in picks else 0.0 for symbol in SYMBOLS} if picks else {symbol: 0.0 for symbol in SYMBOLS}
            turnover = sum(abs(target[symbol] - weights[symbol]) for symbol in SYMBOLS)
            equity *= 1 - side_cost * turnover
            weights = target
            cash_weight = 1.0 - sum(weights.values())
            rebalanced = 1
        daily_returns = {
            symbol: data[symbol][next_day]["open"] / data[symbol][day]["open"] - 1
            for symbol in SYMBOLS
        }
        growth = cash_weight + sum(weights[symbol] * (1 + daily_returns[symbol]) for symbol in SYMBOLS)
        equity *= growth
        net_return = (1 - side_cost * turnover) * growth - 1
        weights = {symbol: weights[symbol] * (1 + daily_returns[symbol]) / growth for symbol in SYMBOLS}
        cash_weight /= growth
        rows.append({
            "date": day, "return": net_return, "turnover": turnover,
            "rebalanced": rebalanced, "risky_weight": 1 - cash_weight,
        })
    return rows


def summarize(rows):
    return {
        "train_2021_2022": metrics([row for row in rows if row["date"] < "2023-01-01"]),
        "validation_2023_2024": metrics([row for row in rows if "2023-01-01" <= row["date"] < "2025-01-01"]),
        "later_2025_2026": metrics([row for row in rows if row["date"] >= "2025-01-01"]),
        **{f"year_{year}": metrics([row for row in rows if row["date"].startswith(str(year))])
           for year in range(2021, 2027)},
        "all": metrics(rows),
    }


def main():
    dates, data = load()
    policies = (
        "low_two_btc200", "low_two_btc200_own200", "low_two",
        "high_two_btc200", "equal_seven_btc200",
        "btc_eth_equal_btc200", "btc_eth_equal",
    )
    results = {
        "source": "retained Binance spot daily candles and frozen 2026-10-28 REST snapshot",
        "date_range": [dates[202], dates[-2]],
        "rule": "Rank 60 prior close-to-close log-return volatilities on the first UTC calendar day of each month; select two lowest, two highest, or equal seven. Optional BTC prior close above SMA200 gate. Hold weights until next monthly rebalance.",
        "execution": "previous closed daily candle signal; rebalance at next open; open-to-next-open returns; cash earns zero",
        "caveats": "seven present-day survivors; Binance signal and fill proxy, not OKX order fills; 2025-26 examined in earlier research",
        "policies": {},
    }
    for policy in policies:
        results["policies"][policy] = {}
        for basis_points in (15, 30, 50):
            results["policies"][policy][str(basis_points)] = summarize(
                run(dates, data, policy, basis_points / 10000)
            )
    OUT.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({
        name: {
            part: {
                "return_percent": results["policies"][name]["15"][part]["return_percent"],
                "max_drawdown_percent": results["policies"][name]["15"][part]["max_drawdown_percent"],
                "return_at_50bp": results["policies"][name]["50"][part]["return_percent"],
            }
            for part in ("train_2021_2022", "validation_2023_2024", "later_2025_2026")
        }
        for name in policies
    }, indent=2))


if __name__ == "__main__":
    main()
