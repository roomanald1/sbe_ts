#!/usr/bin/env python3
"""Point-in-time monthly low-volatility portfolio screen using CoinMarketCap snapshots."""
import json
import math
import statistics
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data/cryptwatch_weekly_top500_2019_2026.json"
EXTRA = ROOT / "data/cryptwatch_snapshot_2026-10-25.json"
OUT = ROOT / "point_in_time_low_volatility_results.json"
K = 10
WINDOW_WEEKS = 8
COST = 0.0015
STABLE = {"USDT", "USDC", "BUSD", "TUSD", "DAI", "USDE", "FDUSD", "USDD", "UST", "USTC", "USDP", "PYUSD", "FRAX", "USDS", "USD1"}


def monthly_rebalance_dates(dates):
    selected = []
    seen = set()
    for d in dates:
        month = d[:7]
        if month not in seen:
            selected.append(d)
            seen.add(month)
    return selected


def load():
    doc = json.loads(DATA.read_text())
    snapshots = dict(doc["snapshots"])
    if EXTRA.exists():
        extra = json.loads(EXTRA.read_text())["data"]
        snapshots[extra["date"]] = extra["rows"]
    dates = sorted(d for d in snapshots if date.fromisoformat(d).weekday() == 6)
    rows_by_date = {d: {row["slug"]: row for row in snapshots[d]} for d in dates}
    return doc.get("source", "unknown"), dates, snapshots, rows_by_date


def volatility_rank_candidates(dates, snapshots, rows_by_date, index):
    d = dates[index]
    historical_dates = dates[index - WINDOW_WEEKS:index + 1]
    prior = dates[index - WINDOW_WEEKS]
    prior_rows = rows_by_date[prior]
    candidates = []
    for row in snapshots[d]:
        if row.get("rank", 10_000) > 100 or row.get("symbol") in STABLE or row.get("status") != "active":
            continue
        slug = row.get("slug")
        if not row.get("price") or not slug:
            continue
        prices = []
        complete = True
        for hist_date in historical_dates:
            item = rows_by_date[hist_date].get(slug)
            if not item or not item.get("price") or item.get("status") != "active":
                complete = False
                break
            prices.append(float(item["price"]))
        if not complete or len(prices) != WINDOW_WEEKS + 1:
            continue
        weekly_returns = [math.log(prices[j] / prices[j - 1]) for j in range(1, len(prices))]
        vol = statistics.pstdev(weekly_returns)
        if math.isfinite(vol):
            candidates.append((vol, slug, row["symbol"], float(row["price"])))
    return sorted(candidates), prior_rows


def run(dates, snapshots, rows_by_date, selector, side_cost):
    rebalance_dates = monthly_rebalance_dates(dates)
    index_by_date = {d: i for i, d in enumerate(dates)}
    intervals = []
    prior_weights = {}
    for start_date, next_date in zip(rebalance_dates, rebalance_dates[1:]):
        i = index_by_date[start_date]
        if i < WINDOW_WEEKS:
            continue
        ranked, prior_rows = volatility_rank_candidates(dates, snapshots, rows_by_date, i)
        if not ranked:
            continue
        if selector == "low_vol":
            picks = ranked[:K]
        elif selector == "high_vol":
            picks = ranked[-K:]
        elif selector == "momentum":
            picks = sorted(ranked, key=lambda x: x[0])
            prior_prices = {}
            for _, slug, _, _ in ranked:
                old = prior_rows.get(slug)
                current = rows_by_date[start_date].get(slug)
                if old and current and old.get("price") and current.get("price"):
                    prior_prices[slug] = float(current["price"]) / float(old["price"]) - 1
            picks = sorted((row for row in ranked if row[1] in prior_prices),
                           key=lambda row: prior_prices[row[1]], reverse=True)[:K]
        else:
            raise ValueError(selector)
        if not picks:
            continue
        target = {row[1]: 1 / len(picks) for row in picks}
        turnover = sum(abs(target.get(slug, 0) - prior_weights.get(slug, 0))
                       for slug in set(target) | set(prior_weights))
        marks = rows_by_date[next_date]
        gross = 0.0
        missing = 0
        holding_returns = {}
        for _, slug, _, start_price in picks:
            end = marks.get(slug)
            if end and end.get("price") and end.get("status") == "active":
                value = float(end["price"]) / start_price - 1
            else:
                value = -1.0
                missing += 1
            gross += target[slug] * value
            holding_returns[slug] = value
        net = (1 - side_cost * turnover) * (1 + gross) - 1
        intervals.append({
            "date": start_date, "next_date": next_date, "net_return": net,
            "gross_return": gross, "turnover": turnover, "held_count": len(picks),
            "eligible_count": len(ranked), "missing_next_mark_count": missing,
        })
        growth = 1 + gross
        prior_weights = {
            slug: weight * (1 + holding_returns.get(slug, -1)) / growth
            for slug, weight in target.items() if growth > 0 and holding_returns.get(slug, -1) > -1
        }
    return intervals


def stats(rows):
    equity = peak = 1.0
    dd = 0.0
    for row in rows:
        equity *= 1 + row["net_return"]
        peak = max(peak, equity)
        dd = min(dd, equity / peak - 1)
    vals = [row["net_return"] for row in rows]
    avg = statistics.mean(vals) if vals else 0
    sd = statistics.stdev(vals) if len(vals) > 1 else 0
    return {
        "monthly_periods": len(rows),
        "return_percent": 100 * (equity - 1),
        "max_drawdown_percent": 100 * dd,
        "annualized_monthly_sharpe": avg / sd * math.sqrt(12) if sd else 0,
        "rebalance_events": len(rows),
        "absolute_weight_turnover": sum(row["turnover"] for row in rows),
        "missing_next_mark_count": sum(row["missing_next_mark_count"] for row in rows),
        "average_eligible_count": statistics.mean(row["eligible_count"] for row in rows) if rows else 0,
    }


def main():
    source, dates, snapshots, rows_by_date = load()
    rebalance_dates = monthly_rebalance_dates(dates)
    result = {
        "source": source,
        "snapshot_range": [dates[0], dates[-1]],
        "rebalance_range": [rebalance_dates[0], rebalance_dates[-1]],
        "universe": "Point-in-time active top-100 at each rebalance; stablecoins excluded; require complete prior eight-week price history; hold top 10 by eight-week realized volatility",
        "timing": "First available Sunday snapshot each month; hold equal-weight top-10 for one month; missing next snapshot mark is treated as total loss",
        "cost": "15, 30, 50 bps per traded dollar on absolute weight turnover; includes entry and exit",
        "selection_warning": "The rule is predeclared from 2026 low-vol literature, but this market history and point-in-time universe have been inspected in other repo research; this is discovery, not a clean confirmatory holdout.",
        "policies": {},
    }
    for selector in ("low_vol", "high_vol", "momentum"):
        result["policies"][selector] = {}
        for bps in (15, 30, 50):
            intervals = run(dates, snapshots, rows_by_date, selector, bps / 10000)
            segments = {
                "train_2019_2022": [row for row in intervals if row["date"] < "2023-01-01"],
                "validation_2023_2024": [row for row in intervals if "2023-01-01" <= row["date"] < "2025-01-01"],
                "later_2025_2026": [row for row in intervals if row["date"] >= "2025-01-01"],
            }
            result["policies"][selector][str(bps)] = {
                key: stats(rows) for key, rows in segments.items()
            }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({
        name: {
            period: {
                "return_percent": result["policies"][name]["15"][period]["return_percent"],
                "max_drawdown_percent": result["policies"][name]["15"][period]["max_drawdown_percent"],
                "at_50bp": result["policies"][name]["50"][period]["return_percent"],
            } for period in ("train_2019_2022", "validation_2023_2024", "later_2025_2026")
        } for name in result["policies"]
    }, indent=2))


if __name__ == "__main__":
    main()
