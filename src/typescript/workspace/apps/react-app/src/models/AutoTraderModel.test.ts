import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel, projectStrategyDiagnostics } from './AutoTraderModel';
import { setAutoTraderRpcRequesterForTests } from '../auto_trader_rsocket';

describe('AutoTraderModel', () => {
  it('preserves distinct authoritative BTC diagnostic source anchors', () => {
    const diagnostics = projectStrategyDiagnostics([
      {
        strategy: 'risk-adjusted-monthly-momentum',
        as_of_ms: 10_000,
        btc_close: 100,
        btc_sma: 90,
        btc_distance_percent: 11.1,
        btc_window_days: 200,
        btc_gate_required: true,
        btc_gate_passed: true,
        btc_source_close_time_ms: 8_000,
        btc_source_timeframe: 'monthly_rebalance_anchor_daily',
        symbols: [],
      },
      {
        strategy: 'donchian-55-btc200',
        as_of_ms: 10_000,
        btc_close: 102,
        btc_sma: 90,
        btc_distance_percent: 13.3,
        btc_window_days: 200,
        btc_gate_required: true,
        btc_gate_passed: true,
        btc_source_close_time_ms: 9_000,
        btc_source_timeframe: 'daily',
        symbols: [],
      },
    ]);

    assert.deepEqual(
      diagnostics.map(({ btc_source_close_time_ms, btc_source_timeframe }) => [
        btc_source_close_time_ms,
        btc_source_timeframe,
      ]),
      [
        [8_000, 'monthly_rebalance_anchor_daily'],
        [9_000, 'daily'],
      ],
    );
    assert.deepEqual(projectStrategyDiagnostics(undefined), []);
  });

  it('updates global risk limits without creating strategy budgets', () => {
    const model = new AutoTraderModel();
    model.setLimits('250', '2000', '25', '15', '8');
    model.setStrategies(['trend-50-200']);
    assert.equal(model.state$.value.maxOrderNotional, '250');
    assert.equal(model.state$.value.strategies.includes('trend-50-200'), true);
    assert.equal('dailyStrategyBudgets' in model.state$.value, false);
    model.stop();
  });

  it('saves global controls and position safety caps without legacy allocation maps', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    setAutoTraderRpcRequesterForTests(async (method, path, body) => {
      calls.push({ method, path, body });
      return new Response('{}', { status: method === 'PUT' ? 200 : 503 });
    });
    try {
      const model = new AutoTraderModel();
      model.setStrategies(['trend-50-200', 'live-momentum']);
      model.setLimits('250', '2000', '25', '15', '8');
      model.setCategoryLimits({
        maxIntradayOpenPositions: '3',
        maxDailyOpenPositions: '4',
      });
      await model.saveLimits();
      const payload = calls.find(
        (call) => call.path === '/positions/auto-trader/settings',
      )?.body as Record<string, unknown>;
      assert.ok(payload);
      assert.equal(payload.max_order_notional, 250);
      assert.equal(payload.daily_notional_limit, 2000);
      assert.equal(payload.daily_realized_loss_limit, 25);
      assert.equal(payload.exploration_percent, 15);
      assert.equal(payload.max_open_positions, 8);
      assert.equal(payload.max_intraday_open_positions, 3);
      assert.equal(payload.max_daily_open_positions, 4);
      assert.equal('daily_strategy_budgets' in payload, false);
      assert.equal('intraday_strategy_budgets' in payload, false);
      assert.equal('daily_strategy_notional_limit' in payload, false);
      assert.equal('intraday_notional_limit' in payload, false);
      model.stop();
    } finally {
      setAutoTraderRpcRequesterForTests();
    }
  });
});
