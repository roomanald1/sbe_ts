#!/usr/bin/env python3
"""Weekly point-in-time cross-sectional momentum using public CMC snapshots.

Source: CryptWatch, based on CoinMarketCap. Pulls weekly snapshots, ranks only
assets actually in the top 100 at each rebalance, and uses next-snapshot marks
from the top 500. Missing next marks are conservatively treated as total loss.
"""
import json, time, urllib.request
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OUT = ROOT / 'data/cryptwatch_weekly_top500_2019_2026.json'
RESULT = ROOT / 'spot_point_in_time_weekly_results.json'
BASE = 'https://cryptwat.ch/api/v1'
UA = 'historical-strategy-research (local backtest; contact: research@example.invalid)'
COST = 0.0015
STABLE = {'USDT','USDC','BUSD','TUSD','DAI','USDE','FDUSD','USDD','UST','USTC','USDP','PYUSD','FRAX','USDS','USD1'}

def get_json(url):
    req = urllib.request.Request(url, headers={'User-Agent': UA})
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)

def fetch():
    if OUT.exists():
        return json.loads(OUT.read_text())
    dates_doc = get_json(BASE + '/snapshots')['data']
    dates = sorted(x['date'] for x in dates_doc if x['date'] >= '2019-01-01' and date.fromisoformat(x['date']).weekday() == 6)
    points = {}
    for n, d in enumerate(dates, 1):
        doc = get_json(f'{BASE}/snapshots/{d}?top=500')
        payload = doc['data']
        points[payload['date']] = payload['rows']
        if n % 20 == 0 or n == len(dates):
            OUT.parent.mkdir(parents=True, exist_ok=True)
            OUT.write_text(json.dumps({'source': doc.get('source'), 'requested_dates': dates, 'snapshots': points}, separators=(',', ':')))
            print(f'fetched {n}/{len(dates)} snapshots through {payload["date"]}', flush=True)
        # API fair-use limit is 60/minute.
        if n < len(dates): time.sleep(1.05)
    return json.loads(OUT.read_text())

def stats(returns):
    wealth = peak = 1.0
    dd = 0.0
    for r in returns:
        wealth *= 1 + r
        peak = max(peak, wealth)
        dd = min(dd, wealth / peak - 1)
    avg = sum(returns) / len(returns) if returns else 0.0
    var = sum((r - avg) ** 2 for r in returns) / max(1, len(returns) - 1) if returns else 0.0
    return {'return_pct': (wealth - 1) * 100, 'sharpe': avg / var ** .5 * 52 ** .5 if var else 0,
            'max_dd_pct': dd * 100, 'weeks': len(returns)}

def evaluate(snapshots, dates, rows_by_date, holdings):
    weights = {}
    samples = []
    for i in range(12, len(dates) - 1):
        d, nxt = dates[i], dates[i + 1]
        prior = rows_by_date[dates[i - 12]]
        eligible = []
        for row in snapshots[d]:
            if row['rank'] > 100 or row['symbol'] in STABLE or row['status'] != 'active':
                continue
            old = prior.get(row['slug'])
            if old and old.get('price', 0) and row.get('price', 0):
                eligible.append((row['price'] / old['price'] - 1, row['slug'], row['symbol']))
        picks = sorted(eligible, reverse=True)[:holdings]
        target = {slug: 1.0 / len(picks) for _, slug, _ in picks} if picks else {}
        turnover = sum(abs(target.get(k, 0.0) - weights.get(k, 0.0)) for k in set(target) | set(weights))
        marks = rows_by_date[nxt]
        gross = 0.0
        missing = 0
        for slug, w in target.items():
            start_price = rows_by_date[d][slug]['price']
            end = marks.get(slug)
            if end and end.get('price'):
                gross += w * (end['price'] / start_price - 1)
            else:
                # Conservative terminal mark for a selected coin missing from top 500.
                gross -= w
                missing += 1
        samples.append({'date': d, 'next_date': nxt, 'symbols': [symbol for _, _, symbol in picks],
                        'gross_return': gross, 'turnover': turnover, 'net_return': gross - COST * turnover,
                        'eligible_count': len(eligible), 'missing_next_mark_count': missing})
        weights = target
    segments = {'train_2019_2022': [], 'validation_2023_2024': [], 'holdout_2025_2026': []}
    for x in samples:
        k = 'train_2019_2022' if x['date'] < '2023-01-01' else 'validation_2023_2024' if x['date'] < '2025-01-01' else 'holdout_2025_2026'
        segments[k].append(x['net_return'])
    return {'results': {k: stats(v) for k, v in segments.items()},
            'full_period': stats([x['net_return'] for x in samples]),
            'missing_next_mark_count': sum(x['missing_next_mark_count'] for x in samples),
            'weekly_observations': len(samples), 'sample_trade_log': samples}

def main():
    fetched = fetch()
    snapshots = fetched['snapshots']
    dates = sorted(snapshots)
    rows_by_date = {d: {r['slug']: r for r in snapshots[d]} for d in dates}
    variants = {f'top_{n}_equal_weight': evaluate(snapshots, dates, rows_by_date, n) for n in [1, 5, 10]}
    report = {
        'source': 'CryptWatch public API, data based on CoinMarketCap; https://cryptwat.ch/api',
        'snapshot_count': len(dates), 'snapshot_range': [dates[0], dates[-1]],
        'universe': 'At each weekly rebalance, assets in that point-in-time snapshot top 100; current top 500 snapshot used to mark next-week exits; stablecoins excluded by symbol list in script',
        'strategy': 'Rank top-100 assets by trailing 12-week price return; hold top 1, top 5 or top 10; equal weight; hold one week; weekly rebalance; cash if no eligible assets',
        'execution_and_cost': 'Snapshot price rebalance, no bid/ask or market-hours model; 15 bps per side on weight turnover. Missing held asset from next top-500 snapshot is treated as a 100% loss on that asset allocation.',
        'splits': 'Train 2019-2022; validation 2023-2024; holdout 2025 through latest Sunday snapshot; fixed rule across splits.',
        'variants': variants,
        'limitations': 'Snapshots are weekly before 18 Dec 2021 and daily thereafter, sampled on Sundays. Ranking snapshots, not exchange OHLCV; source coverage can be incomplete; token redenominations, airdrops, forks, symbol/contract identity, exchange availability, liquidity, and execution costs are not fully modeled. Missing next top-500 prices are treated as total loss, a conservative proxy that may overstate losses for assets merely falling below rank 500.'
    }
    RESULT.write_text(json.dumps(report, indent=2))
    for name, v in variants.items():
        print(name, json.dumps({k: v[k] for k in ['results', 'full_period', 'missing_next_mark_count', 'weekly_observations']}), flush=True)
    print('saved', RESULT, flush=True)

if __name__ == '__main__': main()
