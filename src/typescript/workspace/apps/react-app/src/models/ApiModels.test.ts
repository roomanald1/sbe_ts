import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AccountModel } from './AccountModel';
import { AlternativeStrategiesModel } from './AlternativeStrategiesModel';
import { DatedCarryModel } from './DatedCarryModel';
import { ExecutionFeesModel } from './ExecutionFeesModel';
import { FillsModel } from './FillsModel';
import { UsersModel } from './UsersModel';

function jsonResponse(body: unknown, ok = true, status = 200) {
  return { ok, status, statusText: ok ? 'OK' : 'Failure', json: async () => body, text: async () => typeof body === 'string' ? body : JSON.stringify(body) } as Response;
}
async function withFetch(impl: typeof fetch, run: () => Promise<void>) {
  const original = globalThis.fetch;
  globalThis.fetch = impl;
  try { await run(); } finally { globalThis.fetch = original; }
}
const account = { cash: 10, equity: 12, realized_pnl: 1, unrealized_pnl: 1, fees_paid: 0, estimated_exit_fees: 0, equity_available: true, initial_cash: 10, topups: [], max_leverage: 3 };

describe('AccountModel API behavior', () => {
  it('loads account and converts auto-trader quote totals into USD', async () => withFetch(async (input) => {
    if (String(input) === '/positions/account') return jsonResponse(account);
    return jsonResponse({ realized_pnl_by_quote: { USD: 2, BTC: 0.5 }, unrealized_pnl_by_quote: { USD: -1 } });
  }, async () => {
    const model = new AccountModel();
    await model.refresh();
    assert.equal(model.state$.value.loading, false);
    assert.equal(model.state$.value.equity, 12);
    // No BTC/USD rate is available in a cold stream, so an unconvertible total is withheld.
    assert.equal(model.state$.value.auto_trader_total, null);
  }));

  it('records API errors and clears submitting after leverage updates fail', async () => withFetch(async () => jsonResponse('leverage rejected', false, 400), async () => {
    const model = new AccountModel();
    await model.setMaxLeverage(4);
    assert.equal(model.state$.value.submitting, false);
    assert.equal(model.state$.value.error, 'leverage rejected');
  }));
});

describe('AlternativeStrategiesModel API behavior', () => {
  it('loads a snapshot and derives rows, events, positions, and feed freshness', async () => {
    const now = Date.now();
    const sleeve = { id: 'residual-momentum', config: { enabled: true, max_trade_usd: 100, daily_loss_limit_usd: 50, spot_fee_bps: 5, short_borrow_apr: 0, min_edge_bps: 2 }, initial_capital_usd: 100, cash_usd: 100, equity_usd: 100, max_drawdown_percent: 0, realized_pnl_usd: 0, fees_usd: 0, last_observation_ms: now, status: 'ready', positions: { BTC: { kind: 'spot', quantity: 1 } }, events: [{ timestamp_ms: now, observation_id: 'e1', action: 'hold', detail: 'ok', cash_usd: 100, equity_usd: 100 }], feed: { ready: true, signal: 'positive', source_timestamp_ms: now, computed_at_ms: now, summary: 'fresh', features: {} } };
    await withFetch(async () => jsonResponse({ mode: 'paper', auto_trader_enabled: true, book: { sleeves: { [sleeve.id]: sleeve } } }), async () => {
      const model = new AlternativeStrategiesModel();
      model.start();
      await new Promise<void>((resolve) => setImmediate(resolve));
      assert.equal(model.state$.value.loading, false);
      assert.equal(model.feedReady(sleeve.id), true);
      assert.equal(model.rows()[0].open_positions, 1);
      assert.equal(model.events()[0].key, sleeve.id + ':e1:0');
      assert.equal(model.positions()[0].instrument, 'BTC');
      model.stop();
    });
  });

  it('rejects an invalid draft without sending a save request', async () => {
    let requests = 0;
    await withFetch(async () => { requests++; return jsonResponse({ mode: 'paper', auto_trader_enabled: false, book: { sleeves: {} } }); }, async () => {
      const model = new AlternativeStrategiesModel();
      model.start();
      await new Promise<void>((resolve) => setImmediate(resolve));
      model.state$.next({ ...model.state$.value, drafts: { x: { enabled: true, max_trade_usd: 0, daily_loss_limit_usd: 1, spot_fee_bps: 0, short_borrow_apr: 0, min_edge_bps: 0 } } });
      await model.save('x');
      assert.equal(requests, 1);
      assert.match(model.state$.value.error ?? '', /Check the paper size/);
      model.stop();
    });
  });
});

describe('ExecutionFeesModel API behavior', () => {
  it('publishes fee rows from a valid summary and reports malformed payloads', async () => {
    let next: unknown = { fills: 3, rows: [{ source: 'OKX', liquidity: 'maker', fee_currency: 'USDT', fills: 3, traded_notional: 100, fees_paid: 0.1, first_fill_ms: 1, last_fill_ms: 2 }] };
    await withFetch(async () => jsonResponse(next), async () => {
      const model = new ExecutionFeesModel();
      await model.refresh();
      assert.equal(model.state$.value.fills, 3);
      assert.equal(model.state$.value.rows[0].source, 'OKX');
      assert.equal(model.state$.value.loading, false);
      next = { fills: 'bad', rows: [] };
      await model.refresh();
      assert.match(model.state$.value.error ?? '', /invalid data/);
    });
  });
});

describe('FillsModel API behavior', () => {
  it('reverses ledger order and preserves an error on failed requests', async () => {
    let ok = true;
    await withFetch(async () => jsonResponse({ trades: [{ id: 'old' }, { id: 'new' }] }, ok, 503), async () => {
      const model = new FillsModel();
      await model.refresh();
      assert.deepEqual(model.state$.value.fills.map((trade) => trade.id), ['new', 'old']);
      ok = false;
      await model.refresh();
      assert.match(model.state$.value.error ?? '', /Could not load fills/);
    });
  });
});

describe('DatedCarryModel API behavior', () => {
  it('loads configured size and validates submitted notional bounds', async () => {
    const book = { enabled: true, max_spot_notional_usdt: 2500, candidates: [], positions: [], checked_at_ms: 1 };
    let calls = 0;
    await withFetch(async () => { calls++; return jsonResponse(book); }, async () => {
      const model = new DatedCarryModel();
      await model.refresh();
      assert.equal(model.state$.value.notional, '2500');
      model.setNotional('99');
      await model.saveSize();
      assert.match(model.state$.value.error ?? '', /between 100 and 10,000/);
      assert.equal(calls, 1);
      model.setNotional('3000');
      await model.saveSize();
      assert.equal(calls, 2);
      assert.equal(model.state$.value.book?.max_spot_notional_usdt, 2500);
      model.stop();
    });
  });
});

describe('UsersModel API behavior', () => {
  it('maps valid page rows into a grid and rejects invalid timestamps', async () => {
    let result: unknown = { data: [{ id: 7, last_logged_in: 1_700_000_000 }], has_more: true };
    await withFetch(async () => jsonResponse(result), async () => {
      const model = new UsersModel();
      await model.refresh();
      assert.equal(model.state$.value.rowData[0].users[0]?.id, 7);
      assert.equal(model.state$.value.hasMore, true);
      result = { data: [{ id: 8, last_logged_in: Number.NaN }], has_more: false };
      await model.refresh();
      assert.equal(model.state$.value.isError, true);
      assert.match(model.state$.value.error ?? '', /invalid data/);
      model.stop();
    });
  });
});
