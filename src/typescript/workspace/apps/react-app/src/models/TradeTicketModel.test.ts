import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { TradeTicketModel } from './TradeTicketModel';
import type { Connection } from '../connection';

describe('TradeTicketModel', () => {
  it('initializes order defaults and updates editable order fields', () => {
    const model = new TradeTicketModel({
      symbol: 'BTC-USDT',
      initialSide: 'sell',
      connection: {} as Connection,
    });
    assert.equal(model.state$.value.side, 'sell');
    assert.equal(model.state$.value.barrierDirection, 'above');
    model.setQuantity('2.5');
    model.setOrderType('limit');
    assert.equal(model.state$.value.quantity, '2.5');
    assert.equal(model.state$.value.orderType, 'limit');
    model.stop();
  });
});
