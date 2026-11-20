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

type LearningSocket = Awaited<ReturnType<typeof connectLearningRSocket>>;
let sharedSocket: Promise<LearningSocket> | undefined;
let socketGeneration = 0;
let requestLearning = performLearningRpc;

export function setLearningRpcRequesterForTests(
  requester?: typeof performLearningRpc,
) {
  requestLearning = requester ?? performLearningRpc;
}

function getSocket() {
  if (!sharedSocket) {
    const generation = ++socketGeneration;
    sharedSocket = connectLearningRSocket()
      .then((client) => {
        const invalidate = () => {
          if (socketGeneration === generation) sharedSocket = undefined;
          client.close();
        };
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
          onError: invalidate,
          onComplete: invalidate,
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

export async function connectLearningRSocket() {
  const url =
    import.meta.env.VITE_LEARNING_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10110'
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

async function performLearningRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  const socket = getSocket();
  const client = await socket;
  let reply: Uint8Array;
  try {
    reply = await new Promise<Uint8Array>((resolve, reject) => {
      client
        .requestResponse({
          data: Buffer.from(
            JSON.stringify({
              operation: 'learning_route',
              request: {
                request_id: `${Date.now()}:${Math.random()}`,
                strategy: null,
                method: method.toLowerCase(),
                path,
                payload: body ?? null,
              },
            }),
          ),
        })
        .subscribe({
          onComplete: (payload) => resolve(payload.data as Uint8Array),
          onError: reject,
        });
    });
  } catch (error) {
    // A failed request can come from a socket that closed between calls.
    // Invalidate only this generation so a newer connection isn't discarded.
    if (sharedSocket === socket) sharedSocket = undefined;
    client.close();
    throw error;
  }
  const envelope = JSON.parse(Buffer.from(reply).toString()) as {
    operation?: string;
    response?: { status: number; body: unknown };
  };
  if (envelope.operation !== 'learning_route' || !envelope.response)
    throw new Error('Learning returned an invalid Scout route response.');
  const { status, body: responseBody } = envelope.response;
  const text =
    typeof responseBody === 'string'
      ? responseBody
      : responseBody === null
        ? ''
        : JSON.stringify(responseBody);
  return new Response(text, {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function performScoutWorkerRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  const socket = getSocket();
  const client = await socket;
  let reply: Uint8Array;
  try {
    reply = await new Promise<Uint8Array>((resolve, reject) => {
      client
        .requestResponse({
          data: Buffer.from(
            JSON.stringify({
              operation: 'worker_route',
              request: {
                request_id: `${Date.now()}:${Math.random()}`,
                strategy: null,
                method: method.toLowerCase(),
                path,
                payload: body ?? null,
              },
            }),
          ),
        })
        .subscribe({
          onComplete: (payload) => resolve(payload.data as Uint8Array),
          onError: reject,
        });
    });
  } catch (error) {
    if (sharedSocket === socket) sharedSocket = undefined;
    client.close();
    throw error;
  }
  const envelope = JSON.parse(Buffer.from(reply).toString()) as {
    operation?: string;
    response?: { status: number; body: unknown };
  };
  if (envelope.operation !== 'worker_route' || !envelope.response)
    throw new Error(
      'Learning returned an invalid Scout worker route response.',
    );
  const { status, body: responseBody } = envelope.response;
  const text =
    typeof responseBody === 'string'
      ? responseBody
      : responseBody === null
        ? ''
        : JSON.stringify(responseBody);
  return new Response(text, {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

export function learningRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  // Dashboard reads are safe to repeat after a stale RSocket connection. Do
  // not retry mutations, whose outcome may be unknown when the socket closes.
  if (method.toUpperCase() !== 'GET')
    return requestLearning(method, path, body);
  return requestLearning(method, path, body).catch(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
    return requestLearning(method, path, body);
  });
}

export function scoutWorkerRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  if (method.toUpperCase() !== 'GET')
    return performScoutWorkerRpc(method, path, body);
  return performScoutWorkerRpc(method, path, body).catch(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
    return performScoutWorkerRpc(method, path, body);
  });
}
