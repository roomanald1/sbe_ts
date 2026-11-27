import assert from 'node:assert/strict';
import test from 'node:test';
import { BehaviorSubject } from 'rxjs';
import {
  AutoTraderStrategyPerformanceModel,
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
