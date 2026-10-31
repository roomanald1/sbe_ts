#!/usr/bin/env python3
"""Assess Deribit BTC DVOL as a risk scaler for a long-only BTC trend sleeve.

The same file records a descriptive monthly implied-versus-forward-realized
volatility spread. That spread is not an options-trading P&L calculation.
"""
import json
import math
import statistics
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data/binance_spot_1d_api_2019_2026/BTCUSDT-1d.json"
RECENT = ROOT / "data/binance_spot_recent_2026-10-28.json"
DVOL_CACHE = ROOT / "data/deribit_btc_dvol_daily_2021_2026.json"
OUT = ROOT / "dvol_risk_sizing_results.json"
START_MS = int(datetime(2021, 1, 1, tzinfo=timezone.utc).timestamp() * 1000)
END_MS = int(datetime(2026, 10, 29, tzinfo=timezone.utc).timestamp() * 1000)
PERIODS = {
    "2021-03-24-to-2022": ("2021-03-24", "2023-01-01"),
    "2023-2024": ("2023-01-01", "2025-01-01"),
    "2025-to-2026-10-28": ("2025-01-01", "2026-10-29"),
}


def fetch_dvol():
    if DVOL_CACHE.exists():
        return json.loads(DVOL_CACHE.read_text())
    values = {}
    cursor = END_MS
    while cursor:
        url = (
            "https://www.deribit.com/api/v2/public/get_volatility_index_data"
            f"?currency=BTC&start_timestamp={START_MS}&end_timestamp={cursor}&resolution=1D"
        )
        request = urllib.request.Request(url, headers={"User-Agent": "research-screen/1.0"})
        with urllib.request.urlopen(request, timeout=30) as response:
            result = json.load(response)["result"]
        values.update({str(row[0]): float(row[4]) for row in result["data"]})
        cursor = result["continuation"]
    cache = {
        "source": "Deribit public/get_volatility_index_data, currency BTC, resolution 1D",
        "unit": "annualized volatility percent",
        "close_by_timestamp_ms": values,
    }
    DVOL_CACHE.parent.mkdir(parents=True, exist_ok=True)
    DVOL_CACHE.write_text(json.dumps(cache, separators=(",", ":")) + "\n")
    return cache


def load_spot():
    rows = json.loads(DATA.read_text())
    rows.extend(json.loads(RECENT.read_text())["symbols"]["BTCUSDT"])
    return {
        datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"):
            {"open": float(row[1]), "close": float(row[4])}
        for row in rows
    }


def stdev(values):
    return statistics.stdev(values) if len(values) > 1 else 0.0


def backtest(dates, spot, dvol, side_cost_bps):
    result = {"fixed": [], "realized_vol_target": [], "dvol_target": []}
    cost = side_cost_bps / 10_000.0
    previous_weight = {key: 0.0 for key in result}
    for i in range(202, len(dates) - 1):
        day, prior, next_day = dates[i], dates[i - 1], dates[i + 1]
        if prior not in dvol:
            continue
        signal = float(spot[prior]["close"] > spot[dates[i - 61]]["close"])
        realized = stdev([
            spot[dates[k]]["close"] / spot[dates[k - 1]]["close"] - 1.0
            for k in range(i - 30, i)
        ]) * math.sqrt(365.0)
        dvol_decimal = dvol[prior] / 100.0
        targets = {
            "fixed": signal,
            "realized_vol_target": min(1.0, 0.50 / realized) * signal if realized else signal,
            "dvol_target": min(1.0, 0.50 / dvol_decimal) * signal if dvol_decimal else signal,
        }
        market_return = spot[next_day]["open"] / spot[day]["open"] - 1.0
        for key, target in targets.items():
            turnover = abs(target - previous_weight[key])
            net = (1.0 + target * market_return) * (1.0 - turnover * cost) - 1.0
            result[key].append((day, net))
            previous_weight[key] = target
    return result


def summarize(rows):
    output = {}
    for period, (start, end) in PERIODS.items():
        values = [value for day, value in rows if start <= day < end]
        wealth = peak = 1.0
        drawdown = 0.0
        for value in values:
            wealth *= 1.0 + value
            peak = max(peak, wealth)
            drawdown = min(drawdown, wealth / peak - 1.0)
        output[period] = {
            "net_return_percent": 100.0 * (wealth - 1.0),
            "max_drawdown_percent": 100.0 * drawdown,
            "days": len(values),
        }
    return output


def implied_minus_forward_realized(dates, spot, dvol):
    common = [day for day in dates if day in dvol]
    samples = []
    for index, day in enumerate(common):
        if index + 31 >= len(common) or common[index + 1][5:7] == day[5:7]:
            continue
        future = [
            math.log(spot[common[j + 1]]["close"] / spot[common[j]]["close"])
            for j in range(index + 1, index + 31)
        ]
        forward_rv_percent = stdev(future) * math.sqrt(365.0) * 100.0
        samples.append((dvol[day], forward_rv_percent))
    spreads = [iv - rv for iv, rv in samples]
    return {
        "observations": len(samples),
        "mean_dvol_percent": statistics.mean(iv for iv, _ in samples),
        "mean_next_30d_realized_vol_percent": statistics.mean(rv for _, rv in samples),
        "mean_spread_volatility_points": statistics.mean(spreads),
        "median_spread_volatility_points": statistics.median(spreads),
        "positive_spread_percent": 100.0 * sum(value > 0 for value in spreads) / len(spreads),
        "correlation_dvol_forward_realized_vol": statistics.correlation(
            [iv for iv, _ in samples], [rv for _, rv in samples]
        ),
        "interpretation": "Descriptive volatility spread only; not option premium P&L.",
    }


def main():
    spot = load_spot()
    dates = sorted(spot)
    cached = fetch_dvol()
    dvol = {
        datetime.fromtimestamp(int(timestamp) / 1000, timezone.utc).strftime("%Y-%m-%d"): value
        for timestamp, value in cached["close_by_timestamp_ms"].items()
    }
    output = {
        "data": {
            "spot_source": "retained Binance BTCUSDT daily candles plus 2026-10-28 snapshot",
            "implied_vol_source": cached["source"],
            "dvol_range": [min(dvol), max(dvol)],
            "dvol_first_available_date": min(dvol),
            "execution": "prior daily close signal and volatility, next open exposure, open-to-next-open returns",
            "position_rule": "60-day positive BTC time-series momentum; 50% annualized volatility target; exposure capped at 100%",
            "costs": "15/30/50 bp per side on absolute exposure change",
        },
        "option_premium_proxy": implied_minus_forward_realized(dates, spot, dvol),
        "risk_sizing_results": {},
        "limitations": [
            "The DVOL index is not a complete historical options surface or executable option quote.",
            "No entry/exit bid-ask, option-specific fees, delta hedging, margin, or tail-loss P&L is modeled for option selling.",
            "The DVOL strategy starts on the first date with available prior-day DVOL, 2021-03-24; its first period is shorter than the existing BTC trend replay.",
            "This historical test uses Binance BTC spot returns, not OKX fills, and the latest dates were examined in prior research.",
        ],
    }
    for bps in (15, 30, 50):
        runs = backtest(dates, spot, dvol, bps)
        output["risk_sizing_results"][str(bps)] = {
            key: summarize(rows) for key, rows in runs.items()
        }
    OUT.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
