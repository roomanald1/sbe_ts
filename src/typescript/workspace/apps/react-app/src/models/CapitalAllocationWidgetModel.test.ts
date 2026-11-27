import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel, type AutoTraderState } from './AutoTraderModel';
import { CapitalAllocationWidgetModel } from './CapitalAllocationWidgetModel';

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

    model.stop();
    model.stop();
    assert.equal(stops, 1);
    assert.equal(model.state$.value.active, false);
    assert.equal(model.state$.value.capitalAllocation, allocation);

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
});
