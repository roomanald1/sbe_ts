import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
import type { Position } from './PositionTypes';

type PositionsResponse = { positions: Position[]; trade_count: number };
type ActiveFeed = {
  feed: BookFeed;
  data: { unsubscribe(): void };
  errors: { unsubscribe(): void };
};
export type PnlRow = {
  position: Position;
  mark?: number;
  unrealized?: number;
  bookError?: string;
};
export type PnlTotal = {
  realized: number;
  unrealized: number;
  hasUnmarkedExposure: boolean;
};
export type PnlState = {
  rows: PnlRow[];
  totals: Record<string, PnlTotal>;
  currencies: string[];
  loading: boolean;
  error?: string;
};

const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';
export class PnlModel {
  readonly props$;
  readonly state$ = new BehaviorSubject<PnlState>({
    rows: [],
    totals: {},
    currencies: [],
    loading: true,
  });
  private positions: Position[] = [];
  private books: Record<string, BookDepth> = {};
  private errors: Record<string, string> = {};
  private feeds = new Map<string, ActiveFeed>();
  private timer?: ReturnType<typeof setInterval>;
  private connection: Connection;
  constructor(connection: Connection) {
    this.connection = connection;
    this.props$ = new BehaviorSubject({ connection });
  }
  getSnapshot = (): PnlState => this.state$.value;
  subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  start(): void {
    if (this.timer) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 5000);
  }
  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    for (const feed of this.feeds.values()) this.closeFeed(feed);
    this.feeds.clear();
  }
  setConnection(connection: Connection): void {
    if (connection === this.connection) return;
    this.stop();
    this.connection = connection;
    this.props$.next({ connection });
    this.start();
  }
  async refresh(): Promise<void> {
    try {
      const response = await fetch('/positions');
      if (!response.ok)
        throw new Error(
          'Could not load fill-based P&L (' + response.status + ')',
        );
      const data = (await response.json()) as PositionsResponse;
      if (!Array.isArray(data.positions))
        throw new Error('The positions API returned invalid data');
      this.positions = data.positions;
      this.syncFeeds();
      this.publishState();
    } catch (reason) {
      this.publishState(
        reason instanceof Error ? reason.message : 'Could not load P&L',
      );
    }
  }
  private syncFeeds(): void {
    const symbols = new Set(
      this.positions
        .filter((position) => Math.abs(position.quantity) > 1e-12)
        .map((position) => position.symbol),
    );
    for (const symbol of symbols) {
      if (this.feeds.has(symbol)) continue;
      const feed = this.connection.openBook('pnl-mark:' + symbol, symbol);
      const data = feed.data.subscribe((raw) => {
        const depth = decodeBookDepth(raw);
        if (!depth || depth.symbol !== symbol) return;
        this.books[symbol] = depth;
        delete this.errors[symbol];
        this.publishState();
      });
      const errors = feed.error.subscribe((error) => {
        if (error) {
          this.errors[symbol] = error;
          this.publishState();
        }
      });
      this.feeds.set(symbol, { feed, data, errors });
    }
    for (const [symbol, active] of this.feeds) {
      if (symbols.has(symbol)) continue;
      this.closeFeed(active);
      this.feeds.delete(symbol);
      delete this.books[symbol];
      delete this.errors[symbol];
    }
  }
  private closeFeed(active: ActiveFeed): void {
    active.data.unsubscribe();
    active.errors.unsubscribe();
    this.connection.closeBook(active.feed.key, active.feed);
  }
  private publishState(error?: string): void {
    const rows = this.positions.map((position) => {
      const book = this.books[position.symbol];
      const mark =
        position.quantity > 0
          ? book?.bids[0]?.price
          : position.quantity < 0
            ? book?.asks[0]?.price
            : undefined;
      return {
        position,
        mark,
        unrealized:
          mark === undefined
            ? undefined
            : (mark - position.average_entry_price) * position.quantity,
        bookError: this.errors[position.symbol],
      };
    });
    const totals: Record<string, PnlTotal> = {};
    for (const row of rows) {
      const currency = quoteCurrency(row.position.symbol);
      const total =
        totals[currency] ??
        (totals[currency] = {
          realized: 0,
          unrealized: 0,
          hasUnmarkedExposure: false,
        });
      total.realized += row.position.realized_pnl;
      if (Math.abs(row.position.quantity) > 1e-12) {
        if (row.unrealized === undefined) total.hasUnmarkedExposure = true;
        else total.unrealized += row.unrealized;
      }
    }
    const visibleRows = rows.filter(
      (row) =>
        Math.abs(row.position.quantity) > 1e-12 ||
        Math.abs(row.position.realized_pnl) > 1e-12,
    );
    this.state$.next({
      rows: visibleRows,
      totals,
      currencies: Object.keys(totals).sort(),
      loading: false,
      error,
    });
  }
}
