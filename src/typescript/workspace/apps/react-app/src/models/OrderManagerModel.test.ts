import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { OrderManagerModel } from './OrderManagerModel';
import type { Connection } from '../connection';

function response(body: unknown): Response {
  return { ok: true, status: 200, json: async () => body } as Response;
}

function order(id: string, remaining = 0) {
  return {
    id,
    symbol: 'BTC-USDT',
    side: 'buy' as const,
    quantity: 1,
    remaining_quantity: remaining,
    limit_price: 100,
    timestamp: Number(id.replace(/\D/g, '')) || 1,
    maker_fee_rate: 0,
    estimated_fee: 0,
  };
}

function barrier(id: string, status = 'cancelled') {
  return {
    id,
    symbol: 'BTC-USDT',
    side: 'sell' as const,
    quantity: 1,
    remaining_quantity: status === 'armed' ? 1 : 0,
    filled_quantity: 0,
    trigger_price: 110,
    trigger_direction: 'above' as const,
    created_at: Number(id.replace(/\D/g, '')) || 1,
    status,
  };
}

function createModel() {
  const connection = {
    openBook: (key: string, symbol: string) => ({
      key,
      symbol,
      data: new BehaviorSubject<unknown | undefined>(undefined),
      error: new BehaviorSubject<string | undefined>(undefined),
    }),
    closeBook: () => undefined,
  } as unknown as Connection;
  return new OrderManagerModel({ connection, symbols: ['BTC-USDT'] });
}

describe('OrderManagerModel', () => {
  it('selects the first available symbol and updates selection', () => {
    const connection = {
      openBook: (key: string, symbol: string) => ({
        key,
        symbol,
        data: new BehaviorSubject<unknown | undefined>(undefined),
        error: new BehaviorSubject<string | undefined>(undefined),
      }),
      closeBook: () => undefined,
    } as unknown as Connection;
    const model = new OrderManagerModel({
      connection,
      symbols: ['BTC-USDT', 'ETH-USDT'],
    });
    assert.equal(model.state$.value.selectedSymbol, 'BTC-USDT');
    assert.equal(model.state$.value.orderType, 'ioc');
    model.setSymbol('ETH-USDT');
    assert.equal(model.state$.value.selectedSymbol, 'ETH-USDT');
    model.stop();
  });

  it('loads a bounded first page and incrementally merges older order and barrier pages', async () => {
    const originalFetch = globalThis.fetch;
    const requests: string[] = [];
    globalThis.fetch = (async (input) => {
      const url = String(input);
      requests.push(url);
      if (url.endsWith('limit=50'))
        return response({
          orders: [order('order-3')],
          barrier_orders: [barrier('barrier-3')],
          active_orders: [order('order-3', 1), order('active-1', 1)],
          active_barrier_orders: [barrier('active-barrier', 'armed')],
          next_orders_cursor: 'orders-next',
          has_more_orders: true,
          next_barrier_orders_cursor: 'barriers-next',
          has_more_barrier_orders: true,
        });
      return response({
        orders: [order('order-2'), order('order-1')],
        barrier_orders: [barrier('barrier-2'), barrier('barrier-1')],
        active_orders: [order('active-1', 1), order('active-2', 1)],
        active_barrier_orders: [barrier('active-barrier', 'triggering')],
        has_more_orders: false,
        has_more_barrier_orders: false,
      });
    }) as typeof fetch;
    const model = createModel();
    try {
      await model.refreshOrders();
      assert.match(requests[0], /\/positions\/orders\/history\?limit=50$/);
      assert.deepEqual(
        model.state$.value.orders.map((item) => item.id),
        ['order-3', 'active-1'],
      );
      assert.deepEqual(
        model.state$.value.barrierOrders.map((item) => item.id),
        ['barrier-3', 'active-barrier'],
      );
      assert.equal(model.state$.value.hasMoreOrdersHistory, true);
      assert.equal(model.state$.value.hasMoreBarrierHistory, true);

      await model.loadMoreHistory();
      const next = new URL(requests[1], 'http://localhost');
      assert.equal(next.searchParams.get('limit'), '50');
      assert.equal(next.searchParams.get('orders_cursor'), 'orders-next');
      assert.equal(
        next.searchParams.get('barrier_orders_cursor'),
        'barriers-next',
      );
      assert.deepEqual(
        model.state$.value.orders.map((item) => item.id),
        ['order-3', 'order-2', 'order-1', 'active-1', 'active-2'],
      );
      assert.equal(
        model.state$.value.orders.find((item) => item.id === 'active-1')
          ?.remaining_quantity,
        1,
      );
      assert.deepEqual(
        model.state$.value.barrierOrders.map((item) => item.id),
        ['barrier-3', 'barrier-2', 'barrier-1', 'active-barrier'],
      );
      assert.equal(
        model.state$.value.barrierOrders.find(
          (item) => item.id === 'active-barrier',
        )?.status,
        'triggering',
      );
      assert.equal(model.state$.value.hasMoreOrdersHistory, false);
      assert.equal(model.state$.value.hasMoreBarrierHistory, false);
    } finally {
      model.stop();
      globalThis.fetch = originalFetch;
    }
  });

  it('keeps loaded history while stream snapshots update active and loaded rows', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () =>
      response({
        orders: [order('order-2')],
        barrier_orders: [barrier('barrier-2')],
        active_orders: [order('order-2', 1), order('stream-active', 1)],
        active_barrier_orders: [
          barrier('barrier-2', 'armed'),
          barrier('stream-barrier', 'armed'),
        ],
        next_orders_cursor: 'orders-next',
        has_more_orders: true,
        next_barrier_orders_cursor: 'barriers-next',
        has_more_barrier_orders: true,
      })) as typeof fetch;
    const model = createModel();
    try {
      await model.refreshOrders();
      (model as any).applyStreamOrders(
        [
          order('order-2', 0),
          order('stream-active', 0),
          order('new-active', 1),
          order('unloaded-old'),
        ],
        [
          barrier('barrier-2', 'triggered'),
          barrier('stream-barrier', 'triggered'),
          barrier('new-active-barrier', 'armed'),
          barrier('unloaded-old-barrier', 'cancelled'),
        ],
      );
      assert.deepEqual(
        model.state$.value.orders.map((item) => item.id),
        ['order-2', 'stream-active', 'new-active'],
      );
      assert.equal(model.state$.value.orders[0]?.remaining_quantity, 0);
      assert.equal(model.state$.value.orders[1]?.remaining_quantity, 0);
      assert.deepEqual(
        model.state$.value.barrierOrders.map((item) => item.id),
        ['barrier-2', 'stream-barrier', 'new-active-barrier'],
      );
      assert.equal(model.state$.value.barrierOrders[0]?.status, 'triggered');
      assert.equal(model.state$.value.barrierOrders[1]?.status, 'triggered');
    } finally {
      model.stop();
      globalThis.fetch = originalFetch;
    }
  });

  it('preserves older pages when a post-submit refresh updates the first page', async () => {
    const originalFetch = globalThis.fetch;
    let requestCount = 0;
    globalThis.fetch = (async () => {
      requestCount += 1;
      if (requestCount === 1)
        return response({
          orders: [order('order-3')],
          barrier_orders: [],
          active_orders: [],
          active_barrier_orders: [],
          next_orders_cursor: 'orders-next',
          has_more_orders: true,
          has_more_barrier_orders: false,
        });
      if (requestCount === 2)
        return response({
          orders: [order('order-2')],
          barrier_orders: [],
          active_orders: [],
          active_barrier_orders: [],
          next_orders_cursor: 'orders-oldest',
          has_more_orders: true,
          has_more_barrier_orders: false,
        });
      return response({
        orders: [order('new-order'), order('order-3', 1)],
        barrier_orders: [],
        active_orders: [order('new-order', 1)],
        active_barrier_orders: [],
        next_orders_cursor: 'orders-next',
        has_more_orders: true,
        has_more_barrier_orders: false,
      });
    }) as typeof fetch;
    const model = createModel();
    try {
      await model.refreshOrders();
      await model.loadMoreHistory();
      await model.refreshOrders(false);
      assert.deepEqual(
        model.state$.value.orders.map((item) => item.id),
        ['new-order', 'order-3', 'order-2'],
      );
    } finally {
      model.stop();
      globalThis.fetch = originalFetch;
    }
  });

  it('rejects a has-more response without a cursor instead of refetching the first page', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () =>
      response({
        orders: [order('order-1')],
        barrier_orders: [],
        active_orders: [],
        active_barrier_orders: [],
        has_more_orders: true,
        has_more_barrier_orders: false,
      })) as typeof fetch;
    const model = createModel();
    try {
      await model.refreshOrders();
      assert.match(model.state$.value.error ?? '', /invalid history cursor/);
      assert.equal(model.state$.value.hasMoreOrdersHistory, false);
    } finally {
      model.stop();
      globalThis.fetch = originalFetch;
    }
  });
});
