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

type FeedHandlersSocket = Awaited<ReturnType<typeof createSocket>>;
let socket: Promise<FeedHandlersSocket> | undefined;
let socketGeneration = 0;
let requestInstrumentList = requestInstrumentListOverSocket;

function getSocket() {
  if (!socket) {
    const generation = ++socketGeneration;
    socket = createSocket()
      .then((client) => {
        client.connectionStatus().subscribe({
          onSubscribe: (subscription) =>
            subscription.request(Number.MAX_SAFE_INTEGER),
          onNext: (status) => {
            if (
              (status.kind === 'CLOSED' || status.kind === 'ERROR') &&
              socketGeneration === generation
            ) {
              socket = undefined;
              client.close();
            }
          },
          onError: () => {
            if (socketGeneration === generation) socket = undefined;
            client.close();
          },
        });
        return client;
      })
      .catch((error) => {
        if (socketGeneration === generation) socket = undefined;
        throw error;
      });
  }
  return socket;
}

async function createSocket() {
  const url =
    import.meta.env.VITE_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10101'
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

async function requestInstrumentListOverSocket(): Promise<unknown> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const currentSocket = getSocket();
    const client = await currentSocket;
    try {
      const data = await new Promise<Uint8Array>((resolve, reject) => {
        let settled = false;
        let timer: ReturnType<typeof setTimeout> | undefined;
        let cancelRequest: (() => void) | undefined;
        client
          .requestResponse({ data: Buffer.from('instruments:list') })
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
          reject(new Error('Feed handler instruments request timed out'));
        }, 5_000);
      });
      return JSON.parse(Buffer.from(data).toString()) as unknown;
    } catch (error) {
      if (socket === currentSocket) socket = undefined;
      client.close();
      if (attempt === 1) throw error;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  throw new Error('Feed handler instruments request failed');
}

export function getInstrumentsOverRSocket(): Promise<unknown> {
  return requestInstrumentList();
}

export function setInstrumentListRequesterForTests(
  requester?: typeof requestInstrumentListOverSocket,
) {
  requestInstrumentList = requester ?? requestInstrumentListOverSocket;
}
