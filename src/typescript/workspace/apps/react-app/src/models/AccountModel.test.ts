import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AccountModel } from './AccountModel';

describe('AccountModel', () => {
  it('starts with safe account defaults and can stop repeatedly', () => {
    const model = new AccountModel();
    assert.equal(model.state$.value.loading, true);
    assert.equal(model.state$.value.max_leverage, 2);
    assert.equal(model.state$.value.submitting, false);
    model.stop();
    model.stop();
  });
});
