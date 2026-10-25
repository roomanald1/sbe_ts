#!/usr/bin/env python3
"""Compare simple long-only spot strategy families on Binance daily candles."""
import json
from datetime import datetime, timezone
from pathlib import Path
ROOT = Path(__file__).resolve().parent
DATA = ROOT / 'data/binance_spot_1d_api_2019_2026'
SYMBOLS = ['BTCUSDT', 'ETHUSDT', 'BNBUSDT', 'XRPUSDT', 'SOLUSDT', 'ADAUSDT', 'DOGEUSDT']
COST = 0.0015

def load():
    data = {}
    for symbol in SYMBOLS:
        rows = json.loads((DATA / f'{symbol}-1d.json').read_text())
        data[symbol] = {datetime.fromtimestamp(row[0] / 1000, timezone.utc).strftime('%Y-%m-%d'):
                        {'o': float(row[1]), 'h': float(row[2]), 'l': float(row[3]), 'c': float(row[4])} for row in rows}
    dates = sorted(set.intersection(*(set(data[s]) for s in SYMBOLS)))
    return dates, data

def stats(rs):
    wealth = peak = 1.0
    max_dd = 0.0
    for r in rs:
        wealth *= 1 + r
        peak = max(peak, wealth)
        max_dd = min(max_dd, wealth / peak - 1)
    avg = sum(rs) / len(rs) if rs else 0
    var = sum((r - avg) ** 2 for r in rs) / max(1, len(rs) - 1) if rs else 0
    return {'return_pct': 100 * (wealth - 1), 'sharpe': avg / var ** .5 * 365 ** .5 if var else 0,
            'max_dd_pct': 100 * max_dd, 'days': len(rs)}

def backtest(dates, data, family, lookback=55, cost=COST):
    weights = {s: 0.0 for s in SYMBOLS}
    daily = []
    hold_left = {s: 0 for s in SYMBOLS}
    for i in range(202, len(dates) - 1):
        prior = dates[i - 1]
        target = {s: 0.0 for s in SYMBOLS}
        if family == 'relative_strength_rotation':
            eligible = []
            for s in SYMBOLS:
                closes = [data[s][dates[k]]['c'] for k in range(i - 200, i)]
                if data[s][prior]['c'] > sum(closes) / len(closes):
                    mom = data[s][prior]['c'] / data[s][dates[i - 91]]['c'] - 1
                    eligible.append((mom, s))
            if eligible:
                target[max(eligible)[1]] = 1.0
        else:
            for s in SYMBOLS:
                closes = [data[s][dates[k]]['c'] for k in range(i - 200, i)]
                c = data[s][prior]['c']
                if family == 'trend_50_200':
                    signal = c > sum(closes[-50:]) / 50 and c > sum(closes) / 200
                elif family == 'donchian_55':
                    signal = c > max(data[s][dates[k]]['h'] for k in range(i - lookback - 1, i - 1))
                elif family == 'volatility_breakout':
                    signal = c > max(data[s][dates[k]]['h'] for k in range(i - 21, i - 1)) and c > sum(closes) / 200
                elif family == 'buy_and_hold':
                    signal = True
                elif family == 'oversold_rebound':
                    # Two-day RSI(2) <= 10, only above the 200-day SMA; hold at most 5 days.
                    changes = [data[s][dates[k]]['c'] - data[s][dates[k - 1]]['c'] for k in (i - 2, i - 1)]
                    gains = sum(max(x, 0) for x in changes) / 2
                    losses = sum(max(-x, 0) for x in changes) / 2
                    rsi2 = 100.0 if losses == 0 else 100 - 100 / (1 + gains / losses)
                    if hold_left[s] > 0 and c > sum(closes) / 200 and rsi2 < 70:
                        hold_left[s] -= 1
                        signal = True
                    elif c > sum(closes) / 200 and rsi2 <= 10:
                        hold_left[s] = 4
                        signal = True
                    else:
                        hold_left[s] = 0
                        signal = False
                else:
                    raise ValueError(family)
                target[s] = float(signal) / len(SYMBOLS)
        if family != 'relative_strength_rotation':
            target = {s: float(target[s] > 0) / len(SYMBOLS) for s in SYMBOLS}
        turnover = sum(abs(target[s] - weights[s]) for s in SYMBOLS)
        gross = sum(target[s] * (data[s][dates[i + 1]]['o'] / data[s][dates[i]]['o'] - 1) for s in SYMBOLS)
        daily.append((dates[i], gross - cost * turnover))
        weights = target
    splits = {'train_2019_2022': [], 'validation_2023_2024': [], 'holdout_2025_2026': []}
    for day, r in daily:
        key = 'train_2019_2022' if day < '2023-01-01' else 'validation_2023_2024' if day < '2025-01-01' else 'holdout_2025_2026'
        splits[key].append(r)
    return {k: stats(v) for k, v in splits.items()}, stats([r for _, r in daily])

def main():
    dates, data = load()
    families = ['buy_and_hold', 'trend_50_200', 'donchian_55', 'volatility_breakout', 'oversold_rebound', 'relative_strength_rotation']
    results = {}
    for family in families:
        splits, full = backtest(dates, data, family)
        results[family] = {'full_period': full, 'splits': splits}
    sensitivity = {}
    for lookback in [20, 40, 55, 90]:
        for cost_bps in [15, 30, 50]:
            key = f'donchian_{lookback}d_cost_{cost_bps}bps'
            split_stats, full_stats = backtest(dates, data, 'donchian_55', lookback, cost_bps / 10000)
            sensitivity[key] = {'full_period': full_stats, 'splits': split_stats}
    report = {'source': 'Existing local Binance spot daily candles', 'symbols': SYMBOLS,
        'date_range': [dates[202], dates[-2]],
        'execution': 'prior-close signal; current-open rebalance; open-to-next-open returns',
        'cost': '15 bps per side on absolute portfolio weight traded (10 bps assumed fee + 5 bps slippage)',
        'positioning': 'long-only; uninvested capital held as cash; equal weights across qualifying assets; rotation holds the single highest-momentum eligible asset',
        'splits': 'train 2019-2022; validation 2023-2024; holdout 2025-2026; no parameter selection on holdout',
        'limitations': 'daily bars cannot model intraday stops, queue position, partial fills, market impact, taxes or custody; coin universe has survivorship and availability bias; final 2026 data are partial',
        'results': results, 'donchian_sensitivity': sensitivity}
    out = ROOT / 'spot_regime_strategy_results.json'
    out.write_text(json.dumps(report, indent=2))
    print(json.dumps(results, indent=2))
    print('Saved', out)

if __name__ == '__main__': main()
