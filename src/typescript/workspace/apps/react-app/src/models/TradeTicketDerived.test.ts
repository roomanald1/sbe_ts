import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { TradeTicketModel } from './TradeTicketModel';
import { deriveTradeTicketState } from './TradeTicketDerived';
import type { Connection } from '../connection';

function initialState() {
  return new TradeTicketModel({ symbol: 'BTC-USDT', initialSide: 'buy', connection: {} as Connection }).state$.value;
}

describe('deriveTradeTicketState', () => {
  it('derives midpoint, spread, notional and depth weighted fill estimates', () => {
    const state = deriveTradeTicketState(initialState(), { quantity: '3', book: { bids: [{ price: 99, size: 5 }], asks: [{ price: 101, size: 1 }, { price: 103, size: 4 }] } as never });
    assert.equal(state.currentPrice, 100);
    assert.equal(state.spread, 2);
    assert.equal(state.estimatedNotional, 300);
    assert.equal(state.estimatedFillQuantity, 3);
    assert.equal(state.estimatedFillAverage, 101 + 4 / 3);
    assert.equal(state.estimatedUnfilledQuantity, 0);
  });
  it('clears quantity estimates for invalid quantities', () => {
    const state = deriveTradeTicketState(initialState(), { quantity: 'not-a-number', book: { bids: [], asks: [{ price: 101, size: 2 }] } as never });
    assert.equal(state.estimatedNotional, undefined);
    assert.equal(state.estimatedFillQuantity, undefined);
  });
});
