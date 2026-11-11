import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  estimateSlippage,
  findLiquidityGaps,
  getBookMid,
  getLiquidityBands,
  isUnusualTrade,
} from '../market_liquidity';
import type { BookDepth, TradeTick } from '../types/symbol_price';

const book: BookDepth = {
  symbol: 'BTC-USDT',
  timestamp: 1,
  bids: [
    { price: 99.9, size: 2, cumulativeSize: 2 },
    { price: 99.4, size: 3, cumulativeSize: 5 },
    { price: 98, size: 5, cumulativeSize: 10 },
  ],
  asks: [
    { price: 100.1, size: 1, cumulativeSize: 1 },
    { price: 100.6, size: 2, cumulativeSize: 3 },
    { price: 102, size: 4, cumulativeSize: 7 },
  ],
};

const trade = (tradeId: string, price: number, size: number): TradeTick => ({
  symbol: 'BTC-USDT',
  tradeId,
  timestamp: 1,
  price,
  size,
  side: 'buy',
});

describe('market liquidity helpers', () => {
  it('calculates the midpoint and depth bands', () => {
    assert.equal(getBookMid(book), 100);
    assert.deepEqual(getLiquidityBands(book), [
      {
        percent: 0.1,
        bidSize: 2,
        askSize: 1,
        bidNotional: 199.8,
        askNotional: 100.1,
        imbalance: 1 / 3,
      },
      {
        percent: 0.5,
        bidSize: 2,
        askSize: 1,
        bidNotional: 199.8,
        askNotional: 100.1,
        imbalance: 1 / 3,
      },
      {
        percent: 1,
        bidSize: 5,
        askSize: 3,
        bidNotional: 498.00000000000006,
        askNotional: 301.29999999999995,
        imbalance: 0.25,
      },
    ]);
  });

  it('returns no midpoint or bands for empty and invalid top-of-book data', () => {
    const empty: BookDepth = {
      symbol: 'BTC-USDT',
      timestamp: 1,
      bids: [],
      asks: [],
    };
    assert.equal(getBookMid(empty), undefined);
    assert.deepEqual(getLiquidityBands(empty), []);
    assert.equal(
      getBookMid({ ...book, bids: [{ price: 0, size: 1, cumulativeSize: 1 }] }),
      undefined,
    );
  });

  it('estimates full and partial fills for both sides', () => {
    const buy = estimateSlippage(book, 'buy', 2);
    assert.equal(buy.filledQuantity, 2);
    assert.equal(buy.unfilledQuantity, 0);
    assert.equal(buy.averagePrice, 100.35);
    assert.ok(Math.abs((buy.slippageBps ?? 0) - 35) < 1e-9);

    const sell = estimateSlippage(book, 'sell', 3);
    assert.equal(sell.filledQuantity, 3);
    assert.equal(sell.unfilledQuantity, 0);
    assert.ok(
      Math.abs((sell.averagePrice ?? 0) - (2 * 99.9 + 99.4) / 3) < 1e-12,
    );

    const partial = estimateSlippage(book, 'buy', 8);
    assert.equal(partial.filledQuantity, 7);
    assert.equal(partial.unfilledQuantity, 1);
    assert.ok(
      Math.abs(
        (partial.averagePrice ?? 0) - (100.1 + 2 * 100.6 + 4 * 102) / 7,
      ) < 1e-12,
    );
  });

  it('handles empty books and invalid requested quantities safely', () => {
    const empty: BookDepth = {
      symbol: 'BTC-USDT',
      timestamp: 1,
      bids: [],
      asks: [],
    };
    assert.deepEqual(estimateSlippage(empty, 'buy', 3), {
      filledQuantity: 0,
      unfilledQuantity: 3,
    });
    assert.deepEqual(estimateSlippage(book, 'buy', 0), {
      filledQuantity: 0,
      unfilledQuantity: 0,
    });
    assert.equal(estimateSlippage(book, 'sell', Number.NaN).filledQuantity, 0);
  });

  it('detects bid and ask price gaps and ignores smaller gaps', () => {
    const gaps = findLiquidityGaps(
      [
        { price: 100, size: 1, cumulativeSize: 1 },
        { price: 99.95, size: 1, cumulativeSize: 2 },
        { price: 99, size: 1, cumulativeSize: 3 },
      ],
      [
        { price: 100, size: 1, cumulativeSize: 1 },
        { price: 100.05, size: 1, cumulativeSize: 2 },
        { price: 101, size: 1, cumulativeSize: 3 },
      ],
    );
    assert.deepEqual(
      gaps.map(({ side, from, to }) => ({ side, from, to })),
      [
        { side: 'bid', from: 99, to: 99.95 },
        { side: 'ask', from: 100.05, to: 101 },
      ],
    );
    assert.ok(Math.abs(gaps[0].gapPercent - (0.95 / 99.95) * 100) < 1e-8);
    assert.ok(Math.abs(gaps[1].gapPercent - (0.95 / 100.05) * 100) < 1e-8);
    assert.deepEqual(findLiquidityGaps([], []), []);
  });

  it('flags large trades against absolute and recent-median thresholds', () => {
    assert.equal(isUnusualTrade(trade('large', 100, 100), []), true);
    const recent = Array.from({ length: 10 }, (_, index) =>
      trade(String(index), 100, 1),
    );
    assert.equal(isUnusualTrade(trade('same', 100, 1), recent), false);
    assert.equal(isUnusualTrade(trade('outlier', 100, 6), recent), true);
    assert.equal(isUnusualTrade(trade('ordinary', 100, 1), []), false);
    assert.equal(
      isUnusualTrade(trade('invalid', Number.NaN, 1), recent),
      false,
    );
  });
});
