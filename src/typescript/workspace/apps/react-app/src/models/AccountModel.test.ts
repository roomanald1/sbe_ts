import { describe, it, afterEach, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { AccountModel } from './AccountModel';
import { setAutoTraderRpcRequesterForTests } from '../auto_trader_rsocket';

const account = {
  cash: 100,
  equity: 120,
  realized_pnl: 10,
  unrealized_pnl: 10,
  fees_paid: 1,
  estimated_exit_fees: 0.5,
  equity_available: true,
  initial_cash: 100,
  topups: [],
  max_leverage: 3,
};
const response = (body: unknown, ok = true, status = 200) => ({
  ok,
  status,
  json: async () => body,
  text: async () => '',
});
const originalFetch = globalThis.fetch;

beforeEach(() => {
  setAutoTraderRpcRequesterForTests(async (_method, path) =>
    globalThis.fetch(path),
  );
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  setAutoTraderRpcRequesterForTests();
});

describe('AccountModel', () => {
  it('starts with safe account defaults and can stop repeatedly', () => {
    const model = new AccountModel();
    assert.equal(model.state$.value.loading, true);
    assert.equal(model.state$.value.max_leverage, 2);
    assert.equal(model.state$.value.submitting, false);
    model.stop();
    model.stop();
  });

  it('loads account data and converts realized and unrealized quote P&L to USD', async () => {
    globalThis.fetch = async (input) =>
      String(input).includes('/account')
        ? (response(account) as Response)
        : (response({
            realized_pnl_by_quote: { USD: 2 },
            unrealized_pnl_by_quote: { USD: -1 },
          }) as Response);
    const model = new AccountModel();
    await model.refresh();
    assert.equal(model.state$.value.cash, 100);
    assert.equal(model.state$.value.loading, false);
    assert.equal(model.state$.value.auto_trader_total, 1);
  });

  it('leaves auto-trader total unavailable when a quote rate is missing', async () => {
    globalThis.fetch = async (input) =>
      String(input).includes('/account')
        ? (response(account) as Response)
        : (response({
            realized_pnl_by_quote: { UNKNOWN: 2 },
            unrealized_pnl_by_quote: {},
          }) as Response);
    const model = new AccountModel();
    await model.refresh();
    assert.equal(model.state$.value.auto_trader_total, null);
  });

  it('reports account load failures', async () => {
    globalThis.fetch = async () => response({}, false, 503) as Response;
    const model = new AccountModel();
    await model.refresh();
    assert.equal(model.state$.value.loading, false);
    assert.match(model.state$.value.error ?? '', /503/);
  });

  it('submits a top-up and refreshes account state', async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    globalThis.fetch = async (input, init) => {
      const url = String(input);
      calls.push({ url, init });
      return url.endsWith('/topups')
        ? (response({}) as Response)
        : url.endsWith('/account')
          ? (response({ ...account, cash: 150 }) as Response)
          : (response({
              realized_pnl_by_quote: {},
              unrealized_pnl_by_quote: {},
            }) as Response);
    };
    const model = new AccountModel();
    await model.topUp(50);
    assert.equal(JSON.parse(String(calls[0].init?.body)).amount, 50);
    assert.equal(model.state$.value.cash, 150);
    assert.equal(model.state$.value.submitting, false);
  });

  it('updates leverage and surfaces failed top-up errors', async () => {
    globalThis.fetch = async (input) =>
      String(input).endsWith('/leverage')
        ? (response({ ...account, max_leverage: 5 }) as Response)
        : ({
            ...response({}, false, 400),
            text: async () => 'insufficient funds',
          } as Response);
    const model = new AccountModel();
    await model.setMaxLeverage(5);
    assert.equal(model.state$.value.max_leverage, 5);
    await model.topUp(1000);
    assert.equal(model.state$.value.error, 'insufficient funds');
    assert.equal(model.state$.value.submitting, false);
  });
});
