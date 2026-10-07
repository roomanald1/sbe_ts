import { BehaviorSubject } from "rxjs";
import { BufferEncoder, BufferEncoders, RSocketClient, type Encoder } from 'rsocket-core';

import * as WebSocketClient from 'rsocket-websocket-client';

export type ConnectionState = 'connecting' | 'connected' | 'reconnecting';
export type FeedSource = 'okx' | 'synthetic';

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
    private socket: Awaited<ReturnType<typeof this.createClient>> | undefined;
    private subscription: { cancel: () => void } | undefined;
    private retryTimer: ReturnType<typeof setTimeout> | undefined;
    private retryAttempt = 0;

    public connection_state: BehaviorSubject<ConnectionState> = new BehaviorSubject<ConnectionState>('connecting');
    public error_message: BehaviorSubject<String | undefined> = new BehaviorSubject<String | undefined>(undefined);
    public data: BehaviorSubject<unknown | undefined> = new BehaviorSubject<unknown | undefined>(undefined);

    public async connect() {
        if (this.socket) return;//already connected

        console.log("Connecting")
        let requestNext: (() => void) | undefined;
        let syntheticPending = 0;
        const scheduleReconnect = (message: string) => {
            if (this.cancelled || this.retryTimer !== undefined) return;

            this.connection_state.next('reconnecting');
            this.error_message.next(message);
            this.subscription?.cancel();
            this.subscription = undefined;
            this.socket?.close();
            this.socket = undefined;

            const delay = Math.min(1000 * 2 ** this.retryAttempt, 30000);
            this.retryAttempt += 1;
            this.retryTimer = setTimeout(() => {
                this.retryTimer = undefined;
                void this.connect();
            }, delay);
        };

        this.connection_state.next(this.retryAttempt === 0 ? 'connecting' : 'reconnecting');
        try {
            const rsocket = await this.createClient();

            if (this.cancelled) {
                rsocket.close();
                return;
            }
            this.socket = rsocket;

            rsocket.requestStream({
                data: Buffer.from(this.source),
            }).subscribe({
                onSubscribe: (sub) => {
                    console.log("OnSubscribe");
                    this.retryAttempt = 0;
                    this.subscription = sub;
                    this.connection_state.next('connected');
                    this.error_message.next(undefined);
                    if (this.source === 'synthetic') {
                        requestNext = () => sub.request(500);
                        sub.request(1000);
                    } else {
                        requestNext = () => sub.request(1);
                        requestNext();
                    }
                },
                onNext: (payload) => {
                    this.data.next(payload.data)
                    if (this.source === 'synthetic') {
                        syntheticPending += 1;
                        if (syntheticPending >= 500) {
                            syntheticPending = 0;
                            requestNext?.();
                        }
                    } else {
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
        if (this.retryTimer !== undefined) {
            clearTimeout(this.retryTimer);
            this.retryTimer = undefined;
        }
        this.subscription?.cancel();
        try { this.socket?.close(); } catch { }
    }
}
