import assert from 'node:assert/strict';
import test from 'node:test';
import { BehaviorSubject } from 'rxjs';
import {
  AutoTraderExcursionsModel,
  EXCURSION_EVIDENCE_NOTICE,
  excursionPanelStatus,
} from './AutoTraderExcursionsModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';

const row = (id: string, extra: Partial<AutoTraderOutcome> = {}) =>
  ({
    rowId: id,
    tradeId: id,
    strategy: 'momentum',
    source: 'strict',
    status: 'Closed',
    closedAt: Date.now(),
    ...extra,
  }) as AutoTraderOutcome;

test('discloses surfaced records are non-empirical and bars performance or promotion claims', () => {
  assert.match(EXCURSION_EVIDENCE_NOTICE, /non-empirical/);
  assert.match(
    EXCURSION_EVIDENCE_NOTICE,
    /natural-lifecycle, profitability, Learning, or promotion/,
  );
});

test('each research panel reports its own evidence limits without synthetic results', () => {
  assert.match(
    excursionPanelStatus('what-if', 24, 0, 0),
    /no authoritative deterministic replay/,
  );
  assert.match(
    excursionPanelStatus('continuation', 24, 0, 0),
    /no post-exit observation/,
  );
  assert.match(
    excursionPanelStatus('transitions', 24, 0, 0),
    /complete-window denominators/,
  );
  assert.match(
    excursionPanelStatus('sensitivity', 24, 0, 0),
    /no replay-backed/,
  );
  assert.match(
    excursionPanelStatus('replay', 24, 0, 0),
    /no complete executable paths/,
  );
  assert.equal(
    excursionPanelStatus('overview', 0, 0, 0),
    'No records match the selected filters.',
  );
});

function fixture(rows: AutoTraderOutcome[]) {
  const state$ = new BehaviorSubject({
    loading: false,
    rows,
    closedLedgerRows: [] as AutoTraderOutcome[],
  });
  let starts = 0;
  let stops = 0;
  const source = {
    state$,
    start: () => {
      starts++;
    },
    stop: () => {
      stops++;
    },
  };
  return { state$, source: source as never, counts: () => [starts, stops] };
}

test('projects source-backed complete paths and leaves incomplete metrics unavailable', () => {
  const complete = row('complete', {
    executablePricePathEvidence: {
      complete: true,
      grossExecutableMfePercent: 1.2,
      grossExecutableMaePercent: -0.4,
    },
    initialRiskEvidence: {
      eligibleForRMultiple: true,
      quoteAmount: 20,
    } as never,
  });
  const incomplete = row('partial', {
    executablePricePathEvidence: {
      complete: false,
      grossExecutableMfePercent: 9,
    },
  });
  const f = fixture([complete, incomplete]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  assert.equal(model.state$.value.sourceCount, 2);
  assert.equal(model.state$.value.eligiblePathCount, 1);
  assert.equal(
    model.state$.value.trades.find((t) => t.id === 'partial')?.grossMfePercent,
    undefined,
  );
  assert.equal(model.state$.value.whatIfAvailable, false);
  model.stop();
  assert.deepEqual(f.counts(), [1, 1]);
});

test('classifies incomplete path records only from source-provided coverage', () => {
  const reported = row('reported', {
    executablePricePathEvidence: {
      complete: false,
      missingCoverage: ['pre-entry boundary missing', 'sampling gap'],
    },
  });
  const unspecified = row('unspecified', {
    executablePricePathEvidence: { complete: false },
  });
  const absent = row('absent');
  const f = fixture([reported, unspecified, absent]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  assert.deepEqual(
    model.state$.value.trades.map((trade) => trade.incompletePathReason),
    [
      'pre-entry boundary missing, sampling gap',
      'Source reports an incomplete path without a specific reason.',
      'No executable path evidence was returned for this record.',
    ],
  );
  model.stop();
});

test('filters by source, range and strategy, and clears stale selection', () => {
  const recent = row('recent');
  const old = row('old', { closedAt: Date.now() - 100 * 86_400_000 });
  const scout = row('scout', { source: 'scout' });
  const f = fixture([recent, old, scout]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  model.selectTrade('old');
  model.setRange('30d');
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['recent', 'scout'],
  );
  assert.equal(model.state$.value.selectedTradeId, undefined);
  model.setSource('scouts');
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['scout'],
  );
  model.setStrategy('missing');
  assert.equal(model.state$.value.sourceCount, 0);
  model.stop();
});

test('repeated activation is idempotent and source updates reproject', () => {
  const f = fixture([row('one')]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  model.start();
  assert.deepEqual(f.counts(), [1, 0]);
  f.state$.next({ loading: true, rows: [row('two')], closedLedgerRows: [] });
  assert.equal(model.state$.value.loading, true);
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['two'],
  );
  model.stop();
  model.stop();
  assert.deepEqual(f.counts(), [1, 1]);
});
