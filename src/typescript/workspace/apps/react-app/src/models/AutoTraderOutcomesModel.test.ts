import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';

describe('AutoTraderOutcomesModel', () => {
  it('tracks close confirmation state', () => {
    const model = new AutoTraderOutcomesModel();
    assert.equal(model.state$.value.loading, true);
    model.requestClose('BTC-USDT');
    assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), true);
    model.cancelClose('BTC-USDT');
    assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), false);
    model.stop();
  });
});
