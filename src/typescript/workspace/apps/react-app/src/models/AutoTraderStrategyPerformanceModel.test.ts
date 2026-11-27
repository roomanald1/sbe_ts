import assert from 'node:assert/strict';
import test from 'node:test';
import { BehaviorSubject } from 'rxjs';
import {
  AutoTraderStrategyPerformanceModel,
  isValidatedWalkForwardProfile,
  projectWalkForwardProfileRows,
  projectExitProfileSnapshot,
  projectStrategyCoverageByStrategy,
  projectStrategyCoverageHeatmap,
  projectStrategyOutcomeCoverage,
  projectStrategyOutcomeHistory,
  projectScoutOutcomeCounts,
  type StrategyPerformanceSources,
  type StrategyPerformanceStorage,
  summarizeStrategyOutcomeEvidence,
} from './AutoTraderStrategyPerformanceModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';

const outcome = (
  source: AutoTraderOutcome['source'],
  status: AutoTraderOutcome['status'] = 'Closed',
): AutoTraderOutcome => ({ source, status }) as AutoTraderOutcome;

test('summarizes only closed audited rows and keeps source cohorts separate', () => {
  const summary = summarizeStrategyOutcomeEvidence([
    outcome('strict'),
    outcome('exploratory'),
    outcome('scout'),
    outcome('paper-carry'),
    outcome('unknown'),
    outcome('strict', 'Open'),
  ]);

  assert.deepEqual(summary, {
    auditClosedRows: 5,
    strictRows: 1,
    exploratoryRows: 1,
    scoutRows: 1,
    paperCarryRows: 1,
    unknownSourceRows: 1,
    independentReconciliation: 'not-provided-by-outcomes-contract',
  });
});

test('empty outcomes remain zero evidence and never imply reconciliation', () => {
  assert.deepEqual(summarizeStrategyOutcomeEvidence([]), {
    auditClosedRows: 0,
    strictRows: 0,
    exploratoryRows: 0,
    scoutRows: 0,
    paperCarryRows: 0,
    unknownSourceRows: 0,
    independentReconciliation: 'not-provided-by-outcomes-contract',
  });
});

test('coverage projection groups only closed audit rows and preserves source classification', () => {
  const rows = [
    {
      ...outcome('strict'),
      rowId: 'strict',
      strategy: 'momentum',
      symbol: 'BTC-USDT',
      closedAt: 10,
    },
    {
      ...outcome('exploratory'),
      rowId: 'explore',
      strategy: 'momentum',
      symbol: 'BTC-USDT',
      closedAt: 20,
    },
    {
      ...outcome('scout'),
      rowId: 'scout',
      strategy: 'flow',
      symbol: 'ETH-USDT',
      closedAt: 15,
    },
    {
      ...outcome('strict', 'Open'),
      rowId: 'open',
      strategy: 'momentum',
      symbol: 'BTC-USDT',
      closedAt: 30,
    },
  ] as AutoTraderOutcome[];
  assert.deepEqual(projectStrategyOutcomeCoverage(rows), [
    {
      strategy: 'flow',
      symbol: 'ETH-USDT',
      closedAuditRows: 1,
      strictRows: 0,
      exploratoryRows: 0,
      scoutRows: 1,
      carryRows: 0,
      unknownRows: 0,
      latestClosedAt: 15,
    },
    {
      strategy: 'momentum',
      symbol: 'BTC-USDT',
      closedAuditRows: 2,
      strictRows: 1,
      exploratoryRows: 1,
      scoutRows: 0,
      carryRows: 0,
      unknownRows: 0,
      latestClosedAt: 20,
    },
  ]);
  const coverage = projectStrategyOutcomeCoverage(rows);
  assert.deepEqual(projectStrategyCoverageByStrategy(coverage), [
    { strategy: 'flow', closedAuditRows: 1 },
    { strategy: 'momentum', closedAuditRows: 2 },
  ]);
  assert.deepEqual(projectStrategyCoverageHeatmap(coverage), {
    symbols: ['BTC-USDT', 'ETH-USDT'],
    rows: [
      {
        strategy: 'flow',
        cells: [
          { symbol: 'BTC-USDT', closedAuditRows: 0, intensity: 0 },
          { symbol: 'ETH-USDT', closedAuditRows: 1, intensity: 3 },
        ],
      },
      {
        strategy: 'momentum',
        cells: [
          { symbol: 'BTC-USDT', closedAuditRows: 2, intensity: 5 },
          { symbol: 'ETH-USDT', closedAuditRows: 0, intensity: 0 },
        ],
      },
    ],
  });
  assert.deepEqual(
    projectStrategyOutcomeHistory(rows, 1).map((row) => row.rowId),
    [rows[1].rowId],
  );
});

test('walk-forward validation does not promote incomplete or missing evidence', () => {
  assert.equal(isValidatedWalkForwardProfile(undefined), null);
  assert.equal(
    isValidatedWalkForwardProfile({
      sampleCount: 100,
      lastSampleTimestampMs: 10,
      finalHoldoutStatus: 'validated',
      historyCoverage: 'complete',
      costCoverage: 'complete',
      searchFamilyCoverage: 'complete',
      oosFoldCount: 3,
      effectiveSampleSize: 20,
    }),
    true,
  );
  assert.equal(
    isValidatedWalkForwardProfile({
      sampleCount: 100,
      lastSampleTimestampMs: 10,
      finalHoldoutStatus: 'validated',
      historyCoverage: 'complete',
      costCoverage: 'partial',
      searchFamilyCoverage: 'complete',
      oosFoldCount: 3,
      effectiveSampleSize: 20,
    }),
    false,
  );
});

test('exit policy snapshot exposes profile parameters without inventing validation', () => {
  const [row] = projectExitProfileSnapshot([
    {
      symbol: 'BTC-USDT',
      strategy: 'live-momentum',
      profile_id: 'p-1',
      profile_version: 3,
      learning_samples: 10,
      realized_pnl: 0,
      parameters: [
        {
          key: 'stop_loss',
          label: 'Stop loss',
          value: 0.42,
          learned_value: 0.42,
          default_value: 0.5,
          min: 0.1,
          max: 1,
          step: 0.1,
          unit: '%',
          overridden: false,
        },
        {
          key: 'entry_score',
          label: 'Entry score',
          value: 5,
          learned_value: 5,
          default_value: 4,
          min: 0,
          max: 10,
          step: 1,
          unit: '',
          overridden: false,
        },
      ],
      walk_forward_evidence: null,
    },
  ]);
  assert.deepEqual(row, {
    symbol: 'BTC-USDT',
    strategy: 'live-momentum',
    profile: 'p-1 · v3',
    parameters: [{ label: 'Stop loss', value: 0.42, unit: '%' }],
    effectiveSamples: null,
    validation: 'unavailable',
  });
});

test('Scout evidence remains separated as raw counts with no inferred gains', () => {
  assert.deepEqual(
    projectScoutOutcomeCounts([
      { strategy: 'zeta', count: 3, positive: 1 },
      { strategy: 'alpha', count: 2, positive: 2 },
      { strategy: 'invalid', count: Number.NaN, positive: 4 },
    ]),
    [
      { strategy: 'alpha', outcomes: 2, positiveOutcomes: 2 },
      { strategy: 'zeta', outcomes: 3, positiveOutcomes: 1 },
    ],
  );
});

test('projects walk-forward rows from declared profile evidence and preserves missing values', () => {
  const rows = projectWalkForwardProfileRows([
    {
      symbol: 'BTC-USDT',
      strategy: 'live-momentum',
      learning_samples: 0,
      realized_pnl: 0,
      parameters: [],
      walk_forward_evidence: {
        sampleCount: 37,
        positiveNetRate: 0.6,
        averageNetReturnPercent: 0.12,
        expectancyCiLowPercent: 0.04,
        expectancyCiHighPercent: 0.18,
        effectiveSampleSize: 24.5,
        windowCount: 8,
        maxDrawdownPercent: -2.1,
        downsideLossPercent: -0.4,
        lastSampleTimestampMs: 1_800_000_000_000,
      },
    },
    {
      symbol: 'ETH-USDT',
      strategy: 'live-momentum',
      learning_samples: 0,
      realized_pnl: 0,
      parameters: [],
      walk_forward_evidence: null,
    },
  ]);

  assert.equal(rows[0].samples, 37);
  assert.equal(rows[0].winRate, 0.6);
  assert.equal(rows[0].averageNet, 0.12);
  assert.equal(rows[0].expectancyCiLowPercent, 0.04);
  assert.equal(rows[1].samples, null);
  assert.equal(rows[1].winRate, null);
});

type FakeSource<T> = {
  state$: BehaviorSubject<T>;
  starts: number;
  stops: number;
  refreshes: number;
  start: () => void;
  stop: () => void;
  refresh: () => Promise<void>;
  retry: () => void;
};

function source<T>(initial: T): FakeSource<T> {
  return {
    state$: new BehaviorSubject(initial),
    starts: 0,
    stops: 0,
    refreshes: 0,
    start() {
      this.starts += 1;
    },
    stop() {
      this.stops += 1;
    },
    async refresh() {
      this.refreshes += 1;
    },
    retry() {
      this.refreshes += 1;
    },
  };
}

function memoryStorage(): StrategyPerformanceStorage & {
  values: Map<string, string>;
} {
  const values = new Map<string, string>();
  return {
    values,
    getItem(key) {
      return values.get(key) ?? null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };
}

function createModel(
  instance: string,
  storage: StrategyPerformanceStorage = memoryStorage(),
) {
  const sources = {
    outcomes: source({ loading: true, rows: [] as unknown[] }),
    settings: source({ strategies: [] as unknown[] }),
    scout: source({ loading: true, learningOutcomes: [] as unknown[] }),
  };
  const performanceSources: StrategyPerformanceSources = {
    ...sources,
    refresh: async () => {
      await Promise.all([
        sources.outcomes.refresh(),
        sources.settings.refresh(),
      ]);
      sources.scout.retry();
    },
  } as unknown as StrategyPerformanceSources;
  const model = new AutoTraderStrategyPerformanceModel(
    performanceSources,
    instance,
    storage,
  );
  return { model, sources, storage };
}

test('model lifecycle owns source consumers, subscribes once, and restarts cleanly', () => {
  const { model, sources } = createModel('widget-a');
  assert.equal(model.state$.value.active, false);
  model.start();
  model.start();
  assert.deepEqual(
    Object.values(sources).map(({ starts }) => starts),
    [1, 1, 1],
  );
  sources.outcomes.state$.next({ loading: false, rows: ['new'] });
  assert.deepEqual(model.state$.value.outcomes.rows, ['new']);

  model.stop();
  model.stop();
  assert.equal(model.state$.value.active, false);
  assert.deepEqual(
    Object.values(sources).map(({ stops }) => stops),
    [1, 1, 1],
  );
  sources.outcomes.state$.next({ loading: false, rows: ['after-stop'] });
  assert.deepEqual(model.state$.value.outcomes.rows, ['new']);

  model.start();
  sources.outcomes.state$.next({ loading: false, rows: ['after-restart'] });
  assert.deepEqual(model.state$.value.outcomes.rows, ['after-restart']);
  assert.equal(sources.outcomes.starts, 2);
  model.dispose();
  assert.equal(sources.outcomes.stops, 2);
  assert.equal(model.state$.isStopped, true);
});

test('refresh command delegates to authoritative source models only while active', async () => {
  const { model, sources } = createModel('refresh');
  await model.refresh();
  assert.equal(sources.outcomes.refreshes, 0);
  model.start();
  await model.refresh();
  assert.deepEqual(
    Object.values(sources).map(({ refreshes }) => refreshes),
    [1, 1, 1],
  );
  model.stop();
  model.dispose();
});

test('filter and column settings persist independently per widget instance', () => {
  const storage = memoryStorage();
  const first = createModel('first', storage);
  const second = createModel('second', storage);
  first.model.setHideDisabledStrategies(true);
  first.model.setColumnGroupVisible('learning', false);
  assert.equal(first.model.state$.value.hideDisabledStrategies, true);
  assert.equal(first.model.state$.value.visibleColumnGroups.learning, false);
  assert.equal(second.model.state$.value.hideDisabledStrategies, false);
  assert.equal(second.model.state$.value.visibleColumnGroups.learning, true);

  const restored = createModel('first', storage);
  assert.equal(restored.model.state$.value.hideDisabledStrategies, true);
  assert.equal(restored.model.state$.value.visibleColumnGroups.learning, false);
  first.model.dispose();
  second.model.dispose();
  restored.model.dispose();
});

test('view tab selection belongs to the model lifecycle', () => {
  const { model } = createModel('view-state');
  assert.equal(model.state$.value.view, 'performance');
  model.setView('heatmap');
  assert.equal(model.state$.value.view, 'heatmap');
  model.setView('history');
  assert.equal(model.state$.value.view, 'history');
  model.setSourceView('scouts');
  assert.equal(model.state$.value.sourceView, 'scouts');
  model.dispose();
});

test('storage failures leave the view usable and expose the persistence error', () => {
  const storage: StrategyPerformanceStorage = {
    getItem: () => {
      throw new Error('read denied');
    },
    setItem: () => {
      throw new Error('write denied');
    },
  };
  const { model } = createModel('storage-error', storage);
  model.setHideDisabledStrategies(true);
  assert.equal(model.state$.value.hideDisabledStrategies, true);
  assert.equal(model.state$.value.error, 'write denied');
  model.dispose();
});
