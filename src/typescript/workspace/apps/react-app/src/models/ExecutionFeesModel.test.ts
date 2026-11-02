import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { ExecutionFeesModel } from './ExecutionFeesModel';

describe('ExecutionFeesModel', () => {
  it('starts loading with no fee rows', () => {
    const model = new ExecutionFeesModel();
    assert.equal(model.state$.value.loading, true);
    assert.deepEqual(model.state$.value.rows, []);
    model.stop();
  });
});
