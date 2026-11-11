import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { decodeNewsSnapshot } from './NewsTickerModel';

describe('NewsTickerModel', () => {
  it('decodes the Insights news snapshot stream payload', () => {
    const snapshot = {
      updated_at_ms: 123,
      source: 'CoinDesk RSS',
      stale: false,
      error: null,
      articles: [],
    };
    const decoded = decodeNewsSnapshot(
      new TextEncoder().encode(JSON.stringify(snapshot)),
    );
    assert.deepEqual(decoded, snapshot);
  });

  it('rejects malformed news stream payloads', () => {
    assert.throws(
      () => decodeNewsSnapshot(new TextEncoder().encode('{"articles":[]}')),
      /Invalid news snapshot/,
    );
  });
});
