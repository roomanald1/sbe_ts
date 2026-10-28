#!/usr/bin/env python3
"""Compare daily Donchian signal exits with fixed net stop/target policies."""
import json
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data/binance_spot_1d_api_2019_2026"
SYMBOLS = ["BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT"]
BASE_COST = 0.0015


def load_data():
    data = {}
    for symbol in SYMBOLS:
        rows = json.loads((DATA / f"{symbol}-1d.json").read_text())
        data[symbol] = {
            datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"):
                {"open": float(row[1]), "high": float(row[2]), "low": float(row[3]), "close": float(row[4])}
            for row in rows
        }
    days = sorted(set.intersection(*(set(data[symbol]) for symbol in SYMBOLS)))
    return days, data


def metrics(series):
    if not series:
        return {"days": 0, "return_pct": None, "max_drawdown_pct": None, "sharpe": None}
    equity = peak = 1.0
    drawdown = 0.0
    for value in series:
        equity *= 1.0 + value
        peak = max(peak, equity)
        drawdown = min(drawdown, equity / peak - 1.0)
    average = sum(series) / len(series)
    variance = sum((value - average) ** 2 for value in series) / max(1, len(series) - 1)
    sharpe = average / variance ** 0.5 * 365 ** 0.5 if variance else 0.0
    return {
        "days": len(series),
        "return_pct": 100.0 * (equity - 1.0),
        "max_drawdown_pct": 100.0 * drawdown,
        "sharpe": sharpe,
    }


def run(days, data, policy, stop_pct=0.5, target_pct=0.5, cost=BASE_COST):
    equity = 1.0
    positions = {symbol: None for symbol in SYMBOLS}
    daily = []
    counts = {"entries": 0, "exits": 0, "stop_first_touches": 0, "target_first_touches": 0}

    for index in range(202, len(days) - 1):
        day, prior, next_day = days[index], days[index - 1], days[index + 1]
        btc_sma200 = sum(data["BTCUSDT"][days[k]]["close"] for k in range(index - 200, index)) / 200.0
        btc_risk_on = data["BTCUSDT"][prior]["close"] > btc_sma200
        active = {}
        for symbol in SYMBOLS:
            prior_high = max(data[symbol][days[k]]["high"] for k in range(index - 56, index - 1))
            active[symbol] = btc_risk_on and data[symbol][prior]["close"] > prior_high

        portfolio_return = 0.0
        turnover = 0.0
        for symbol in SYMBOLS:
            position = positions[symbol]
            open_price = data[symbol][day]["open"]
            is_active = active[symbol]

            # The strategy signal is known before this open; an invalidated position exits here.
            if position is not None and not is_active:
                positions[symbol] = None
                position = None
                turnover += 1.0 / len(SYMBOLS)
                counts["exits"] += 1

            if position is None and is_active:
                position = {"entry": open_price}
                positions[symbol] = position
                turnover += 1.0 / len(SYMBOLS)
                counts["entries"] += 1

            if position is None:
                continue

            if policy == "fixed_net_brackets":
                entry = position["entry"]
                # Solve for prices where PnL after entry and exit costs reaches each net threshold.
                stop_price = entry * (1.0 - stop_pct / 100.0 + cost) / (1.0 - cost)
                target_price = entry * (1.0 + target_pct / 100.0 + cost) / (1.0 - cost)
                bar = data[symbol][day]
                stop_touched = bar["low"] <= stop_price
                target_touched = bar["high"] >= target_price
                if stop_touched or target_touched:
                    # Daily OHLC does not reveal which level traded first; assume the stop came first.
                    if stop_touched:
                        exit_price = min(open_price, stop_price)
                        counts["stop_first_touches"] += 1
                    else:
                        exit_price = max(open_price, target_price)
                        counts["target_first_touches"] += 1
                    portfolio_return += (1.0 / len(SYMBOLS)) * (exit_price / open_price - 1.0)
                    turnover += 1.0 / len(SYMBOLS)
                    positions[symbol] = None
                    counts["exits"] += 1
                    continue

            portfolio_return += (1.0 / len(SYMBOLS)) * (
                data[symbol][next_day]["open"] / open_price - 1.0
            )

        net_return = portfolio_return - cost * turnover
        daily.append((day, net_return))
        equity *= 1.0 + net_return

    splits = {}
    for name, start, end in [
        ("train_2021_2022", "2021-01-01", "2023-01-01"),
        ("validation_2023_2024", "2023-01-01", "2025-01-01"),
        ("holdout_2025_to_latest", "2025-01-01", "9999-12-31"),
    ]:
        values = [value for day, value in daily if start <= day < end]
        splits[name] = metrics(values)
    return {"splits": splits, "counts": counts, "full_period_return_pct": 100.0 * (equity - 1.0)}


def main():
    days, data = load_data()
    policies = {
        "daily_signal_exit": run(days, data, "daily_signal_exit"),
        "net_stop_0_5_target_0_5": run(days, data, "fixed_net_brackets", 0.5, 0.5),
        "net_stop_0_5_target_1_0": run(days, data, "fixed_net_brackets", 0.5, 1.0),
        "net_stop_0_5_target_1_5": run(days, data, "fixed_net_brackets", 0.5, 1.5),
        "net_stop_1_0_target_1_5": run(days, data, "fixed_net_brackets", 1.0, 1.5),
    }
    cost_stress = {
        "daily_signal_exit": {
            str(int(cost * 10000)) + "_bps_per_side": run(days, data, "daily_signal_exit", cost=cost)["splits"]["holdout_2025_to_latest"]
            for cost in (0.0015, 0.003, 0.005)
        }
    }
    result = {
        "data_source": "Local Binance daily spot candles",
        "date_range": [days[202], days[-2]],
        "universe": SYMBOLS,
        "signal": "Prior 55-day high breakout, long only when BTC prior close is above its trailing 200-day close SMA",
        "sizing_and_timing": "Each active symbol has fixed 1/7 portfolio weight; signal uses prior close; entry and strategy invalidation exit at current open; daily open-to-next-open returns",
        "base_cost": "15 bps per side on weight turnover; fixed bracket thresholds are net of estimated entry and exit costs",
        "fixed_bracket_assumption": "Use daily high/low to detect touches; if stop and target are both touched in one daily bar, count stop first; gap exits use the worse opening price",
        "results": policies,
        "signal_exit_holdout_cost_stress": cost_stress,
        "limitations": [
            "Daily OHLC cannot identify intraday order of stop and target touches; stop-first is conservative.",
            "This is a research replay, not a live fill simulation; it does not include latency, partial fills, or market impact.",
            "The fixed bracket replay reinvests using portfolio-level daily returns and re-enters on a later active day; exact production cooldown behavior may differ.",
            "The signal-driven result reproduces the historical daily backtest convention; production strategy configuration and holdings can differ.",
        ],
    }
    path = ROOT / "exit_policy_counterfactual_results.json"
    path.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
