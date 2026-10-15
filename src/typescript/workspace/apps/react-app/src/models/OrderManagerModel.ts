import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
import { getLimitPriceMarkWarning } from './limit_price_guard';

type TradeSide = 'buy' | 'sell';
type OrderType = 'ioc' | 'limit' | 'vwap';
type RestingOrder = {
  id: string;
  symbol: string;
  side: TradeSide;
  quantity: number;
  remaining_quantity: number;
  limit_price: number;
  timestamp: number;
};
type OrdersResponse = { orders: RestingOrder[] };
export type VwapRun = {
  id: string;
  symbol: string;
  side: TradeSide;
  quantity: number;
  executed_quantity: number;
  duration_minutes: number;
  started_at: number;
  status: string;
  market_vwap: number;
  execution_vwap: number;
  slippage: number;
  error?: string;
};
type VwapRunsResponse = { runs: VwapRun[] };
type FeedEntry = {
  feed: BookFeed;
  data: { unsubscribe(): void };
  errors: { unsubscribe(): void };
};
export type OrderRow = {
  order: RestingOrder;
  current?: number;
  bookError?: string;
};
export type OrderManagerState = {
  symbols: string[];
  books: Record<string, BookDepth>;
  bookErrors: Record<string, string>;
  selectedSymbol: string;
  side: TradeSide;
  quantity: string;
  orderType: OrderType;
  limitPrice: string;
  orders: RestingOrder[];
  vwapRuns: VwapRun[];
  rows: OrderRow[];
  selectedCurrentPrice?: number;
  limitPriceWarning?: string;
  confirmedLimitPrice?: string;
  durationMinutes: string;
  participationRate: string;
  vwapRunId?: string;
  vwapStatus?: string;
  vwapExecutedQuantity?: number;
  loading: boolean;
  submitting: boolean;
  error?: string;
};
type OrderManagerProps = { connection: Connection; symbols: string[] };
const qtyFormat = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 0,
  maximumFractionDigits: 8,
});

export class OrderManagerModel {
  readonly props$: BehaviorSubject<OrderManagerProps>;
  readonly state$: BehaviorSubject<OrderManagerState>;
  private activeFeeds = new Map<string, FeedEntry>();
  private timer?: ReturnType<typeof setInterval>;
  private vwapTimer?: ReturnType<typeof setInterval>;
  constructor(props: OrderManagerProps) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject<OrderManagerState>({
      symbols: props.symbols,
      books: {},
      bookErrors: {},
      selectedSymbol: props.symbols[0] ?? '',
      side: 'buy',
      quantity: '',
      orderType: 'ioc',
      limitPrice: '',
      durationMinutes: '60',
      participationRate: '10',
      orders: [],
      vwapRuns: [],
      rows: [],
      loading: true,
      submitting: false,
    });
  }
  private get connection(): Connection {
    return this.props$.value.connection;
  }
  readonly getSnapshot = (): OrderManagerState => this.state$.value;
  readonly subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  updateProps(props: OrderManagerProps): void {
    const oldConnection = this.connection;
    this.props$.next(props);
    const selectedSymbol = props.symbols.includes(
      this.state$.value.selectedSymbol,
    )
      ? this.state$.value.selectedSymbol
      : (props.symbols[0] ?? '');
    this.patch({ symbols: props.symbols, selectedSymbol });
    if (oldConnection !== props.connection) {
      this.closeFeeds();
      this.syncFeeds();
    }
  }
  start(): void {
    if (this.timer) return;
    this.syncFeeds();
    void this.refreshOrders();
    void this.loadActiveVwap(this.state$.value.selectedSymbol);
    this.timer = setInterval(() => void this.refreshOrders(), 5000);
  }
  stop(): void {
    if (this.vwapTimer) clearInterval(this.vwapTimer);
    this.vwapTimer = undefined;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.closeFeeds();
  }
  setSymbol(selectedSymbol: string): void {
    this.patch({ selectedSymbol, confirmedLimitPrice: undefined });
    this.syncFeeds();
  }
  setSide(side: TradeSide): void {
    this.patch({ side });
  }
  setQuantity(quantity: string): void {
    this.patch({ quantity });
  }
  setOrderType(orderType: OrderType): void {
    this.patch({ orderType, confirmedLimitPrice: undefined });
  }
  setDurationMinutes(durationMinutes: string): void {
    this.patch({ durationMinutes });
  }
  setParticipationRate(participationRate: string): void {
    this.patch({ participationRate });
  }
  setLimitPrice(limitPrice: string): void {
    this.patch({ limitPrice, confirmedLimitPrice: undefined, error: undefined });
  }
  async refreshOrders(): Promise<void> {
    try {
      const response = await fetch('/positions/orders');
      if (!response.ok)
        throw new Error(
          'Could not load resting orders (' + response.status + ')',
        );
      const data = (await response.json()) as OrdersResponse;
      if (!Array.isArray(data.orders))
        throw new Error('The orders API returned invalid data');
      this.patch({ orders: data.orders, loading: false, error: undefined });
      this.syncFeeds();
    } catch (reason) {
      this.patch({
        loading: false,
        error:
          reason instanceof Error
            ? reason.message
            : 'Could not load resting orders',
      });
    }
  }
  async submit(): Promise<void> {
    const state = this.state$.value;
    const quantity = Number(state.quantity);
    const limitPrice = Number(state.limitPrice);
    const durationMinutes = Number(state.durationMinutes);
    const participationRate = Number(state.participationRate) / 100;
    if (
      !state.selectedSymbol ||
      !Number.isFinite(quantity) ||
      quantity <= 0 ||
      (state.orderType === 'limit' &&
        (!Number.isFinite(limitPrice) || limitPrice <= 0)) ||
      (state.orderType === 'vwap' &&
        (!Number.isInteger(durationMinutes) || durationMinutes < 5 || durationMinutes > 390 ||
          !Number.isFinite(participationRate) || participationRate < 0.01 || participationRate > 1))
    )
      return;
    if (
      state.orderType === 'limit' &&
      state.limitPriceWarning &&
      state.confirmedLimitPrice !== state.limitPrice
    ) {
      this.patch({
        confirmedLimitPrice: state.limitPrice,
        error: state.limitPriceWarning + ' Submit again to confirm this price.',
      });
      return;
    }
    this.patch({ submitting: true, error: undefined });
    try {
      const isVwap = state.orderType === 'vwap';
      const response = await fetch(isVwap ? '/positions/vwap' : '/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          symbol: state.selectedSymbol,
          side: state.side,
          ...(isVwap ? { duration_minutes: durationMinutes, max_participation_rate: participationRate } : { order_type: state.orderType }),
          quantity,
          ...(state.orderType === 'limit' ? { limit_price: limitPrice } : {}),
        }),
      });
      if (!response.ok) {
        const detail = await response.text();
        throw new Error(detail || 'Order rejected (' + response.status + ')');
      }
      const result = (await response.json()) as {
        filled_quantity?: number; remaining_quantity?: number; id?: string; status?: string; executed_quantity?: number;
      };
      if (isVwap && result.id) {
        this.patch({ vwapRunId: result.id, vwapStatus: result.status ?? 'running', vwapExecutedQuantity: result.executed_quantity ?? 0, vwapRuns: [...this.state$.value.vwapRuns.filter((run) => run.id !== result.id), result as VwapRun], error: undefined, quantity: '', orderType: 'ioc' });
        this.startVwapPolling(result.id);
        return;
      }
      await this.refreshOrders();
      this.patch({
        error:
          (result.remaining_quantity ?? 0) > 0
            ? 'Filled ' +
              qtyFormat.format(result.filled_quantity ?? 0) +
              '; ' +
              qtyFormat.format(result.remaining_quantity ?? 0) +
              ' resting'
            : 'Filled ' + qtyFormat.format(result.filled_quantity ?? 0),
        quantity: '',
        confirmedLimitPrice: undefined,
      });
    } catch (reason) {
      this.patch({
        error: reason instanceof Error ? reason.message : 'Order failed',
      });
    } finally {
      this.patch({ submitting: false });
    }
  }
  private async loadActiveVwap(symbol: string): Promise<void> {
    try {
      const response = await fetch('/positions/vwap');
      if (!response.ok || !symbol || this.state$.value.selectedSymbol !== symbol) return;
      const data = (await response.json()) as VwapRunsResponse;
      this.patch({ vwapRuns: data.runs });
      const run = [...data.runs].reverse().find((item) => item.symbol === symbol && item.status === 'running');
      if (!run) return;
      this.patch({ vwapRunId: run.id, vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity });
      this.startVwapPolling(run.id);
    } catch { /* retry when Orders is reopened */ }
  }
  private startVwapPolling(runId: string): void {
    if (this.vwapTimer) clearInterval(this.vwapTimer);
    this.vwapTimer = setInterval(async () => {
      try {
        const response = await fetch('/positions/vwap');
        if (!response.ok) return;
        const data = (await response.json()) as VwapRunsResponse;
        const run = data.runs.find((item) => item.id === runId);
        this.patch({ vwapRuns: data.runs });
        if (!run) return;
        this.patch({ vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity });
        if (run.status !== 'running') { clearInterval(this.vwapTimer); this.vwapTimer = undefined; }
      } catch { /* retry on the next interval */ }
    }, 1000);
  }
  async cancelVwap(runId = this.state$.value.vwapRunId): Promise<void> {
    if (!runId) return;
    try {
      const response = await fetch('/positions/vwap/' + encodeURIComponent(runId), { method: 'DELETE' });
      if (!response.ok) throw new Error((await response.text()) || 'Could not cancel VWAP execution');
      const vwapRuns = this.state$.value.vwapRuns.map((run) => run.id === runId ? { ...run, status: 'cancelled' } : run);
      this.patch({ vwapRuns, vwapStatus: 'cancelled', error: undefined });
    } catch (reason) {
      this.patch({ error: reason instanceof Error ? reason.message : 'Could not cancel VWAP execution' });
    }
  }
  async cancelOrder(id: string): Promise<void> {
    try {
      const response = await fetch(
        '/positions/orders/' + encodeURIComponent(id),
        { method: 'DELETE' },
      );
      if (!response.ok) throw new Error(await response.text());
      this.patch({
        orders: this.state$.value.orders.filter((order) => order.id !== id),
      });
      this.syncFeeds();
    } catch (reason) {
      this.patch({
        error:
          reason instanceof Error ? reason.message : 'Could not cancel order',
      });
    }
  }
  private patch(patch: Partial<OrderManagerState>): void {
    this.state$.next(this.derive({ ...this.state$.value, ...patch }));
  }
  private derive(state: OrderManagerState): OrderManagerState {
    const rows = state.orders.map((order) => {
      const book = state.books[order.symbol];
      const current =
        book?.bids[0] && book.asks[0]
          ? (book.bids[0].price + book.asks[0].price) / 2
          : (book?.bids[0]?.price ?? book?.asks[0]?.price);
      return { order, current, bookError: state.bookErrors[order.symbol] };
    });
    const book = state.books[state.selectedSymbol];
    const selectedCurrentPrice =
      book?.bids[0] && book.asks[0]
        ? (book.bids[0].price + book.asks[0].price) / 2
        : (book?.bids[0]?.price ?? book?.asks[0]?.price);
    const limitPriceWarning =
      state.orderType === 'limit'
        ? getLimitPriceMarkWarning(state.limitPrice, selectedCurrentPrice)
        : undefined;
    return {
      ...state,
      rows,
      selectedCurrentPrice,
      limitPriceWarning,
      confirmedLimitPrice: limitPriceWarning ? state.confirmedLimitPrice : undefined,
    };
  }
  private syncFeeds(): void {
    const state = this.state$.value;
    const symbols = new Set(
      [
        state.selectedSymbol,
        ...state.orders.map((order) => order.symbol),
      ].filter(Boolean),
    );
    for (const symbol of symbols) {
      if (this.activeFeeds.has(symbol)) continue;
      const feed = this.connection.openBook('order-manager:' + symbol, symbol);
      const data = feed.data.subscribe((raw) => {
        if (raw === undefined) {
          const books = { ...this.state$.value.books };
          delete books[symbol];
          this.patch({ books });
          return;
        }
        const depth = decodeBookDepth(raw);
        if (!depth || depth.symbol !== symbol) return;
        this.patch({
          books: { ...this.state$.value.books, [symbol]: depth },
          bookErrors: {
            ...this.state$.value.bookErrors,
            [symbol]: undefined,
          } as Record<string, string>,
        });
      });
      const errors = feed.error.subscribe((error) => {
        if (error)
          this.patch({
            bookErrors: { ...this.state$.value.bookErrors, [symbol]: error },
          });
      });
      this.activeFeeds.set(symbol, { feed, data, errors });
    }
    for (const [symbol, active] of this.activeFeeds) {
      if (symbols.has(symbol)) continue;
      active.data.unsubscribe();
      active.errors.unsubscribe();
      this.connection.closeBook(active.feed.key, active.feed);
      this.activeFeeds.delete(symbol);
      const books = { ...this.state$.value.books };
      delete books[symbol];
      const bookErrors = { ...this.state$.value.bookErrors };
      delete bookErrors[symbol];
      this.patch({ books, bookErrors });
    }
  }
  private closeFeeds(): void {
    for (const active of this.activeFeeds.values()) {
      active.data.unsubscribe();
      active.errors.unsubscribe();
      this.connection.closeBook(active.feed.key, active.feed);
    }
    this.activeFeeds.clear();
  }
}


