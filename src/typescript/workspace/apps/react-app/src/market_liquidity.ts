import type { BookDepth, BookLevel, TradeTick } from './types/symbol_price';

export type LiquidityBand = { percent: number; bidSize: number; askSize: number; bidNotional: number; askNotional: number; imbalance: number };
export type LiquiditySide = 'buy' | 'sell';
export type SlippageEstimate = { averagePrice?: number; slippagePrice?: number; slippageBps?: number; filledQuantity: number; unfilledQuantity: number };
export type LiquidityGap = { side: 'bid' | 'ask'; from: number; to: number; gapPercent: number };

export function getBookMid(book: BookDepth): number | undefined {
  const bid = book.bids[0]?.price, ask = book.asks[0]?.price;
  return bid && ask ? (bid + ask) / 2 : undefined;
}
export function getLiquidityBands(book: BookDepth): LiquidityBand[] {
  const mid = getBookMid(book);
  if (!mid) return [];
  return [0.1, 0.5, 1].map((percent) => {
    const bids = book.bids.filter((level) => level.price >= mid * (1 - percent / 100));
    const asks = book.asks.filter((level) => level.price <= mid * (1 + percent / 100));
    const bidSize = bids.reduce((sum, level) => sum + level.size, 0), askSize = asks.reduce((sum, level) => sum + level.size, 0), total = bidSize + askSize;
    return { percent, bidSize, askSize, bidNotional: bids.reduce((sum, level) => sum + level.price * level.size, 0), askNotional: asks.reduce((sum, level) => sum + level.price * level.size, 0), imbalance: total ? (bidSize - askSize) / total : 0 };
  });
}
export function estimateSlippage(book: BookDepth, side: LiquiditySide, quantity: number): SlippageEstimate {
  const mid = getBookMid(book), levels = side === 'buy' ? book.asks : book.bids;
  if (!mid || !Number.isFinite(quantity) || quantity <= 0) return { filledQuantity: 0, unfilledQuantity: Math.max(0, quantity || 0) };
  let remaining = quantity, filledQuantity = 0, notional = 0;
  for (const level of levels) { const fill = Math.min(remaining, level.size); filledQuantity += fill; notional += fill * level.price; remaining -= fill; if (remaining <= 0) break; }
  const averagePrice = filledQuantity ? notional / filledQuantity : undefined;
  const slippagePrice = averagePrice === undefined ? undefined : (side === 'buy' ? averagePrice - mid : mid - averagePrice);
  return { averagePrice, slippagePrice, slippageBps: slippagePrice === undefined ? undefined : slippagePrice / mid * 10000, filledQuantity, unfilledQuantity: remaining };
}
export function findLiquidityGaps(bids: BookLevel[], asks: BookLevel[]): LiquidityGap[] {
  const gaps: LiquidityGap[] = [];
  for (const [side, levels] of [['bid', bids], ['ask', asks]] as const) for (let i = 0; i + 1 < levels.length; i++) {
    const near = levels[i].price, far = levels[i + 1].price, gapPercent = Math.abs(near - far) / near * 100;
    if (gapPercent >= 0.1) gaps.push({ side, from: Math.min(near, far), to: Math.max(near, far), gapPercent });
  }
  return gaps;
}
export function isUnusualTrade(trade: TradeTick, recent: TradeTick[]): boolean {
  const notional = trade.price * trade.size, values = recent.filter((item) => item.tradeId !== trade.tradeId).slice(0, 100).map((item) => item.price * item.size).sort((a, b) => a - b);
  const median = values.length ? values[Math.floor(values.length / 2)] : 0;
  return notional >= 10000 || (values.length >= 10 && median > 0 && notional >= median * 5);
}
