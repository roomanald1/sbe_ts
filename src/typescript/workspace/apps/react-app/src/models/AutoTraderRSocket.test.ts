import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  autoTraderRpc,
  autoTraderRequestStream,
  setAutoTraderRpcRequesterForTests,
  setAutoTraderSocketFactoryForTests,
} from '../auto_trader_rsocket';
import { isAutoTraderSnapshotFresh } from '../auto_trader_stream';

describe('AutoTrader RSocket API client', () => {
  it('uses stream snapshots only while they are recent', () => {
    assert.equal(isAutoTraderSnapshotFresh(55_000, 100_000), true);
    assert.equal(isAutoTraderSnapshotFresh(54_999, 100_000), false);
    assert.equal(isAutoTraderSnapshotFresh(0, 1_000), false);
  });

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

  it('invalidates a closed socket and reconnects the AutoTrader stream', async () => {
    const sockets: Array<{
      client: unknown;
      close: () => void;
    }> = [];
    setAutoTraderSocketFactoryForTests(async () => {
      const observers: Array<{
        emit: (status: { kind: string }) => void;
      }> = [];
      const socket = {
        client: {
          connectionStatus: () => ({
            subscribe: (observer: {
              onSubscribe: (subscription: {
                cancel: () => void;
                request: (count: number) => void;
              }) => void;
              onNext: (status: { kind: string }) => void;
            }) => {
              let demand = 0;
              observer.onSubscribe({
                cancel: () => undefined,
                request: (count) => {
                  demand += count;
                },
              });
              observers.push({
                emit: (status) => {
                  if (demand > 0) {
                    demand -= 1;
                    observer.onNext(status);
                  }
                },
              });
            },
          }),
          requestResponse: () => ({
            subscribe: (observer: {
              onComplete: (payload: { data: Uint8Array }) => void;
            }) => observer.onComplete({ data: new Uint8Array() }),
          }),
        },
        close: () => {
          for (const observer of observers) observer.emit({ kind: 'CLOSED' });
        },
      };
      sockets.push(socket);
      return socket.client;
    });

    try {
      const errors: string[] = [];
      const first = await autoTraderRequestStream(
        'autotrader:snapshot',
        () => undefined,
        (error) => errors.push(error.message),
      );
      sockets[0].close();
      assert.deepEqual(errors, ['AutoTrader RSocket connection closed']);

      const second = await autoTraderRequestStream(
        'autotrader:snapshot',
        () => undefined,
        () => undefined,
      );
      assert.equal(sockets.length, 2);
      first.cancel();
      second.cancel();
    } finally {
      setAutoTraderSocketFactoryForTests();
    }
  });

  it('reports synchronous request failures so the shared stream can retry', async () => {
    setAutoTraderSocketFactoryForTests(async () => ({
      connectionStatus: () => ({
        subscribe: (observer: {
          onSubscribe: (subscription: {
            cancel: () => void;
            request: (count: number) => void;
          }) => void;
        }) =>
          observer.onSubscribe({
            cancel: () => undefined,
            request: () => undefined,
          }),
      }),
      requestResponse: () => {
        throw new Error('Connection closed');
      },
    }));

    try {
      const errors: string[] = [];
      await autoTraderRequestStream(
        'autotrader:snapshot',
        () => undefined,
        (error) => errors.push(error.message),
      );
      assert.deepEqual(errors, ['Connection closed']);
    } finally {
      setAutoTraderSocketFactoryForTests();
    }
  });

  it('times out and cancels an AutoTrader RPC that never responds', async () => {
    let cancelled = false;
    setAutoTraderSocketFactoryForTests(async () => ({
      connectionStatus: () => ({
        subscribe: (observer: {
          onSubscribe: (subscription: {
            cancel: () => void;
            request: (count: number) => void;
          }) => void;
        }) =>
          observer.onSubscribe({
            cancel: () => undefined,
            request: () => undefined,
          }),
      }),
      requestResponse: () => ({
        subscribe: (observer: {
          onSubscribe: (cancel: () => void) => void;
        }) => {
          observer.onSubscribe(() => {
            cancelled = true;
          });
        },
      }),
    }));

    try {
      await assert.rejects(
        autoTraderRpc('GET', '/positions/auto-trader'),
        /AutoTrader GET \/positions\/auto-trader timed out after 5000 ms/,
      );
      assert.equal(cancelled, true);
    } finally {
      setAutoTraderSocketFactoryForTests();
    }
  });
});
