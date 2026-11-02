import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { OrderBookModel } from './OrderBookModel';
import type { Connection } from '../connection';

describe('OrderBookModel', () => {
  it('initializes the symbol and exposes a stable snapshot', () => {
    const model = new OrderBookModel({ symbol: 'ETH-USDT', connection: {} as Connection, streamKey: 'book' });
    assert.equal(model.getSnapshot().symbol, 'ETH-USDT');
    assert.equal(model.getSnapshot().frozen, false);
    model.stop();
  });
});
