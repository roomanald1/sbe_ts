#!/usr/bin/env python3
"""Monthly long/cash time-series momentum screen on retained Binance daily bars.

At the first daily open of each month, use the previous completed daily close
to set each asset sleeve long when its trailing close-to-close return is
positive, otherwise cash. Hold the target until the next monthly rebalance.
Each asset has an independent, equally funded sleeve; sleeves compound without
periodic rebalancing. This is a research replay, not an OKX fill simulation.
"""
import json
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data/binance_spot_1d_api_2019_2026"
RECENT = ROOT / "data/binance_spot_recent_2026-10-28.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")
LOOKBACKS = (90, 180, 270, 365, 540)
PERIODS = {
    "2020-2022": ("2020-01-01", "2023-01-01"),
    "2023-2024": ("2023-01-01", "2025-01-01"),
    "2025-2026-to-2026-10-28": ("2025-01-01", "2026-10-29"),
}
OUT = ROOT / "long_horizon_momentum_results.json"


def load():
    recent = json.loads(RECENT.read_text())["symbols"]
    by_symbol = {}
    for symbol in SYMBOLS:
        raw = json.loads((DATA / f"{symbol}-1d.json").read_text())
        raw.extend(recent[symbol])
        by_symbol[symbol] = {
            datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"):
                {"open": float(row[1]), "close": float(row[4])}
            for row in raw
        }
    dates = sorted(set.intersection(*(set(bars) for bars in by_symbol.values())))
    return dates, by_symbol


def replay(dates, data, lookback, side_cost):
    wealth = {symbol: 1.0 for symbol in SYMBOLS}
    held = {symbol: False for symbol in SYMBOLS}
    rows = []
    for i in range(1, len(dates) - 1):
        date, prior, next_date = dates[i], dates[i - 1], dates[i + 1]
        before = sum(wealth.values())
        month_start = date[5:7] != prior[5:7]
        for symbol in SYMBOLS:
            previous = held[symbol]
            if lookback == 0:
                held[symbol] = True
            elif month_start and i > lookback:
                momentum = (
                    data[symbol][prior]["close"]
                    / data[symbol][dates[i - lookback - 1]]["close"]
                    - 1.0
                )
                held[symbol] = momentum > 0.0
            exposure_change = abs(int(held[symbol]) - int(previous))
            asset_return = (
                data[symbol][next_date]["open"] / data[symbol][date]["open"] - 1.0
            )
            wealth[symbol] *= (
                1.0 + int(held[symbol]) * asset_return - side_cost * exposure_change
            )
        rows.append({"date": date, "return": sum(wealth.values()) / before - 1.0})
    return rows


def summarize(rows):
    result = {}
    for period, (start, end) in PERIODS.items():
        values = [row["return"] for row in rows if start <= row["date"] < end]
        wealth = peak = 1.0
        drawdown = 0.0
        for value in values:
            wealth *= 1.0 + value
            peak = max(peak, wealth)
            drawdown = min(drawdown, wealth / peak - 1.0)
        result[period] = {
            "return_percent": 100.0 * (wealth - 1.0),
            "max_drawdown_percent": 100.0 * drawdown,
            "days": len(values),
        }
    return result


def main():
    dates, data = load()
    result = {
        "source": "retained Binance Spot daily klines plus frozen 2026-10-28 market-data snapshot",
        "universe": list(SYMBOLS),
        "data_range": [dates[0], dates[-1]],
        "signal": "At the first daily open of each month, long if prior completed close is above its close N calendar bars earlier; otherwise cash. Rebalance monthly.",
        "execution": "Prior UTC close signal, current UTC open trade, current-to-next-open return; idle cash earns zero.",
        "portfolio": "Equal initial capital in seven independent unleveraged sleeves; sleeve equity compounds independently, no rebalancing between sleeves.",
        "limitations": "Today's surviving seven-coin universe; Binance OHLC rather than OKX fills; 2025-26 has been inspected in earlier research and is not a clean holdout.",
        "periods": PERIODS,
        "results": {},
    }
    for lookback in LOOKBACKS:
        result["results"][str(lookback)] = {}
        for bps in (15, 30, 50):
            rows = replay(dates, data, lookback, bps / 10_000.0)
            result["results"][str(lookback)][str(bps)] = summarize(rows)
    for bps in (15, 30, 50):
        result["results"]["buy_and_hold"] = result["results"].get("buy_and_hold", {})
        result["results"]["buy_and_hold"][str(bps)] = summarize(
            replay(dates, data, 0, bps / 10_000.0)
        )
    # Buy-and-hold is represented by continuously held sleeves from the first
    # common-calendar open, with the same initial entry cost as other sleeves.
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    for key, costs in result["results"].items():
        print(key, json.dumps(costs["15"], separators=(",", ":")))


if __name__ == "__main__":
    main()
