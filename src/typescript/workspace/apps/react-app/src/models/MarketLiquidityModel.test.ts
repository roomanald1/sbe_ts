import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { MarketLiquidityModel } from './MarketLiquidityModel';
import type { Connection } from '../connection';

describe('MarketLiquidityModel', () => {
  it('initializes state for its selected symbol', () => {
    const model = new MarketLiquidityModel({ symbol: 'BTC-USDT', connection: {} as Connection, streamKey: 'test' });
    assert.equal(model.state$.value.symbol, 'BTC-USDT');
    assert.deepEqual(model.state$.value.bands, []);
    model.stop();
  });
});
