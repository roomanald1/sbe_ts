import { BehaviorSubject, NEVER, throttleTime, type Subscription } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
type Props = { symbol: string; connection: Connection; streamKey: string };
export type OrderBookState = {
  symbol: string;
  depth?: BookDepth;
  error?: string;
  frozen: boolean;
};
export class OrderBookModel {
  readonly props$: BehaviorSubject<Props>;
  readonly state$: BehaviorSubject<OrderBookState>;
  private feed?: BookFeed;
  private feedSub?: Subscription;
  private errorSub?: Subscription;
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
      this.patch({ symbol: props.symbol, depth: undefined, error: undefined });
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
  }
  stop(): void {
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
  private patch(patch: Partial<OrderBookState>): void {
    this.state$.next({ ...this.state$.value, ...patch });
  }
}
