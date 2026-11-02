import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { DatedCarryModel } from './DatedCarryModel';

describe('DatedCarryModel', () => {
  it('stores the editable notional draft', () => {
    const model = new DatedCarryModel();
    model.setNotional('1500');
    assert.equal(model.state$.value.notional, '1500');
    model.stop();
  });
});
