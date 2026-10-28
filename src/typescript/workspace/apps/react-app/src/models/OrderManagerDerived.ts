import { getLimitPriceMarkWarning } from './limit_price_guard';
import type { OrderManagerState } from './OrderManagerTypes';

type BookSide = { price: number }[] | undefined;

function midpoint(bids: BookSide, asks: BookSide): number | undefined {
  if (bids?.[0] && asks?.[0]) return (bids[0].price + asks[0].price) / 2;
  return bids?.[0]?.price ?? asks?.[0]?.price;
}

export function deriveOrderManagerState(state: OrderManagerState): OrderManagerState {
  const rows = state.orders.map((order) => ({
    order,
    current: midpoint(state.books[order.symbol]?.bids, state.books[order.symbol]?.asks),
    bookError: state.bookErrors[order.symbol],
  }));
  const selectedBook = state.books[state.selectedSymbol];
  const selectedCurrentPrice = midpoint(selectedBook?.bids, selectedBook?.asks);
  const quantity = Number(state.quantity);
  const limitPrice = Number(state.limitPrice);
  const feePrice = state.orderType === 'limit' && limitPrice > 0 ? limitPrice : selectedCurrentPrice;
  const feeRate = state.orderType === 'limit' ? state.makerFeeRate : state.takerFeeRate;
  const estimatedFee = Number.isFinite(quantity) && quantity > 0 && feePrice !== undefined
    ? quantity * feePrice * feeRate
    : undefined;
  const limitPriceWarning = state.orderType === 'limit'
    ? getLimitPriceMarkWarning(state.limitPrice, selectedCurrentPrice)
    : undefined;

  return {
    ...state,
    rows,
    selectedCurrentPrice,
    estimatedFee,
    limitPriceWarning,
    confirmedLimitPrice: limitPriceWarning ? state.confirmedLimitPrice : undefined,
  };
}
