import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';

type TradeSide = 'buy' | 'sell';
type OrderType = 'ioc' | 'limit';
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
  rows: OrderRow[];
  selectedCurrentPrice?: number;
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
      orders: [],
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
    this.timer = setInterval(() => void this.refreshOrders(), 5000);
  }
  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.closeFeeds();
  }
  setSymbol(selectedSymbol: string): void {
    this.patch({ selectedSymbol });
    this.syncFeeds();
  }
  setSide(side: TradeSide): void {
    this.patch({ side });
  }
  setQuantity(quantity: string): void {
    this.patch({ quantity });
  }
  setOrderType(orderType: OrderType): void {
    this.patch({ orderType });
  }
  setLimitPrice(limitPrice: string): void {
    this.patch({ limitPrice });
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
    if (
      !state.selectedSymbol ||
      !Number.isFinite(quantity) ||
      quantity <= 0 ||
      (state.orderType === 'limit' &&
        (!Number.isFinite(limitPrice) || limitPrice <= 0))
    )
      return;
    this.patch({ submitting: true, error: undefined });
    try {
      const response = await fetch('/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          symbol: state.selectedSymbol,
          side: state.side,
          order_type: state.orderType,
          quantity,
          ...(state.orderType === 'limit' ? { limit_price: limitPrice } : {}),
        }),
      });
      if (!response.ok) {
        const detail = await response.text();
        throw new Error(detail || 'Order rejected (' + response.status + ')');
      }
      const result = (await response.json()) as {
        filled_quantity: number;
        remaining_quantity: number;
      };
      await this.refreshOrders();
      this.patch({
        error:
          result.remaining_quantity > 0
            ? 'Filled ' +
              qtyFormat.format(result.filled_quantity) +
              '; ' +
              qtyFormat.format(result.remaining_quantity) +
              ' resting'
            : 'Filled ' + qtyFormat.format(result.filled_quantity),
        quantity: '',
      });
    } catch (reason) {
      this.patch({
        error: reason instanceof Error ? reason.message : 'Order failed',
      });
    } finally {
      this.patch({ submitting: false });
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
    return { ...state, rows, selectedCurrentPrice };
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
