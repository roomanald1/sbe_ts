"""Costed weekly funding cross-section replay from public Binance USD-M archives."""
import argparse
import csv
import io
import json
import math
import statistics
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import date, timedelta
from pathlib import Path

BASE_SYMBOLS = ('BTCUSDT','ETHUSDT','BNBUSDT','XRPUSDT','SOLUSDT','ADAUSDT','DOGEUSDT')
EXTRA_SYMBOLS = ('LINKUSDT','LTCUSDT','AVAXUSDT','DOTUSDT','BCHUSDT','ATOMUSDT','ETCUSDT')
SYMBOLS = BASE_SYMBOLS
MONTHS = [(year, month) for year in (2024, 2025, 2026) for month in range(1, 13)
          if (year, month) <= (2026, 9)]
BASE = 'https://data.binance.vision/data/futures/um/monthly'
CACHE = Path('/tmp/sbe_ts_funding_cross_section')
DAY_MS = 86_400_000


def fetch(symbol, year, month, kind):
    stamp = f'{year}-{month:02d}'
    folder, name = (f'fundingRate/{symbol}', f'{symbol}-fundingRate-{stamp}.zip') if kind == 'funding' else (f'klines/{symbol}/1d', f'{symbol}-1d-{stamp}.zip')
    cache_file = CACHE / name
    if cache_file.exists():
        raw = cache_file.read_bytes()
    else:
        with urllib.request.urlopen(f'{BASE}/{folder}/{name}', timeout=30) as response:
            raw = response.read()
        CACHE.mkdir(parents=True, exist_ok=True)
        cache_file.write_bytes(raw)
    with zipfile.ZipFile(io.BytesIO(raw)) as package:
        with package.open(package.namelist()[0]) as stream:
            rows = list(csv.reader(io.TextIOWrapper(stream, encoding='utf-8')))
    if rows and not rows[0][0].isdigit():
        rows.pop(0)
    return symbol, kind, rows


def load_panel():
    prices = {s: {} for s in SYMBOLS}
    funding = {s: {} for s in SYMBOLS}
    errors = []
    with ThreadPoolExecutor(max_workers=10) as pool:
        jobs = {pool.submit(fetch, s, y, m, kind): (s, y, m, kind)
                for s in SYMBOLS for y, m in MONTHS for kind in ('price', 'funding')}
        for future in as_completed(jobs):
            try:
                symbol, kind, rows = future.result()
            except Exception as exc:
                errors.append(f'{jobs[future]}: {exc}')
                continue
            if kind == 'price':
                for row in rows:
                    prices[symbol][int(row[0]) // DAY_MS] = float(row[1])
            else:
                for row in rows:
                    # Rebalance at the 00:00 daily open. Exclude midnight
                    # settlements because their publication/order sequence at
                    # that boundary is not recoverable from daily candles.
                    event_ms = int(row[0])
                    if event_ms % DAY_MS < 300_000:
                        continue
                    day = event_ms // DAY_MS
                    funding[symbol][day] = funding[symbol].get(day, 0.0) + float(row[2])
    return prices, funding, errors


def replay(prices, funding, cost_rate):
    days = sorted(set.intersection(*(set(prices[s]) for s in SYMBOLS)))
    day_set = set(days)
    units = {s: 0.0 for s in SYMBOLS}
    equity = peak = 1.0
    max_drawdown = 0.0
    rows = []
    rebalances = 0
    for day in days:
        current = date(1970, 1, 1) + timedelta(days=day)
        if current < date(MONTHS[0][0], MONTHS[0][1], 8) or day + 1 not in day_set:
            continue
        if any(any(past not in funding[s] for past in range(day - 7, day + 1)) for s in SYMBOLS):
            continue
        start_equity = equity
        cost = 0.0
        if current.weekday() == 0:
            ranked = sorted(SYMBOLS, key=lambda s: (sum(funding[s][past] for past in range(day - 7, day)), s))
            target = {s: 0.0 for s in SYMBOLS}
            tail_count = max(2, len(SYMBOLS) // 3)
            for symbol in ranked[:tail_count]:
                target[symbol] = 0.5 / tail_count
            for symbol in ranked[-tail_count:]:
                target[symbol] = -0.5 / tail_count
            turnover_value = sum(abs(target[s] * equity - units[s] * prices[s][day]) for s in SYMBOLS)
            cost = turnover_value * cost_rate
            equity -= cost
            units = {s: target[s] * equity / prices[s][day] for s in SYMBOLS}
            rebalances += 1
        price_value = sum(units[s] * (prices[s][day + 1] - prices[s][day]) for s in SYMBOLS)
        # Daily-open price is a proxy for the notional at each funding event.
        funding_value = -sum(units[s] * prices[s][day] * funding[s][day] for s in SYMBOLS)
        equity += price_value + funding_value
        peak = max(peak, equity)
        max_drawdown = min(max_drawdown, equity / peak - 1.0)
        rows.append(dict(day=current.isoformat(), price=price_value / start_equity,
                         funding=funding_value / start_equity, cost=cost / start_equity,
                         net=equity / start_equity - 1.0))
    if rows:
        final_day = days[-1]
        close_value = sum(abs(units[s] * prices[s][final_day]) for s in SYMBOLS)
        close_cost = close_value * cost_rate
        before_close = equity
        equity -= close_cost
        rows[-1]['cost'] += close_cost / before_close
        rows[-1]['net'] = (1 + rows[-1]['net']) * (1 - close_cost / before_close) - 1
        max_drawdown = min(max_drawdown, equity / peak - 1.0)
    by_year = {}
    for year in sorted({r['day'][:4] for r in rows}):
        subset = [r for r in rows if r['day'].startswith(year)]
        by_year[year] = dict(days=len(subset), return_percent=100 * (math.prod(1 + r['net'] for r in subset) - 1),
                             price_percent=100 * sum(r['price'] for r in subset),
                             funding_percent=100 * sum(r['funding'] for r in subset),
                             cost_percent=100 * sum(r['cost'] for r in subset))
    net = [r['net'] for r in rows]
    sharpe = statistics.mean(net) / statistics.stdev(net) * math.sqrt(365) if len(net) > 1 and statistics.stdev(net) else None
    return dict(days=len(rows), rebalances=rebalances, return_percent=100 * (equity - 1),
                max_drawdown_percent=100 * max_drawdown, sharpe=sharpe, by_year=by_year)


def main():
    global SYMBOLS, MONTHS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--universe', choices=('7', '14'), default='14')
    parser.add_argument('--start-year', type=int, choices=(2023, 2024), default=2023)
    parser.add_argument('--leave-one-out', action='store_true')
    args = parser.parse_args()
    SYMBOLS = BASE_SYMBOLS + (EXTRA_SYMBOLS if args.universe == '14' else ())
    MONTHS = [(year, month) for year in range(args.start_year, 2027)
              for month in range(1, 13) if (year, month) <= (2026, 9)]
    prices, funding, errors = load_panel()
    result = dict(strategy='Weekly long lowest trailing-7-day funding tercile, short highest; gross exposure 1',
                  source=BASE, universe=SYMBOLS, months=[f'{y}-{m:02d}' for y, m in MONTHS],
                  errors=errors, panel_counts={s: {'prices': len(prices[s]), 'funding': len(funding[s])} for s in SYMBOLS},
                  cost_6bps=replay(prices, funding, .0006), cost_10bps=replay(prices, funding, .0010))
    suffix = '_14' if args.universe == '14' else ''
    path = Path(__file__).with_name(f'funding_cross_section{suffix}_results.json')
    path.write_text(json.dumps(result, indent=2) + '\n')
    if args.leave_one_out:
        original_symbols = SYMBOLS
        robustness = {}
        for removed in original_symbols:
            SYMBOLS = tuple(symbol for symbol in original_symbols if symbol != removed)
            result = replay(prices, funding, .0010)
            robustness[removed] = {'return_percent': result['return_percent'],
                                   'by_year': {year: values['return_percent'] for year, values in result['by_year'].items()}}
        SYMBOLS = original_symbols
        Path(__file__).with_name('funding_cross_section_leave_one_out.json').write_text(json.dumps(robustness, indent=2) + '\n')
    print(json.dumps({'result_file': str(path), 'errors': errors}, indent=2))


if __name__ == '__main__':
    main()
