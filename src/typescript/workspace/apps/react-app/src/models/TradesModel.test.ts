import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { TradesModel } from './TradesModel';
import type { Connection, TradeFeed } from '../connection';

function createMockConnection() {
  const openCalls: Array<{ key: string; symbol: string }> = [];
  const closeCalls: Array<{ key: string; feed: TradeFeed }> = [];
  const feeds = new Map<string, TradeFeed>();

  const connection = {
    openTrades(key: string, symbol: string): TradeFeed {
      openCalls.push({ key, symbol });
      let feed = feeds.get(key);
      if (!feed || feed.symbol !== symbol) {
        feed = {
          key,
          symbol,
          data: new BehaviorSubject<unknown | undefined>(undefined),
          error: new BehaviorSubject<string | undefined>(undefined),
        };
        feeds.set(key, feed);
      }
      return feed;
    },
    closeTrades(key: string, feed: TradeFeed) {
      closeCalls.push({ key, feed });
      feeds.delete(key);
    },
    openCalls,
    closeCalls,
    feeds,
  } as unknown as Connection & {
    openCalls: Array<{ key: string; symbol: string }>;
    closeCalls: Array<{ key: string; feed: TradeFeed }>;
    feeds: Map<string, TradeFeed>;
  };

  return connection;
}

function makeTradeBuffer(params: {
  symbol: string;
  tradeId: string;
  timestamp: number;
  price: number;
  size: number;
  side: 'buy' | 'sell';
}): Uint8Array {
  const buffer = new Uint8Array(97);
  buffer.fill(32, 8, 72); // space-pad symbol and tradeId
  const view = new DataView(buffer.buffer);
  view.setUint16(0, 89, true); // blockLength
  view.setUint16(2, 3, true);  // templateId = TRADE_TICK
  view.setUint16(4, 1, true);  // schemaId = SYMBOL_PRICE_SCHEMA_ID
  view.setUint16(6, 0, true);  // version

  const encoder = new TextEncoder();
  buffer.set(encoder.encode(params.symbol).subarray(0, 32), 8);
  buffer.set(encoder.encode(params.tradeId).subarray(0, 32), 40);

  view.setBigUint64(72, BigInt(params.timestamp), true);
  view.setFloat64(80, params.price, true);
  view.setFloat64(88, params.size, true);
  view.setUint8(96, params.side === 'buy' ? 1 : 2);

  return buffer;
}

describe('TradesModel', () => {
  it('initializes with the given symbol and an empty trades array', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    const state = model.state$.value;
    assert.strictEqual(state.symbol, 'BTC-USDT');
    assert.deepStrictEqual(state.trades, []);
    assert.strictEqual(state.error, undefined);
    assert.strictEqual(connection.openCalls.length, 0);
  });

  it('opens a trade feed when started', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();

    assert.strictEqual(connection.openCalls.length, 1);
    assert.deepStrictEqual(connection.openCalls[0], {
      key: 'widget-1',
      symbol: 'BTC-USDT',
    });

    // Idempotent start
    model.start();
    assert.strictEqual(connection.openCalls.length, 1);

    model.stop();
  });

  it('receives trade ticks, decodes them, and prepends in reverse chronological order', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    const now = Date.now();
    const trade1 = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-101',
      timestamp: now - 2000,
      price: 65000.5,
      size: 1.25,
      side: 'buy',
    });

    const trade2 = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-102',
      timestamp: now - 1000,
      price: 65010.0,
      size: 0.75,
      side: 'sell',
    });

    feed.data.next(trade1);

    assert.strictEqual(model.state$.value.trades.length, 1);
    assert.deepStrictEqual(model.state$.value.trades[0], {
      symbol: 'BTC-USDT',
      tradeId: 't-101',
      timestamp: now - 2000,
      price: 65000.5,
      size: 1.25,
      side: 'buy',
    });

    feed.data.next(trade2);

    assert.strictEqual(model.state$.value.trades.length, 2);
    // trade2 prepended to the front
    assert.strictEqual(model.state$.value.trades[0].tradeId, 't-102');
    assert.strictEqual(model.state$.value.trades[1].tradeId, 't-101');

    model.stop();
  });

  it('ignores invalid trade data or trades for a different symbol', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    // Non-buffer or malformed payloads
    feed.data.next(undefined);
    feed.data.next('invalid');
    feed.data.next(new Uint8Array(10));
    assert.strictEqual(model.state$.value.trades.length, 0);

    // Trade for another symbol
    const otherSymbolTrade = makeTradeBuffer({
      symbol: 'ETH-USDT',
      tradeId: 't-eth-1',
      timestamp: Date.now(),
      price: 3500.0,
      size: 5.0,
      side: 'buy',
    });
    feed.data.next(otherSymbolTrade);
    assert.strictEqual(model.state$.value.trades.length, 0);

    model.stop();
  });

  it('discards trades older than the 5-minute retention cutoff', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    const now = Date.now();
    const fiveMinutesOneSecondAgo = now - 5 * 60 * 1000 - 1000;
    const oldTrade = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-old',
      timestamp: fiveMinutesOneSecondAgo,
      price: 64000.0,
      size: 2.0,
      side: 'buy',
    });

    feed.data.next(oldTrade);
    assert.strictEqual(model.state$.value.trades.length, 0);

    model.stop();
  });

  it('updates state error when the feed emits an error', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    feed.error.next('Connection timeout to OKX trade channel');
    assert.strictEqual(
      model.state$.value.error,
      'Connection timeout to OKX trade channel',
    );
    assert.strictEqual(model.state$.value.symbol, 'BTC-USDT');

    // Falsy errors should not overwrite existing state
    feed.error.next(undefined);
    assert.strictEqual(
      model.state$.value.error,
      'Connection timeout to OKX trade channel',
    );

    model.stop();
  });

  it('purges expired trades when the retention timer ticks', (t) => {
    t.mock.timers.enable({ apis: ['setInterval', 'Date'] });

    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    const now = Date.now();
    const recentTrade = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-recent',
      timestamp: now,
      price: 65000.0,
      size: 1.0,
      side: 'buy',
    });

    feed.data.next(recentTrade);
    assert.strictEqual(model.state$.value.trades.length, 1);

    // Fast-forward 2 minutes (trades still within 5 min retention)
    t.mock.timers.tick(2 * 60 * 1000);
    assert.strictEqual(model.state$.value.trades.length, 1);

    // Fast-forward past 5 minutes total (3.5 minutes more = 5.5 minutes)
    t.mock.timers.tick(3.5 * 60 * 1000);
    assert.strictEqual(model.state$.value.trades.length, 0);

    model.stop();
    t.mock.timers.reset();
  });

  it('handles updateProps correctly when symbol changes', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed1 = connection.feeds.get('widget-1')!;

    const trade = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-btc',
      timestamp: Date.now(),
      price: 65000.0,
      size: 1.0,
      side: 'buy',
    });
    feed1.data.next(trade);
    assert.strictEqual(model.state$.value.trades.length, 1);

    // Switch symbol to SOL-USDT
    model.updateProps({
      symbol: 'SOL-USDT',
      connection,
      streamKey: 'widget-1',
    });

    // Previous feed closed, trades cleared, new feed started
    assert.strictEqual(connection.closeCalls.length, 1);
    assert.strictEqual(connection.closeCalls[0].key, 'widget-1');
    assert.strictEqual(connection.openCalls.length, 2);
    assert.strictEqual(connection.openCalls[1].symbol, 'SOL-USDT');

    assert.strictEqual(model.state$.value.symbol, 'SOL-USDT');
    assert.strictEqual(model.state$.value.trades.length, 0);

    // New feed receives SOL trade
    const feed2 = connection.feeds.get('widget-1')!;
    const solTrade = makeTradeBuffer({
      symbol: 'SOL-USDT',
      tradeId: 't-sol',
      timestamp: Date.now(),
      price: 150.0,
      size: 10.0,
      side: 'buy',
    });
    feed2.data.next(solTrade);
    assert.strictEqual(model.state$.value.trades.length, 1);
    assert.strictEqual(model.state$.value.trades[0].tradeId, 't-sol');

    model.stop();
  });

  it('ignores updateProps if props have not changed', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    assert.strictEqual(connection.openCalls.length, 1);

    model.updateProps({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    // Did not close or re-open
    assert.strictEqual(connection.openCalls.length, 1);
    assert.strictEqual(connection.closeCalls.length, 0);

    model.stop();
  });

  it('unsubscribes and cleans up feeds on stop', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    const feed = connection.feeds.get('widget-1')!;

    model.stop();

    assert.strictEqual(connection.closeCalls.length, 1);
    assert.strictEqual(connection.closeCalls[0].feed, feed);

    // Further emissions on old feed are ignored after stop
    const trade = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-late',
      timestamp: Date.now(),
      price: 65000.0,
      size: 1.0,
      side: 'buy',
    });
    feed.data.next(trade);
    assert.strictEqual(model.state$.value.trades.length, 0);

    // Calling stop() repeatedly is safe
    model.stop();
    assert.strictEqual(connection.closeCalls.length, 1);
  });

  it('can restart after being stopped', () => {
    const connection = createMockConnection();
    const model = new TradesModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'widget-1',
    });

    model.start();
    model.stop();

    assert.strictEqual(connection.openCalls.length, 1);
    assert.strictEqual(connection.closeCalls.length, 1);

    model.start();
    assert.strictEqual(connection.openCalls.length, 2);

    const feed = connection.feeds.get('widget-1')!;
    const trade = makeTradeBuffer({
      symbol: 'BTC-USDT',
      tradeId: 't-restarted',
      timestamp: Date.now(),
      price: 65000.0,
      size: 1.0,
      side: 'buy',
    });
    feed.data.next(trade);
    assert.strictEqual(model.state$.value.trades.length, 1);
    assert.strictEqual(model.state$.value.trades[0].tradeId, 't-restarted');

    model.stop();
  });
});
