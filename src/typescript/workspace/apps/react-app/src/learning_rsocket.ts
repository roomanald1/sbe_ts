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
let requestLearning = performLearningRpc;

export function setLearningRpcRequesterForTests(
  requester?: typeof performLearningRpc,
) {
  requestLearning = requester ?? performLearningRpc;
}

function getSocket() {
  if (!sharedSocket) {
    sharedSocket = connectLearningRSocket().catch((error) => {
      sharedSocket = undefined;
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
      ? 'ws://localhost:10010'
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
  const client = await getSocket();
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
    sharedSocket = undefined;
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

export function learningRpc(
  method: string,
  path: string,
  body?: unknown,
): Promise<Response> {
  return requestLearning(method, path, body);
}
