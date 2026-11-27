import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel, type AutoTraderState } from './AutoTraderModel';
import {
  CapitalAllocationWidgetModel,
  groupCapitalAllocationRows,
} from './CapitalAllocationWidgetModel';

const allocationFixture: NonNullable<AutoTraderState['capitalAllocation']> = {
  version: 'v1',
  model: 'shared_pool',
  poolLimit: 500,
  used: 60,
  reserved: 50,
  available: 390,
  rows: [
    {
      strategy: 'momentum',
      symbol: 'BTC-USDT',
      horizon: 'daily',
      effectiveAllocation: 100,
      reserved: 10,
      used: 30,
      utilization: 0.3,
      evidenceEligible: true,
      state: 'allocated',
      reason: 'validated profile',
    },
    {
      strategy: 'momentum',
      symbol: 'ETH-USDT',
      horizon: 'daily',
      effectiveAllocation: 200,
      reserved: 40,
      used: 30,
      utilization: 0.15,
      evidenceEligible: null,
      state: 'allocated',
      reason: 'profile coverage unknown',
    },
    {
      strategy: 'trade-flow',
      symbol: 'BTC-USDT',
      horizon: 'intraday',
      effectiveAllocation: 0,
      reserved: 0,
      used: 0,
      utilization: null,
      evidenceEligible: false,
      state: 'blocked',
      reason: 'evidence insufficient',
    },
  ],
};

describe('CapitalAllocationWidgetModel', () => {
  it('subscribes to authoritative allocation fields and balances source lifecycle', () => {
    const source = new AutoTraderModel();
    let starts = 0;
    let stops = 0;
    const model = new CapitalAllocationWidgetModel('allocation-a', {
      state$: source.state$,
      start: () => {
        starts += 1;
      },
      stop: () => {
        stops += 1;
      },
    });
    const allocation: NonNullable<AutoTraderState['capitalAllocation']> = {
      version: 'v1',
      model: 'shared_pool',
      poolLimit: 500,
      used: 75,
      reserved: 25,
      available: 400,
      rows: [],
    };

    model.start();
    model.start();
    source.state$.next({
      ...source.state$.value,
      capitalAllocation: allocation,
      availableNotional: 400,
    });
    assert.equal(starts, 1);
    assert.equal(model.state$.value.active, true);
    assert.equal(model.state$.value.capitalAllocation, allocation);
    assert.equal(model.state$.value.availableNotional, 400);
    assert.equal(model.state$.value.poolReconciles, true);
    assert.deepEqual(
      model.state$.value.poolSegments.map(({ id, sharePercent }) => [
        id,
        sharePercent,
      ]),
      [
        ['used', 15],
        ['reserved', 5],
        ['available', 80],
      ],
    );

    const inconsistent = { ...allocation, available: 399 };
    source.state$.next({
      ...source.state$.value,
      capitalAllocation: inconsistent,
    });
    assert.equal(model.state$.value.poolReconciles, false);

    model.stop();
    model.stop();
    assert.equal(stops, 1);
    assert.equal(model.state$.value.active, false);
    assert.equal(model.state$.value.capitalAllocation, inconsistent);

    model.start();
    assert.equal(starts, 2);
    model.stop();
    assert.equal(stops, 2);
  });

  it('keeps missing allocation distinct from a reported zero allocation', () => {
    const source = new AutoTraderModel();
    const model = new CapitalAllocationWidgetModel(
      'allocation-missing',
      source,
    );
    assert.equal(model.state$.value.capitalAllocation, undefined);

    const zero: NonNullable<AutoTraderState['capitalAllocation']> = {
      version: 'v1',
      model: 'shared_pool',
      poolLimit: 0,
      used: 0,
      reserved: 0,
      available: 0,
      rows: [],
    };
    source.state$.next({ ...source.state$.value, capitalAllocation: zero });
    assert.equal(model.state$.value.capitalAllocation, undefined);
    model.start();
    assert.equal(model.state$.value.capitalAllocation, zero);
    model.stop();
  });

  it('isolates widget instances and ignores source updates while stopped', () => {
    const source = new AutoTraderModel();
    const first = new CapitalAllocationWidgetModel('first', source);
    const second = new CapitalAllocationWidgetModel('second', source);
    first.start();
    second.start();
    const firstAllocation: NonNullable<AutoTraderState['capitalAllocation']> = {
      version: 'v1',
      model: 'shared_pool',
      poolLimit: 100,
      used: 10,
      reserved: 0,
      available: 90,
      rows: [],
    };
    source.state$.next({
      ...source.state$.value,
      capitalAllocation: firstAllocation,
    });
    assert.equal(first.state$.value.capitalAllocation, firstAllocation);
    assert.equal(second.state$.value.capitalAllocation, firstAllocation);

    first.stop();
    const secondAllocation: NonNullable<AutoTraderState['capitalAllocation']> =
      {
        version: 'v1',
        model: 'shared_pool',
        poolLimit: 200,
        used: 25,
        reserved: 5,
        available: 170,
        rows: [],
      };
    source.state$.next({
      ...source.state$.value,
      capitalAllocation: secondAllocation,
    });
    assert.equal(first.state$.value.capitalAllocation, firstAllocation);
    assert.equal(second.state$.value.capitalAllocation, secondAllocation);
    second.stop();
  });

  it('groups only source-reported dimensions and leaves mixed evidence unknown', () => {
    const byStrategy = groupCapitalAllocationRows(
      allocationFixture,
      'strategy',
    );
    assert.equal(byStrategy[0].label, 'momentum');
    assert.equal(byStrategy[0].rowCount, 2);
    assert.equal(byStrategy[0].effectiveAllocation, 300);
    assert.equal(byStrategy[0].used, 60);
    assert.equal(byStrategy[0].reserved, 50);
    assert.equal(byStrategy[0].sharePercent, 60);
    assert.equal(byStrategy[0].utilizationPercent, 20);
    assert.equal(byStrategy[0].evidenceEligible, null);
    assert.deepEqual(byStrategy[0].reasons, [
      'validated profile',
      'profile coverage unknown',
    ]);

    const bySymbol = groupCapitalAllocationRows(allocationFixture, 'symbol');
    assert.deepEqual(
      bySymbol.map((row) => row.label),
      ['ETH-USDT', 'BTC-USDT'],
    );
  });

  it('keeps tab/group selections per instance and withholds seeded limits while loading', () => {
    const upstream = new AutoTraderModel();
    const source = {
      state$: upstream.state$,
      start: () => undefined,
      stop: () => undefined,
    } as unknown as AutoTraderModel;
    const first = new CapitalAllocationWidgetModel('first', source);
    const second = new CapitalAllocationWidgetModel(
      'second',
      new AutoTraderModel(),
    );
    first.setTab('candidates');
    first.setGroupBy('symbol');
    assert.equal(first.state$.value.tab, 'candidates');
    assert.equal(first.state$.value.groupBy, 'symbol');
    assert.equal(first.state$.value.dailyNotionalLimit, null);
    assert.equal(second.state$.value.tab, 'allocation');
    assert.equal(second.state$.value.groupBy, 'strategy');

    first.start();
    upstream.state$.next({
      ...upstream.state$.value,
      loading: false,
      capitalAllocation: allocationFixture,
      availableNotional: 390,
      serverDailyNotionalLimit: 1_000,
      usedNotionalToday: 125,
      maxOpenPositions: '20',
      serverExplorationPercent: 2,
    });
    assert.equal(first.state$.value.dailyNotionalRemaining, 875);
    assert.equal(first.state$.value.maxOpenPositions, 20);
    assert.equal(first.state$.value.explorationLimitPercent, 2);
    assert.deepEqual(
      first.state$.value.displayedRows.map((row) => row.label),
      ['ETH-USDT', 'BTC-USDT'],
    );
    first.stop();
  });
});
