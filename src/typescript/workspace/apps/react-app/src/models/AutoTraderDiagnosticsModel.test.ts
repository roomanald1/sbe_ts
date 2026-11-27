import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { AutoTraderModel } from './AutoTraderModel';
import {
  AutoTraderDiagnosticsModel,
  boundedFunnelHistory,
  candidateFunnelRecords,
  groupCandidateFunnelRecords,
  type FunnelCycle,
  buildAutoTraderDiagnosticRows,
} from './AutoTraderDiagnosticsModel';

function diagnosticsSourceFixture() {
  const base = new AutoTraderModel().state$.value;
  const state$ = new BehaviorSubject(base);
  let starts = 0;
  let stops = 0;
  return {
    source: {
      state$,
      start: () => starts++,
      stop: () => stops++,
    } as unknown as AutoTraderModel,
    get starts() {
      return starts;
    },
    get stops() {
      return stops;
    },
  };
}

function funnelCycle(
  cycleId: string,
  timestampMs: number,
  candidateDetails: FunnelCycle['candidateDetails'] = [],
): FunnelCycle {
  return {
    cycleId,
    timestampMs,
    outcome: 'candidates_rejected',
    rawSignalCount: 1,
    allowedSignalCount: 1,
    strategyCandidateCount: 1,
    eligibleCandidateCount: 1,
    rejectedCandidateCount: 1,
    selectedCandidateCount: 0,
    orderAttemptCount: 0,
    submittedOrderCount: 0,
    fillConfirmed: false,
    detail: '',
    candidateDetails,
  };
}

describe('buildAutoTraderDiagnosticRows', () => {
  it('groups gate hits, computes readiness and expands details', () => {
    const rows = buildAutoTraderDiagnosticRows(
      [
        {
          symbol: 'ETH-USDT',
          direction: 'long',
          gate: 'Spread',
          count: 2,
          last_detail: 'wide',
          distance_to_pass: 0.5,
        },
        {
          symbol: 'ETH-USDT',
          direction: 'long',
          gate: 'Volume',
          count: 1,
          last_detail: 'low',
          distance_to_pass: 0.25,
        },
        {
          symbol: 'BTC-USDT',
          direction: 'short',
          gate: 'Readiness',
          count: 1,
          last_detail: 'ready',
          distance_to_pass: 0,
        },
      ],
      new Set(['ETH-USDT:long']),
    );

    assert.deepEqual(
      rows.map((row) => row.kind),
      ['symbol', 'symbol', 'gate', 'gate'],
    );
    const btc = rows[0];
    assert.equal(btc.kind, 'symbol');
    if (btc.kind !== 'symbol') return;
    assert.equal(btc.readinessScore, 100);
    assert.equal(btc.expanded, false);
    const eth = rows[1];
    assert.equal(eth.kind, 'symbol');
    if (eth.kind !== 'symbol') return;
    assert.equal(eth.distance, 0.5);
    assert.equal(eth.readinessScore, 57);
    assert.equal(eth.expanded, true);
  });

  it('treats missing distances as a full readiness penalty', () => {
    const [row] = buildAutoTraderDiagnosticRows(
      [
        {
          symbol: 'SOL-USDT',
          direction: 'long',
          gate: 'Liquidity',
          count: 1,
          last_detail: 'missing',
          distance_to_pass: null,
        },
      ],
      new Set(),
    );
    assert.equal(row.kind, 'symbol');
    if (row.kind === 'symbol') {
      assert.equal(row.distance, null);
      assert.equal(row.readinessScore, 50);
    }
  });
});

describe('AutoTraderDiagnosticsModel', () => {
  it('bounds cycle history, deduplicates by cycle identity and includes latest', () => {
    const history = Array.from({ length: 69 }, (_, index) =>
      funnelCycle(`cycle-${index}`, index),
    );
    const latest = funnelCycle('cycle-68', 70);
    const result = boundedFunnelHistory(history, latest);
    assert.equal(result.length, 35);
    assert.equal(result.at(-1)?.timestampMs, 70);

    const source = new AutoTraderModel();
    source.state$.next({
      ...source.state$.value,
      entryDiagnosticHistory: history,
      latestEntryDiagnostic: latest,
    });
    const model = new AutoTraderDiagnosticsModel(source);
    assert.equal(model.state$.value.availableFromApiCount, 69);
    assert.equal(model.state$.value.displayedCycleCount, 35);
  });

  it('keeps under-cap history count and displayed count distinct from completeness', () => {
    const history = [funnelCycle('one', 1), funnelCycle('two', 2)];
    const source = new AutoTraderModel();
    source.state$.next({
      ...source.state$.value,
      entryDiagnosticHistory: history,
      latestEntryDiagnostic: history[1],
    });
    const model = new AutoTraderDiagnosticsModel(source);
    assert.equal(model.state$.value.availableFromApiCount, 2);
    assert.equal(model.state$.value.displayedCycleCount, 2);
  });

  it('groups recorded candidate rows without inventing eligibility or modes', () => {
    const cycle = funnelCycle('c1', 10, [
      {
        decisionId: 'd1',
        symbol: 'BTC-USDT',
        strategy: 'trend-50-200',
        stage: 'rejected',
        observed: { edge: 0.1 },
        required: { edge: 0.2 },
        checks: { edge: false },
        evidenceContract: null,
        reasons: ['edge below threshold'],
        selectionMode: null,
        strictEligibility: null,
        exploratoryEligibility: false,
      },
      {
        decisionId: 'd2',
        symbol: 'BTC-USDT',
        strategy: 'trend-50-200',
        stage: 'selected',
        observed: {},
        required: {},
        checks: {},
        evidenceContract: null,
        reasons: [],
        selectionMode: 'exploratory',
        strictEligibility: false,
        exploratoryEligibility: true,
      },
    ]);
    const records = candidateFunnelRecords([cycle]);
    const groups = groupCandidateFunnelRecords(records, 'symbol_strategy');
    assert.equal(groups.length, 1);
    assert.equal(groups[0].records.length, 2);
    assert.equal(
      groups[0].records.filter(
        (row) => row.candidate.strictEligibility === true,
      ).length,
      0,
    );
    assert.equal(
      groups[0].records.filter(
        (row) => row.candidate.strictEligibility === false,
      ).length,
      1,
    );
    assert.equal(
      groups[0].records.filter((row) => row.candidate.strictEligibility == null)
        .length,
      1,
    );
  });

  it('owns group expansion and publishes updated rows when toggled', () => {
    const source = new AutoTraderModel();
    source.state$.next({
      ...source.state$.value,
      symbolGateHits: [
        {
          symbol: 'BTC-USDT',
          direction: 'long',
          gate: 'Liquidity',
          count: 1,
          last_detail: 'thin book',
          distance_to_pass: 0.2,
        },
      ],
    });
    const model = new AutoTraderDiagnosticsModel(source);
    const observedCounts: number[] = [];
    model.state$.subscribe((state) =>
      observedCounts.push(state.diagnosticRows.length),
    );

    assert.equal(model.state$.value.symbolGroupCount, 1);
    assert.equal(model.state$.value.diagnosticRows.length, 1);
    model.toggleGroup('BTC-USDT', 'long');
    assert.equal(model.state$.value.diagnosticRows.length, 2);
    assert.equal(model.state$.value.diagnosticRows[0].kind, 'symbol');
    assert.equal(model.state$.value.diagnosticRows[0].expanded, true);
    assert.deepEqual(observedCounts, [1, 2]);

    model.toggleGroup('BTC-USDT', 'long');
    assert.equal(model.state$.value.diagnosticRows.length, 1);
  });

  it('owns funnel navigation and filters independently per widget model', () => {
    const first = new AutoTraderDiagnosticsModel(
      diagnosticsSourceFixture().source,
    );
    const second = new AutoTraderDiagnosticsModel(
      diagnosticsSourceFixture().source,
    );
    first.setFunnelTab('heatmap');
    first.setFunnelDimension('pair');
    first.setEligibilityMode('exploratoryEligibility');
    first.setSymbolFilter('BTC-USDT');
    first.setStrategyFilter('trend-50-200');
    first.setCycleFilter('cycle-a');
    assert.deepEqual(first.state$.value.funnelView, {
      tab: 'heatmap',
      dimension: 'pair',
      eligibilityMode: 'exploratoryEligibility',
      symbolFilter: 'BTC-USDT',
      strategyFilter: 'trend-50-200',
      cycleFilter: 'cycle-a',
    });
    assert.deepEqual(second.state$.value.funnelView, {
      tab: 'funnel',
      dimension: 'strategy',
      eligibilityMode: 'strictEligibility',
      symbolFilter: '',
      strategyFilter: '',
      cycleFilter: 'all',
    });
  });

  it('owns the freshness clock lifecycle and disposes its interval once', () => {
    const fixture = diagnosticsSourceFixture();
    const callbacks: Array<() => void> = [];
    const cancelled: Array<ReturnType<typeof setInterval>> = [];
    let nowMs = 1_000;
    const model = new AutoTraderDiagnosticsModel(
      fixture.source,
      () => nowMs,
      (callback) => {
        callbacks.push(callback);
        return callbacks.length as unknown as ReturnType<typeof setInterval>;
      },
      (timer) => cancelled.push(timer),
    );
    assert.equal(model.state$.value.nowMs, 1_000);
    model.start();
    model.start();
    assert.equal(fixture.starts, 1);
    assert.equal(callbacks.length, 1);
    nowMs = 6_000;
    callbacks[0]();
    assert.equal(model.state$.value.nowMs, 6_000);
    model.stop();
    model.stop();
    assert.equal(fixture.stops, 1);
    assert.deepEqual(cancelled, [1]);
  });
});
