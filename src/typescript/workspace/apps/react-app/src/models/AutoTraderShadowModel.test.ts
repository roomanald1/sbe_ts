import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderShadowModel } from './AutoTraderShadowModel';

describe('AutoTraderShadowModel', () => {
  it('starts with an empty loading state and stops safely', () => {
    const model = new AutoTraderShadowModel();
    assert.deepEqual(model.state$.value.rows, []);
    assert.equal(model.state$.value.loading, true);
    model.stop();
  });
});
