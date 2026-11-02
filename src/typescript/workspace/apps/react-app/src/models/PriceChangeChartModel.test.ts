import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { PriceChangeChartModel } from './PriceChangeChartModel';

describe('PriceChangeChartModel', () => {
  it('validates resize values and forwards controlled symbol changes', () => {
    const selected: string[] = [];
    const model = new PriceChangeChartModel({ changes: { ticks: [], histories: new Map() }, initialSymbol: 'BTC-USDT', onSymbolChange: (symbol) => selected.push(symbol) });
    model.setChartSize(0, 300);
    assert.equal(model.getSnapshot().chartSize.width, 900);
    model.setChartSize(640, 320);
    assert.deepEqual(model.getSnapshot().chartSize, { width: 640, height: 320 });
    model.selectSymbol('ETH-USDT');
    assert.deepEqual(selected, ['ETH-USDT']);
  });
});
