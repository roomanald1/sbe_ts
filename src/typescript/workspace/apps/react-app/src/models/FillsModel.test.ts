import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { FillsModel } from './FillsModel';

describe('FillsModel', () => {
  it('exposes its empty snapshot consistently', () => {
    const model = new FillsModel();
    assert.deepEqual(model.getSnapshot().fills, []);
    assert.equal(model.getSnapshot(), model.state$.value);
    model.stop();
  });
});
