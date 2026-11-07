#!/usr/bin/env python3
"""Screen signal-database features against execution-adjusted outcomes.

Read-only inputs come from the local Insights GET endpoints. The newest bounded
feature/evaluation windows are used because the API deliberately caps responses.
"""
import argparse
import bisect
import json
import math
import statistics
import urllib.parse
import urllib.request
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path


HORIZONS = (60, 300, 900)
EVALUATION_LIMIT = 150_000
FEATURE_LIMIT = 50_000
EXTERNAL_LIMIT = 50_000


def get_json(url):
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=180) as response:
        return json.load(response)


def endpoint(base, path, **query):
    return base.rstrip("/") + "/" + path + "?" + urllib.parse.urlencode(query)


def indexed_records(records, key, time_key):
    result = defaultdict(list)
    for record in records:
        result[record[key]].append(record)
    for items in result.values():
        items.sort(key=lambda row: row[time_key])
    return result


def prior_record(index, key, timestamp, max_age_ms):
    items = index.get(key, ())
    if not items:
        return None
    times = [row["timestamp"] for row in items]
    position = bisect.bisect_right(times, timestamp) - 1
    if position < 0 or timestamp - times[position] > max_age_ms:
        return None
    return items[position]


def net_for_side(row, side):
    return row["long_net_percent"] if side == "long" else row["short_net_percent"]


def summarize(values):
    if not values:
        return {"samples": 0, "mean_net_percent": None, "median_net_percent": None,
                "positive_net_rate": None, "positive_net_wilson_95": None}
    count = len(values)
    positives = sum(value > 0 for value in values)
    rate = positives / count
    z = 1.959963984540054
    denom = 1 + z * z / count
    center = (rate + z * z / (2 * count)) / denom
    margin = z * math.sqrt(rate * (1 - rate) / count + z * z / (4 * count * count)) / denom
    return {
        "samples": count,
        "mean_net_percent": statistics.fmean(values),
        "median_net_percent": statistics.median(values),
        "positive_net_rate": rate,
        "positive_net_wilson_95": [max(0.0, center - margin), min(1.0, center + margin)],
    }


def non_overlapping(evaluations):
    """Keep a deterministic horizon-spaced sample per symbol, before filtering."""
    result = []
    last_kept = {}
    for row in sorted(evaluations, key=lambda item: (item["symbol"], item["horizon_seconds"], item["signal_timestamp"])):
        horizon = row["horizon_seconds"]
        if horizon not in HORIZONS or row["signal_direction"] not in ("bullish", "bearish"):
            continue
        net = net_for_side(row, "long" if row["signal_direction"] == "bullish" else "short")
        if net is None:
            continue
        key = (row["symbol"], horizon)
        timestamp = row["signal_timestamp"]
        if timestamp - last_kept.get(key, -horizon * 1000) < horizon * 1000:
            continue
        last_kept[key] = timestamp
        result.append((row, net))
    return result


def analyze(api_base):
    evaluations = get_json(endpoint(api_base, "signals/evaluations", limit=EVALUATION_LIMIT,
                                   horizons=",".join(map(str, HORIZONS))))
    features = get_json(endpoint(api_base, "signals/research-features", limit=FEATURE_LIMIT))
    external = get_json(endpoint(api_base, "signals/external-market", limit=EXTERNAL_LIMIT))
    feature_index = indexed_records(features, "symbol", "timestamp")
    external_index = indexed_records(external, "underlying", "timestamp")
    sample = non_overlapping(evaluations)

    results = {}
    total_baseline = []
    total_confirmed = []
    total_disconfirmed = []
    total_squeeze = []
    detail = {}

    for horizon in HORIZONS:
        base, confirmed, disconfirmed, squeeze = [], [], [], []
        feature_joined = 0
        external_joined = 0
        for row, baseline_net in sample:
            if row["horizon_seconds"] != horizon:
                continue
            total_baseline.append(baseline_net)
            base.append(baseline_net)

            feature = prior_record(feature_index, row["symbol"], row["signal_timestamp"], 30_000)
            if feature is not None:
                feature_joined += 1
                btc_return = feature.get("btc_return_5m_percent")
                if btc_return is not None and btc_return != 0:
                    signal_sign = 1 if row["signal_direction"] == "bullish" else -1
                    if btc_return * signal_sign > 0:
                        confirmed.append(baseline_net)
                        total_confirmed.append(baseline_net)
                    else:
                        disconfirmed.append(baseline_net)
                        total_disconfirmed.append(baseline_net)

            underlying = row["symbol"].split("-", 1)[0]
            if underlying not in ("BTC", "ETH") or row["symbol"] not in ("BTC-USDT", "ETH-USDT"):
                continue
            market = prior_record(external_index, underlying, row["signal_timestamp"], 120_000)
            if market is None:
                continue
            external_joined += 1
            funding = market.get("funding_rate")
            basis = market.get("perp_basis_bps")
            oi_change = market.get("open_interest_change_percent")
            if funding is None or basis is None or oi_change is None or oi_change <= 0 or funding == 0 or basis == 0:
                continue
            # Crowded side is considered confirmed only when funding and basis
            # point the same way; trade against it while open interest expands.
            if funding * basis <= 0:
                continue
            squeeze_side = "short" if funding > 0 else "long"
            squeeze_net = net_for_side(row, squeeze_side)
            if squeeze_net is not None:
                squeeze.append(squeeze_net)
                total_squeeze.append(squeeze_net)

        detail[str(horizon)] = {
            "directional_signal_baseline": summarize(base),
            "btc_5m_confirms_signal": summarize(confirmed),
            "btc_5m_disagrees_with_signal_control": summarize(disconfirmed),
            "perp_crowding_squeeze_btc_eth": summarize(squeeze),
            "feature_joined_samples": feature_joined,
            "btc_eth_external_joined_samples": external_joined,
        }

    results["overall"] = {
        "directional_signal_baseline": summarize(total_baseline),
        "btc_5m_confirms_signal": summarize(total_confirmed),
        "btc_5m_disagrees_with_signal_control": summarize(total_disconfirmed),
        "perp_crowding_squeeze_btc_eth": summarize(total_squeeze),
    }
    results["by_horizon_seconds"] = detail
    results["metadata"] = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "api_base": api_base.rstrip("/"),
        "query_limits": {"evaluations": EVALUATION_LIMIT, "research_features": FEATURE_LIMIT,
                         "external_market": EXTERNAL_LIMIT},
        "evaluation_records_fetched": len(evaluations),
        "research_feature_records_fetched": len(features),
        "external_market_records_fetched": len(external),
        "evaluation_window_utc": {
            "first_signal": datetime.fromtimestamp(min(r["signal_timestamp"] for r in evaluations) / 1000, timezone.utc).isoformat() if evaluations else None,
            "last_signal": datetime.fromtimestamp(max(r["signal_timestamp"] for r in evaluations) / 1000, timezone.utc).isoformat() if evaluations else None,
        },
        "non_overlapping_directional_evaluation_samples": len(sample),
        "point_in_time_join_rules": {"research_features_max_age_seconds": 30,
                                     "external_market_max_age_seconds": 120,
                                     "feature_timestamp_must_not_exceed_signal_timestamp": True},
        "candidate_rules": {
            "btc_confirmation": "Trade the evaluation signal direction only when the most recent prior BTC 5m return has the same sign.",
            "perp_squeeze": "BTC-USDT and ETH-USDT only; when funding and perp basis have the same nonzero sign and open-interest change is positive, trade opposite that crowded side.",
        },
        "limitations": [
            "Outcomes overlap across symbols and market regimes even after horizon-spaced sampling per symbol.",
            "The endpoint returns only its newest bounded sample; results describe this window, not a full-market cycle.",
            "Net outcome labels use the database's recorded entry/exit quote and taker-fee assumptions; actual live fill quality may differ.",
            "The perp squeeze screen covers only BTC-USDT and ETH-USDT because external-market features are available only for BTC and ETH.",
        ],
    }
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-base", default="http://127.0.0.1:3004")
    parser.add_argument("--output", type=Path, default=Path(__file__).with_name("signal_feature_hypothesis_screen_results.json"))
    args = parser.parse_args()
    result = analyze(args.api_base)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    print(f"Saved {args.output}")


if __name__ == "__main__":
    main()
