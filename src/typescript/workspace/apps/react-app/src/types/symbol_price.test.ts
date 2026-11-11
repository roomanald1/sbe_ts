import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { decodeSymbolPriceBatch } from './symbol_price';

function priceMessage(enriched: boolean, symbol = 'BTC-USDT'): Uint8Array {
  const bytes = new Uint8Array(enriched ? 72 : 56);
  const view = new DataView(bytes.buffer);
  view.setUint16(0, enriched ? 64 : 48, true);
  view.setUint16(2, enriched ? 4 : 1, true);
  view.setUint16(4, 1, true);
  view.setUint16(6, 9, true);
  new TextEncoder().encodeInto(symbol, bytes.subarray(8, 40));
  view.setFloat64(40, 100.5, true);
  view.setBigUint64(48, 1234n, true);
  if (enriched) {
    view.setFloat64(56, 0.2, true);
    view.setFloat64(64, 0.4, true);
  }
  return bytes;
}

describe('SymbolPrice SBE decoding', () => {
  it('decodes Insights-enriched tickers with server volatility values', () => {
    const [tick] = decodeSymbolPriceBatch(priceMessage(true));
    assert.deepEqual(tick, {
      symbol: 'BTC-USDT',
      price: 100.5,
      direction: 'up',
      volatility1m: 0.2,
      volatility5m: 0.4,
      receivedAt: 1234,
    });
  });

  it('decodes raw FeedHandlers prices without inventing volatility', () => {
    const [tick] = decodeSymbolPriceBatch(priceMessage(false));
    assert.equal(tick?.price, 100.5);
    assert.equal(tick?.receivedAt, 1234);
    assert.equal(tick?.volatility1m, undefined);
    assert.equal(tick?.volatility5m, undefined);
  });

  it('decodes batches from the Insights stream', () => {
    const first = priceMessage(true, 'BTC-USDT');
    const second = priceMessage(true, 'ETH-USDT');
    const batch = new Uint8Array(first.length + second.length);
    batch.set(first);
    batch.set(second, first.length);
    assert.deepEqual(
      decodeSymbolPriceBatch(batch).map((tick) => tick.symbol),
      ['BTC-USDT', 'ETH-USDT'],
    );
  });

  it('discriminates raw and enriched messages in a mixed batch', () => {
    const raw = priceMessage(false, 'BTC-USDT');
    const enriched = priceMessage(true, 'ETH-USDT');
    const batch = new Uint8Array(raw.length + enriched.length);
    batch.set(raw);
    batch.set(enriched, raw.length);
    const ticks = decodeSymbolPriceBatch(batch);
    assert.equal(ticks.length, 2);
    assert.equal(ticks[0].volatility1m, undefined);
    assert.equal(ticks[1].volatility1m, 0.2);
  });
});
