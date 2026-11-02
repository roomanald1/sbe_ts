import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { PnlModel } from './PnlModel';
import type { Connection } from '../connection';

describe('PnlModel', () => {
  it('starts with an empty snapshot and accepts its current connection', () => {
    const connection = {} as Connection;
    const model = new PnlModel(connection);
    assert.deepEqual(model.getSnapshot().rows, []);
    assert.equal(model.props$.value.connection, connection);
    model.setConnection(connection);
    model.stop();
  });
});
