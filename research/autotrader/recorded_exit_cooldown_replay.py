#!/usr/bin/env python3
"""Read-only, matched replay of retained AutoTrader exits and post-exit quotes."""

import argparse
import collections
import datetime as dt
import gzip
import json
import math
import sqlite3
import statistics


def read_audits(path, since_ms):
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=30)
    rows = db.execute(
        "SELECT decision_id, stage, timestamp, payload FROM auto_trader_audit "
        "WHERE timestamp >= ? AND (stage = 'entry' OR stage LIKE 'exit:%') ORDER BY timestamp",
        (since_ms,),
    )
    entries, closes, posts = {}, {}, {}
    for decision_id, stage, _, payload in rows:
        try:
            item = json.loads(gzip.decompress(payload))
        except (ValueError, OSError, EOFError):
            continue
        trade_id = item.get('trade_id') or decision_id
        if stage == 'entry':
            entries[decision_id] = item
        elif stage.startswith('exit:post:'):
            posts[trade_id] = item.get('post_exit_observation') or {}
        elif item.get('trade_closed') is True:
            # A residual/dust stop can repeat the same trade ID. Prefer the
            # actual economic close rather than its near-zero residual.
            old = closes.get(trade_id)
            if old is None or abs(item.get('realized_pnl') or 0) > abs(old.get('realized_pnl') or 0):
                closes[trade_id] = item
    return entries, closes, posts


def finite_number(value):
    return isinstance(value, (int, float)) and math.isfinite(value)


def ledger_matches(close, fills):
    order_id = close.get('barrier_order_id')
    matched = fills.get(order_id, []) if order_id else []
    if not matched or any(row.get('symbol') != close.get('symbol') or row.get('side') != close.get('side') for row in matched):
        return False
    qty = sum(row.get('quantity', 0) for row in matched)
    fee = sum(abs(row.get('fee') or 0) for row in matched)
    cost = sum(row.get('quantity', 0) * row.get('execution_price', 0) for row in matched)
    expected_qty, expected_fee, expected_price = close.get('exit_filled_quantity'), close.get('exit_fees'), close.get('exit_average_price')
    return (all(finite_number(x) for x in (expected_qty, expected_fee, expected_price)) and qty > 0
            and abs(qty - expected_qty) <= max(qty * 1e-6, 1e-12)
            and abs(fee - expected_fee) <= max(fee * 1e-5, 1e-8)
            and abs(cost / qty - expected_price) <= max(expected_price * 1e-6, 1e-12))


def quote_return(entry, quantity, entry_fee, exit_price, fee_rate, long):
    gross = quantity * (exit_price - entry if long else entry - exit_price)
    return (gross - entry_fee - quantity * exit_price * fee_rate) / (quantity * entry) * 100


def stop_replay(close, post, stop):
    context = close.get('entry_context') or {}
    entry, qty, entry_fee = close.get('entry_average_price'), close.get('exit_filled_quantity'), close.get('entry_fees_paid')
    exit_price, exit_fee = close.get('exit_average_price'), close.get('exit_fees')
    exit_time, end_time = post.get('exit_timestamp_ms'), post.get('observed_at_ms')
    if not all(finite_number(x) for x in (entry, qty, entry_fee, exit_price, exit_fee, exit_time, end_time)):
        return None
    if min(entry, qty, exit_price) <= 0 or entry_fee < 0 or exit_fee < 0:
        return None
    initial_qty = context.get('quantity')
    if not finite_number(initial_qty) or abs(initial_qty - qty) > max(qty * 1e-6, 1e-12):
        return None
    if close.get('side') not in ('sell', 'buy'):
        return None
    long = close['side'] == 'sell'
    fee_rate = exit_fee / (qty * exit_price)
    reported_pnl, reported_return = close.get('realized_pnl'), close.get('net_return_percent')
    reconstructed = quote_return(entry, qty, entry_fee, exit_price, fee_rate, long)
    if not finite_number(reported_pnl) or not finite_number(reported_return):
        return None
    if abs(reconstructed - reported_return) > 0.001 or abs(reconstructed * entry * qty / 100 - reported_pnl) > 0.002:
        return None
    target = close.get('effective_take_profit_percent')
    if not finite_number(target) or target <= 0 or fee_rate > 0.01:
        return None
    horizon = post.get('horizon_ms')
    if not finite_number(horizon) or horizon < 1_800_000 or end_time - exit_time < 1_800_000:
        return None
    points = []
    for point in post.get('price_path') or []:
        t, bid, ask = point.get('timestamp'), point.get('bid'), point.get('ask')
        if all(finite_number(x) for x in (t, bid, ask)) and bid > 0 and ask >= bid and exit_time <= t <= end_time:
            points.append((t, bid if long else ask))
    points.sort()
    if not points or points[0][0] - exit_time > 120_000 or end_time - points[-1][0] > 120_000:
        return None
    if any(b[0] - a[0] > 120_000 for a, b in zip(points, points[1:])):
        return None
    # Exit at the first sampled executable quote beyond the net stop/target;
    # include the actual stop fill to model an immediate gap through a wider stop.
    path = [(exit_time, exit_price)] + points
    for timestamp, price in path:
        value = quote_return(entry, qty, entry_fee, price, fee_rate, long)
        if value <= -stop or value >= target:
            return value, timestamp - exit_time
    return quote_return(entry, qty, entry_fee, path[-1][1], fee_rate, long), path[-1][0] - exit_time


def summarize(values):
    return {'count': len(values), 'sum_usd': round(sum(values), 4),
            'mean_usd': round(statistics.mean(values), 4) if values else None,
            'winners': sum(value > 0 for value in values), 'losers': sum(value < 0 for value in values)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--insights-db', required=True)
    parser.add_argument('--positions-db', required=True)
    parser.add_argument('--since', default='2026-11-06T00:00:00+00:00')
    args = parser.parse_args()
    since_ms = int(dt.datetime.fromisoformat(args.since).timestamp() * 1000)
    entries, closes, posts = read_audits(args.insights_db, since_ms)
    ledger = sqlite3.connect(f'file:{args.positions_db}?mode=ro', uri=True, timeout=30)
    fills = collections.defaultdict(list)
    for payload, in ledger.execute('SELECT payload FROM trades WHERE timestamp >= ?', (since_ms,)):
        trade = json.loads(payload)
        if trade.get('parent_order_id'):
            fills[trade['parent_order_id']].append(trade)
    records = []
    for trade_id, close in closes.items():
        context = close.get('entry_context') or {}
        start, end, pnl = context.get('timestamp_ms'), close.get('timestamp'), close.get('realized_pnl')
        if (close.get('symbol', '').endswith(('-USDT', '-USDC')) and
                all(finite_number(x) for x in (start, end, pnl)) and start < end):
            records.append((trade_id, close, start, end, pnl))
    records.sort(key=lambda r: r[2])
    grouped = collections.defaultdict(list)
    for row in records:
        grouped[(row[1].get('symbol'), row[1].get('strategy'))].append(row)
    cooldown = {}
    for mins in (5, 15, 30):
        daily = collections.defaultdict(list)
        for group in grouped.values():
            # Use all observed stops, even when a trade would be skipped. This
            # is an observed-history screen, not a recursively simulated book.
            stops = sorted(r[3] for r in group if r[1].get('reason') == 'stop-loss' and r[4] < 0)
            for _, close, start, _, pnl in group:
                prior = [t for t in stops if t < start]
                if prior and start - prior[-1] < mins * 60_000:
                    day = dt.datetime.fromtimestamp(start / 1000, dt.timezone.utc).date().isoformat()
                    daily[day].append(pnl)
        cooldown[str(mins)] = {day: summarize(v) for day, v in sorted(daily.items())}
    stops = collections.defaultdict(lambda: collections.defaultdict(list))
    excluded = collections.Counter()
    for trade_id, close, start, end, pnl in records:
        if close.get('reason') != 'stop-loss':
            continue
        if not ledger_matches(close, fills):
            excluded['unmatched_ledger_stop'] += 1
            continue
        post = posts.get(trade_id)
        if not post:
            excluded['missing_post_exit'] += 1
            continue
        # Avoid claiming a counterfactual can coexist with a real re-entry in
        # the same strategy and market during the observation period.
        if any(other[2] > start and other[2] < post.get('observed_at_ms', 0)
               for other in grouped[(close.get('symbol'), close.get('strategy'))]):
            excluded['overlapping_reentry'] += 1
            continue
        baseline_return = close.get('net_return_percent')
        if not finite_number(baseline_return):
            excluded['missing_baseline_return'] += 1
            continue
        candidates = {s: stop_replay(close, post, s) for s in (0.75, 1.0)}
        if any(v is None for v in candidates.values()):
            excluded['incomplete_or_unsafe_path'] += 1
            continue
        day = dt.datetime.fromtimestamp(end / 1000, dt.timezone.utc).date().isoformat()
        notional = close['entry_average_price'] * close['exit_filled_quantity']
        for stop, (alternative, duration) in candidates.items():
            stops[day][str(stop)].append({'baseline_usd': pnl, 'alternative_usd': alternative * notional / 100,
                                          'extra_minutes': duration / 60_000, 'extra_usd_minutes': notional * duration / 60_000})
    stop_results = {}
    for day, choices in sorted(stops.items()):
        stop_results[day] = {}
        for stop, values in choices.items():
            stop_results[day][stop] = {'count': len(values),
                'baseline_usd': round(sum(v['baseline_usd'] for v in values), 4),
                'alternative_usd': round(sum(v['alternative_usd'] for v in values), 4),
                'extra_minutes_mean': round(statistics.mean(v['extra_minutes'] for v in values), 2),
                'extra_capital_usd_minutes': round(sum(v['extra_usd_minutes'] for v in values), 2)}
    print(json.dumps({'since': args.since, 'queried_at_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
        'latest_closed_exit_utc': dt.datetime.fromtimestamp(max((r[3] for r in records), default=since_ms) / 1000, dt.timezone.utc).isoformat(),
        'entry_audits': len(entries), 'closed_trade_ids': len(closes),
        'eligible_closed_trades': len(records), 'post_exit_records': len(posts),
        'cooldown_observed_history_skipped_pnl': cooldown, 'stop_screen': stop_results,
        'stop_exclusions': dict(excluded)}, indent=2))


if __name__ == '__main__':
    main()
