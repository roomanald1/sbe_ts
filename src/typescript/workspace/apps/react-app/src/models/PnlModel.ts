import { BehaviorSubject } from 'rxjs';
import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth, type BookDepth } from '../types/symbol_price';
import type { Position } from './PositionTypes';
import { subscribePositionStream } from '../position_stream';

type ActiveFeed = {
  feed: BookFeed;
  data: { unsubscribe(): void };
  errors: { unsubscribe(): void };
};
export type PnlRow = {
  position: Position;
  mark?: number;
  unrealized?: number;
  realizedUsd?: number;
  unrealizedUsd?: number;
  feesPaidUsd?: number;
  bookError?: string;
  exposure?: number;
  exposureUsd?: number;
  averageEntryUsd?: number;
  markTimestamp?: number;
};
export type PnlTotal = {
  realized: number;
  unrealized: number;
  feesPaid: number;
  hasUnmarkedExposure: boolean;
  hasUnconvertedPnl: boolean;
  missingCurrencies: string[];
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
  private usdRates: Record<string, number> = {};
  private books: Record<string, BookDepth> = {};
  private errors: Record<string, string> = {};
  private feeds = new Map<string, ActiveFeed>();
  private stopPositionStream?: () => void;
  private connection: Connection;
  private readonly feedKeyPrefix: string;
  constructor(connection: Connection, feedKeyPrefix = 'pnl-mark') {
    this.connection = connection;
    this.feedKeyPrefix = feedKeyPrefix;
    this.props$ = new BehaviorSubject({ connection });
  }
  getSnapshot = (): PnlState => this.state$.value;
  subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  start(): void {
    if (this.stopPositionStream) return;
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      this.positions = snapshot.positions.positions;
      if (snapshot.usd_rates?.rates) this.usdRates = snapshot.usd_rates.rates;
      this.syncFeeds();
      this.publishState();
    });
  }
  stop(): void {
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
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
  private syncFeeds(): void {
    const symbols = new Set(
      this.positions
        .filter((position) => Math.abs(position.quantity) > 1e-12)
        .map((position) => position.symbol),
    );
    for (const symbol of symbols) {
      if (this.feeds.has(symbol)) continue;
      const feed = this.connection.openBook(this.feedKeyPrefix + ':' + symbol, symbol);
      const data = feed.data.subscribe((raw) => {
        if (raw === undefined) {
          delete this.books[symbol];
          this.publishState();
          return;
        }
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
      const mark = position.quantity > 0
        ? book?.bids[0]?.price
        : position.quantity < 0
          ? book?.asks[0]?.price
          : undefined;
      const currency = quoteCurrency(position.symbol);
      const rate = this.usdRates[currency];
      const unrealized = mark === undefined
        ? undefined
        : (mark - position.average_entry_price) * position.quantity - (position.entry_fees ?? 0) - Math.abs(position.quantity * mark) * (position.estimated_exit_fee_rate ?? 0);
      return {
        position,
        mark,
        exposure: mark === undefined ? undefined : Math.abs(position.quantity * mark),
        exposureUsd: mark === undefined || rate === undefined ? undefined : Math.abs(position.quantity * mark) * rate,
        averageEntryUsd: rate === undefined ? undefined : position.average_entry_price * rate,
        markTimestamp: mark === undefined ? undefined : book?.timestamp,
        unrealized,
        realizedUsd: rate === undefined ? undefined : position.realized_pnl * rate,
        feesPaidUsd: rate === undefined ? undefined : position.fees_paid * rate,
        unrealizedUsd: unrealized === undefined || rate === undefined ? undefined : unrealized * rate,
        bookError: this.errors[position.symbol],
      };
    });
    const total: PnlTotal = {
      realized: 0,
      unrealized: 0,
      feesPaid: 0,
      hasUnmarkedExposure: false,
      hasUnconvertedPnl: false,
      missingCurrencies: [],
    };
    const missing = new Set<string>();
    for (const row of rows) {
      const hasRealized = Math.abs(row.position.realized_pnl) > 1e-12;
      const hasExposure = Math.abs(row.position.quantity) > 1e-12;
      if (row.realizedUsd === undefined && hasRealized || row.unrealizedUsd === undefined && hasExposure && row.unrealized !== undefined) {
        total.hasUnconvertedPnl = true;
        missing.add(quoteCurrency(row.position.symbol));
      }
      if (row.realizedUsd !== undefined) total.realized += row.realizedUsd;
      if (row.feesPaidUsd !== undefined) total.feesPaid += row.feesPaidUsd;
      if (hasExposure) {
        if (row.unrealized === undefined) total.hasUnmarkedExposure = true;
        else if (row.unrealizedUsd !== undefined) total.unrealized += row.unrealizedUsd;
      }
    }
    total.missingCurrencies = [...missing].sort();
    const visibleRows = rows.filter((row) =>
      Math.abs(row.position.quantity) > 1e-12 ||
      Math.abs(row.position.realized_pnl) > 1e-12,
    );
    this.state$.next({
      rows: visibleRows,
      totals: visibleRows.length ? { USD: total } : {},
      currencies: visibleRows.length ? ['USD'] : [],
      loading: false,
      error,
    });
  }
}
