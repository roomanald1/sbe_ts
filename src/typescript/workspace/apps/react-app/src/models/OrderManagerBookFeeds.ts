import type { BookFeed, Connection } from '../connection';
import { decodeBookDepth } from '../types/symbol_price';

type FeedEntry = {
  feed: BookFeed;
  data: { unsubscribe(): void };
  errors: { unsubscribe(): void };
};
export class OrderManagerBookFeeds {
  private connection: Connection;
  private readonly active = new Map<string, FeedEntry>();

  private readonly onBook: (symbol: string, depth: ReturnType<typeof decodeBookDepth>) => void;
  private readonly onError: (symbol: string, error?: string) => void;

  constructor(
    connection: Connection,
    onBook: (symbol: string, depth: ReturnType<typeof decodeBookDepth>) => void,
    onError: (symbol: string, error?: string) => void,
  ) {
    this.connection = connection;
    this.onBook = onBook;
    this.onError = onError;
  }

  updateConnection(connection: Connection): void {
    this.close();
    this.connection = connection;
  }

  sync(symbols: Set<string>): void {
    for (const symbol of symbols) this.subscribe(symbol);
    for (const [symbol, active] of this.active) {
      if (symbols.has(symbol)) continue;
      this.unsubscribe(active);
      this.active.delete(symbol);
      this.onBook(symbol, undefined);
      this.onError(symbol, undefined);
    }
  }

  close(): void {
    for (const active of this.active.values()) this.unsubscribe(active);
    this.active.clear();
  }

  private subscribe(symbol: string): void {
    if (!symbol || this.active.has(symbol)) return;
    const feed = this.connection.openBook('order-manager:' + symbol, symbol);
    const active: FeedEntry = {
      feed,
      data: feed.data.subscribe((raw) => {
        if (raw === undefined) {
          this.onBook(symbol, undefined);
          return;
        }
        const depth = decodeBookDepth(raw);
        if (depth?.symbol === symbol) {
          this.onBook(symbol, depth);
          this.onError(symbol, undefined);
        }
      }),
      errors: feed.error.subscribe((error) => this.onError(symbol, error)),
    };
    this.active.set(symbol, active);
  }

  private unsubscribe(active: FeedEntry): void {
    active.data.unsubscribe();
    active.errors.unsubscribe();
    this.connection.closeBook(active.feed.key, active.feed);
  }
}
