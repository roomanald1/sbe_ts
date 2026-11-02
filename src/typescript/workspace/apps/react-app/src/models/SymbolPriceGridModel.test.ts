import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { SymbolPriceGridModel } from './SymbolPriceGridModel';
import type { SymbolPriceTick } from '../types/symbol_price';

describe('SymbolPriceGridModel', () => {
  it('builds a 10 by 10 grid and routes symbol actions', () => {
    const selected: string[] = [];
    const traded: string[] = [];
    const tick = { symbol: 'BTC-USDT', price: 10, volatility1m: 1, volatility5m: 2, direction: 'up' } as SymbolPriceTick;
    const model = new SymbolPriceGridModel({ ticks: [tick], feedSource: 'okx', compact: false, onSelectSymbol: (s, side) => selected.push(s + ':' + side), onTradeSymbol: (s, side) => traded.push(s + ':' + side) });
    assert.equal(model.state$.value.rows.length, 10);
    assert.equal(model.state$.value.rows[0].cells.length, 10);
    assert.equal(model.state$.value.rows[0].cells[0], tick);
    model.selectSymbol('BTC-USDT', 'buy');
    model.tradeSymbol('BTC-USDT', 'sell');
    assert.deepEqual(selected, ['BTC-USDT:buy']);
    assert.deepEqual(traded, ['BTC-USDT:sell']);
  });
  it('reuses unchanged row objects when props update', () => {
    const tick = { symbol: 'BTC-USDT', price: 10 } as SymbolPriceTick;
    const model = new SymbolPriceGridModel({ ticks: [tick], feedSource: 'okx', compact: false });
    const previous = model.state$.value.rows[0];
    model.setProps({ ticks: [{ ...tick }], feedSource: 'okx', compact: true });
    assert.equal(model.state$.value.rows[0], previous);
    assert.equal(model.state$.value.compact, true);
  });
});
