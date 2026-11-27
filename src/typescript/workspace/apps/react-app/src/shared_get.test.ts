import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';
import {
  sharedGet,
  usesExecutionRSocket,
  usesPositionManagementRSocket,
} from './shared_get';

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe('sharedGet', () => {
  it('routes operational position reads to Position Management', () => {
    assert.equal(
      usesPositionManagementRSocket('/positions/operational?source=auto'),
      true,
    );
    assert.equal(
      usesExecutionRSocket('/positions/operational?source=auto'),
      false,
    );
  });

  it('shares concurrent GETs while giving each caller a readable response', async () => {
    let calls = 0;
    globalThis.fetch = (async () => {
      calls += 1;
      await new Promise((resolve) => setTimeout(resolve, 0));
      return new Response(JSON.stringify({ value: 42 }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof fetch;

    const [first, second] = await Promise.all([
      sharedGet('/positions/account'),
      sharedGet('/positions/account'),
    ]);

    assert.equal(calls, 1);
    assert.deepEqual(await first.json(), { value: 42 });
    assert.deepEqual(await second.json(), { value: 42 });
  });
});
