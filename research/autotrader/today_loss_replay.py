#!/usr/bin/env python3
"""Replay the paper fill ledger to split realized P&L into price and fee effects."""
import argparse
import collections
import datetime as dt
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / "src/rust/position_management/auto_trader_state.json"
LEDGER = ROOT / "src/rust/position_management/positions.json"


def replay(trades, rates, start_ms, end_ms):
    positions = collections.defaultdict(lambda: [0.0, 0.0, 0.0])
    by_symbol = collections.defaultdict(lambda: [0, 0.0, 0.0, 0.0, 0.0])
    fills = 0
    all_fees = 0.0
    for trade in sorted(trades, key=lambda row: row["timestamp"]):
        timestamp = trade["timestamp"]
        if timestamp >= end_ms:
            break
        symbol = trade["symbol"]
        rate = rates.get(symbol.rsplit("-", 1)[-1], 1.0)
        quantity, average, entry_fees = positions[symbol]
        incoming = trade["quantity"] * (1 if trade["side"] == "buy" else -1)
        fee = -abs(trade.get("fee") or 0.0)
        if timestamp >= start_ms:
            fills += 1
            all_fees += -fee * rate
        if quantity == 0 or quantity * incoming > 0:
            total = abs(quantity) + abs(incoming)
            positions[symbol] = [
                quantity + incoming,
                (abs(quantity) * average + abs(incoming) * trade["execution_price"]) / total,
                entry_fees + fee,
            ]
            continue
        closed = min(abs(quantity), abs(incoming))
        allocated_entry_fee = entry_fees * closed / abs(quantity)
        allocated_close_fee = fee * closed / abs(incoming)
        gross = closed * (
            trade["execution_price"] - average if quantity > 0
            else average - trade["execution_price"]
        )
        remaining = quantity + incoming
        if abs(remaining) < 1e-10:
            positions[symbol] = [0.0, 0.0, 0.0]
        elif remaining * quantity < 0:
            positions[symbol] = [remaining, trade["execution_price"], fee - allocated_close_fee]
        else:
            positions[symbol] = [remaining, average, entry_fees - allocated_entry_fee]
        if timestamp >= start_ms:
            row = by_symbol[symbol]
            row[0] += 1
            row[1] += (gross + allocated_entry_fee + allocated_close_fee) * rate
            row[2] += gross * rate
            row[3] += (allocated_entry_fee + allocated_close_fee) * rate
            row[4] += closed * average * rate
    def labeled(row):
        return dict(closing_fills=row[0], net_usd=row[1], gross_usd=row[2],
                    allocated_fees_usd=row[3], closed_entry_notional_usd=row[4])
    totals = [sum(row[i] for row in by_symbol.values()) for i in range(5)]
    return dict(trade_fills_today=fills, all_fill_fees_today_usd=all_fees,
                realized=labeled(totals),
                by_symbol={key: labeled(value) for key, value in sorted(by_symbol.items())})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--day", default="2026-10-27")
    parser.add_argument("--as-of", help="UTC ISO timestamp; default current time")
    args = parser.parse_args()
    start = dt.datetime.fromisoformat(args.day).replace(tzinfo=dt.timezone.utc)
    end = dt.datetime.fromisoformat(args.as_of.replace("Z", "+00:00")) if args.as_of else dt.datetime.now(dt.timezone.utc)
    state = json.loads(STATE.read_text())
    ledger = json.loads(LEDGER.read_text())
    result = replay(ledger["trades"], state["usd_rates"],
                    int(start.timestamp() * 1000), int(end.timestamp() * 1000))
    result["day"] = args.day
    result["as_of_utc"] = end.isoformat()
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
