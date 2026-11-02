import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { AutoTraderModel } from './AutoTraderModel';
import { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';
import { MarketLiquidityModel } from './MarketLiquidityModel';
import { OrderManagerModel } from './OrderManagerModel';
import { PnlModel } from './PnlModel';
import { PriceChangeChartModel } from './PriceChangeChartModel';
import { TradeTicketModel } from './TradeTicketModel';
import type { Connection } from '../connection';
import type { Position } from './PositionTypes';

async function withFetch(impl: typeof fetch, run: () => Promise<void>) {
  const original = globalThis.fetch;
  globalThis.fetch = impl;
  try { await run(); } finally { globalThis.fetch = original; }
}
function response(body: unknown, ok = true, status = 200) {
  return { ok, status, json: async () => body, text: async () => '' } as Response;
}

describe('AutoTraderModel', () => {
  it('creates appropriate budget defaults as strategies are selected', () => {
    const model = new AutoTraderModel();
    model.setDailyStrategyBudget('donchian-55-btc200', '150');
    model.setIntradayStrategyBudget('live-momentum', '75');
    model.setStrategies(['donchian-55-btc200', 'live-momentum', 'trend-50-200', 'dated-futures-carry']);
    assert.equal(model.state$.value.dailyStrategyBudgets['donchian-55-btc200'], '150');
    assert.equal(model.state$.value.dailyStrategyBudgets['trend-50-200'], '0');
    assert.equal(model.state$.value.dailyStrategyBudgets['dated-futures-carry'], undefined);
    assert.equal(model.state$.value.intradayStrategyBudgets['live-momentum'], '75');
    model.setStrategyParameter('BTC-USDT:live-momentum:lookback', 12);
    assert.equal(model.state$.value.strategyParameterOverrides['BTC-USDT:live-momentum:lookback'], 12);
    model.setStrategyParameter('BTC-USDT:live-momentum:lookback');
    assert.equal(model.state$.value.strategyParameterOverrides['BTC-USDT:live-momentum:lookback'], undefined);
    model.stop();
  });
});

describe('AutoTraderOutcomesModel', () => {
  it('adds and removes close confirmations and clears them after a failed close', async () => {
    await withFetch(async () => response('risk gate denied', false, 409), async () => {
      const model = new AutoTraderOutcomesModel();
      model.requestClose('BTC-USDT');
      assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), true);
      model.cancelClose('BTC-USDT');
      assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), false);
      model.requestClose('BTC-USDT');
      await model.confirmClose('BTC-USDT');
      assert.equal(model.state$.value.closingSymbols.has('BTC-USDT'), false);
      assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), false);
      assert.equal(model.state$.value.error, 'Close IOC failed (HTTP 409)');
      model.stop();
    });
  });
});

function feedConnection() {
  const bookFeeds: Array<{ key: string; symbol: string; data: BehaviorSubject<unknown | undefined>; error: BehaviorSubject<string | undefined> }> = [];
  const closes: string[] = [];
  const connection = {
    openBook(key: string, symbol: string) {
      const feed = { key, symbol, data: new BehaviorSubject<unknown | undefined>(undefined), error: new BehaviorSubject<string | undefined>(undefined) };
      bookFeeds.push(feed);
      return feed;
    },
    closeBook(key: string) { closes.push(key); },
  } as unknown as Connection;
  return { connection, bookFeeds, closes };
}

describe('MarketLiquidityModel', () => {
  it('surfaces feed errors and releases both feed subscriptions on stop', () => {
    const mock = feedConnection();
    const tradeFeeds: Array<{ key: string; symbol: string; data: BehaviorSubject<unknown | undefined>; error: BehaviorSubject<string | undefined> }> = [];
    const connection = { ...mock.connection,
      openTrades(key: string, symbol: string) { const feed = { key, symbol, data: new BehaviorSubject<unknown | undefined>(undefined), error: new BehaviorSubject<string | undefined>(undefined) }; tradeFeeds.push(feed); return feed; },
      closeTrades(key: string) { mock.closes.push(key); },
    } as unknown as Connection;
    const model = new MarketLiquidityModel({ symbol: 'BTC-USDT', connection, streamKey: 'liq' });
    model.start();
    mock.bookFeeds[0].error.next('book feed down');
    assert.equal(model.state$.value.error, 'book feed down');
    tradeFeeds[0].error.next('trade feed down');
    assert.equal(model.state$.value.error, 'trade feed down');
    model.stop();
    assert.deepEqual(mock.closes.sort(), ['liq:book', 'liq:trades']);
  });
});

describe('OrderManagerModel', () => {
  it('loads orders, derives prices, and reports server errors', async () => {
    const mock = feedConnection();
    let ok = true;
    let calls = 0;
    const body = { orders: [{ id: 'o1', symbol: 'ETH-USDT', side: 'buy', order_type: 'limit', quantity: 2, remaining_quantity: 2, price: 100, status: 'open', timestamp: 1 }], barrier_orders: [] };
    await withFetch(async () => { calls++; return response(ok ? body : 'server unavailable', ok, 503); }, async () => {
      const model = new OrderManagerModel({ connection: mock.connection, symbols: ['BTC-USDT', 'ETH-USDT'] });
      await model.refreshOrders();
      assert.equal(model.state$.value.orders[0].id, 'o1');
      assert.equal(model.state$.value.loading, false);
      assert.equal(mock.bookFeeds.length, 2); // selected and working-order symbols
      ok = false;
      await model.refreshOrders();
      assert.match(model.state$.value.error ?? '', /Could not load resting orders/);
      assert.equal(calls, 2);
      model.stop();
    });
  });
});

describe('PnlModel', () => {
  it('marks long positions from bid depth and totals converted net PnL', () => {
    const model = new PnlModel(feedConnection().connection);
    const position = { symbol: 'BTC-USDT', quantity: 2, average_entry_price: 100, entry_fees: 1, estimated_exit_fee_rate: 0.01, realized_pnl: 5, fees_paid: 0.5 } as Position;
    (model as any).positions = [position];
    (model as any).usdRates = { USDT: 1 };
    (model as any).books = { 'BTC-USDT': { symbol: 'BTC-USDT', bids: [{ price: 110, size: 2 }], asks: [{ price: 111, size: 2 }], timestamp: 123 } };
    (model as any).publishState();
    const state = model.getSnapshot();
    assert.equal(state.rows[0].mark, 110);
    assert.equal(state.rows[0].unrealized, 16.8);
    assert.equal(state.rows[0].realizedUsd, 5);
    assert.equal(state.totals.USD.unrealized, 16.8);
    assert.equal(state.loading, false);
    model.stop();
  });
});

describe('PriceChangeChartModel', () => {
  it('uses controlled selection callbacks and ignores nonpositive resize values', () => {
    const selections: string[] = [];
    const model = new PriceChangeChartModel({ changes: { ticks: [], histories: new Map() }, initialSymbol: 'BTC-USDT', onSymbolChange: (symbol) => selections.push(symbol) });
    model.selectSymbol('ETH-USDT');
    assert.deepEqual(selections, ['ETH-USDT']);
    assert.equal(model.getSnapshot().chartSize.width, 900);
    model.setChartSize(600, 200);
    assert.deepEqual(model.getSnapshot().chartSize, { width: 600, height: 200 });
    model.updateProps({ changes: { ticks: [], histories: new Map() }, selectedSymbol: 'SOL-USDT', onSymbolChange: (symbol) => selections.push(symbol) });
    assert.deepEqual(selections, ['ETH-USDT']);
  });
});

describe('AutoTraderModel refresh', () => {
  it('maps server limits, ledger positions, and order PnL into client state', async () => {
    const status = {
      enabled: true, strategies: ['trend-50-200'], max_order_notional: 250, daily_notional_limit: 1000,
      daily_realized_loss_limit: 50, exploration_percent: 15, max_open_positions: 8,
      max_intraday_open_positions: 4, max_daily_open_positions: 6, intraday_notional_limit: 500,
      daily_strategy_notional_limit: 700, intraday_used_notional: 20, daily_strategy_used_notional: 30,
      max_entries_per_auto_position: 2, stop_loss_percent: 1, take_profit_percent: 2,
      vwap_duration_minutes: 20, vwap_participation_percent: 15, limit_offset_bps: 3,
      trigger_offset_bps: 4, trailing_distance_percent: 0.2, used_notional_today: 40,
      available_notional: 960,
    };
    const activity = { orders: [{ symbol: 'BTC-USDT', realized_pnl: 2 }], open_position_symbols: ['BTC-USDT'], exit_learning_samples: 3, learning_samples: 5, realized_pnl_by_quote: { USD: 4 }, unrealized_pnl_by_quote: { USD: 1 } };
    await withFetch(async (url) => {
      const path = String(url);
      if (path === '/positions/auto-trader') return response(status);
      if (path === '/positions/auto-trader/activity') return response(activity);
      if (path === '/positions?source=auto') return response({ positions: [{ symbol: 'BTC-USDT', quantity: 1 }] });
      return response({ items: [] });
    }, async () => {
      const model = new AutoTraderModel();
      (model as any).usdRates = { USDT: 1 };
      await model.refresh();
      assert.equal(model.state$.value.loading, false);
      assert.equal(model.state$.value.enabled, true);
      assert.deepEqual(model.state$.value.strategies, ['trend-50-200']);
      assert.deepEqual(model.state$.value.ledgerOpenPositionSymbols, ['BTC-USDT']);
      assert.equal(model.state$.value.orders[0].realized_pnl_usd, 2);
      assert.equal(model.state$.value.realizedPnl, 4);
      model.stop();
    });
  });
});


describe('TradeTicketModel submission', () => {
  it('rejects a nonpositive quantity without posting an order', async () => {
    const paths: string[] = [];
    await withFetch(async (url) => { paths.push(String(url)); return response({ symbols: [], instruments: [] }); }, async () => {
      const model = new TradeTicketModel({ symbol: 'TEST-TICKET', initialSide: 'buy', connection: {} as Connection });
      model.setQuantity('0');
      await model.submit();
      assert.deepEqual(paths, ['/instruments']);
      assert.equal(model.state$.value.busy, false);
      assert.equal(model.state$.value.quantity, '0');
      model.stop();
    });
  });
});

describe('OrderManagerModel submission', () => {
  it('posts a valid order body and refreshes the resting-order list', async () => {
    const mock = feedConnection();
    const requests: Array<{ path: string; init?: RequestInit }> = [];
    await withFetch(async (url, init) => {
      const path = String(url);
      requests.push({ path, init });
      if (path === '/instruments') return response({ symbols: [], instruments: [] });
      if (path === '/positions/orders' && init?.method === 'POST') return response({ filled_quantity: 1, remaining_quantity: 0 });
      return response({ orders: [], barrier_orders: [] });
    }, async () => {
      const model = new OrderManagerModel({ connection: mock.connection, symbols: ['TEST-ORDER'] });
      model.setQuantity('1');
      await model.submit();
      const post = requests.find((request) => request.path === '/positions/orders' && request.init?.method === 'POST');
      assert.ok(post);
      assert.deepEqual(JSON.parse(String(post.init?.body)), { symbol: 'TEST-ORDER', side: 'buy', order_type: 'ioc', quantity: 1 });
      assert.equal(model.state$.value.quantity, '');
      assert.equal(model.state$.value.submitting, false);
      assert.equal(requests.some((request) => request.path === '/positions/orders' && !request.init?.method), true);
      model.stop();
    });
  });
});
