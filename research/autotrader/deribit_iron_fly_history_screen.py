#!/usr/bin/env python3
"""Monthly BTC inverse iron-fly replay from Deribit option trades and settlements."""
import concurrent.futures
import json
import math
import statistics
import urllib.parse
import urllib.request
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SPOT_FILE = ROOT / "data/binance_spot_1d_api_2019_2026/BTCUSDT-1d.json"
RECENT_FILE = ROOT / "data/binance_spot_recent_2026-10-28.json"
DVOL_FILE = ROOT / "data/deribit_btc_dvol_daily_2021_2026.json"
CACHE_FILE = ROOT / "data/deribit_ironfly_entry_candles.json"
OUT = ROOT / "deribit_iron_fly_history_results.json"
HISTORY = "https://history.deribit.com/api/v2/public"
LIVE = "https://www.deribit.com/api/v2/public"

def get_json(url, timeout=60):
    request = urllib.request.Request(url, headers={"User-Agent": "historical-options-research/1.0"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        payload = json.load(response)
    if payload.get("error"):
        raise RuntimeError(payload["error"])
    return payload["result"]

def load_instruments():
    return get_json(f"{HISTORY}/get_instruments?currency=BTC&kind=option&expired=true&include_old=true")

def load_delivery():
    output = {}
    offset = 0
    while True:
        page = get_json(f"{LIVE}/get_delivery_prices?index_name=btc_usd&offset={offset}&count=1000")
        for row in page["data"]:
            output[row["date"]] = float(row["delivery_price"])
        offset += len(page["data"])
        if not page["data"] or offset >= page.get("records_total", 0):
            return output

def load_spot():
    rows = json.loads(SPOT_FILE.read_text())
    rows.extend(json.loads(RECENT_FILE.read_text())["symbols"]["BTCUSDT"])
    return {
        datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime("%Y-%m-%d"):
            {"open": float(row[1]), "close": float(row[4]), "timestamp": int(row[0])}
        for row in rows
    }

def last_friday(year, month):
    end = date(year, month, 28) + timedelta(days=4)
    end -= timedelta(days=end.day)
    return end - timedelta(days=(end.weekday() - 4) % 7)

def entry_schedule(spot):
    entries = []
    for entry in sorted(day for day in spot if day[8:10] == "01" and "2021-04-01" <= day <= "2026-09-01"):
        year, month, _ = map(int, entry.split("-"))
        expiry = last_friday(year, month).isoformat()
        days_to_expiry = (date.fromisoformat(expiry) - date.fromisoformat(entry)).days
        if 20 <= days_to_expiry <= 40 and expiry in spot:
            entries.append((entry, expiry))
    return entries

def select_legs(instruments, entry, expiry, spot):
    entry_ms = int(datetime.fromisoformat(entry).replace(tzinfo=timezone.utc).timestamp() * 1000)
    eligible = [
        row for row in instruments
        if row.get("expiration_timestamp")
        and datetime.fromtimestamp(row["expiration_timestamp"] / 1000, timezone.utc).date().isoformat() == expiry
        and row.get("creation_timestamp", 0) <= entry_ms
    ]
    targets = {
        "short_put": ("put", spot),
        "long_put": ("put", 0.95 * spot),
        "short_call": ("call", spot),
        "long_call": ("call", 1.05 * spot),
    }
    legs = {}
    for name, (kind, target) in targets.items():
        choices = [row for row in eligible if row.get("option_type") == kind]
        if not choices:
            return None
        legs[name] = min(choices, key=lambda row: abs(float(row["strike"]) - target))
    if legs["long_put"]["strike"] >= legs["short_put"]["strike"]:
        return None
    if legs["long_call"]["strike"] <= legs["short_call"]["strike"]:
        return None
    return legs

def option_candle(name, entry_ms):
    query = urllib.parse.urlencode({
        "instrument_name": name,
        "start_timestamp": entry_ms,
        "end_timestamp": entry_ms + 86_400_000,
        "resolution": "1D",
    })
    try:
        candles = get_json(f"{HISTORY}/get_tradingview_chart_data?{query}")
    except Exception:
        return None
    entry_day = datetime.fromtimestamp(entry_ms / 1000, timezone.utc).strftime("%Y-%m-%d")
    for index, timestamp in enumerate(candles.get("ticks", [])):
        if datetime.fromtimestamp(timestamp / 1000, timezone.utc).strftime("%Y-%m-%d") == entry_day:
            return {key: float(candles[key][index]) for key in ("open", "high", "low", "close")}
    return None

def pnl(legs, fills, entry_spot, delivery_spot):
    premium_btc = 0.0
    entry_fee_btc = 0.0
    expiry_cashflow_usd = 0.0
    delivery_fee_btc = 0.0
    for name, instrument in legs.items():
        sign = 1.0 if name.startswith("long") else -1.0
        price = fills[name]
        premium_btc -= sign * price
        commission = float(instrument.get("taker_commission", 0.0003))
        entry_fee_btc += min(commission, 0.125 * price)
        strike = float(instrument["strike"])
        intrinsic = max(delivery_spot - strike, 0.0) if instrument["option_type"] == "call" else max(strike - delivery_spot, 0.0)
        expiry_cashflow_usd += sign * intrinsic
        if intrinsic > 0:
            delivery_fee_btc += min(0.00015, 0.125 * intrinsic / delivery_spot)
    net_premium_btc = premium_btc - entry_fee_btc
    net_usd = net_premium_btc * delivery_spot + expiry_cashflow_usd - delivery_fee_btc * delivery_spot
    width = max(
        float(legs["short_put"]["strike"]) - float(legs["long_put"]["strike"]),
        float(legs["long_call"]["strike"]) - float(legs["short_call"]["strike"]),
    )
    risk_usd = max(1.0, width - net_premium_btc * entry_spot)
    return {
        "net_pnl_usd": net_usd,
        "net_pnl_percent_underlying_notional": 100.0 * net_usd / entry_spot,
        "net_pnl_percent_defined_risk": 100.0 * net_usd / risk_usd,
        "defined_risk_usd_per_1btc_structure": risk_usd,
        "entry_fee_btc": entry_fee_btc,
        "delivery_fee_btc": delivery_fee_btc,
    }

def naked_straddle_pnl(legs, bars, entry_spot, delivery_spot, adverse=False):
    """Diagnostic only: short ATM straddle, with unbounded downside risk."""
    premium_btc = entry_fee_btc = payout_usd = delivery_fee_btc = 0.0
    for key in ("short_put", "short_call"):
        instrument = legs[key]
        candle = bars[key]
        premium = candle["low"] if adverse else candle["open"]
        premium_btc += premium
        commission = float(instrument.get("taker_commission", 0.0003))
        entry_fee_btc += min(commission, 0.125 * premium)
        strike = float(instrument["strike"])
        intrinsic = max(delivery_spot - strike, 0.0) if instrument["option_type"] == "call" else max(strike - delivery_spot, 0.0)
        payout_usd += intrinsic
        if intrinsic > 0:
            delivery_fee_btc += min(0.00015, 0.125 * intrinsic / delivery_spot)
    net_usd = (premium_btc - entry_fee_btc) * delivery_spot - payout_usd - delivery_fee_btc * delivery_spot
    return net_usd, 100.0 * net_usd / entry_spot

def stats(trades):
    if not trades:
        return {"trades": 0}
    pnls = [trade["net_pnl_percent_underlying_notional"] for trade in trades]
    risk_returns = [trade["net_pnl_percent_defined_risk"] for trade in trades]
    return {
        "trades": len(trades),
        "win_rate_percent": 100.0 * sum(value > 0 for value in pnls) / len(pnls),
        "mean_net_pnl_percent_underlying_notional": statistics.mean(pnls),
        "median_net_pnl_percent_underlying_notional": statistics.median(pnls),
        "mean_net_pnl_percent_defined_risk": statistics.mean(risk_returns),
        "compounded_defined_risk_return_percent": 100.0 * (math.prod(1 + value / 100 for value in risk_returns) - 1),
    }

def stats_naked(trades):
    if not trades:
        return {"trades": 0}
    values = [trade["net_pnl_percent_underlying_notional"] for trade in trades]
    return {
        "trades": len(values),
        "win_rate_percent": 100.0 * sum(value > 0 for value in values) / len(values),
        "mean_net_pnl_percent_underlying_notional": statistics.mean(values),
        "median_net_pnl_percent_underlying_notional": statistics.median(values),
        "total_net_pnl_percent_of_one_btc_notional_per_month": sum(values),
    }

def main():
    spot = load_spot()
    dvol_raw = json.loads(DVOL_FILE.read_text())["close_by_timestamp_ms"]
    dvol = {
        datetime.fromtimestamp(int(ts) / 1000, timezone.utc).strftime("%Y-%m-%d"): float(value)
        for ts, value in dvol_raw.items()
    }
    instruments = load_instruments()
    deliveries = load_delivery()
    schedule = entry_schedule(spot)
    selected = []
    jobs = {}
    for entry, expiry in schedule:
        legs = select_legs(instruments, entry, expiry, spot[entry]["open"])
        if legs is None:
            continue
        selected.append((entry, expiry, legs))
        for instrument in legs.values():
            jobs[instrument["instrument_name"]] = spot[entry]["timestamp"]
    candles = json.loads(CACHE_FILE.read_text()) if CACHE_FILE.exists() else {}
    missing = [(name, timestamp) for name, timestamp in jobs.items() if name not in candles]
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
        pending = {pool.submit(option_candle, name, timestamp): name for name, timestamp in missing}
        for future in concurrent.futures.as_completed(pending):
            candles[pending[future]] = future.result()
    CACHE_FILE.parent.mkdir(parents=True, exist_ok=True)
    CACHE_FILE.write_text(json.dumps(candles, separators=(",", ":")) + "\n")
    all_trades, gated_trades, adverse_trades = [], [], []
    straddle_trades, gated_straddles, adverse_straddles = [], [], []
    skipped = {"missing_delivery": 0, "missing_entry_candle": 0}
    for entry, expiry, legs in selected:
        if expiry not in deliveries:
            skipped["missing_delivery"] += 1
            continue
        bars = {key: candles.get(instrument["instrument_name"]) for key, instrument in legs.items()}
        if bars.get("short_put") is not None and bars.get("short_call") is not None:
            straddle_net, straddle_pct = naked_straddle_pnl(legs, bars, spot[entry]["open"], deliveries[expiry])
            straddle = {"entry_date": entry, "expiry_date": expiry, "net_pnl_usd": straddle_net, "net_pnl_percent_underlying_notional": straddle_pct}
            straddle_trades.append(straddle)
            prior = (date.fromisoformat(entry) - timedelta(days=1)).isoformat()
            prior_dates = sorted(day for day in spot if day <= prior)
            if prior in dvol and len(prior_dates) >= 31:
                returns = [spot[prior_dates[i]]["close"] / spot[prior_dates[i - 1]]["close"] - 1 for i in range(len(prior_dates) - 30, len(prior_dates))]
                realized_vol = statistics.stdev(returns) * math.sqrt(365) * 100
                if dvol[prior] > realized_vol:
                    gated_straddles.append(straddle)
            adverse_net, adverse_pct = naked_straddle_pnl(legs, bars, spot[entry]["open"], deliveries[expiry], adverse=True)
            adverse_straddles.append({"entry_date": entry, "net_pnl_usd": adverse_net, "net_pnl_percent_underlying_notional": adverse_pct})
        if any(bar is None for bar in bars.values()):
            skipped["missing_entry_candle"] += 1
            continue
        entry_spot, delivery_spot = spot[entry]["open"], deliveries[expiry]
        fills = {key: bar["open"] for key, bar in bars.items()}
        trade = {
            "entry_date": entry, "expiry_date": expiry,
            "entry_spot": entry_spot, "delivery_spot": delivery_spot,
            "contracts": {key: instrument["instrument_name"] for key, instrument in legs.items()},
            **pnl(legs, fills, entry_spot, delivery_spot),
        }
        all_trades.append(trade)
        prior = (date.fromisoformat(entry) - timedelta(days=1)).isoformat()
        prior_dates = sorted(day for day in spot if day <= prior)
        if prior in dvol and len(prior_dates) >= 31:
            returns = [
                spot[prior_dates[i]]["close"] / spot[prior_dates[i - 1]]["close"] - 1
                for i in range(len(prior_dates) - 30, len(prior_dates))
            ]
            realized_vol = statistics.stdev(returns) * math.sqrt(365) * 100
            if dvol[prior] > realized_vol:
                gated_trades.append(trade)
        worst_fills = {
            key: bars[key]["low"] if key.startswith("short") else bars[key]["high"]
            for key in legs
        }
        adverse_trades.append({"entry_date": entry, **pnl(legs, worst_fills, entry_spot, delivery_spot)})

    periods = {
        "2021-2022": lambda trade: trade["entry_date"] < "2023-01-01",
        "2023-2024": lambda trade: "2023-01-01" <= trade["entry_date"] < "2025-01-01",
        "2025-2026": lambda trade: trade["entry_date"] >= "2025-01-01",
    }
    by_period = {
        label: {
            "all_months_first_trade_prices": stats([trade for trade in all_trades if predicate(trade)]),
            "dvol_above_realized_gate": stats([trade for trade in gated_trades if predicate(trade)]),
            "adverse_intraday_range_fills": stats([trade for trade in adverse_trades if predicate(trade)]),
        }
        for label, predicate in periods.items()
    }
    straddle_periods = {
        label: {
            "all_months_open_prices": stats_naked([trade for trade in straddle_trades if predicate(trade)]),
            "dvol_above_realized_gate": stats_naked([trade for trade in gated_straddles if predicate(trade)]),
            "adverse_entry_low_prices": stats_naked([trade for trade in adverse_straddles if predicate(trade)]),
        }
        for label, predicate in periods.items()
    }
    result = {
        "strategy": "Monthly BTC inverse iron fly: sell ATM call/put, buy approximately 5%-OTM wings, hold to last-Friday expiry.",
        "fees": "Historical instrument taker_commission capped at 12.5% of premium; 0.00015 BTC delivery fee on ITM legs, also capped at 12.5% of intrinsic.",
        "entry_price": "Historical option trade-candle open; adverse sensitivity sells at the entry-day low and buys wings at the entry-day high.",
        "data": {"scheduled_entries": len(schedule), "selected_contract_sets": len(selected), "priced_trades": len(all_trades), "dvol_gated_trades": len(gated_trades), "skipped": skipped},
        "by_period": by_period,
        "naked_short_straddle_diagnostic_not_recommended": {
            "warning": "This is not the defined-risk iron fly; short-straddle loss is unbounded and no margin/liquidation path is modeled.",
            "priced_trades": len(straddle_trades),
            "by_period": straddle_periods,
        },
        "trade_log": all_trades,
        "limitations": [
            "Trade-candle prices are not historical bid/ask quotes or guaranteed fills; no queue or liquidation replay.",
            "The adverse OHLC sensitivity is a stress proxy, not proof all legs could fill at those prices.",
            "Binance entry spot sizes strikes; Deribit options and delivery prices determine trade cash flows.",
        ],
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"data": result["data"], "by_period": by_period}, indent=2))

if __name__ == "__main__":
    main()
