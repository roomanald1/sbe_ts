import { getLimitPriceMarkWarning } from './limit_price_guard';
import type { TradeTicketState } from './TradeTicketModel';

type TradeTicketUpdate = Partial<TradeTicketState>;

export function deriveTradeTicketState(state: TradeTicketState, update: TradeTicketUpdate): TradeTicketState {
const next = { ...state, ...update };
  const book = next.book;
  next.currentPrice =
    book?.bids[0] && book.asks[0]
      ? (book.bids[0].price + book.asks[0].price) / 2
      : (book?.bids[0]?.price ?? book?.asks[0]?.price);
  next.bestBid = book?.bids[0]?.price;
  next.bestAsk = book?.asks[0]?.price;
  next.spread = next.bestBid !== undefined && next.bestAsk !== undefined
    ? next.bestAsk - next.bestBid
    : undefined;
  const requestedQuantity = Number(next.quantity);
  const validQuantity = Number.isFinite(requestedQuantity) && requestedQuantity > 0;
  next.estimatedNotional = validQuantity && next.currentPrice !== undefined
    ? requestedQuantity * next.currentPrice
    : undefined;
  const levels = next.side === 'buy' ? book?.asks : book?.bids;
  let remaining = validQuantity ? requestedQuantity : 0;
  let filled = 0;
  let cost = 0;
  for (const level of levels ?? []) {
    if (next.orderType === 'limit') {
      const crossesLimit = next.side === 'buy'
        ? level.price <= Number(next.limitPrice)
        : level.price >= Number(next.limitPrice);
      if (!crossesLimit) continue;
    }
    const levelFill = Math.min(remaining, level.size);
    filled += levelFill;
    cost += levelFill * level.price;
    remaining -= levelFill;
    if (remaining <= 0) break;
  }
  next.estimatedFillQuantity = validQuantity && levels?.length ? filled : undefined;
  next.estimatedFillAverage = filled > 0 ? cost / filled : undefined;
  next.estimatedUnfilledQuantity = validQuantity && levels?.length ? remaining : undefined;
  next.limitPriceWarning =
    next.orderType === 'limit'
      ? getLimitPriceMarkWarning(next.limitPrice, next.currentPrice)
      : undefined;
  if (!next.limitPriceWarning) next.confirmedLimitPrice = undefined;
  const makerOrder = ['limit', 'stop-limit', 'post-only-limit', 'gtd'].includes(next.orderType);
  const feeRate = makerOrder ? next.makerFeeRate : next.takerFeeRate;
  const feePrice = makerOrder && Number(next.limitPrice) > 0 ? Number(next.limitPrice) : (next.estimatedFillAverage ?? next.currentPrice);
  const feeQuantity = next.orderType === "ioc" && next.estimatedFillQuantity !== undefined ? next.estimatedFillQuantity : (next.orderType === "ioc" && next.book ? 0 : requestedQuantity);
  next.estimatedFee = feePrice !== undefined && validQuantity ? feeQuantity * feePrice * feeRate : undefined;
return next;
}
