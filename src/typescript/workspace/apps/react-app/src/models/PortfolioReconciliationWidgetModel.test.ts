import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  financialSnapshotValue,
  PortfolioReconciliationWidgetModel,
  projectPortfolioIntervals,
  visiblePortfolioIntervals,
} from './PortfolioReconciliationWidgetModel';

function response(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('PortfolioReconciliationWidgetModel', () => {
  it('preserves explicit zero and does not turn absent snapshot fields into zero', async () => {
    const requestedPaths: string[] = [];
    const model = new PortfolioReconciliationWidgetModel(async (path) => {
      requestedPaths.push(path);
      return path.includes('financial-snapshot')
        ? response({ account: { cash: 0, fees_paid: 0 }, captured_at_ms: 5 })
        : response({
            coverage: 'no_durable_checkpoint',
            truncated: false,
            intervals: [],
          });
    });
    await model.refresh();
    assert.ok(requestedPaths.includes('/positions/financial-snapshot'));
    assert.ok(
      requestedPaths.includes(
        '/positions/portfolio-evidence/intervals?limit=250',
      ),
    );
    assert.equal(
      financialSnapshotValue(model.state$.value.snapshot, 'account', 'cash'),
      0,
    );
    assert.equal(
      financialSnapshotValue(model.state$.value.snapshot, 'account', 'equity'),
      undefined,
    );
    assert.equal(model.state$.value.intervalCoverage, 'no_durable_checkpoint');
    model.stop();
  });

  it('keeps PM completeness separate from independent reconciliation and contribution-adjusted returns', () => {
    const result = projectPortfolioIntervals({
      coverage: 'pm_internal_complete',
      truncated: false,
      intervals: [
        {
          evidence_id: 'interval-1',
          interval_end_utc_ms: 10,
          complete: true,
          valuation_status: 'complete',
          portfolio_return_complete: true,
          flat_cash_period_return_pct: 0,
          flat_cash_period_net_pnl_usd: 0,
          independent_execution_ledger_reconciled: false,
          quantitative_eligibility:
            'blocked_pending_independent_reconciliation',
          external_contribution_events: [{ contribution_id: 'capital-1' }],
        },
      ],
    });
    assert.equal(result.intervals[0].completeness, 'complete');
    assert.equal(result.intervals[0].independentStatus, 'not_evaluated');
    assert.equal(result.intervals[0].returnPercent, 0);
    assert.equal(result.intervals[0].netPnlUsd, 0);
    assert.equal(result.intervals[0].evidenceId, 'interval-1');
  });

  it('does not infer returns for incomplete intervals or treat match probes as evidence rows', () => {
    const result = projectPortfolioIntervals({
      coverage: 'incomplete_or_unknown',
      intervals: [
        {
          evidence_id: 'interval-incomplete',
          complete: false,
          portfolio_return_complete: false,
          flat_cash_period_return_pct: 4.2,
          flat_cash_period_net_pnl_usd: 8,
          completeness_reasons: ['missing_external_evidence'],
          provenance: ['PM ledger interval'],
        },
      ],
    });
    assert.equal(result.intervals[0].returnPercent, null);
    assert.equal(result.intervals[0].netPnlUsd, null);
    assert.equal(result.intervals[0].independentStatus, 'unavailable');
    assert.equal(result.intervals.length, 1);
    assert.equal(result.intervals[0].raw.execution_origin, undefined);
  });

  it('exposes an unavailable legacy interval contract instead of fabricating empty coverage', () => {
    assert.deepEqual(projectPortfolioIntervals({ intervals: null }), {
      coverage: null,
      intervals: [],
      truncated: null,
    });
  });

  it('owns per-instance interval navigation and status filtering', () => {
    const model = new PortfolioReconciliationWidgetModel(async () =>
      response({}),
    );
    const other = new PortfolioReconciliationWidgetModel(async () =>
      response({}),
    );
    model.state$.next({
      ...model.state$.value,
      intervals: [
        {
          raw: {},
          evidenceId: 'complete',
          endMs: 1,
          completeness: 'complete',
          independentStatus: 'complete',
          returnPercent: 0,
          netPnlUsd: 0,
          reasons: [],
        },
        {
          raw: {},
          evidenceId: 'incomplete',
          endMs: 2,
          completeness: 'incomplete',
          independentStatus: 'unavailable',
          returnPercent: null,
          netPnlUsd: null,
          reasons: ['missing source'],
        },
      ],
    });
    model.setTab('balances');
    assert.equal(model.state$.value.tab, 'balances');
    model.setTab('intervals');
    model.setIntervalFilter('complete');
    assert.equal(model.state$.value.tab, 'intervals');
    assert.equal(
      visiblePortfolioIntervals(model.state$.value)
        .map(({ evidenceId }) => evidenceId)
        .join(','),
      'complete',
    );
    assert.equal(other.state$.value.tab, 'summary');
    assert.equal(other.state$.value.intervalFilter, 'all');
    model.stop();
    other.stop();
  });

  it('keeps the last authoritative snapshot visible as stale when refresh fails', async () => {
    let fail = false;
    const model = new PortfolioReconciliationWidgetModel(async (path) => {
      if (fail) return response({}, 503);
      return path.includes('financial-snapshot')
        ? response({ account: { cash: 7 } })
        : response({ coverage: 'partial', intervals: [], truncated: false });
    });
    await model.refresh();
    assert.equal(model.state$.value.dataStatus, 'available');
    fail = true;
    await model.refresh();
    assert.equal(
      model.state$.value.snapshot?.account &&
        (model.state$.value.snapshot.account as { cash: number }).cash,
      7,
    );
    assert.equal(model.state$.value.stale, true);
    assert.equal(model.state$.value.dataStatus, 'partial');
    assert.match(
      model.state$.value.error ?? '',
      /Financial snapshot unavailable/,
    );
    model.stop();
  });

  it('keeps the account snapshot and marks partial coverage when interval retrieval fails', async () => {
    const model = new PortfolioReconciliationWidgetModel(async (path) => {
      if (path.includes('financial-snapshot'))
        return response({ account: { cash: 12 } });
      throw new Error('interval source disconnected');
    });
    await model.refresh();
    assert.equal(
      (model.state$.value.snapshot?.account as { cash: number }).cash,
      12,
    );
    assert.equal(model.state$.value.dataStatus, 'partial');
    assert.match(
      model.state$.value.intervalError ?? '',
      /interval source disconnected/,
    );
    model.stop();
  });

  it('fences a late response after widget disposal', async () => {
    let resolveSnapshot!: (value: Response) => void;
    const model = new PortfolioReconciliationWidgetModel((path) =>
      path.includes('financial-snapshot')
        ? new Promise((resolve) => {
            resolveSnapshot = resolve;
          })
        : Promise.resolve(response({ coverage: 'partial', intervals: [] })),
    );
    const refresh = model.refresh();
    model.stop();
    resolveSnapshot(response({ account: { cash: 999 } }));
    await refresh;
    assert.equal(model.state$.value.snapshot, null);
    assert.equal(model.state$.value.dataStatus, 'inactive');
  });
});
