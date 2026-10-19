import { alignsToStep, getInstrumentPrecision } from '../instrument_precision';
import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
import { getLimitPriceMarkWarning } from './limit_price_guard';

type Side = 'buy' | 'sell';
type OrderType = 'market' | 'ioc' | 'limit' | 'vwap' | 'twap' | 'barrier' | 'stop-market' | 'stop-limit' | 'trailing-stop' | 'post-only-limit' | 'fok' | 'gtd';
type BarrierDirection = 'above' | 'below';
type TrailingMode = 'amount' | 'percent';
type Props = { symbol: string; initialSide: Side; connection: Connection };
export type TradeTicketState = {
  symbol: string;
  book?: BookDepth;
  bookError: string;
  side: Side;
  orderType: OrderType;
  quantity: string;
  limitPrice: string;
  barrierPrice: string;
  barrierDirection: BarrierDirection;
  trailingValue: string;
  trailingMode: TrailingMode;
  expiresAtLocal: string;
  busy: boolean;
  message: string;
  currentPrice?: number;
  bestBid?: number;
  bestAsk?: number;
  spread?: number;
  estimatedNotional?: number;
  estimatedFee?: number;
  makerFeeRate: number;
  takerFeeRate: number;
  estimatedFillQuantity?: number;
  estimatedFillAverage?: number;
  estimatedUnfilledQuantity?: number;
  limitPriceWarning?: string;
  confirmedLimitPrice?: string;
  durationMinutes: string;
  participationRate: string;
  vwapRunId?: string;
  vwapStatus?: string;
  vwapExecutedQuantity?: number;
};
export class TradeTicketModel {
  readonly props$: BehaviorSubject<Props>;
  readonly state$: BehaviorSubject<TradeTicketState>;
  private feed?: BookFeed;
  private dataSub?: { unsubscribe(): void };
  private errorSub?: { unsubscribe(): void };
  private vwapTimer?: ReturnType<typeof setInterval>;
  constructor(props: Props) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject<TradeTicketState>({
      symbol: props.symbol,
      side: props.initialSide,
      orderType: 'ioc' as OrderType,
      quantity: '',
      limitPrice: '',
      barrierPrice: '',
      barrierDirection: props.initialSide === 'sell' ? 'above' : 'below',
      trailingValue: '', trailingMode: 'percent', expiresAtLocal: '',
      busy: false,
      bookError: '',
      message: '',
      confirmedLimitPrice: undefined,
      durationMinutes: '60',
      participationRate: '10',
      makerFeeRate: 0.0008,
      takerFeeRate: 0.001,
    });
  }
  readonly getSnapshot = () => this.state$.value;
  readonly subscribe = (notify: () => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  updateProps(props: Props): void {
    const previous = this.props$.value;
    this.props$.next(props);
    if (
      previous.connection !== props.connection ||
      previous.symbol !== props.symbol
    ) {
      this.stop();
      this.update({
        symbol: props.symbol,
        book: undefined,
        bookError: '',
        side: props.initialSide,
        confirmedLimitPrice: undefined,
      });
      this.start();
    }
  }
  start(): void {
    if (this.feed) return;
    const { connection, symbol } = this.props$.value;
    void this.loadFeeRates();
    void this.loadActiveVwap(symbol);
    const key = 'order-ticket:' + symbol;
    const feed = connection.openBook(key, symbol);
    this.feed = feed;
    this.dataSub = feed.data.subscribe((raw) => {
      if (raw === undefined) {
        this.update({ book: undefined, currentPrice: undefined });
        return;
      }
      const book = decodeBookDepth(raw);
      if (book?.symbol === symbol) this.update({ book, bookError: '' });
    });
    this.errorSub = feed.error.subscribe((bookError) => {
      if (bookError) this.update({ bookError });
    });
  }
  stop(): void {
    if (this.vwapTimer) clearInterval(this.vwapTimer);
    this.vwapTimer = undefined;
    const feed = this.feed;
    if (!feed) return;
    this.dataSub?.unsubscribe();
    this.errorSub?.unsubscribe();
    this.props$.value.connection.closeBook(feed.key, feed);
    this.feed = undefined;
    this.dataSub = undefined;
    this.errorSub = undefined;
  }
  setSide(side: Side): void {
    this.update({ side, barrierDirection: side === 'sell' ? 'above' : 'below' });
  }
  setOrderType(orderType: OrderType): void {
    this.update({ orderType, confirmedLimitPrice: undefined });
  }
  setQuantity(quantity: string): void {
    this.update({ quantity });
  }
  setDurationMinutes(durationMinutes: string): void {
    this.update({ durationMinutes });
  }
  setParticipationRate(participationRate: string): void {
    this.update({ participationRate });
  }
  setLimitPrice(limitPrice: string): void {
    this.update({ limitPrice, confirmedLimitPrice: undefined, message: '' });
  }
  setBarrierPrice(barrierPrice: string): void {
    this.update({ barrierPrice, message: '' });
  }
  setBarrierDirection(barrierDirection: BarrierDirection): void { this.update({ barrierDirection }); }
  setTrailingValue(trailingValue: string): void { this.update({ trailingValue, message: '' }); }
  setTrailingMode(trailingMode: TrailingMode): void { this.update({ trailingMode }); }
  setExpiresAtLocal(expiresAtLocal: string): void { this.update({ expiresAtLocal, message: '' }); }
  private async loadFeeRates(): Promise<void> {
    try {
      const response = await fetch("/positions/fee-rates");
      if (!response.ok) return;
      const rates = await response.json() as { maker: number; taker: number };
      if (Number.isFinite(rates.maker) && Number.isFinite(rates.taker)) this.update({ makerFeeRate: rates.maker, takerFeeRate: rates.taker });
    } catch { /* use displayed paper fee defaults until rates load */ }
  }
  async submit(): Promise<void> {
    const state = this.state$.value;
    const quantity = Number(state.quantity);
    const limitPrice = Number(state.limitPrice);
    const barrierPrice = Number(state.barrierPrice);
    const durationMinutes = Number(state.durationMinutes);
    const trailingValue = Number(state.trailingValue);
    const expiresAt = state.expiresAtLocal ? new Date(state.expiresAtLocal).getTime() : undefined;
    const hasLimit = ['limit', 'stop-limit', 'post-only-limit', 'gtd'].includes(state.orderType);
    const hasTrigger = ['barrier', 'stop-market', 'stop-limit'].includes(state.orderType);
    const participationRate = Number(state.participationRate) / 100;
    const instrument = await getInstrumentPrecision(state.symbol);
    if (instrument && (!alignsToStep(quantity, instrument.lotSz) || (hasLimit && !alignsToStep(limitPrice, instrument.tickSz)) || (hasTrigger && !alignsToStep(barrierPrice, instrument.tickSz)))) {
      this.update({ message: 'Quantity or price does not match this instrument tick or lot size.' }); return;
    }
    if (
      !Number.isFinite(quantity) ||
      quantity <= 0 ||
      (hasLimit && (!Number.isFinite(limitPrice) || limitPrice <= 0)) ||
      (hasTrigger && (!Number.isFinite(barrierPrice) || barrierPrice <= 0)) ||
      (state.orderType === 'trailing-stop' && (!Number.isFinite(trailingValue) || trailingValue <= 0 || (state.trailingMode === 'percent' && trailingValue >= 100))) ||
      (state.orderType === 'gtd' && (!expiresAt || expiresAt <= Date.now())) ||
      ((state.orderType === 'vwap' || state.orderType === 'twap') && (!Number.isInteger(durationMinutes) || durationMinutes < 5 || durationMinutes > 390)) ||
      (state.orderType === 'vwap' && (!Number.isFinite(participationRate) || participationRate < 0.01 || participationRate > 1))
    )
      return;
    if (
      hasLimit &&
      state.limitPriceWarning &&
      state.confirmedLimitPrice !== state.limitPrice
    ) {
      this.update({
        confirmedLimitPrice: state.limitPrice,
        message: state.limitPriceWarning + ' Submit again to confirm this price.',
      });
      return;
    }
    this.update({ busy: true, message: '' });
    try {
      const isVwap = state.orderType === 'vwap';
      const isTwap = state.orderType === 'twap';
      const scheduled = isVwap || isTwap;
      const requestOrderType = state.orderType === 'gtd' ? 'limit' : state.orderType;
      const response = await fetch(isVwap ? '/positions/vwap' : isTwap ? '/positions/twap' : '/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          symbol: state.symbol,
          side: state.side,
          ...(scheduled ? { duration_minutes: durationMinutes, ...(isVwap ? { max_participation_rate: participationRate } : {}) } : { order_type: requestOrderType }),
          quantity,
          ...(hasLimit ? { limit_price: limitPrice } : {}),
          ...(hasTrigger ? { trigger_price: barrierPrice, trigger_direction: state.barrierDirection } : {}),
          ...(state.orderType === 'trailing-stop' ? (state.trailingMode === 'amount' ? { trailing_amount: trailingValue } : { trailing_percent: trailingValue / 100 }) : {}),
          ...(state.orderType === 'gtd' ? { time_in_force: 'gtd', expires_at: expiresAt } : {}),
        }),
      });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(
          detail || 'Order rejected (HTTP ' + response.status + ')',
        );
      }
      const result = (await response.json()) as {
        filled_quantity?: number; remaining_quantity?: number; average_execution_price?: number; estimated_fee?: number;
        id?: string; status?: string; executed_quantity?: number;
        barrier_order?: { id: string; status: string; trigger_price: number; trigger_direction: BarrierDirection; order_kind?: string };
      };
      if (result.barrier_order) {
        this.update({ message: (result.barrier_order.order_kind ?? 'Barrier') + ' order ' + result.barrier_order.id + ' armed ' + result.barrier_order.trigger_direction + ' ' + result.barrier_order.trigger_price + '.', quantity: '', barrierPrice: '' });
        return;
      }
      if (scheduled && result.id) {
        this.update({ vwapRunId: result.id, vwapStatus: result.status ?? 'running', vwapExecutedQuantity: result.executed_quantity ?? 0, message: (isTwap ? 'TWAP' : 'VWAP') + ' execution started.' });
        this.startVwapPolling(result.id);
        return;
      }
      const fillText =
        'Filled ' +
        result.filled_quantity +
        (result.average_execution_price
          ? ' at ' + result.average_execution_price
          : '');
      this.update({
        message:
          (result.remaining_quantity ?? 0) > 0
            ? fillText + '; ' + result.remaining_quantity + ' resting'
            : fillText,
        quantity: '',
        confirmedLimitPrice: undefined,
      });
    } catch (reason) {
      this.update({
        message: reason instanceof Error ? reason.message : 'Order failed',
      });
    } finally {
      this.update({ busy: false });
    }
  }
  private async loadActiveVwap(symbol: string): Promise<void> {
    try {
      const response = await fetch('/positions/vwap');
      if (!response.ok) return;
      const data = (await response.json()) as { runs: Array<{ id: string; symbol: string; status: string; executed_quantity: number; strategy?: string }> };
      if (!this.feed || this.props$.value.symbol !== symbol) return;
      const run = [...data.runs].reverse().find((item) => item.symbol === symbol && item.status === 'running');
      if (!run) return;
      this.update({ orderType: run.strategy === 'twap' ? 'twap' : 'vwap', vwapRunId: run.id, vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity, message: 'Resumed monitoring ' + (run.strategy === 'twap' ? 'TWAP' : 'VWAP') + ' execution.' });
      this.startVwapPolling(run.id);
    } catch { /* execution status can be loaded on the next ticket open */ }
  }
  private startVwapPolling(runId: string): void {
    if (this.vwapTimer) clearInterval(this.vwapTimer);
    this.vwapTimer = setInterval(async () => {
      try {
        const response = await fetch('/positions/vwap');
        if (!response.ok) return;
        const data = (await response.json()) as { runs: Array<{ id: string; status: string; executed_quantity: number; error?: string; strategy?: string }> };
        const run = data.runs.find((item) => item.id === runId);
        if (!run) return;
        this.update({ vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity, message: run.error ?? ((run.strategy === 'twap' ? 'TWAP ' : 'VWAP ') + run.status + ' · filled ' + run.executed_quantity) });
        if (run.status !== 'running') { clearInterval(this.vwapTimer); this.vwapTimer = undefined; }
      } catch { /* polling retries on the next interval */ }
    }, 1000);
  }
  async cancelVwap(): Promise<void> {
    const id = this.state$.value.vwapRunId;
    if (!id) return;
    try { await fetch('/positions/vwap/' + encodeURIComponent(id), { method: 'DELETE' }); }
    finally { this.update({ vwapStatus: 'cancelled', message: (this.state$.value.orderType === 'twap' ? 'TWAP' : 'VWAP') + ' execution cancelled.' }); }
  }
  private update(update: Partial<TradeTicketState>): void {
    const next = { ...this.state$.value, ...update };
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
    this.state$.next(next);
  }
}







