import {
  BufferEncoder,
  BufferEncoders,
  RSocketClient,
  type Encoder,
} from 'rsocket-core';
import * as WebSocketClient from 'rsocket-websocket-client';

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
let requestAutoTrader = performAutoTraderRpc;

export function setAutoTraderRpcRequesterForTests(
  requester?: typeof performAutoTraderRpc,
) {
  requestAutoTrader = requester ?? performAutoTraderRpc;
}

function getSocket() {
  if (!sharedSocket) {
    sharedSocket = connectAutoTraderRSocket().catch((error) => {
      sharedSocket = undefined;
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
      ? 'ws://localhost:10008'
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
  const client = await getSocket();
  let reply: Uint8Array;
  try {
    reply = await new Promise<Uint8Array>((resolve, reject) => {
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
          onComplete: (payload) => resolve(payload.data as Uint8Array),
          onError: reject,
        });
    });
  } catch (error) {
    sharedSocket = undefined;
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
  return requestAutoTrader(method, path, body);
}

export async function autoTraderRequestStream(
  route: string,
  onData: (data: Uint8Array) => void,
  onError: (error: Error) => void,
): Promise<{ cancel: () => void }> {
  let cancelled = false;
  let subscription:
    | { cancel: () => void; request: (n: number) => void }
    | undefined;
  try {
    const client = await getSocket();
    if (cancelled) return { cancel: () => undefined };
    client
      .requestStream({ data: Buffer.from(JSON.stringify({ route })) })
      .subscribe({
        onSubscribe: (nextSubscription) => {
          if (cancelled) {
            nextSubscription.cancel();
            return;
          }
          subscription = nextSubscription;
          nextSubscription.request(1);
        },
        onNext: (payload) => {
          onData(payload.data as Uint8Array);
          subscription?.request(1);
        },
        onError: (error) => {
          sharedSocket = undefined;
          onError(error instanceof Error ? error : new Error(String(error)));
        },
        onComplete: () => {
          sharedSocket = undefined;
          onError(new Error('AutoTrader RSocket stream completed'));
        },
      });
  } catch (error) {
    onError(error instanceof Error ? error : new Error(String(error)));
  }
  return {
    cancel: () => {
      cancelled = true;
      subscription?.cancel();
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
