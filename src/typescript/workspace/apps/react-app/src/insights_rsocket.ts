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
let socketGeneration = 0;
let requestInsightsRpc = performInsightsRpc;

export function setInsightsRpcRequesterForTests(
  requester?: typeof performInsightsRpc,
) {
  requestInsightsRpc = requester ?? performInsightsRpc;
}

function getSocket() {
  if (!sharedSocket) {
    const generation = ++socketGeneration;
    sharedSocket = connectInsightsRSocket()
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
              client.close();
            }
          },
          onError: () => {
            if (socketGeneration === generation) sharedSocket = undefined;
            client.close();
          },
        });
        return client;
      })
      .catch((error) => {
        if (socketGeneration === generation) sharedSocket = undefined;
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
      ? 'ws://localhost:10111'
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
  const clientPromise = getSocket();
  const client = await clientPromise;
  const reply = await new Promise<Uint8Array>((resolve, reject) => {
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
          // A request can fail before the connection status observer reports
          // CLOSED. Drop this client so the next GET retry creates a fresh one.
          if (sharedSocket === clientPromise) sharedSocket = undefined;
          client.close();
          reject(error);
        },
      });
    timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      // Cancel only this request. The RSocket client is shared with
      // long-lived subscriptions, so a request timeout must not close it.
      cancelRequest?.();
      reject(new Error(`Insights ${method} ${path} timed out after 5000 ms`));
    }, 5_000);
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
  const request = requestInsightsRpc;
  const isAuditHistoryRead =
    method.toUpperCase() === 'GET' &&
    path.split('?', 1)[0] === '/signals/auto-trader-audit';
  // A timed-out audit scan can keep running in the server's blocking SQLite
  // worker. Retrying it would start duplicate scans against the same database.
  const attempts =
    method.toUpperCase() === 'GET' &&
    request === performInsightsRpc &&
    !isAuditHistoryRead
      ? 3
      : 1;
  const run = async (): Promise<Response> => {
    let lastError: unknown;
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      try {
        return await request(method, path, body);
      } catch (error) {
        lastError = error;
        if (attempt + 1 < attempts)
          await new Promise((resolve) =>
            setTimeout(resolve, 250 * (attempt + 1)),
          );
      }
    }
    throw lastError;
  };
  return run();
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
  let connectedClient: InsightsSocket | undefined;
  const socket = getSocket();
  try {
    const client = await socket;
    connectedClient = client;
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
          // Invalidate this socket so the next attempt creates a new client.
          if (sharedSocket === socket) sharedSocket = undefined;
          client.close();
          onError(error);
        },
        onComplete: () => {
          if (sharedSocket === socket) sharedSocket = undefined;
          client.close();
          onError(new Error('Insights stream completed'));
        },
      });
  } catch (error) {
    connectedClient?.close();
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
