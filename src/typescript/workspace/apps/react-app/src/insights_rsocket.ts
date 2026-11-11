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

export type InsightsSocket = Awaited<ReturnType<typeof connectInsightsRSocket>>;

let sharedSocket: Promise<InsightsSocket> | undefined;
let requestInsightsRpc = performInsightsRpc;

export function setInsightsRpcRequesterForTests(
  requester?: typeof performInsightsRpc,
) {
  requestInsightsRpc = requester ?? performInsightsRpc;
}

function getSocket() {
  if (!sharedSocket) {
    sharedSocket = connectInsightsRSocket().catch((error) => {
      sharedSocket = undefined;
      throw error;
    });
  }
  return sharedSocket;
}

export async function connectInsightsRSocket() {
  const url =
    import.meta.env.VITE_INSIGHTS_BROWSER_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10011'
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

async function performInsightsRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  const client = await getSocket();
  const reply = await new Promise<Uint8Array>((resolve, reject) => {
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

export function insightsRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  return requestInsightsRpc(method, path, body);
}

export async function insightsRequestStream(
  route: string,
  onData: (data: Uint8Array) => void,
  onError: (error: Error) => void,
): Promise<{ cancel: () => void }> {
  let subscription:
    | { cancel: () => void; request: (n: number) => void }
    | undefined;
  let cancelled = false;
  try {
    const client = await getSocket();
    if (cancelled) return { cancel: () => undefined };
    client
      .requestStream({ data: Buffer.from(JSON.stringify({ route })) })
      .subscribe({
        onSubscribe: (value) => {
          if (cancelled) {
            value.cancel();
            return;
          }
          subscription = value;
          value.request(1_000_000);
        },
        onNext: (payload) => onData(payload.data as Uint8Array),
        onError: (error) => {
          sharedSocket = undefined;
          onError(error);
        },
        onComplete: () => onError(new Error('Insights stream completed')),
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

export function insightsFetch(
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
  return insightsRpc(
    init.method ?? 'GET',
    `${url.pathname}${url.search}`,
    body,
  );
}
