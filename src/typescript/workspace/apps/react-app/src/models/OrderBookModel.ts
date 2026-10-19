import { BehaviorSubject, NEVER, throttleTime, type Subscription } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
type Props = { symbol: string; connection: Connection; streamKey: string };
export type OrderBookState = {
  symbol: string;
  depth?: BookDepth;
  deepDepth?: BookDepth;
  error?: string;
  frozen: boolean;
};
export class OrderBookModel {
  readonly props$: BehaviorSubject<Props>;
  readonly state$: BehaviorSubject<OrderBookState>;
  private feed?: BookFeed;
  private feedSub?: Subscription;
  private errorSub?: Subscription;
  private deepTimer?: ReturnType<typeof setInterval>;
  private deepRequest?: AbortController;
  private frozen = false;
  constructor(props: Props) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject<OrderBookState>({
      symbol: props.symbol,
      frozen: false,
    });
  }
  readonly getSnapshot = () => this.state$.value;
  readonly subscribe = (notify: () => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  updateProps(props: Props): void {
    const old = this.props$.value;
    this.props$.next(props);
    if (
      old.connection !== props.connection ||
      old.symbol !== props.symbol ||
      old.streamKey !== props.streamKey
    ) {
      this.stop();
      this.patch({ symbol: props.symbol, depth: undefined, deepDepth: undefined, error: undefined });
      this.start();
    }
  }
  start(): void {
    if (this.feed) return;
    const { connection, symbol, streamKey } = this.props$.value;
    const feed = connection.openBook(streamKey, symbol);
    this.feed = feed;
    this.feedSub = (
      this.frozen
        ? NEVER
        : feed.data.pipe(
            throttleTime(250, undefined, { leading: true, trailing: true }),
          )
    ).subscribe((raw) => {
      if (raw === undefined) {
        this.patch({ depth: undefined });
        return;
      }
      const depth = decodeBookDepth(raw);
      if (depth?.symbol === symbol) this.patch({ depth, error: undefined });
    });
    this.errorSub = feed.error.subscribe((error) => {
      if (error) this.patch({ error });
    });
    if (!this.frozen) {
      void this.refreshDeepBook();
      this.deepTimer = setInterval(() => void this.refreshDeepBook(), 2000);
    }
  }
  stop(): void {
    if (this.deepTimer) clearInterval(this.deepTimer);
    this.deepTimer = undefined;
    this.deepRequest?.abort();
    this.deepRequest = undefined;
    if (!this.feed) return;
    this.feedSub?.unsubscribe();
    this.errorSub?.unsubscribe();
    this.props$.value.connection.closeBook(this.feed.key, this.feed);
    this.feed = undefined;
    this.feedSub = undefined;
    this.errorSub = undefined;
  }
  toggleFrozen(): void {
    this.frozen = !this.frozen;
    this.patch({ frozen: this.frozen });
    this.stop();
    this.start();
  }
  private async refreshDeepBook(): Promise<void> {
    const { symbol } = this.props$.value;
    this.deepRequest?.abort();
    const controller = new AbortController();
    this.deepRequest = controller;
    try {
      const response = await fetch('https://www.okx.com/api/v5/market/books?instId=' + encodeURIComponent(symbol) + '&sz=50', { signal: controller.signal });
      if (!response.ok) throw new Error('OKX returned HTTP ' + response.status);
      const result = await response.json() as { code: string; msg?: string; data?: Array<{ ts: string; bids: string[][]; asks: string[][] }> };
      if (result.code !== '0' || !result.data?.[0]) throw new Error(result.msg || 'No deeper book returned');
      const book = result.data[0];
      const mapSide = (rows: string[][]): BookDepth['bids'] => {
        let cumulativeSize = 0, cumulativeNotional = 0;
        return rows.map(([rawPrice, rawSize]) => {
          const price = Number(rawPrice), size = Number(rawSize);
          cumulativeSize += size;
          cumulativeNotional += price * size;
          return { price, size, cumulativeSize, cumulativeNotional };
        }).filter((level) => Number.isFinite(level.price) && level.price > 0 && Number.isFinite(level.size) && level.size >= 0);
      };
      const deepDepth: BookDepth = { symbol, timestamp: Number(book.ts), bids: mapSide(book.bids), asks: mapSide(book.asks) };
      if (this.props$.value.symbol === symbol) this.patch({ deepDepth });
    } catch (error) {
      if (!controller.signal.aborted && this.props$.value.symbol === symbol) this.patch({ error: 'Deep book unavailable: ' + (error instanceof Error ? error.message : 'request failed') });
    }
  }
  private patch(patch: Partial<OrderBookState>): void {
    this.state$.next({ ...this.state$.value, ...patch });
  }
}
