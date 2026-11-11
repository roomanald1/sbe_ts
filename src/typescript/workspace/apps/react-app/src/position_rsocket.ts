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

export type PositionSocket = Awaited<ReturnType<typeof createRSocket>>;

const sharedSockets = new Map<string, Promise<PositionSocket>>();

function positionUrl() {
  return (
    import.meta.env.VITE_POSITION_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10005'
      : 'wss://sbe-ts.onrender.com')
  );
}

function executionUrl() {
  return (
    import.meta.env.VITE_EXECUTION_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10006'
      : 'wss://sbe-ts.onrender.com')
  );
}

function getSocket(url: string) {
  let socket = sharedSockets.get(url);
  if (!socket) {
    socket = createRSocket(url).catch((error) => {
      sharedSockets.delete(url);
      throw error;
    });
    sharedSockets.set(url, socket);
  }
  return socket;
}

async function createRSocket(url: string) {
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

export function connectPositionRSocket() {
  return getSocket(positionUrl());
}

export function connectExecutionRSocket() {
  return getSocket(executionUrl());
}

export function resetPositionRSocket(client: PositionSocket) {
  for (const [url, pending] of sharedSockets) {
    void pending.then((current) => {
      if (current === client) {
        sharedSockets.delete(url);
        client.close();
      }
    });
  }
}

export async function positionRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  // Models are exercised in Node without a browser socket; keep their fetch mocks usable.
  if (typeof window === 'undefined') {
    return fetch(
      path,
      method === 'GET' && body === undefined
        ? undefined
        : {
            method,
            ...(body === undefined ? {} : { body: JSON.stringify(body) }),
          },
    );
  }
  return rpcOverSocket(await connectPositionRSocket(), method, path, body);
}

export async function executionRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  if (typeof window === 'undefined') {
    return fetch(
      path,
      method === 'GET' && body === undefined
        ? undefined
        : {
            method,
            ...(body === undefined ? {} : { body: JSON.stringify(body) }),
          },
    );
  }
  return rpcOverSocket(await connectExecutionRSocket(), method, path, body);
}

async function rpcOverSocket(
  client: PositionSocket,
  method: string,
  path: string,
  body?: unknown,
) {
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
    resetPositionRSocket(client);
    throw error;
  }
  const response = JSON.parse(Buffer.from(reply).toString()) as {
    status: number;
    body: unknown;
  };
  const text =
    typeof response.body === 'string'
      ? response.body
      : JSON.stringify(response.body);
  const init = {
    status: response.status,
    headers: { 'content-type': 'application/json' },
  };
  return [204, 205, 304].includes(response.status)
    ? new Response(null, init)
    : new Response(text, init);
}
