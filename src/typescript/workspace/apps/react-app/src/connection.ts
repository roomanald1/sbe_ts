import { BehaviorSubject } from "rxjs";
import { BufferEncoder, BufferEncoders, RSocketClient, type Encoder } from 'rsocket-core';

import * as WebSocketClient from 'rsocket-websocket-client';

export type ConnectionState = 'connecting' | 'connected' | 'reconnecting';
export type FeedSource = 'okx' | 'synthetic';

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

    public readonly url = import.meta.env.VITE_RSOCKET_URL ?? (import.meta.env.DEV
        ? 'ws://localhost:10000'
        : 'wss://sbe-ts.onrender.com');
    private cancelled: boolean = false;
    private lifecycleGeneration = 0;
    private socket: Awaited<ReturnType<typeof this.createClient>> | undefined;
    private subscription: { cancel: () => void } | undefined;
    private bookSubscription: { cancel: () => void } | undefined;
    private selectedBook: string | undefined;
    private activeBook: string | undefined;
    private retryTimer: ReturnType<typeof setTimeout> | undefined;
    private retryAttempt = 0;

    public connection_state: BehaviorSubject<ConnectionState> = new BehaviorSubject<ConnectionState>('connecting');
    public error_message: BehaviorSubject<string | undefined> = new BehaviorSubject<string | undefined>(undefined);
    public data: BehaviorSubject<unknown | undefined> = new BehaviorSubject<unknown | undefined>(undefined);
    public book_data: BehaviorSubject<unknown | undefined> = new BehaviorSubject<unknown | undefined>(undefined);
    public book_error: BehaviorSubject<string | undefined> = new BehaviorSubject<string | undefined>(undefined);

    public async connect() {
        if (this.socket) return;//already connected

        this.cancelled = false;
        const generation = ++this.lifecycleGeneration;
        console.log("Connecting")
        let requestNext: (() => void) | undefined;
        let updatesSinceRequest = 0;
        const scheduleReconnect = (message: string) => {
            if (this.cancelled || generation !== this.lifecycleGeneration || this.retryTimer !== undefined) return;

            this.connection_state.next('reconnecting');
            this.error_message.next(message);
            this.subscription?.cancel();
            this.subscription = undefined;
            this.cancelBookSubscription('connection reconnecting');
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

        this.connection_state.next(this.retryAttempt === 0 ? 'connecting' : 'reconnecting');
        try {
            const rsocket = await this.createClient();

            if (this.cancelled || generation !== this.lifecycleGeneration) {
                rsocket.close();
                return;
            }
            this.socket = rsocket;

            rsocket.requestStream({
                data: Buffer.from(this.source),
            }).subscribe({
                onSubscribe: (sub) => {
                    if (this.cancelled || generation !== this.lifecycleGeneration) {
                        sub.cancel();
                        return;
                    }
                    console.log("OnSubscribe");
                    this.retryAttempt = 0;
                    this.subscription = sub;
                    this.connection_state.next('connected');
                    this.error_message.next(undefined);
                    this.startBookSubscription();
                    if (this.source === 'synthetic') {
                        requestNext = () => sub.request(REQUEST_BATCH_SIZE);
                        sub.request(1000);
                    } else {
                        requestNext = () => sub.request(REQUEST_BATCH_SIZE);
                        requestNext();
                    }
                },
                onNext: (payload) => {
                    if (this.cancelled || generation !== this.lifecycleGeneration) return;
                    this.data.next(payload.data)
                    updatesSinceRequest += 1;
                    if (updatesSinceRequest >= REQUEST_BATCH_SIZE) {
                        updatesSinceRequest = 0;
                        requestNext?.();
                    }
                },
                onError: (error) => scheduleReconnect(error.message ?? 'RSocket stream disconnected'),
                onComplete: () => scheduleReconnect('Stream completed; reconnecting'),
            });

        } catch (error) {
            scheduleReconnect(error instanceof Error ? error.message : 'Unable to connect to RSocket server');
        }
    }

    public selectBook(instrument: string | undefined) {
        if (this.selectedBook === instrument) return;

        this.selectedBook = instrument;
        this.cancelBookSubscription(instrument ? 'symbol changed to ' + instrument : 'book view closed');
        this.book_data.next(undefined);
        this.book_error.next(undefined);
        this.startBookSubscription();
    }

    private startBookSubscription() {
        const socket = this.socket;
        const instrument = this.selectedBook;
        if (!socket || !instrument || this.activeBook === instrument) return;

        this.activeBook = instrument;
        let updatesSinceRequest = 0;
        let requestNext: (() => void) | undefined;
        socket.requestStream({
            data: Buffer.from(`book:${instrument}`),
        }).subscribe({
            onSubscribe: subscription => {
                if (this.selectedBook !== instrument) {
                    console.info('[book] UI unsubscribing from ' + instrument + ' (selection changed before subscribe completed)');
                    subscription.cancel();
                    return;
                }
                this.bookSubscription = subscription;
                requestNext = () => subscription.request(REQUEST_BATCH_SIZE);
                requestNext();
            },
            onNext: payload => {
                if (this.selectedBook === instrument) {
                    this.book_data.next(payload.data);
                    this.book_error.next(undefined);
                    updatesSinceRequest += 1;
                    if (updatesSinceRequest >= REQUEST_BATCH_SIZE) {
                        updatesSinceRequest = 0;
                        requestNext?.();
                    }
                }
            },
            onError: error => {
                if (this.selectedBook !== instrument) return;
                this.bookSubscription = undefined;
                this.activeBook = undefined;
                this.book_error.next(error.message ?? 'Order book stream disconnected');
            },
            onComplete: () => {
                if (this.selectedBook !== instrument) return;
                this.bookSubscription = undefined;
                this.activeBook = undefined;
                this.book_error.next('Order book stream completed');
            },
        });
    }


    private cancelBookSubscription(reason: string) {
        if (this.bookSubscription) {
            console.info('[book] UI unsubscribing from ' + (this.activeBook ?? this.selectedBook ?? 'unknown symbol') + ' (' + reason + ')');
            this.bookSubscription.cancel();
        }
        this.bookSubscription = undefined;
        this.activeBook = undefined;
    }

    private async createClient() {
        const setupOptions = {
            keepAlive: 30000,
            lifetime: 120000,
            dataMimeType: 'application/octet-stream',
            metadataMimeType: 'text/plain'
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
        const transport = new TransportClass({
            url: this.url,
            wsCreator: (url: string) => new WebSocket(url),
        }, encoders);

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
        this.cancelBookSubscription('connection disposed');
        this.socket?.close();
    }
}
