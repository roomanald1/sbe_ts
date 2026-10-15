import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';

type Side = 'buy' | 'sell';
type OrderType = 'ioc' | 'limit';
type Props = { symbol: string; initialSide: Side; connection: Connection };
export type TradeTicketState = {
  symbol: string;
  book?: BookDepth;
  bookError: string;
  side: Side;
  orderType: OrderType;
  quantity: string;
  limitPrice: string;
  busy: boolean;
  message: string;
  currentPrice?: number;
};
export class TradeTicketModel {
  readonly props$: BehaviorSubject<Props>;
  readonly state$: BehaviorSubject<TradeTicketState>;
  private feed?: BookFeed;
  private dataSub?: { unsubscribe(): void };
  private errorSub?: { unsubscribe(): void };
  constructor(props: Props) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject<TradeTicketState>({
      symbol: props.symbol,
      side: props.initialSide,
      orderType: 'ioc' as OrderType,
      quantity: '',
      limitPrice: '',
      busy: false,
      bookError: '',
      message: '',
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
      });
      this.start();
    }
  }
  start(): void {
    if (this.feed) return;
    const { connection, symbol } = this.props$.value;
    const key = 'order-ticket:' + symbol;
    const feed = connection.openBook(key, symbol);
    this.feed = feed;
    this.dataSub = feed.data.subscribe((raw) => {
      const book = decodeBookDepth(raw);
      if (book?.symbol === symbol) this.update({ book, bookError: '' });
    });
    this.errorSub = feed.error.subscribe((bookError) => {
      if (bookError) this.update({ bookError });
    });
  }
  stop(): void {
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
    this.update({ side });
  }
  setOrderType(orderType: OrderType): void {
    this.update({ orderType });
  }
  setQuantity(quantity: string): void {
    this.update({ quantity });
  }
  setLimitPrice(limitPrice: string): void {
    this.update({ limitPrice });
  }
  async submit(): Promise<void> {
    const state = this.state$.value;
    const quantity = Number(state.quantity);
    const limitPrice = Number(state.limitPrice);
    if (
      !Number.isFinite(quantity) ||
      quantity <= 0 ||
      (state.orderType === 'limit' &&
        (!Number.isFinite(limitPrice) || limitPrice <= 0))
    )
      return;
    this.update({ busy: true, message: '' });
    try {
      const response = await fetch('/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          symbol: state.symbol,
          side: state.side,
          order_type: state.orderType,
          quantity,
          ...(state.orderType === 'limit' ? { limit_price: limitPrice } : {}),
        }),
      });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(
          detail || 'Order rejected (HTTP ' + response.status + ')',
        );
      }
      const result = (await response.json()) as {
        filled_quantity: number;
        remaining_quantity: number;
        average_execution_price?: number;
      };
      const fillText =
        'Filled ' +
        result.filled_quantity +
        (result.average_execution_price
          ? ' at ' + result.average_execution_price
          : '');
      this.update({
        message:
          result.remaining_quantity > 0
            ? fillText + '; ' + result.remaining_quantity + ' resting'
            : fillText,
        quantity: '',
      });
    } catch (reason) {
      this.update({
        message: reason instanceof Error ? reason.message : 'Order failed',
      });
    } finally {
      this.update({ busy: false });
    }
  }
  private update(update: Partial<TradeTicketState>): void {
    const next = { ...this.state$.value, ...update };
    const book = next.book;
    next.currentPrice =
      book?.bids[0] && book.asks[0]
        ? (book.bids[0].price + book.asks[0].price) / 2
        : (book?.bids[0]?.price ?? book?.asks[0]?.price);
    this.state$.next(next);
  }
}
