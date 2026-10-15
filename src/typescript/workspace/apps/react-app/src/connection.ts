import { BehaviorSubject } from 'rxjs';
import {
  BufferEncoder,
  BufferEncoders,
  RSocketClient,
  type Encoder,
} from 'rsocket-core';

import * as WebSocketClient from 'rsocket-websocket-client';

export type ConnectionState = 'connecting' | 'connected' | 'reconnecting';
export type FeedSource = 'okx' | 'synthetic';

export type TradeFeed = {
  key: string;
  symbol: string;
  data: BehaviorSubject<unknown | undefined>;
  error: BehaviorSubject<string | undefined>;
};

export type BookFeed = {
  key: string;
  symbol: string;
  data: BehaviorSubject<unknown | undefined>;
  error: BehaviorSubject<string | undefined>;
};

type ActiveTradeFeed = TradeFeed & {
  subscription?: { cancel: () => void; request: (count: number) => void };
  active: boolean;
};

type ActiveBookFeed = BookFeed & {
  subscription?: { cancel: () => void; request: (count: number) => void };
  active: boolean;
};

const REQUEST_BATCH_SIZE = 500;

const BufferDataEncoder: Encoder<Buffer> = {
  ...BufferEncoder,
  encode: (value, buffer, start, end) => {
    if (!Buffer.isBuffer(value)) {
      throw new TypeError('RSocket data must be a Buffer');
    }
    buffer.set(value, start);
    return end;
  },
};

export class Connection {
  private readonly source: FeedSource;

  public constructor(source: FeedSource = 'okx') {
    this.source = source;
  }

  public readonly url =
    import.meta.env.VITE_RSOCKET_URL ??
    (import.meta.env.DEV
      ? 'ws://localhost:10000'
      : 'wss://sbe-ts.onrender.com');
  private cancelled: boolean = false;
  private lifecycleGeneration = 0;
  private socket: Awaited<ReturnType<typeof this.createClient>> | undefined;
  private subscription: { cancel: () => void } | undefined;
  private readonly bookFeeds = new Map<string, ActiveBookFeed>();
  private readonly tradeFeeds = new Map<string, ActiveTradeFeed>();
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private retryAttempt = 0;

  public connection_state: BehaviorSubject<ConnectionState> =
    new BehaviorSubject<ConnectionState>('connecting');
  public error_message: BehaviorSubject<string | undefined> =
    new BehaviorSubject<string | undefined>(undefined);
  public data: BehaviorSubject<unknown | undefined> = new BehaviorSubject<
    unknown | undefined
  >(undefined);

  public async connect() {
    if (this.socket) return; //already connected

    this.cancelled = false;
    const generation = ++this.lifecycleGeneration;
    console.log('Connecting');
    let requestNext: (() => void) | undefined;
    let updatesSinceRequest = 0;
    const scheduleReconnect = (message: string) => {
      if (
        this.cancelled ||
        generation !== this.lifecycleGeneration ||
        this.retryTimer !== undefined
      )
        return;

      this.connection_state.next('reconnecting');
      this.error_message.next(message);
      this.subscription?.cancel();
      this.subscription = undefined;
      this.cancelBookSubscriptions('connection reconnecting');
      this.cancelTradeSubscriptions('connection reconnecting');
      this.socket?.close();
      this.socket = undefined;

      const delay = Math.min(1000 * 2 ** this.retryAttempt, 30000);
      this.retryAttempt += 1;
      this.retryTimer = setTimeout(() => {
        this.retryTimer = undefined;
        if (generation === this.lifecycleGeneration && !this.cancelled) {
          void this.connect();
        }
      }, delay);
    };

    this.connection_state.next(
      this.retryAttempt === 0 ? 'connecting' : 'reconnecting',
    );
    try {
      const rsocket = await this.createClient();

      if (this.cancelled || generation !== this.lifecycleGeneration) {
        rsocket.close();
        return;
      }
      this.socket = rsocket;

      rsocket
        .requestStream({
          data: Buffer.from(this.source),
        })
        .subscribe({
          onSubscribe: (sub) => {
            if (this.cancelled || generation !== this.lifecycleGeneration) {
              sub.cancel();
              return;
            }
            console.log('OnSubscribe');
            this.retryAttempt = 0;
            this.subscription = sub;
            this.connection_state.next('connected');
            this.error_message.next(undefined);
            this.startBookSubscriptions();
            this.startTradeSubscriptions();
            if (this.source === 'synthetic') {
              requestNext = () => sub.request(REQUEST_BATCH_SIZE);
              sub.request(1000);
            } else {
              requestNext = () => sub.request(REQUEST_BATCH_SIZE);
              requestNext();
            }
          },
          onNext: (payload) => {
            if (this.cancelled || generation !== this.lifecycleGeneration)
              return;
            this.data.next(payload.data);
            updatesSinceRequest += 1;
            if (updatesSinceRequest >= REQUEST_BATCH_SIZE) {
              updatesSinceRequest = 0;
              requestNext?.();
            }
          },
          onError: (error) =>
            scheduleReconnect(error.message ?? 'RSocket stream disconnected'),
          onComplete: () => scheduleReconnect('Stream completed; reconnecting'),
        });
    } catch (error) {
      scheduleReconnect(
        error instanceof Error
          ? error.message
          : 'Unable to connect to RSocket server',
      );
    }
  }

  public openBook(key: string, symbol: string): BookFeed {
    const existing = this.bookFeeds.get(key);
    if (existing?.symbol === symbol) return existing;
    if (existing) this.closeBook(key, existing);

    const feed: ActiveBookFeed = {
      key,
      symbol,
      data: new BehaviorSubject<unknown | undefined>(undefined),
      error: new BehaviorSubject<string | undefined>(undefined),
      active: false,
    };
    this.bookFeeds.set(key, feed);
    this.startBookSubscription(feed);
    return feed;
  }

  public openTrades(key: string, symbol: string): TradeFeed {
    const existing = this.tradeFeeds.get(key);
    if (existing?.symbol === symbol) return existing;
    if (existing) this.closeTrades(key, existing);
    const feed: ActiveTradeFeed = {
      key,
      symbol,
      data: new BehaviorSubject<unknown | undefined>(undefined),
      error: new BehaviorSubject<string | undefined>(undefined),
      active: false,
    };
    this.tradeFeeds.set(key, feed);
    this.startTradeSubscription(feed);
    return feed;
  }

  public closeTrades(key: string, feed: TradeFeed) {
    const activeFeed = this.tradeFeeds.get(key);
    if (!activeFeed || activeFeed !== feed) return;
    activeFeed.subscription?.cancel();
    activeFeed.subscription = undefined;
    activeFeed.active = false;
    this.tradeFeeds.delete(key);
    activeFeed.data.complete();
    activeFeed.error.complete();
  }

  private cancelTradeSubscriptions(reason: string) {
    for (const feed of this.tradeFeeds.values()) {
      if (feed.subscription) {
        console.info(
          '[trades] UI unsubscribing from ' + feed.symbol + ' (' + reason + ')',
        );
        feed.subscription.cancel();
      }
      feed.subscription = undefined;
      feed.active = false;
      feed.data.next(undefined);
    }
  }

  private startTradeSubscriptions() {
    for (const feed of this.tradeFeeds.values())
      this.startTradeSubscription(feed);
  }

  private startTradeSubscription(feed: ActiveTradeFeed) {
    const socket = this.socket;
    const generation = this.lifecycleGeneration;
    if (!socket || feed.active || this.tradeFeeds.get(feed.key) !== feed)
      return;
    feed.active = true;
    let updatesSinceRequest = 0;
    let requestNext: (() => void) | undefined;
    socket
      .requestStream({ data: Buffer.from('trades:' + feed.symbol) })
      .subscribe({
        onSubscribe: (subscription) => {
          if (
            this.tradeFeeds.get(feed.key) !== feed ||
            this.socket !== socket ||
            this.lifecycleGeneration !== generation
          ) {
            subscription.cancel();
            return;
          }
          feed.subscription = subscription;
          requestNext = () => subscription.request(REQUEST_BATCH_SIZE);
          requestNext();
        },
        onNext: (payload) => {
          if (
            this.tradeFeeds.get(feed.key) !== feed ||
            this.socket !== socket ||
            this.lifecycleGeneration !== generation
          )
            return;
          feed.data.next(payload.data);
          feed.error.next(undefined);
          updatesSinceRequest += 1;
          if (updatesSinceRequest >= REQUEST_BATCH_SIZE) {
            updatesSinceRequest = 0;
            requestNext?.();
          }
        },
        onError: (error) => {
          if (this.tradeFeeds.get(feed.key) !== feed || this.socket !== socket)
            return;
          feed.subscription = undefined;
          feed.active = false;
          feed.error.next(error.message ?? 'Trades stream disconnected');
        },
        onComplete: () => {
          if (this.tradeFeeds.get(feed.key) !== feed || this.socket !== socket)
            return;
          feed.subscription = undefined;
          feed.active = false;
          feed.error.next('Trades stream completed');
        },
      });
  }

  public closeBook(key: string, feed: BookFeed) {
    const activeFeed = this.bookFeeds.get(key);
    if (!activeFeed || activeFeed !== feed) return;
    if (activeFeed.subscription) {
      console.info(
        '[book] UI unsubscribing from ' +
          activeFeed.symbol +
          ' (widget closed)',
      );
      activeFeed.subscription.cancel();
    }
    activeFeed.subscription = undefined;
    activeFeed.active = false;
    this.bookFeeds.delete(key);
    activeFeed.data.complete();
    activeFeed.error.complete();
  }

  private cancelBookSubscriptions(reason: string) {
    for (const feed of this.bookFeeds.values()) {
      if (feed.subscription) {
        console.info(
          '[book] UI unsubscribing from ' + feed.symbol + ' (' + reason + ')',
        );
        feed.subscription.cancel();
      }
      feed.subscription = undefined;
      feed.active = false;
      feed.data.next(undefined);
    }
  }

  private startBookSubscriptions() {
    for (const feed of this.bookFeeds.values())
      this.startBookSubscription(feed);
  }

  private startBookSubscription(feed: ActiveBookFeed) {
    const socket = this.socket;
    const generation = this.lifecycleGeneration;
    if (!socket || feed.active || this.bookFeeds.get(feed.key) !== feed) return;

    feed.active = true;
    let updatesSinceRequest = 0;
    let requestNext: (() => void) | undefined;
    socket
      .requestStream({
        data: Buffer.from('book:' + feed.symbol),
      })
      .subscribe({
        onSubscribe: (subscription) => {
          if (
            this.bookFeeds.get(feed.key) !== feed ||
            this.socket !== socket ||
            this.lifecycleGeneration !== generation
          ) {
            console.info(
              '[book] UI unsubscribing from ' +
                feed.symbol +
                ' (subscription became stale)',
            );
            subscription.cancel();
            return;
          }
          feed.subscription = subscription;
          requestNext = () => subscription.request(REQUEST_BATCH_SIZE);
          requestNext();
        },
        onNext: (payload) => {
          if (
            this.bookFeeds.get(feed.key) !== feed ||
            this.socket !== socket ||
            this.lifecycleGeneration !== generation
          )
            return;
          feed.data.next(payload.data);
          feed.error.next(undefined);
          updatesSinceRequest += 1;
          if (updatesSinceRequest >= REQUEST_BATCH_SIZE) {
            updatesSinceRequest = 0;
            requestNext?.();
          }
        },
        onError: (error) => {
          if (this.bookFeeds.get(feed.key) !== feed || this.socket !== socket)
            return;
          feed.subscription = undefined;
          feed.active = false;
          feed.error.next(error.message ?? 'Order book stream disconnected');
        },
        onComplete: () => {
          if (this.bookFeeds.get(feed.key) !== feed || this.socket !== socket)
            return;
          feed.subscription = undefined;
          feed.active = false;
          feed.error.next('Order book stream completed');
        },
      });
  }

  private async createClient() {
    const setupOptions = {
      keepAlive: 30000,
      lifetime: 120000,
      dataMimeType: 'application/octet-stream',
      metadataMimeType: 'text/plain',
    };

    // IMPORTANT: use the inner default.default class
    const websocketModule = WebSocketClient as unknown as {
      default: { default: typeof WebSocketClient.default };
    };
    const TransportClass = websocketModule.default.default;

    const encoders = {
      ...BufferEncoders,
      data: BufferDataEncoder,
    };
    const transport = new TransportClass(
      {
        url: this.url,
        wsCreator: (url: string) => new WebSocket(url),
      },
      encoders,
    );

    const client = new RSocketClient({
      setup: setupOptions,
      transport,
    });

    return await client.connect();
  }

  public dispose() {
    this.cancelled = true;
    this.lifecycleGeneration += 1;
    if (this.retryTimer !== undefined) {
      clearTimeout(this.retryTimer);
      this.retryTimer = undefined;
    }
    this.subscription?.cancel();
    for (const feed of this.bookFeeds.values()) this.closeBook(feed.key, feed);
    for (const feed of this.tradeFeeds.values())
      this.closeTrades(feed.key, feed);
    this.socket?.close();
  }
}
