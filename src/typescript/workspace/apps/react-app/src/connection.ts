import { BehaviorSubject } from 'rxjs';
import {
  BufferEncoder,
  BufferEncoders,
  RSocketClient,
  type Encoder,
} from 'rsocket-core';

import * as WebSocketClient from 'rsocket-websocket-client';
import {
  ConnectionFeedManager,
  createBookFeed,
  createTradeFeed,
} from './connection_feed_manager';

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

export type SignalFeed = BookFeed;
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
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10001'
      : 'wss://sbe-ts.onrender.com');
  private cancelled: boolean = false;
  private lifecycleGeneration = 0;
  private socket: Awaited<ReturnType<typeof this.createClient>> | undefined;
  private subscription: { cancel: () => void } | undefined;
  private insightsSocket:
    | Awaited<ReturnType<typeof this.createClient>>
    | undefined;
  private insightsSubscription:
    | { cancel: () => void; request: (n: number) => void }
    | undefined;
  private insightsRetryTimer: ReturnType<typeof setTimeout> | undefined;
  private insightsRetryAttempt = 0;
  private readonly bookFeeds = this.createBookManager('book');
  private readonly tradeFeeds = this.createTradeManager();
  private readonly signalFeeds = this.createBookManager('signals');
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private retryAttempt = 0;

  public connection_state: BehaviorSubject<ConnectionState> =
    new BehaviorSubject<ConnectionState>('connecting');
  public error_message: BehaviorSubject<string | undefined> =
    new BehaviorSubject<string | undefined>(undefined);
  public insights_connection_state: BehaviorSubject<ConnectionState> =
    new BehaviorSubject<ConnectionState>('connecting');
  public insights_error_message: BehaviorSubject<string | undefined> =
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
      this.insights_connection_state.next('connecting');
      this.insights_error_message.next(undefined);
      this.subscription?.cancel();
      this.subscription = undefined;
      this.closeInsightsSocket();
      this.bookFeeds.cancelAll();
      this.tradeFeeds.cancelAll();
      this.signalFeeds.cancelAll();
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
            if (this.source === 'okx') void this.connectInsights(generation);
            this.bookFeeds.resumeAll();
            this.tradeFeeds.resumeAll();
            this.signalFeeds.resumeAll();
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
            if (this.source === 'synthetic') this.data.next(payload.data);
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
    return this.bookFeeds.open(key, symbol);
  }
  public closeBook(key: string, feed: BookFeed): void {
    this.bookFeeds.close(key, feed as BookFeed & { active: boolean });
  }
  public openSignals(key: string, symbol: string): SignalFeed {
    return this.signalFeeds.open(key, symbol);
  }
  public closeSignals(key: string, feed: SignalFeed): void {
    this.signalFeeds.close(key, feed as SignalFeed & { active: boolean });
  }
  public openTrades(key: string, symbol: string): TradeFeed {
    return this.tradeFeeds.open(key, symbol);
  }
  public closeTrades(key: string, feed: TradeFeed): void {
    this.tradeFeeds.close(key, feed as TradeFeed & { active: boolean });
  }
  private createBookManager(requestType: 'book' | 'signals') {
    return new ConnectionFeedManager({
      create: createBookFeed,
      request: (feed) => requestType + ':' + feed.symbol,
      demand: REQUEST_BATCH_SIZE,
      getSocket: () => this.socket as never,
      getGeneration: () => this.lifecycleGeneration,
    });
  }

  private createTradeManager() {
    return new ConnectionFeedManager({
      create: createTradeFeed,
      request: (feed) => 'trades:' + feed.symbol,
      demand: REQUEST_BATCH_SIZE,
      getSocket: () => this.socket as never,
      getGeneration: () => this.lifecycleGeneration,
    });
  }

  private async createClient() {
    return this.createClientForUrl(this.url);
  }

  private async createClientForUrl(url: string) {
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
        url,
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

  private async connectInsights(generation: number): Promise<void> {
    const url =
      import.meta.env.VITE_INSIGHTS_RSOCKET_URL ??
      (import.meta.env.DEV ||
      (typeof window !== 'undefined' &&
        ['localhost', '127.0.0.1'].includes(window.location.hostname))
        ? 'ws://localhost:10004'
        : 'wss://sbe-ts.onrender.com');
    try {
      const client = await this.createClientForUrl(url);
      if (
        this.cancelled ||
        generation !== this.lifecycleGeneration ||
        this.source !== 'okx'
      ) {
        client.close();
        return;
      }
      this.insightsSocket = client;
      client.requestStream({ data: Buffer.from('tickers:*') }).subscribe({
        onSubscribe: (sub) => {
          if (this.cancelled || generation !== this.lifecycleGeneration) {
            sub.cancel();
            return;
          }
          this.insightsSubscription = sub;
          this.insightsRetryAttempt = 0;
          this.insights_connection_state.next('connected');
          this.insights_error_message.next(undefined);
          sub.request(REQUEST_BATCH_SIZE);
        },
        onNext: (payload) => {
          if (this.cancelled || generation !== this.lifecycleGeneration) return;
          if (Buffer.isBuffer(payload.data)) {
            this.data.next(payload.data);
            this.insightsSubscription?.request(1);
          }
        },
        onError: (error) =>
          this.scheduleInsightsReconnect(
            generation,
            error.message ?? 'Insights RSocket stream disconnected',
          ),
        onComplete: () =>
          this.scheduleInsightsReconnect(
            generation,
            'Insights stream completed; reconnecting',
          ),
      });
    } catch (error) {
      this.scheduleInsightsReconnect(
        generation,
        error instanceof Error
          ? error.message
          : 'Unable to connect to Insights RSocket server',
      );
    }
  }

  private scheduleInsightsReconnect(generation: number, message: string): void {
    if (
      this.cancelled ||
      generation !== this.lifecycleGeneration ||
      this.insightsRetryTimer !== undefined
    )
      return;
    this.insights_error_message.next(message);
    this.insights_connection_state.next('reconnecting');
    this.closeInsightsSocket();
    const delay = Math.min(1000 * 2 ** this.insightsRetryAttempt, 30000);
    this.insightsRetryAttempt += 1;
    this.insightsRetryTimer = setTimeout(() => {
      this.insightsRetryTimer = undefined;
      if (
        generation === this.lifecycleGeneration &&
        !this.cancelled &&
        this.source === 'okx'
      )
        void this.connectInsights(generation);
    }, delay);
  }

  private closeInsightsSocket(): void {
    if (this.insightsRetryTimer !== undefined) {
      clearTimeout(this.insightsRetryTimer);
      this.insightsRetryTimer = undefined;
    }
    this.insightsSubscription?.cancel();
    this.insightsSubscription = undefined;
    this.insightsSocket?.close();
    this.insightsSocket = undefined;
  }

  public dispose() {
    this.cancelled = true;
    this.lifecycleGeneration += 1;
    if (this.retryTimer !== undefined) {
      clearTimeout(this.retryTimer);
      this.retryTimer = undefined;
    }
    this.subscription?.cancel();
    this.closeInsightsSocket();
    this.bookFeeds.closeAll();
    this.tradeFeeds.closeAll();
    this.signalFeeds.closeAll();
    this.socket?.close();
  }
}
