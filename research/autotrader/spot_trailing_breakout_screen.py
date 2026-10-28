#!/usr/bin/env python3
"""Small predeclared spot strategy screen on retained Binance daily candles.

Signals use the previous closed candle. Trades occur at the next daily open.
Each asset is a separate unleveraged sleeve; idle sleeve capital earns zero.
"""
import json
import math
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data/binance_spot_1d_api_2019_2026"
RECENT = ROOT / "data/binance_spot_recent_2026-10-28.json"
SYMBOLS = ("BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT")
OUT = ROOT / "spot_trailing_breakout_results.json"


def load():
    by_symbol = {}
    recent = json.loads(RECENT.read_text())["symbols"]
    for symbol in SYMBOLS:
        raw = json.loads((DATA / f"{symbol}-1d.json").read_text())
        raw.extend(recent[symbol])
        by_symbol[symbol] = {
            datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"): {
                "open": float(row[1]), "high": float(row[2]),
                "low": float(row[3]), "close": float(row[4]),
            }
            for row in raw
        }
    dates = sorted(set.intersection(*(set(bars) for bars in by_symbol.values())))
    return dates, by_symbol


def load_older_three():
    by_symbol = {}
    for symbol in ("BTCUSDT", "ETHUSDT", "BNBUSDT"):
        raw = json.loads((ROOT / "data/binance_spot_1d_api_2017_2018" / f"{symbol}-1d.json").read_text())
        raw.extend(json.loads((DATA / f"{symbol}-1d.json").read_text()))
        raw.extend(json.loads(RECENT.read_text())["symbols"][symbol])
        by_symbol[symbol] = {
            datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"): {
                "open": float(row[1]), "high": float(row[2]),
                "low": float(row[3]), "close": float(row[4]),
            }
            for row in raw
        }
    dates = sorted(set.intersection(*(set(bars) for bars in by_symbol.values())))
    return dates, by_symbol


def stats(returns, trade_count, cost_turnover):
    wealth = peak = 1.0
    drawdown = 0.0
    for value in returns:
        wealth *= 1 + value
        peak = max(peak, wealth)
        drawdown = min(drawdown, wealth / peak - 1)
    mean = sum(returns) / len(returns) if returns else 0
    variance = sum((value - mean) ** 2 for value in returns) / max(1, len(returns) - 1)
    return {
        "return_percent": 100 * (wealth - 1),
        "max_drawdown_percent": 100 * drawdown,
        "annualized_daily_sharpe": mean / math.sqrt(variance) * math.sqrt(365) if variance else 0,
        "days": len(returns),
        "entry_events": trade_count,
        "absolute_weight_turnover": cost_turnover,
    }


def run(dates, data, policy, side_cost, universe=SYMBOLS):
    symbols = {
        "eth_sma50": ("ETHUSDT",),
        "eth_sma50_100": ("ETHUSDT",),
        "eth_sma50_weekly": ("ETHUSDT",),
        "eth_sma50_btc200": ("ETHUSDT",),
        "btc_eth_sma50_btc200": ("BTCUSDT", "ETHUSDT"),
        "btc_eth_sma50_weekly": ("BTCUSDT", "ETHUSDT"),
        "btc_sma50": ("BTCUSDT",),
        "bnb_sma100": ("BNBUSDT",),
        "btc_eth_sma50": ("BTCUSDT", "ETHUSDT"),
        "core_trend_mix": ("BTCUSDT", "ETHUSDT", "BNBUSDT"),
    }.get(policy, universe)
    wealth = {symbol: 1.0 for symbol in symbols}
    held = {symbol: False for symbol in symbols}
    rows = []
    for i in range(202, len(dates) - 1):
        date, prior, next_date = dates[i], dates[i - 1], dates[i + 1]
        btc_closes = [data["BTCUSDT"][dates[j]]["close"] for j in range(i - 200, i)]
        btc_on = data["BTCUSDT"][prior]["close"] > sum(btc_closes) / 200
        before = sum(wealth.values())
        entries = 0
        turnover = 0.0
        for symbol in symbols:
            close = data[symbol][prior]["close"]
            previous = held[symbol]
            target = previous
            if policy == "buy_hold":
                target = True
            elif policy in ("pulse55_btc200", "pulse20_btc200", "pulse20_own200"):
                days = 55 if policy == "pulse55_btc200" else 20
                high = max(data[symbol][dates[j]]["high"] for j in range(i - days - 1, i - 1))
                gate = btc_on
                if policy == "pulse20_own200":
                    own_sma = sum(data[symbol][dates[j]]["close"] for j in range(i - 200, i)) / 200
                    gate = close > own_sma
                target = gate and close > high
            elif policy.startswith("turtle"):
                entry_days, exit_days = (55, 20) if "55_20" in policy else (20, 10)
                high = max(data[symbol][dates[j]]["high"] for j in range(i - entry_days - 1, i - 1))
                low = min(data[symbol][dates[j]]["low"] for j in range(i - exit_days - 1, i - 1))
                gate = btc_on if policy.endswith("btc200") else True
                if previous:
                    target = gate and close >= low
                else:
                    target = gate and close > high
            elif policy in ("eth_sma50", "btc_sma50", "btc_eth_sma50", "core_trend_mix", "bnb_sma100"):
                days = 100 if policy == "bnb_sma100" or policy == "core_trend_mix" and symbol == "BNBUSDT" else 50
                sma = sum(data[symbol][dates[j]]["close"] for j in range(i - days, i)) / days
                target = close > sma
            elif policy in ("eth_sma50_btc200", "btc_eth_sma50_btc200"):
                sma50 = sum(data[symbol][dates[j]]["close"] for j in range(i - 50, i)) / 50
                target = btc_on and close > sma50
            elif policy in ("eth_sma50_weekly", "btc_eth_sma50_weekly"):
                if datetime.fromisoformat(prior).weekday() == 6:
                    sma50 = sum(data[symbol][dates[j]]["close"] for j in range(i - 50, i)) / 50
                    target = close > sma50
            elif policy == "eth_sma50_100":
                sma50 = sum(data[symbol][dates[j]]["close"] for j in range(i - 50, i)) / 50
                sma100 = sum(data[symbol][dates[j]]["close"] for j in range(i - 100, i)) / 100
                target = close >= sma100 if previous else close > sma50
            else:
                raise ValueError(policy)
            trade = int(target != previous)
            entries += int(target and not previous)
            turnover += trade * wealth[symbol] / before
            price_return = data[symbol][next_date]["open"] / data[symbol][date]["open"] - 1
            wealth[symbol] *= (1 - side_cost * trade) * (1 + price_return if target else 1)
            held[symbol] = target
        rows.append({
            "date": date,
            "return": sum(wealth.values()) / before - 1,
            "entries": entries,
            "turnover": turnover,
            "held": sum(held.values()),
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
        key: stats([row["return"] for row in values],
                   sum(row["entries"] for row in values),
                   sum(row["turnover"] for row in values))
        for key, values in periods.items()
    }


def main():
    dates, data = load()
    policies = (
        "buy_hold", "pulse55_btc200", "pulse20_btc200", "pulse20_own200",
        "turtle55_20_btc200", "turtle55_20",
        "turtle20_10_btc200",
        "eth_sma50", "eth_sma50_100",
        "btc_sma50", "bnb_sma100", "btc_eth_sma50", "core_trend_mix",
        "eth_sma50_weekly", "btc_eth_sma50_weekly",
        "eth_sma50_btc200", "btc_eth_sma50_btc200",
    )
    result = {
        "data_date_range": [dates[0], dates[-1]],
        "trading_date_range": [dates[202], dates[-2]],
        "source": "retained Binance Spot 1d API rows plus frozen 2026-10-28 Binance market-data-only REST snapshot; seven present-day USDT pairs",
        "timing": "prior closed candle signal, current open trade, current to next open return",
        "portfolio": "equal initial capital per asset, separate non-rebalanced spot sleeves, cash earns zero",
        "selection_warning": "2025-2026 was already inspected in earlier repo research; this is not a pristine holdout",
        "policies": {},
    }
    for policy in policies:
        result["policies"][policy] = {}
        for basis_points in (15, 30, 50):
            rows = run(dates, data, policy, basis_points / 10000)
            result["policies"][policy][str(basis_points)] = summarize(rows)
    older_dates, older_data = load_older_three()
    result["older_three_asset_screen"] = {
        "symbols": ["BTCUSDT", "ETHUSDT", "BNBUSDT"],
        "trading_date_range": [older_dates[202], older_dates[-2]],
        "policies": {},
    }
    for policy in ("buy_hold", "pulse55_btc200", "pulse20_btc200", "pulse20_own200"):
        result["older_three_asset_screen"]["policies"][policy] = {}
        for basis_points in (15, 50):
            rows = run(older_dates, older_data, policy, basis_points / 10000,
                       universe=("BTCUSDT", "ETHUSDT", "BNBUSDT"))
            result["older_three_asset_screen"]["policies"][policy][str(basis_points)] = {
                year: stats([row["return"] for row in rows if row["date"].startswith(year)],
                            sum(row["entries"] for row in rows if row["date"].startswith(year)),
                            sum(row["turnover"] for row in rows if row["date"].startswith(year)))
                for year in ("2018", "2019", "2020", "2025", "2026")
            }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({
        "date_range": result["trading_date_range"],
        "later": {
            name: {
                str(cost): {
                    "return_percent": result["policies"][name][str(cost)]["later_2025_2026"]["return_percent"],
                    "max_drawdown_percent": result["policies"][name][str(cost)]["later_2025_2026"]["max_drawdown_percent"],
                    "entry_events": result["policies"][name][str(cost)]["later_2025_2026"]["entry_events"],
                }
                for cost in (15, 30, 50)
            }
            for name in policies
        },
    }, indent=2))


if __name__ == "__main__":
    main()
