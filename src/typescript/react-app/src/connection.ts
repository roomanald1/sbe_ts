import { BehaviorSubject } from "rxjs";
import { BufferEncoders, RSocketClient } from 'rsocket-core';
import { Buffer } from 'buffer';

import * as WebSocketClient from 'rsocket-websocket-client';

export type ConnectionState = 'connecting' | 'connected' | 'reconnecting';
type ClientOptions = {
    host: string;
    port: number;
};

export class Connection {
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
            const rsocket = await this.createClient({
                host: 'sbe-ts.onrender.com',
                port: 10000,
            });

            if (this.cancelled) {
                rsocket.close();
                return;
            }
            this.socket = rsocket;
            let requestNext: (() => void) | undefined;

            rsocket.requestStream({
                data: Buffer.alloc(0),
            }).subscribe({
                onSubscribe: (sub) => {
                    console.log("OnSubscribe");
                    this.retryAttempt = 0;
                    this.subscription = sub;
                    this.connection_state.next('connected');
                    this.error_message.next(undefined);
                    requestNext = () => {
                        sub.request(1)
                    };
                    requestNext();
                },
                onNext: (payload) => {
                    this.data.next(payload.data)
                    requestNext?.();
                },
                onError: (error) => scheduleReconnect(error.message ?? 'RSocket stream disconnected'),
                onComplete: () => scheduleReconnect('Stream completed; reconnecting'),
            });

        } catch (error) {
            scheduleReconnect(error instanceof Error ? error.message : 'Unable to connect to RSocket server');
        }
    }


    private async createClient(options: ClientOptions) {
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

        const transport = new TransportClass({
            url: `wss://${options.host}/rsocket`,
            wsCreator: (url: string) => new WebSocket(url),
        }, BufferEncoders);

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

