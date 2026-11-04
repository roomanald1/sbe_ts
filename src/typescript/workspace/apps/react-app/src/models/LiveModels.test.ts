import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { AutoTraderShadowModel } from './AutoTraderShadowModel';
import { CloseInsightsModel } from './CloseInsightsModel';
import { OrderBookModel } from './OrderBookModel';
import { OrderManagerBookFeeds } from './OrderManagerBookFeeds';
import { SignalsModel } from './SignalsModel';
import type { Connection } from '../connection';

async function withFetch(impl: typeof fetch, run: () => Promise<void>) {
  const original = globalThis.fetch;
  globalThis.fetch = impl;
  try { await run(); } finally { globalThis.fetch = original; }
}
function jsonResponse(body: unknown) { return { ok: true, json: async () => body, text: async () => '' } as Response; }

describe('AutoTraderShadowModel', () => {
  it('marks legacy live scouts without an entry price as outcome unavailable', async () => {
    const timestamp = Date.now();
    await withFetch(async (url) => String(url).includes('/auto-trader-audit')
      ? jsonResponse({ items: [{ decision_id: 'missing-entry', stage: 'decision', status: 'shadowed', timestamp, signal_timestamp: timestamp, symbol: 'ETH-USDT', direction: 'bearish', strategy: 'live-trade-flow', strength: 0.5, scout: true }] })
      : jsonResponse([]), async () => {
      const model = new AutoTraderShadowModel();
      model.start();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      assert.equal(model.state$.value.rows[0].status, 'Outcome unavailable');
      model.stop();
    });
  });

  it('matches five-minute evaluations to shadow decisions and derives outcomes', async () => {
    const timestamp = Date.now();
    await withFetch(async (url) => String(url).includes('/auto-trader-audit')
      ? jsonResponse({ items: [{ decision_id: 'd1', stage: 'decision', status: 'shadowed', timestamp, signal_timestamp: timestamp, symbol: 'BTC-USDT', direction: 'bullish', strategy: 'live-momentum', strength: 0.8, scout: true, shadow_entry_ask: 100, shadow_taker_fee_rate: 0.001 }] })
      : jsonResponse([{ symbol: 'BTC-USDT', signal_timestamp: timestamp, evaluated_timestamp: timestamp + 300_000, horizon_seconds: 300, signal_direction: 'bullish', long_net_percent: 0.5, taker_fee_rate: 0.002 }]), async () => {
      const model = new AutoTraderShadowModel();
      model.start();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      const row = model.state$.value.rows[0];
      assert.equal(row.decisionId, 'd1');
      assert.equal(row.status, 'Won');
      assert.equal(row.source, 'Scout');
      assert.equal(row.netReturnPercent, 0.5);
      assert.equal(row.takerFeeRate, 0.002);
      model.stop();
    });
  });
});

describe('CloseInsightsModel', () => {
  it('joins positions to insights, converts quote values, and filters flat positions', async () => {
    const model = new CloseInsightsModel();
    (model as any).livePositions = [
      { symbol: 'BTC-USDT', quantity: 2 },
      { symbol: 'ETH-USDT', quantity: 0 },
    ];
    (model as any).liveInsights = [
      { symbol: 'BTC-USDT', close_side: 'sell', close_quantity: 1, close_pnl: 3, close_fee: 0.5, depth_covers_position: true, close_depth_quantity: 2, probability_samples: 4 },
      { symbol: 'ETH-USDT', close_side: 'sell', close_quantity: 1, depth_covers_position: true, close_depth_quantity: 1, probability_samples: 1 },
    ];
    (model as any).usdRates = { USDT: 1.2 };
    await model.refresh();
    assert.equal(model.state$.value.loading, false);
    assert.equal(model.state$.value.insights.length, 1);
    assert.ok(Math.abs((model.state$.value.insights[0].closePnlUsd ?? 0) - 3.6) < 1e-9);
    assert.ok(Math.abs((model.state$.value.insights[0].closeFeeUsd ?? 0) - 0.6) < 1e-9);
    const insight = model.state$.value.insights[0];
    model.requestClose(insight);
    assert.equal(model.state$.value.confirmingInsight, insight);
    model.cancelClose();
    assert.equal(model.state$.value.confirmingInsight, undefined);
    model.stop();
  });

  it('requires positive close depth and rejects invalid profit targets locally', async () => {
    let requests = 0;
    await withFetch(async () => { requests++; return jsonResponse({}); }, async () => {
      const model = new CloseInsightsModel();
      const insight = { position: { symbol: 'BTC-USDT', quantity: 1 }, closeDepthQuantity: 0 } as any;
      model.requestClose(insight);
      assert.equal(model.state$.value.confirmingInsight, undefined);
      model.setBarrierTargetUsd('0');
      await model.createProfitBarrier({ ...insight, closeDepthQuantity: 1 });
      assert.match(model.state$.value.error ?? '', /greater than zero/);
      assert.equal(requests, 0);
      model.stop();
    });
  });
});

function createFeedConnection() {
  const openedBooks: Array<{ key: string; symbol: string; data: BehaviorSubject<unknown | undefined>; error: BehaviorSubject<string | undefined> }> = [];
  const openedSignals: Array<{ key: string; symbol: string; data: BehaviorSubject<unknown | undefined>; error: BehaviorSubject<string | undefined> }> = [];
  const closed: string[] = [];
  return {
    connection: {
      openBook: (key: string, symbol: string) => { const feed = { key, symbol, data: new BehaviorSubject<unknown | undefined>(undefined), error: new BehaviorSubject<string | undefined>(undefined) }; openedBooks.push(feed); return feed; },
      closeBook: (key: string) => { closed.push(key); },
      openSignals: (key: string, symbol: string) => { const feed = { key, symbol, data: new BehaviorSubject<unknown | undefined>(undefined), error: new BehaviorSubject<string | undefined>(undefined) }; openedSignals.push(feed); return feed; },
      closeSignals: (key: string) => { closed.push(key); },
    } as unknown as Connection,
    openedBooks, openedSignals, closed,
  };
}

describe('SignalsModel', () => {
  it('decodes matching JSON updates, reports malformed input and closes its feed', () => {
    const mock = createFeedConnection();
    const model = new SignalsModel({ connection: mock.connection, symbol: 'BTC-USDT', streamKey: 'signal-1' });
    model.start();
    const feed = mock.openedSignals[0];
    const signal = { symbol: 'BTC-USDT', timestamp: 1, direction: 'bullish', score: 0.5, strength: 0.7, components: {}, reasons: [], stale: false };
    feed.data.next(JSON.stringify(signal));
    assert.deepEqual(model.state$.value.snapshot, signal);
    feed.data.next('{bad json');
    assert.equal(model.state$.value.error, 'Received an invalid signal update.');
    feed.error.next('offline');
    assert.equal(model.state$.value.error, 'offline');
    model.stop();
    assert.deepEqual(mock.closed, ['signal-1']);
  });
});

describe('OrderBookModel', () => {
  it('handles feed errors and closes and reopens its subscription when frozen state toggles', () => {
    const mock = createFeedConnection();
    const model = new OrderBookModel({ connection: mock.connection, symbol: 'BTC-USDT', streamKey: 'book-1' });
    model.start();
    mock.openedBooks[0].error.next('disconnected');
    assert.equal(model.state$.value.error, 'disconnected');
    model.toggleFrozen();
    assert.equal(model.state$.value.frozen, true);
    assert.equal(mock.openedBooks.length, 2);
    model.toggleFrozen();
    assert.equal(model.state$.value.frozen, false);
    assert.equal(mock.openedBooks.length, 3);
    model.stop();
    assert.deepEqual(mock.closed, ['book-1', 'book-1', 'book-1']);
  });
});

describe('OrderManagerBookFeeds', () => {
  it('subscribes once per symbol, forwards errors, and clears removed symbols', () => {
    const mock = createFeedConnection();
    const errors: Array<[string, string | undefined]> = [];
    const books: Array<[string, unknown]> = [];
    const feeds = new OrderManagerBookFeeds(mock.connection, (symbol, depth) => books.push([symbol, depth]), (symbol, error) => errors.push([symbol, error]));
    feeds.sync(new Set(['BTC-USDT']));
    feeds.sync(new Set(['BTC-USDT']));
    assert.equal(mock.openedBooks.length, 1);
    mock.openedBooks[0].error.next('stale');
    assert.deepEqual(errors.at(-1), ['BTC-USDT', 'stale']);
    feeds.sync(new Set());
    assert.deepEqual(mock.closed, ['order-manager:BTC-USDT']);
    assert.deepEqual(books.at(-1), ['BTC-USDT', undefined]);
    feeds.close();
  });
});
