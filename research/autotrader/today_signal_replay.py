#!/usr/bin/env python3
"""Read-only chronological screen of recorded executable signal outcomes."""
import argparse
import collections
import datetime as dt
import gzip
import json
import sqlite3
import statistics
from pathlib import Path

from current_signal_exit_audit import Reader

DB = Path(__file__).resolve().parents[2] / "src/rust/insights/signals.sqlite"


def summary(items):
    directed = [
        item["long_net"] if item["direction"] == "bullish" else item["short_net"]
        for item in items if item["direction"] in ("bullish", "bearish")
    ]
    costs = [item["long_gross"] - item["long_net"] for item in items]
    moves = [abs(item["exit_reference"] / item["entry_reference"] - 1) * 100
             for item in items if item["entry_reference"] > 0]
    return dict(nonoverlapping_observations=len(items), directional_observations=len(directed),
                directional_mean_net_percent=statistics.mean(directed) if directed else None,
                directional_win_rate=sum(x > 0 for x in directed) / len(directed) if directed else None,
                median_roundtrip_cost_percent=statistics.median(costs) if costs else None,
                median_absolute_reference_move_percent=statistics.median(moves) if moves else None)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--day", default="2026-10-27")
    parser.add_argument("--as-of", help="UTC ISO timestamp; default current time")
    parser.add_argument("--sample-modulus", type=int, default=97)
    args = parser.parse_args()
    start = dt.datetime.fromisoformat(args.day).replace(tzinfo=dt.timezone.utc)
    end = dt.datetime.fromisoformat(args.as_of.replace("Z", "+00:00")) if args.as_of else dt.datetime.now(dt.timezone.utc)
    start_ms, end_ms = int(start.timestamp() * 1000), int(end.timestamp() * 1000)
    split_ms = start_ms + 12 * 60 * 60 * 1000
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    rows = collections.defaultdict(list)
    for (payload,) in db.execute(
        "select payload from signal_records where kind=2 and id%?=0 and timestamp>=? and timestamp<?",
        (args.sample_modulus, start_ms, end_ms),
    ):
        try:
            item = Reader(gzip.decompress(payload)).evaluation()
        except (ValueError, OSError, EOFError):
            continue
        if (item["h"] in (60, 300, 900, 3600)
                and item["t"] >= start_ms
                and item["long_net"] is not None and item["short_net"] is not None):
            rows[item["h"]].append(item)
    result = dict(day=args.day, as_of_utc=end.isoformat(), sample_modulus=args.sample_modulus, horizons={})
    for horizon in (60, 300, 900, 3600):
        next_free, selected = {}, []
        for item in sorted(rows[horizon], key=lambda row: (row["t"], row["symbol"])):
            if item["t"] >= next_free.get(item["symbol"], 0):
                selected.append(item)
                next_free[item["symbol"]] = item["end"]
        result["horizons"][str(horizon)] = dict(
            sampled=len(rows[horizon]),
            first_12h=summary([item for item in selected if item["t"] < split_ms]),
            second_period=summary([item for item in selected if item["t"] >= split_ms]),
        )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
