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
let requestInstrumentList = requestInstrumentListOverSocket;

function getSocket() {
  socket ??= createSocket().catch((error) => {
    socket = undefined;
    throw error;
  });
  return socket;
}

async function createSocket() {
  const url =
    import.meta.env.VITE_RSOCKET_URL ??
    (import.meta.env.DEV ||
    (typeof window !== 'undefined' &&
      ['localhost', '127.0.0.1'].includes(window.location.hostname))
      ? 'ws://localhost:10001'
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
  const client = await getSocket();
  const data = await new Promise<Uint8Array>((resolve, reject) => {
    client
      .requestResponse({ data: Buffer.from('instruments:list') })
      .subscribe({
        onComplete: (payload) => resolve(payload.data as Uint8Array),
        onError: reject,
      });
  });
  return JSON.parse(Buffer.from(data).toString()) as unknown;
}

export function getInstrumentsOverRSocket(): Promise<unknown> {
  return requestInstrumentList();
}

export function setInstrumentListRequesterForTests(
  requester?: typeof requestInstrumentListOverSocket,
) {
  requestInstrumentList = requester ?? requestInstrumentListOverSocket;
}
