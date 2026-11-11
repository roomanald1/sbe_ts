import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { SignalsModel } from './SignalsModel';
import type { Connection } from '../connection';

describe('SignalsModel', () => {
  it('decodes matching updates and reports malformed data', () => {
    const data = new BehaviorSubject<unknown | undefined>(undefined);
    const error = new BehaviorSubject<string | undefined>(undefined);
    const closed: string[] = [];
    const connection = {
      openSignals: (key: string) => ({ key, data, error }),
      closeSignals: (key: string) => closed.push(key),
    } as unknown as Connection;
    const model = new SignalsModel({
      connection,
      symbol: 'BTC-USDT',
      streamKey: 'signals',
    });
    model.start();
    const snapshot = {
      symbol: 'BTC-USDT',
      timestamp: 1,
      direction: 'bullish',
      score: 1,
      strength: 1,
      components: {},
      reasons: [],
      stale: false,
    };
    data.next(JSON.stringify(snapshot));
    assert.deepEqual(model.state$.value.snapshot, snapshot);
    data.next('{bad json');
    assert.equal(
      model.state$.value.error,
      'Received an invalid signal update.',
    );
    model.stop();
    assert.deepEqual(closed, ['signals']);
  });
});
