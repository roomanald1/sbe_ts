import {
  BufferEncoder,
  BufferEncoders,
  RSocketClient,
  type Encoder,
} from 'rsocket-core';
import * as WebSocketClient from 'rsocket-websocket-client';

const AUTO_TRADER_RPC_TIMEOUT_MS = 5_000;
const AUTO_TRADER_SNAPSHOT_TIMEOUT_MS = 10_000;
const AUTO_TRADER_CONNECT_TIMEOUT_MS = 8_000;

const BufferDataEncoder: Encoder<Buffer> = {
  ...BufferEncoder,
  encode: (value, buffer, start, end) => {
    if (!Buffer.isBuffer(value))
      throw new TypeError('RSocket data must be a Buffer');
    buffer.set(value, start);
    return end;
  },
};

type AutoTraderSocket = Awaited<ReturnType<typeof connectAutoTraderRSocket>>;

let sharedSocket: Promise<AutoTraderSocket> | undefined;
let socketConnector: () => Promise<AutoTraderSocket> = connectAutoTraderRSocket;
let socketGeneration = 0;
let requestAutoTrader = performAutoTraderRpc;

export function setAutoTraderRpcRequesterForTests(
  requester?: typeof performAutoTraderRpc,
) {
  requestAutoTrader = requester ?? performAutoTraderRpc;
}

export function setAutoTraderSocketFactoryForTests(
  factory?: () => Promise<unknown>,
) {
  socketConnector = factory
    ? async () => (await factory()) as AutoTraderSocket
    : connectAutoTraderRSocket;
  socketGeneration += 1;
  sharedSocket = undefined;
}

function closeAutoTraderSocket(client: AutoTraderSocket | undefined) {
  try {
    client?.close?.();
  } catch {
    // Preserve the request/connection error that caused invalidation.
  }
}

function getSocket() {
  if (!sharedSocket) {
    const generation = ++socketGeneration;
    const connecting = socketConnector();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timedConnection = new Promise<AutoTraderSocket>((resolve, reject) => {
      timer = setTimeout(() => {
        if (socketGeneration === generation) sharedSocket = undefined;
        reject(
          new Error(
            `AutoTrader RSocket connection timed out after ${AUTO_TRADER_CONNECT_TIMEOUT_MS} ms`,
          ),
        );
      }, AUTO_TRADER_CONNECT_TIMEOUT_MS);
      connecting.then(
        (client) => {
          if (timer !== undefined) clearTimeout(timer);
          resolve(client);
        },
        (error) => {
          if (timer !== undefined) clearTimeout(timer);
          reject(error);
        },
      );
    });
    sharedSocket = timedConnection
      .then((client) => {
        client.connectionStatus().subscribe({
          onSubscribe: (subscription) =>
            subscription.request(Number.MAX_SAFE_INTEGER),
          onNext: (status) => {
            if (
              (status.kind === 'CLOSED' || status.kind === 'ERROR') &&
              socketGeneration === generation
            ) {
              sharedSocket = undefined;
              closeAutoTraderSocket(client);
            }
          },
          onError: () => {
            if (socketGeneration === generation) sharedSocket = undefined;
            closeAutoTraderSocket(client);
          },
        });
        return client;
      })
      .catch((error) => {
        if (socketGeneration === generation) {
          sharedSocket = undefined;
        }
        throw error;
      });
  }
  return sharedSocket;
}

export async function connectAutoTraderRSocket() {
  const url =
    import.meta.env.VITE_AUTO_TRADER_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10009'
      : 'wss://sbe-ts.onrender.com');
  const websocketModule = WebSocketClient as unknown as {
    default: { default: typeof WebSocketClient.default };
  };
  const TransportClass = websocketModule.default.default;
  const transport = new TransportClass(
    { url, wsCreator: (socketUrl: string) => new WebSocket(socketUrl) },
    { ...BufferEncoders, data: BufferDataEncoder },
  );
  return new RSocketClient({
    setup: {
      keepAlive: 30_000,
      lifetime: 120_000,
      dataMimeType: 'application/octet-stream',
      metadataMimeType: 'text/plain',
    },
    transport,
  }).connect();
}

async function performAutoTraderRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  const socket = getSocket();
  const client = await socket;
  let reply: Uint8Array;
  try {
    reply = await new Promise<Uint8Array>((resolve, reject) => {
      let settled = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let cancelRequest: (() => void) | undefined;
      client
        .requestResponse({
          data: Buffer.from(
            JSON.stringify({
              method,
              path,
              ...(body === undefined ? {} : { body }),
            }),
          ),
        })
        .subscribe({
          onSubscribe: (subscription) => {
            cancelRequest = subscription;
            if (settled) subscription();
          },
          onComplete: (payload) => {
            if (settled) return;
            settled = true;
            if (timer !== undefined) clearTimeout(timer);
            cancelRequest = undefined;
            resolve(payload.data as Uint8Array);
          },
          onError: (error) => {
            if (settled) return;
            settled = true;
            if (timer !== undefined) clearTimeout(timer);
            cancelRequest = undefined;
            reject(error);
          },
        });
      timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        cancelRequest?.();
        reject(
          new Error(
            `AutoTrader ${method} ${path} timed out after ${AUTO_TRADER_RPC_TIMEOUT_MS} ms`,
          ),
        );
      }, AUTO_TRADER_RPC_TIMEOUT_MS);
    });
  } catch (error) {
    if (sharedSocket === socket) sharedSocket = undefined;
    closeAutoTraderSocket(client);
    throw error;
  }
  const response = JSON.parse(Buffer.from(reply).toString()) as {
    status: number;
    body: unknown;
  };
  const text =
    typeof response.body === 'string'
      ? response.body
      : response.body === null
        ? ''
        : JSON.stringify(response.body);
  return new Response(text, {
    status: response.status,
    headers: { 'content-type': 'application/json' },
  });
}

export function autoTraderRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  const request = requestAutoTrader;
  return request(method, path, body).catch(async (error: unknown) => {
    // GETs are safe to retry. A transiently closed browser socket otherwise
    // leaves every panel in an unavailable state until its next UI refresh.
    // Writes must remain single-shot because a lost response cannot tell us
    // whether the server applied the command.
    if (method.toUpperCase() !== 'GET' || request !== performAutoTraderRpc) {
      throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
    return request(method, path, body);
  });
}

export async function autoTraderRequestStream(
  route: string,
  onData: (data: Uint8Array) => void,
  onError: (error: Error) => void,
): Promise<{ cancel: () => void }> {
  let cancelled = false;
  let connectionSubscription:
    | { cancel: () => void; request: (n: number) => void }
    | undefined;
  let cancelRequest: (() => void) | undefined;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let pollTimeout: ReturnType<typeof setTimeout> | undefined;
  let socketPromise: Promise<AutoTraderSocket> | undefined;
  let connectedClient: AutoTraderSocket | undefined;
  const fail = (error: Error) => {
    if (cancelled) return;
    cancelled = true;
    if (sharedSocket === socketPromise) sharedSocket = undefined;
    closeAutoTraderSocket(connectedClient);
    if (pollTimer !== undefined) clearTimeout(pollTimer);
    if (pollTimeout !== undefined) clearTimeout(pollTimeout);
    cancelRequest?.();
    connectionSubscription?.cancel();
    onError(error);
  };
  try {
    socketPromise = getSocket();
    const client = await socketPromise;
    connectedClient = client;
    if (cancelled) return { cancel: () => undefined };
    client.connectionStatus().subscribe({
      onSubscribe: (nextSubscription) => {
        if (cancelled) {
          nextSubscription.cancel();
          return;
        }
        connectionSubscription = nextSubscription;
        nextSubscription.request(Number.MAX_SAFE_INTEGER);
      },
      onNext: (status) => {
        if (status.kind === 'CLOSED' || status.kind === 'ERROR') {
          fail(new Error('AutoTrader RSocket connection closed'));
        }
      },
    });
    if (cancelled) return { cancel: () => undefined };
    const poll = () => {
      if (cancelled) return;
      try {
        pollTimeout = setTimeout(() => {
          fail(new Error('AutoTrader snapshot request timed out'));
        }, AUTO_TRADER_SNAPSHOT_TIMEOUT_MS);
        client
          .requestResponse({ data: Buffer.from(JSON.stringify({ route })) })
          .subscribe({
            onSubscribe: (subscription) => {
              cancelRequest = subscription;
              if (cancelled) subscription();
            },
            onComplete: (payload) => {
              if (cancelled) return;
              if (pollTimeout !== undefined) clearTimeout(pollTimeout);
              pollTimeout = undefined;
              cancelRequest = undefined;
              onData(payload.data as Uint8Array);
              pollTimer = setTimeout(poll, 15_000);
            },
            onError: (error) => {
              if (pollTimeout !== undefined) clearTimeout(pollTimeout);
              pollTimeout = undefined;
              cancelRequest = undefined;
              fail(error instanceof Error ? error : new Error(String(error)));
            },
          });
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)));
      }
    };
    poll();
  } catch (error) {
    fail(error instanceof Error ? error : new Error(String(error)));
  }
  return {
    cancel: () => {
      cancelled = true;
      if (pollTimer !== undefined) clearTimeout(pollTimer);
      if (pollTimeout !== undefined) clearTimeout(pollTimeout);
      cancelRequest?.();
      connectionSubscription?.cancel();
    },
  };
}

export function autoTraderFetch(
  input: string,
  init: RequestInit = {},
): Promise<Response> {
  const baseUrl =
    typeof window === 'undefined' ? 'http://localhost' : window.location.href;
  const url = new URL(input, baseUrl);
  let body: unknown;
  if (init.body !== undefined && init.body !== null) {
    const raw = typeof init.body === 'string' ? init.body : String(init.body);
    try {
      body = JSON.parse(raw) as unknown;
    } catch {
      body = raw;
    }
  }
  return autoTraderRpc(
    init.method ?? 'GET',
    `${url.pathname}${url.search}`,
    body,
  );
}
