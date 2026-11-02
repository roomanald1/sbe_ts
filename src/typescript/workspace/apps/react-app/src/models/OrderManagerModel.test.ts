import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { OrderManagerModel } from './OrderManagerModel';
import type { Connection } from '../connection';

describe('OrderManagerModel', () => {
  it('selects the first available symbol and updates selection', () => {
    const connection = {
      openBook: (key: string, symbol: string) => ({ key, symbol, data: new BehaviorSubject<unknown | undefined>(undefined), error: new BehaviorSubject<string | undefined>(undefined) }),
      closeBook: () => undefined,
    } as unknown as Connection;
    const model = new OrderManagerModel({ connection, symbols: ['BTC-USDT', 'ETH-USDT'] });
    assert.equal(model.state$.value.selectedSymbol, 'BTC-USDT');
    assert.equal(model.state$.value.orderType, 'ioc');
    model.setSymbol('ETH-USDT');
    assert.equal(model.state$.value.selectedSymbol, 'ETH-USDT');
    model.stop();
  });
});
