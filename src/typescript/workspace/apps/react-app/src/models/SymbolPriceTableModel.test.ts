import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { SymbolPriceTableModel } from './SymbolPriceTableModel';
import type { SymbolPriceTick } from '../types/symbol_price';

describe('SymbolPriceTableModel', () => {
  it('limits visible rows and reports connection-specific empty text', () => {
    const ticks = [{ symbol: 'BTC-USDT' }, { symbol: 'ETH-USDT' }] as SymbolPriceTick[];
    const model = new SymbolPriceTableModel({ ticks, histories: new Map(), state: 'connected', maxRows: 1 });
    assert.deepEqual(model.state$.value.visibleTicks, [ticks[0]]);
    assert.match(model.state$.value.emptyMessage, /first SBE message/);
    model.setProps({ ticks, histories: new Map(), state: 'connecting' });
    assert.equal(model.state$.value.visibleTicks.length, 2);
    assert.match(model.state$.value.emptyMessage, /Connecting/);
  });
  it('forwards symbol selection to the supplied callback', () => {
    let selected = '';
    const model = new SymbolPriceTableModel({ ticks: [], histories: new Map(), state: 'connected', onSelectSymbol: (symbol, side) => { selected = symbol + ':' + side; } });
    model.selectSymbol('ETH-USDT', 'sell');
    assert.equal(selected, 'ETH-USDT:sell');
  });
});
