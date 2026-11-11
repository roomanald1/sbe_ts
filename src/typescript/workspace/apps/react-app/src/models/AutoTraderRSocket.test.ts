import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  autoTraderRpc,
  setAutoTraderRpcRequesterForTests,
} from '../auto_trader_rsocket';

describe('AutoTrader RSocket API client', () => {
  it('preserves the method, path, body and HTTP-style JSON response contract', async () => {
    let actual: { method: string; path: string; body?: unknown } | undefined;
    setAutoTraderRpcRequesterForTests(async (method, path, body) => {
      actual = { method, path, body };
      return new Response(JSON.stringify({ enabled: true }), {
        status: 202,
        headers: { 'content-type': 'application/json' },
      });
    });

    const response = await autoTraderRpc('PUT', '/positions/auto-trader', {
      enabled: true,
    });
    assert.deepEqual(actual, {
      method: 'PUT',
      path: '/positions/auto-trader',
      body: { enabled: true },
    });
    assert.equal(response.status, 202);
    assert.deepEqual(await response.json(), { enabled: true });
    setAutoTraderRpcRequesterForTests();
  });
});
