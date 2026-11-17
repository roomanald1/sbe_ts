import { BehaviorSubject } from 'rxjs';
import type { BookFeed, TradeFeed } from './connection';

type Feed = (BookFeed | TradeFeed) & {
  symbol?: string;
  active: boolean;
  subscription?: StreamSubscription;
  retryTimer?: ReturnType<typeof setTimeout>;
  retryDelay?: number;
};
type StreamSubscription = { cancel(): void; request(count: number): void };
type StreamPayload = { data: unknown };
type StreamObserver = {
  onSubscribe(subscription: StreamSubscription): void;
  onNext(payload: StreamPayload): void;
  onError(error: { message?: string }): void;
  onComplete(): void;
};
type StreamSocket = {
  requestStream(options: { data: Buffer }): {
    subscribe(observer: StreamObserver): void;
  };
};

type FeedOptions<F extends Feed> = {
  create(key: string, symbol?: string): F;
  request(feed: F): string;
  demand: number;
  getSocket(): StreamSocket | undefined;
  getGeneration(): number;
};

/** Owns one family of RSocket feeds and its resubscription lifecycle. */
export class ConnectionFeedManager<F extends Feed> {
  private readonly feeds = new Map<string, F>();
  private readonly options: FeedOptions<F>;
  constructor(options: FeedOptions<F>) {
    this.options = options;
  }

  open(key: string, symbol?: string): F {
    const existing = this.feeds.get(key);
    if (existing && existing.symbol === symbol) return existing;
    if (existing) this.close(key, existing);
    const feed = this.options.create(key, symbol);
    this.feeds.set(key, feed);
    this.start(feed);
    return feed;
  }

  close(key: string, feed: F): void {
    if (this.feeds.get(key) !== feed) return;
    if (feed.retryTimer) clearTimeout(feed.retryTimer);
    feed.retryTimer = undefined;
    feed.subscription?.cancel();
    feed.subscription = undefined;
    feed.active = false;
    this.feeds.delete(key);
    feed.data.complete();
    feed.error.complete();
  }

  cancelAll(): void {
    for (const feed of this.feeds.values()) {
      if (feed.retryTimer) clearTimeout(feed.retryTimer);
      feed.retryTimer = undefined;
      feed.subscription?.cancel();
      feed.subscription = undefined;
      feed.active = false;
      feed.data.next(undefined);
    }
  }

  resumeAll(): void {
    for (const feed of this.feeds.values()) this.start(feed);
  }

  closeAll(): void {
    for (const feed of this.feeds.values()) this.close(feed.key, feed);
  }

  private start(feed: F): void {
    const socket = this.options.getSocket();
    const generation = this.options.getGeneration();
    if (!socket || feed.active || this.feeds.get(feed.key) !== feed) return;
    feed.active = true;
    let received = 0;
    let requestNext: (() => void) | undefined;
    socket
      .requestStream({ data: Buffer.from(this.options.request(feed)) })
      .subscribe({
        onSubscribe: (subscription) => {
          if (!this.isCurrent(feed, socket, generation)) {
            subscription.cancel();
            return;
          }
          feed.subscription = subscription;
          requestNext = () => subscription.request(this.options.demand);
          requestNext();
        },
        onNext: (payload) => {
          if (!this.isCurrent(feed, socket, generation)) return;
          feed.data.next(payload.data);
          feed.error.next(undefined);
          feed.retryDelay = 1000;
          received += 1;
          if (received >= this.options.demand) {
            received = 0;
            requestNext?.();
          }
        },
        onError: (error) =>
          this.fail(
            feed,
            socket,
            generation,
            error.message ?? 'Feed disconnected',
          ),
        onComplete: () => this.fail(feed, socket, generation, 'Feed completed'),
      });
  }

  private isCurrent(
    feed: F,
    socket: StreamSocket,
    generation: number,
  ): boolean {
    return (
      this.feeds.get(feed.key) === feed &&
      this.options.getSocket() === socket &&
      this.options.getGeneration() === generation
    );
  }

  private fail(
    feed: F,
    socket: StreamSocket,
    generation: number,
    message: string,
  ): void {
    if (
      this.feeds.get(feed.key) !== feed ||
      this.options.getSocket() !== socket ||
      this.options.getGeneration() !== generation
    )
      return;
    feed.subscription = undefined;
    feed.active = false;
    feed.error.next(message);
    if (feed.retryTimer) return;
    const delay = feed.retryDelay ?? 1000;
    feed.retryDelay = Math.min(delay * 2, 30_000);
    feed.retryTimer = setTimeout(() => {
      feed.retryTimer = undefined;
      this.start(feed);
    }, delay);
  }
}

export function createBookFeed(
  key: string,
  symbol = '',
): BookFeed & { active: boolean } {
  return {
    key,
    symbol,
    data: new BehaviorSubject<unknown | undefined>(undefined),
    error: new BehaviorSubject<string | undefined>(undefined),
    active: false,
  };
}

export function createTradeFeed(
  key: string,
  symbol = '',
): TradeFeed & { active: boolean } {
  return {
    key,
    symbol,
    data: new BehaviorSubject<unknown | undefined>(undefined),
    error: new BehaviorSubject<string | undefined>(undefined),
    active: false,
  };
}
