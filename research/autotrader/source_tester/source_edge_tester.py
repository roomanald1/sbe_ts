#!/usr/bin/env python3
"""Paper-only capture/test of public positioning and quarter-hour order-flow sources."""
import argparse
import json
import math
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import defaultdict
from pathlib import Path

BYBIT = "https://api.bybit.com"
BINANCE = "https://fapi.binance.com"


def request_json(base, path, params):
    url = base + path + "?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers={"User-Agent": "source-edge-paper-test/1.0"})
    with urllib.request.urlopen(req, timeout=12) as response:
        payload = json.loads(response.read())
    if base == BYBIT and payload.get("retCode") != 0:
        raise RuntimeError(f"Bybit retCode={payload.get('retCode')}: {payload.get('retMsg')}")
    return payload


def bybit_ratio():
    data = request_json(BYBIT, "/v5/market/account-ratio", {
        "category": "linear", "symbol": "BTCUSDT", "period": "5min", "limit": 1,
    })["result"]["list"]
    if not data:
        return None
    row = data[-1]
    buy, sell = float(row["buyRatio"]), float(row["sellRatio"])
    return {"source_timestamp_ms": int(row["timestamp"]), "buy_ratio": buy,
            "sell_ratio": sell, "long_short_ratio": buy / sell if sell else None}


def bybit_price():
    result = request_json(BYBIT, "/v5/market/tickers", {
        "category": "linear", "symbol": "BTCUSDT",
    })["result"]["list"]
    return float(result[0]["lastPrice"])


def binance_price():
    return float(request_json(BINANCE, "/fapi/v1/ticker/price", {"symbol": "BTCUSDT"})["price"])


def binance_boundary_flow(start_ms):
    trades = request_json(BINANCE, "/fapi/v1/aggTrades", {
        "symbol": "BTCUSDT", "startTime": start_ms,
        "endTime": start_ms + 9_999, "limit": 1000,
    })
    buy, sell = 0.0, 0.0
    for trade in trades:
        notional = float(trade["p"]) * float(trade["q"])
        if trade["m"]:  # buyer was maker => sell aggressor
            sell += notional
        else:
            buy += notional
    total = buy + sell
    return {"buy_taker_notional": buy, "sell_taker_notional": sell,
            "total_notional": total,
            "signed_imbalance": (buy - sell) / total if total else None,
            "trade_count": len(trades)}


def forward_close(venue, target_ms, now_ms):
    if venue == "bybit":
        rows = request_json(BYBIT, "/v5/market/kline", {
            "category": "linear", "symbol": "BTCUSDT", "interval": "1",
            "start": target_ms, "end": target_ms + 180_000, "limit": 5,
        })["result"]["list"]
        eligible = [(int(r[0]) + 59_999, float(r[4])) for r in rows
                    if int(r[0]) + 59_999 >= target_ms and int(r[0]) + 59_999 <= now_ms]
        return min(eligible, default=None)
    rows = request_json(BINANCE, "/fapi/v1/klines", {
        "symbol": "BTCUSDT", "interval": "1m", "startTime": target_ms,
        "endTime": target_ms + 180_000, "limit": 5,
    })
    eligible = [(int(r[6]), float(r[4])) for r in rows
                if int(r[6]) >= target_ms and int(r[6]) <= now_ms]
    return min(eligible, default=None)


def write_event(handle, event, **fields):
    handle.write(json.dumps({"recorded_at_ms": int(time.time() * 1000),
                             "event": event, **fields}, separators=(",", ":")) + "\n")
    handle.flush()


def make_candidate(name, venue, entry_ts, entry_price, side, horizons, fields):
    return {"strategy": name, "venue": venue, "entry_timestamp_ms": entry_ts,
            "entry_price": entry_price, "side": side, "horizons": list(horizons),
            "fields": fields, "resolved": set()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--duration-seconds", type=int, default=259200)
    parser.add_argument("--poll-seconds", type=int, default=30)
    parser.add_argument("--round-trip-cost-bps", type=float, default=10.0,
                        help="assumed cost scenario; not an account-specific fee quote")
    parser.add_argument("--positioning-ratio-threshold", type=float, default=1.3)
    parser.add_argument("--flow-imbalance-threshold", type=float, default=0.10)
    args = parser.parse_args()
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    end_capture = time.time() + args.duration_seconds
    pending = []
    seen_ratio_ts = None
    next_boundary = (int(time.time() * 1000) // 900_000 + 1) * 900_000
    summaries = defaultdict(lambda: {"count": 0, "wins": 0, "net_sum": 0.0})
    last_errors = {}
    with output.open("a", encoding="utf-8", buffering=1) as out:
        write_event(out, "run_start", duration_seconds=args.duration_seconds,
                    positioning_ratio_threshold=args.positioning_ratio_threshold,
                    flow_imbalance_threshold=args.flow_imbalance_threshold,
                    assumed_round_trip_cost_bps=args.round_trip_cost_bps,
                    execution="none_paper_only",
                    sources=["Bybit BTCUSDT 5m account ratio", "Binance BTCUSDT quarter-hour aggTrades"])
        while time.time() < end_capture or pending:
            now_ms = int(time.time() * 1000)
            collecting = time.time() < end_capture
            if collecting:
                try:
                    ratio = bybit_ratio()
                    if ratio and ratio["source_timestamp_ms"] != seen_ratio_ts:
                        seen_ratio_ts = ratio["source_timestamp_ms"]
                        price = bybit_price()
                        side = None
                        threshold = args.positioning_ratio_threshold
                        value = ratio["long_short_ratio"]
                        if value is not None:
                            if value >= threshold:
                                side = "short"
                            elif value <= 1.0 / threshold:
                                side = "long"
                        base = {**ratio, "symbol": "BTCUSDT", "venue": "bybit",
                                "received_timestamp_ms": now_ms, "entry_price": price,
                                "strategy": "fade_extreme_bybit_account_ratio"}
                        write_event(out, "source_observation", **base, hypothetical_side=side)
                        if side:
                            horizons = (300, 900)
                            candidate = make_candidate("fade_extreme_bybit_account_ratio", "bybit",
                                                       now_ms, price, side, horizons, base)
                            pending.append(candidate)
                            write_event(out, "candidate", **base, paper_side=side,
                                        horizons_seconds=list(horizons), execution="none_paper_only")
                except Exception as exc:
                    key = f"bybit_ratio:{type(exc).__name__}:{exc}"
                    if last_errors.get("bybit") != key:
                        write_event(out, "source_error", source="bybit_positioning", message=key)
                        last_errors["bybit"] = key

                if now_ms >= next_boundary + 10_000:
                    boundary = next_boundary
                    next_boundary += 900_000
                    try:
                        flow = binance_boundary_flow(boundary)
                        entry_ts = boundary + 10_000
                        price = binance_price()
                        imbalance = flow["signed_imbalance"]
                        side = None
                        if imbalance is not None and abs(imbalance) >= args.flow_imbalance_threshold:
                            side = "long" if imbalance > 0 else "short"
                        base = {**flow, "symbol": "BTCUSDT", "venue": "binance_usdm",
                                "boundary_timestamp_ms": boundary,
                                "received_timestamp_ms": now_ms, "entry_price": price,
                                "strategy": "quarter_hour_opening_signed_flow"}
                        write_event(out, "source_observation", **base, hypothetical_side=side)
                        if side:
                            horizons = (14_400, 28_800, 43_200)
                            candidate = make_candidate("quarter_hour_opening_signed_flow", "binance",
                                                       entry_ts, price, side, horizons, base)
                            pending.append(candidate)
                            write_event(out, "candidate", **base, paper_side=side,
                                        horizons_seconds=list(horizons), execution="none_paper_only")
                    except Exception as exc:
                        key = f"binance_flow:{type(exc).__name__}:{exc}"
                        if last_errors.get("binance") != key:
                            write_event(out, "source_error", source="binance_quarter_hour_flow", message=key)
                            last_errors["binance"] = key

            remaining = []
            for candidate in pending:
                venue, horizon = candidate["venue"], candidate["horizons"]
                for seconds in horizon:
                    if seconds in candidate["resolved"]:
                        continue
                    target = candidate["entry_timestamp_ms"] + seconds * 1000
                    if now_ms < target + 60_000:
                        continue
                    try:
                        close = forward_close(venue, target, now_ms)
                    except Exception as exc:
                        key = f"{venue}_close:{type(exc).__name__}:{exc}"
                        if last_errors.get(f"close_{venue}") != key:
                            write_event(out, "source_error", source=f"{venue}_forward_price", message=key)
                            last_errors[f"close_{venue}"] = key
                        continue
                    if close is None:
                        continue
                    close_ts, exit_price = close
                    direction = 1.0 if candidate["side"] == "long" else -1.0
                    gross = direction * (exit_price / candidate["entry_price"] - 1.0) * 100.0
                    cost_pct = args.round_trip_cost_bps / 100.0
                    net = gross - cost_pct
                    key = f"{candidate['strategy']}:{seconds}"
                    summary = summaries[key]
                    summary["count"] += 1
                    summary["wins"] += int(net > 0)
                    summary["net_sum"] += net
                    write_event(out, "outcome", strategy=candidate["strategy"],
                                venue=venue, symbol="BTCUSDT",
                                entry_timestamp_ms=candidate["entry_timestamp_ms"],
                                exit_timestamp_ms=close_ts, horizon_seconds=seconds,
                                entry_price=candidate["entry_price"], exit_price=exit_price,
                                paper_side=candidate["side"], gross_return_percent=gross,
                                assumed_round_trip_cost_bps=args.round_trip_cost_bps,
                                net_return_percent_at_assumed_cost=net,
                                execution="none_paper_only", signal=candidate["fields"])
                    candidate["resolved"].add(seconds)
                if len(candidate["resolved"]) < len(candidate["horizons"]):
                    remaining.append(candidate)
            pending = remaining
            if int(time.time()) % 300 < args.poll_seconds:
                for key, value in sorted(summaries.items()):
                    if value["count"]:
                        print(f"SUMMARY {key} n={value['count']} positive_net={value['wins']} "
                              f"mean_net_pct={value['net_sum']/value['count']:.6f}", flush=True)
            time.sleep(max(1, args.poll_seconds))
        for key, value in sorted(summaries.items()):
            if value["count"]:
                write_event(out, "summary", strategy_horizon=key, **value,
                            mean_net_percent=value["net_sum"] / value["count"])
        write_event(out, "run_complete", execution="none_paper_only")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
