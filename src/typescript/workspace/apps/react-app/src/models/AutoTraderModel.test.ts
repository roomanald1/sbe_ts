import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';

describe('AutoTraderModel', () => {
  it('updates risk limits and strategy budgets', () => {
    const model = new AutoTraderModel();
    model.setLimits('250', '2000', '25', '15', '8');
    model.setDailyStrategyBudget('trend-50-200', '150');
    assert.equal(model.state$.value.maxOrderNotional, '250');
    assert.equal(model.state$.value.dailyStrategyBudgets['trend-50-200'], '150');
    model.stop();
  });
});
