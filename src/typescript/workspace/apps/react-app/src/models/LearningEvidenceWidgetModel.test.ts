import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { AutoTraderModel } from './AutoTraderModel';
import {
  AutoTraderLearningWidgetModel,
  projectLearningWidgetState,
  type EvidenceRequestsResponse,
  type AutoTraderLearningWidgetState,
} from './AutoTraderLearningWidgetModel';
import { LearningEvidenceWidgetModel } from './LearningEvidenceWidgetModel';

function baseState() {
  const source = new AutoTraderModel().state$.value;
  return projectLearningWidgetState(source, null, undefined, false);
}

function sourceModelFixture(
  state: AutoTraderLearningWidgetState = baseState(),
) {
  const state$ = new BehaviorSubject(state);
  let starts = 0;
  let stops = 0;
  const pendingRefreshes: Array<() => void> = [];
  const model = {
    state$,
    start: () => {
      starts += 1;
    },
    stop: () => {
      stops += 1;
    },
    refreshSummary: () =>
      new Promise<void>((resolve) => pendingRefreshes.push(resolve)),
    refreshDecisions: () =>
      new Promise<void>((resolve) => pendingRefreshes.push(resolve)),
    refreshEvidenceRequests: () =>
      new Promise<void>((resolve) => pendingRefreshes.push(resolve)),
  } as unknown as AutoTraderLearningWidgetModel;
  return {
    model,
    state$,
    pendingRefreshes,
    get starts() {
      return starts;
    },
    get stops() {
      return stops;
    },
  };
}

function stateWithEvidence(): AutoTraderLearningWidgetState {
  const base = baseState();
  const coverage = (symbol: string, strategy: string) => ({
    symbol,
    strategy,
    profile_id: `${symbol}-profile`,
    regime: null,
    horizon: null,
    coverage_scope: 'symbol_strategy_profile_aggregate',
    coverage_status: 'missing',
    gap_reason: 'No samples reported',
    sample_count: 0,
    effective_sample_size: 0,
    last_sample_timestamp_ms: null,
    last_sample_age_ms: null,
    regime_horizon_coverage: 'unavailable',
  });
  const evidence: EvidenceRequestsResponse = {
    as_of_ms: 1_000,
    total_count: 0,
    omitted_count: 0,
    total_coverage_cohorts: 2,
    omitted_coverage_cohorts: 0,
    requests: [],
    coverage_cohorts: [
      coverage('BTC-USDT', 'live-momentum'),
      coverage('ETH-USDT', 'live-order-book'),
    ],
  };
  return { ...base, evidenceRequests: evidence };
}

describe('LearningEvidenceWidgetModel', () => {
  it('owns tab, filter and refresh state and follows source updates', async () => {
    const fixture = sourceModelFixture();
    const model = new LearningEvidenceWidgetModel(() => fixture.model);
    assert.equal(model.state$.value.dataStatus, 'inactive');

    model.start();
    model.start();
    assert.equal(fixture.starts, 1);
    fixture.state$.next(stateWithEvidence());
    model.setFilters({ symbol: 'BTC-USDT', strategy: 'live-momentum' });
    model.setTab('activity');
    assert.equal(model.state$.value.tab, 'activity');
    assert.deepEqual(
      model.state$.value.visibleCohorts?.map(({ symbol }) => symbol),
      ['BTC-USDT'],
    );
    assert.deepEqual(model.state$.value.filters, {
      symbol: 'BTC-USDT',
      strategy: 'live-momentum',
    });

    const pending = model.refresh();
    assert.equal(model.state$.value.refreshing, true);
    assert.equal(fixture.pendingRefreshes.length, 3);
    fixture.pendingRefreshes.forEach((resolve) => resolve());
    await pending;
    assert.equal(model.state$.value.refreshing, false);
    assert.ok(model.state$.value.lastRefreshAttemptAt != null);
    model.stop();
    assert.equal(fixture.stops, 1);
  });

  it('reports source staleness and errors without replacing missing values with zero', () => {
    const fixture = sourceModelFixture();
    const model = new LearningEvidenceWidgetModel(() => fixture.model);
    model.start();
    const stale = stateWithEvidence();
    stale.evidenceRequests!.coverage_cohorts[0].coverage_status = 'stale';
    stale.evidenceRequestError = 'coverage refresh failed';
    fixture.state$.next(stale);
    assert.equal(model.state$.value.stale, true);
    assert.equal(model.state$.value.dataStatus, 'partial');
    assert.equal(model.state$.value.view.cohorts?.[0]?.rawSamples, 0);
    assert.equal(model.state$.value.view.cohorts?.[0]?.effectiveSamples, 0);
    const recovered = stateWithEvidence();
    fixture.state$.next(recovered);
    assert.equal(model.state$.value.stale, false);
    assert.equal(model.state$.value.view.coverageError, undefined);
    model.stop();
  });

  it('fences old source updates after stop and uses a fresh model on reactivation', async () => {
    const instances: ReturnType<typeof sourceModelFixture>[] = [];
    const create = () => {
      const fixture = sourceModelFixture();
      instances.push(fixture);
      return fixture.model;
    };
    const model = new LearningEvidenceWidgetModel(create);
    model.start();
    const oldInstance = instances[0];
    const pending = model.refresh();
    assert.equal(oldInstance.pendingRefreshes.length, 3);
    model.stop();
    const emissionsAfterStop = model.state$.value.view.cohorts;
    oldInstance.state$.next(stateWithEvidence());
    oldInstance.pendingRefreshes.forEach((resolve) => resolve());
    await pending;
    assert.equal(model.state$.value.view.cohorts, emissionsAfterStop);

    model.start();
    assert.equal(instances.length, 2);
    assert.equal(instances[1].starts, 1);
    assert.equal(model.state$.value.view.cohorts, null);
    model.stop();
  });

  it('keeps filter state isolated between widget instances', () => {
    const firstSource = sourceModelFixture(stateWithEvidence());
    const secondSource = sourceModelFixture(stateWithEvidence());
    const first = new LearningEvidenceWidgetModel(() => firstSource.model);
    const second = new LearningEvidenceWidgetModel(() => secondSource.model);
    first.setFilters({ symbol: 'BTC-USDT' });
    second.setFilters({ symbol: 'ETH-USDT' });
    assert.deepEqual(
      first.state$.value.visibleCohorts?.map(({ symbol }) => symbol),
      ['BTC-USDT'],
    );
    assert.deepEqual(
      second.state$.value.visibleCohorts?.map(({ symbol }) => symbol),
      ['ETH-USDT'],
    );
    first.stop();
    second.stop();
  });
});
