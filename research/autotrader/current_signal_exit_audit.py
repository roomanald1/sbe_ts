"""Read-only summary of current Insights evaluations and AutoTrader exit audits."""
import collections
import datetime
import gzip
import json
import math
import sqlite3
import statistics
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DB = ROOT / 'src/rust/insights/signals.sqlite'
OUT = Path(__file__).with_name('current_signal_exit_audit_results.json')


class Reader:
    def __init__(self, value):
        self.value, self.i = value, 0

    def number(self, fmt):
        result = struct.unpack_from('<' + fmt, self.value, self.i)[0]
        self.i += struct.calcsize('<' + fmt)
        return result

    def string(self):
        size = self.number('Q')
        result = self.value[self.i:self.i + size].decode()
        self.i += size
        return result

    def optional(self, fmt='d'):
        return self.number(fmt) if self.number('B') else None

    def evaluation(self):
        if self.number('I') != 2:
            raise ValueError('not a SignalEvaluation')
        item = {'symbol': self.string(), 't': self.number('Q'), 'end': self.number('Q'),
                'h': self.number('Q'), 'direction': self.string(), 'score': self.number('d')}
        self.optional()  # ranging_probability
        item['entry_reference'] = self.number('d')
        item['exit_reference'] = self.number('d')
        item['entry_bid'] = self.optional()
        item['entry_ask'] = self.optional()
        self.optional('Q')
        item['exit_bid'] = self.optional()
        item['exit_ask'] = self.optional()
        self.optional('Q')
        item['fee_rate'] = self.number('d')
        item['long_gross'] = self.number('d')
        item['short_gross'] = self.number('d')
        item['long_net'] = self.optional()
        item['short_net'] = self.optional()
        item['path_high'] = self.number('d')
        self.number('Q')
        item['path_low'] = self.number('d')
        self.number('Q')
        if self.i < len(self.value):
            self.optional('Q')
            self.optional('Q')
        if self.i != len(self.value):
            raise ValueError('unexpected trailing data')
        return item


def median(values):
    return statistics.median(values) if values else None


def stamped(ms):
    return datetime.datetime.fromtimestamp(ms / 1000, datetime.timezone.utc).isoformat()


def main():
    db = sqlite3.connect(f'file:{DB}?mode=ro', uri=True)
    db.execute('pragma busy_timeout=10000')
    counts = {kind: count for kind, count in db.execute('select kind,count(*) from signal_records group by kind')}
    span = db.execute('select min(timestamp),max(timestamp) from signal_records').fetchone()
    entries, groups = {}, collections.defaultdict(list)
    for decision_id, stage, timestamp, payload in db.execute("select decision_id,stage,timestamp,payload from auto_trader_audit where stage='entry' or stage like 'exit:%'"):
        try:
            item = json.loads(gzip.decompress(payload))
        except (ValueError, OSError, EOFError):
            continue
        if stage == 'entry':
            entries[decision_id] = item
        else:
            groups[decision_id].append((stage, timestamp, item))
    reasons, strategies = collections.Counter(), collections.Counter()
    coverage = collections.Counter()
    strict_paths = collections.Counter()
    costs = []
    for item in entries.values():
        value = item.get('all_in_cost_bps')
        if isinstance(value, (int, float)) and math.isfinite(value):
            costs.append(value)
    for decision_id, events in groups.items():
        # Prefer the event that actually reports a nonzero realized fill. Stop
        # and external reconciliation records can refer to the same close.
        chosen = max(events, key=lambda event: abs(event[2].get('realized_pnl') or 0.0))
        item = chosen[2]
        reasons[item.get('reason') or 'unknown'] += 1
        strategy = entries.get(decision_id, {}).get('strategy')
        if not strategy:
            strategy = (item.get('entry_context') or {}).get('strategy_decision', {}).get('strategy')
        strategies[strategy or 'unattributed'] += 1
        nonzero_fills = [event for event in events if abs(event[2].get('realized_pnl') or 0.0) > 1e-12]
        if len(nonzero_fills) == 1:
            _, exit_time, exit_item = nonzero_fills[0]
            path = exit_item.get('price_path') or []
            entered = (exit_item.get('entry_context') or {}).get('timestamp_ms')
            if path and entered and path[0]['timestamp'] <= entered + 15_000 and path[-1]['timestamp'] >= exit_time - 15_000:
                strict_paths[exit_item.get('reason') or 'unknown'] += 1
        best_path = max((event[2].get('price_path') or [] for event in events), key=len)
        if best_path:
            coverage['any_path'] += 1
            entry_time = (item.get('entry_context') or {}).get('timestamp_ms')
            if entry_time and best_path[0]['timestamp'] <= entry_time + 15_000:
                coverage['begins_near_entry'] += 1
            if best_path[-1]['timestamp'] >= chosen[1] - 15_000:
                coverage['ends_near_exit'] += 1
    sample = collections.defaultdict(list)
    decoded = 0
    bad = 0
    # Prime modulus reduces correlation with the fixed record-kind write cycle.
    for (payload,) in db.execute('select payload from signal_records where kind=2 and id%97=0'):
        try:
            item = Reader(gzip.decompress(payload)).evaluation()
            decoded += 1
            if all(isinstance(item[key], (int, float)) and math.isfinite(item[key])
                   for key in ('long_net', 'short_net', 'long_gross', 'short_gross')):
                sample[item['h']].append(item)
        except (ValueError, OSError, EOFError, struct.error, UnicodeDecodeError):
            bad += 1
    horizons = {}
    for horizon, items in sorted(sample.items()):
        items.sort(key=lambda item: (item['t'], item['symbol']))
        next_free = {}
        chosen = []
        for item in items:
            if item['t'] < next_free.get(item['symbol'], 0):
                continue
            chosen.append(item)
            next_free[item['symbol']] = item['end']
        signal_net = [item['long_net'] if item['direction'] == 'bullish' else item['short_net']
                      for item in chosen if item['direction'] in ('bullish', 'bearish')]
        absolute_move = [abs(item['exit_reference'] / item['entry_reference'] - 1) * 100
                         for item in chosen if item['entry_reference'] > 0]
        cost = [item['long_gross'] - item['long_net'] for item in chosen]
        best_net = [max(item['long_net'], item['short_net']) for item in chosen]
        horizons[str(horizon)] = dict(sampled=len(items), nonoverlap=len(chosen),
                                     directional_n=len(signal_net),
                                     directional_mean_net_percent=statistics.mean(signal_net) if signal_net else None,
                                     median_absolute_move_percent=median(absolute_move),
                                     median_roundtrip_cost_percent=median(cost),
                                     oracle_positive_net_rate=sum(x > 0 for x in best_net) / len(best_net) if best_net else None)
    result = dict(database=str(DB), record_counts=counts,
                  record_span_utc=[stamped(span[0]), stamped(span[1])],
                  entry_audits=len(entries), entry_median_one_way_cost_bps=median(costs),
                  exit_audit_events=sum(map(len, groups.values())), exit_decision_ids=len(groups),
                  exit_reason_by_decision=dict(reasons), exit_strategy_by_decision=dict(strategies),
                  path_coverage=dict(coverage), strict_complete_single_fill_paths_by_reason=dict(strict_paths),
                  sampled_evaluations=decoded,
                  evaluation_decode_failures=bad, sample_modulus=97,
                  horizons=horizons)
    OUT.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
