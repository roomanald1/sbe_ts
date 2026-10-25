#!/usr/bin/env python3
"""Download public Binance 1m klines and screen simple passive spot quoting rules.

This is a bar-touch proxy, not a queue-aware order book replay. Orders are assumed
maker fills at the limit when a 1m candle touches it. Both-side ambiguity and
OHLC path ordering are avoided by maintaining one long-only inventory unit.
"""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import time
import zipfile
import urllib.parse
import urllib.request
from collections import defaultdict, deque
from datetime import datetime, timezone, timedelta

BASE = "https://data.binance.vision/data/spot/monthly/klines"


def fetch(symbol, start_ms, end_ms, cache_dir):
    start = datetime.fromtimestamp(start_ms / 1000, timezone.utc).date().replace(day=1)
    end = datetime.fromtimestamp((end_ms - 1) / 1000, timezone.utc).date().replace(day=1)
    months, cursor = [], start
    while cursor <= end:
        months.append((cursor.year, cursor.month))
        cursor = (cursor.replace(day=28) + timedelta(days=4)).replace(day=1)
    rows = []
    for year, month in months:
        ym = f"{year}-{month:02d}"
        filename = f"{symbol}-1m-{ym}.zip"
        url = f"{BASE}/{symbol}/1m/{filename}"
        cache_path = cache_dir / filename
        checksum_path = cache_dir / (filename + ".CHECKSUM")
        if cache_path.exists():
            print(f"Using cached {cache_path}", flush=True)
            archive = cache_path.read_bytes()
        else:
            print(f"Downloading {url}", flush=True)
            with urllib.request.urlopen(url, timeout=60) as response:
                archive = response.read()
            cache_dir.mkdir(parents=True, exist_ok=True)
            cache_path.write_bytes(archive)
        if checksum_path.exists():
            expected = checksum_path.read_text(encoding="utf-8").split()[0]
        else:
            checksum_url = url + ".CHECKSUM"
            with urllib.request.urlopen(checksum_url, timeout=30) as response:
                checksum_text = response.read().decode()
            expected = checksum_text.split()[0]
            cache_dir.mkdir(parents=True, exist_ok=True)
            checksum_path.write_text(checksum_text, encoding="utf-8")
        actual = hashlib.sha256(archive).hexdigest()
        if actual != expected:
            raise ValueError(f"SHA-256 mismatch for {filename}: {actual} != {expected}")
        with zipfile.ZipFile(io.BytesIO(archive)) as zf:
            member = zf.namelist()[0]
            text = io.TextIOWrapper(zf.open(member), encoding="utf-8")
            for row in csv.reader(text):
                ts = int(row[0])
                # Binance spot archives switched from ms to microseconds in 2025.
                if ts >= 100_000_000_000_000:
                    ts //= 1000
                if start_ms <= ts < end_ms:
                    rows.append((ts, float(row[1]), float(row[2]), float(row[3]), float(row[4])))
    rows.sort(key=lambda r: r[0])
    return rows


def run(rows, offset_bps, fee, gate=False):
    if len(rows) < 2:
        return []
    inventory = None
    trades = []
    past_ranges = deque(maxlen=1440)
    rolling_sum = 0.0
    rolling_sum_sq = 0.0
    for i, (ts, op, hi, lo, close) in enumerate(rows):
        prev_close = rows[i - 1][4]
        current_range = (hi - lo) / prev_close * 10_000
        # Causal approximate 90th-percentile gate: prior 24h mean + 1.282 SD.
        high_vol = False
        if len(past_ranges) >= 1440:
            mean = rolling_sum / 1440
            variance = max(0.0, rolling_sum_sq / 1440 - mean * mean)
            high_vol = current_range > mean + 1.28155 * variance ** 0.5
        if gate and high_vol and inventory is None:
            oldest = past_ranges.popleft() if len(past_ranges) == 1440 else 0.0
            rolling_sum -= oldest
            rolling_sum_sq -= oldest * oldest
            past_ranges.append(current_range)
            rolling_sum += current_range
            rolling_sum_sq += current_range * current_range
            continue
        distance = offset_bps / 10_000
        if inventory is None:
            bid = prev_close * (1 - distance)
            if op > bid and lo <= bid:
                inventory = (bid, ts)
        else:
            buy, opened = inventory
            ask = prev_close * (1 + distance)
            if op < ask and hi >= ask:
                gross = ask / buy - 1
                net = (ask / buy) * (1 - fee) ** 2 - 1
                trades.append({"ts": ts, "open_ts": opened, "gross_pct": gross * 100,
                               "net_pct": net * 100, "net_usd_100": net * 100,
                               "hold_minutes": (ts - opened) / 60_000})
                inventory = None
        oldest = past_ranges.popleft() if len(past_ranges) == 1440 else 0.0
        rolling_sum -= oldest
        rolling_sum_sq -= oldest * oldest
        past_ranges.append(current_range)
        rolling_sum += current_range
        rolling_sum_sq += current_range * current_range
    if inventory is not None:
        buy, opened = inventory
        exit_price = rows[-1][4]
        net = (exit_price / buy) * (1 - fee) ** 2 - 1
        trades.append({"ts": rows[-1][0], "open_ts": opened, "gross_pct": (exit_price / buy - 1) * 100,
                       "net_pct": net * 100, "net_usd_100": net * 100,
                       "hold_minutes": (rows[-1][0] - opened) / 60_000, "forced_close": True})
    return trades


def summarize(trades):
    if not trades:
        return {"n": 0, "mean_net_pct": None, "win_pct": None, "net_usd_at_100": 0,
                "median_hold_minutes": None}
    vals = sorted(t["net_pct"] for t in trades)
    equity, peak, max_dd = 0.0, 0.0, 0.0
    for t in sorted(trades, key=lambda x: x["ts"]):
        equity += t["net_usd_100"]
        peak = max(peak, equity)
        max_dd = max(max_dd, peak - equity)
    gross = [t["gross_pct"] for t in trades]
    return {"n": len(trades), "mean_gross_pct": sum(gross) / len(gross),
            "mean_net_pct": sum(vals) / len(vals),
            "median_net_pct": vals[len(vals)//2],
            "win_pct": sum(v > 0 for v in vals) / len(vals) * 100,
            "net_usd_at_100_per_trade": sum(t["net_usd_100"] for t in trades),
            "max_drawdown_usd_at_100_per_trade": max_dd,
            "median_hold_minutes": sorted(t["hold_minutes"] for t in trades)[len(trades)//2]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--start-date", default="2026-07-01")
    p.add_argument("--end-date", default="2026-10-01", help="exclusive UTC date; monthly archives required")
    p.add_argument("--symbols", nargs="+", default=["BTCUSDT", "ETHUSDT", "SOLUSDT"])
    p.add_argument("--maker-fee-bps", type=float, default=10.0)
    p.add_argument("--cache-dir", default=str(Path(__file__).resolve().parent / "data" / "binance_spot_1m_2026q3"))
    p.add_argument("--output", default="")
    args = p.parse_args()
    start = datetime.fromisoformat(args.start_date).replace(tzinfo=timezone.utc)
    end = datetime.fromisoformat(args.end_date).replace(tzinfo=timezone.utc)
    start_ms, end_ms = int(start.timestamp() * 1000), int(end.timestamp() * 1000)
    fee = args.maker_fee_bps / 10_000
    cache_dir = Path(args.cache_dir)
    all_results = {"source": "Binance public spot 1m klines monthly archives", "source_url": "https://github.com/binance/binance-public-data",
                   "start_utc": start.isoformat(), "end_utc": end.isoformat(), "days": (end-start).days,
                   "symbols": {}, "assumptions": {"maker_fee_bps_per_side": args.maker_fee_bps,
                   "orders": "long-only cycle: post bid while flat, ask while long",
                   "fill": "assume fill at limit only when the candle opens non-marketable and its low/high crosses; no queue, latency, partial-fill model",
                   "gate": "high-volatility pause blocks new bid entries only; inventory exit orders remain active",
                   "exit": "maker ask touch; any final open inventory marked to close with maker fee proxy",
                   "bar_path": "no simultaneous bid/ask fills; one resting order at a time",
                   "split": "first 70% of elapsed time vs final 30%, no parameter tuning",
                   "source_integrity": "official per-file SHA-256 CHECKSUM verified"}, "strategies": {}}
    pooled = defaultdict(lambda: {"train": [], "holdout": []})
    for symbol in args.symbols:
        rows = fetch(symbol, start_ms, end_ms, cache_dir)
        print(f"Verified and loaded {len(rows):,} bars for {symbol}", flush=True)
        if not rows:
            all_results["symbols"][symbol] = {"error": "no rows returned"}
            continue
        split_ts = rows[0][0] + int((rows[-1][0] - rows[0][0]) * .7)
        symbol_out = {"bars": len(rows), "first_utc": datetime.fromtimestamp(rows[0][0]/1000, timezone.utc).isoformat(),
                      "last_utc": datetime.fromtimestamp(rows[-1][0]/1000, timezone.utc).isoformat(),
                      "strategies": {}}
        specs = [(f"fixed_{x}bp", x, False) for x in (2, 5, 10, 20, 30, 50)]
        specs += [(f"skip_top10pct_vol_{x}bp", x, True) for x in (5, 10, 20)]
        for name, distance, gate in specs:
            trades = run(rows, distance, fee, gate)
            train = [t for t in trades if t["ts"] < split_ts]
            test = [t for t in trades if t["ts"] >= split_ts]
            symbol_out["strategies"][name] = {"train": summarize(train), "holdout": summarize(test)}
            pooled[name]["train"].extend(train)
            pooled[name]["holdout"].extend(test)
        all_results["symbols"][symbol] = symbol_out
    all_results["strategies"] = {name: {period: summarize(trades) for period, trades in slices.items()}
                                  for name, slices in pooled.items()}
    rendered = json.dumps(all_results, indent=2)
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(rendered + "\n")
    print(rendered)


if __name__ == "__main__":
    main()
