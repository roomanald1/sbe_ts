import { BehaviorSubject } from 'rxjs';
import {
  connectPositionRSocket,
  resetPositionRSocket,
  type PositionSocket,
} from './position_rsocket';
import type { Position, PositionTrade } from './models/PositionTypes';
import type { OrdersResponse } from './models/OrderManagerTypes';

export type PositionUiSnapshot = {
  positions: { positions: Position[]; trade_count: number };
  auto_positions: { positions: Position[]; trade_count: number };
  orders: OrdersResponse;
  trades: { trades: PositionTrade[] };
  auto_trades: { trades: PositionTrade[] };
  usd_rates?: { rates?: Record<string, number> } | null;
  account?: Record<string, unknown> | null;
  fee_rates?: { maker: number; taker: number };
  vwap_runs?: { runs: Array<Record<string, unknown>> };
  hedge_candidates?: {
    positions: Array<
      Position & {
        latest_price: number | null;
        candidates: Array<{
          symbol: string;
          correlation: number | null;
          paired_samples: number;
          latest_price: number | null;
        }>;
      }
    >;
    pair_correlations: Array<{
      symbol_a: string;
      symbol_b: string;
      correlation: number | null;
    }>;
  };
};

const snapshot$ = new BehaviorSubject<PositionUiSnapshot | undefined>(
  undefined,
);
let socket: PositionSocket | undefined;
let streamSubscription:
  | { cancel: () => void; request: (n: number) => void }
  | undefined;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let retryDelay = 1000;
let subscribers = 0;
let firstSnapshot: Promise<PositionUiSnapshot> | undefined;
let resolveFirstSnapshot: ((snapshot: PositionUiSnapshot) => void) | undefined;

function connect() {
  if (socket || subscribers === 0 || typeof window === 'undefined') return;
  void connectPositionRSocket()
    .then((next) => {
      if (!subscribers) {
        resetPositionRSocket(next);
        return;
      }
      socket = next;
      retryDelay = 1000;
      next.requestStream({ data: Buffer.from('position-ui:*') }).subscribe({
        onSubscribe: (subscription) => {
          if (!subscribers) {
            subscription.cancel();
            next.close();
            return;
          }
          streamSubscription = subscription;
          subscription.request(32);
        },
        onNext: (payload) => {
          try {
            const data = payload.data;
            if (!(data instanceof Uint8Array)) {
              streamSubscription?.request(1);
              return;
            }
            const message = JSON.parse(Buffer.from(data).toString()) as
              | { type: 'snapshot'; state: PositionUiSnapshot }
              | {
                  type: 'delta';
                  changes: Partial<PositionUiSnapshot>;
                  auto_trade_appends?: PositionTrade[];
                };
            let value: PositionUiSnapshot | undefined;
            if (message.type === 'snapshot') {
              value = message.state;
            } else if (message.type === 'delta' && snapshot$.value) {
              value = { ...snapshot$.value, ...message.changes };
              if (message.auto_trade_appends?.length) {
                value.auto_trades = {
                  ...value.auto_trades,
                  trades: [
                    ...value.auto_trades.trades,
                    ...message.auto_trade_appends,
                  ].slice(-256),
                };
              }
            }
            if (!value) return;
            if (
              Array.isArray(value.positions?.positions) &&
              Array.isArray(value.auto_positions?.positions) &&
              Array.isArray(value.orders?.orders) &&
              Array.isArray(value.trades?.trades) &&
              Array.isArray(value.auto_trades?.trades)
            ) {
              snapshot$.next(value);
              resolveFirstSnapshot?.(value);
              resolveFirstSnapshot = undefined;
            }
          } catch {
            /* Ignore malformed snapshots and keep the last good state. */
          }
          streamSubscription?.request(1);
        },
        onError: () => {
          if (socket === next) socket = undefined;
          streamSubscription = undefined;
          resetPositionRSocket(next);
          scheduleReconnect();
        },
        onComplete: () => {
          if (socket === next) socket = undefined;
          streamSubscription = undefined;
          resetPositionRSocket(next);
          scheduleReconnect();
        },
      });
    })
    .catch(() => scheduleReconnect());
}

function scheduleReconnect() {
  if (subscribers === 0 || retryTimer) return;
  retryTimer = setTimeout(() => {
    retryTimer = undefined;
    connect();
  }, retryDelay);
  retryDelay = Math.min(retryDelay * 2, 30_000);
}

export function subscribePositionStream(
  observer: (snapshot: PositionUiSnapshot) => void,
): () => void {
  subscribers += 1;
  const subscription = snapshot$.subscribe((snapshot) => {
    if (snapshot) observer(snapshot);
  });
  connect();
  return () => {
    subscription.unsubscribe();
    subscribers = Math.max(0, subscribers - 1);
    if (subscribers === 0) {
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = undefined;
      streamSubscription?.cancel();
      streamSubscription = undefined;
      socket = undefined;
    }
  };
}

export function currentPositionSnapshot(): PositionUiSnapshot | undefined {
  return snapshot$.value;
}

export function waitForPositionSnapshot(
  timeoutMs = 5000,
): Promise<PositionUiSnapshot> {
  if (snapshot$.value) return Promise.resolve(snapshot$.value);
  if (!firstSnapshot) {
    firstSnapshot = new Promise<PositionUiSnapshot>((resolve) => {
      resolveFirstSnapshot = resolve;
    });
  }
  return Promise.race([
    firstSnapshot,
    new Promise<PositionUiSnapshot>((_, reject) =>
      setTimeout(
        () => reject(new Error('Position stream unavailable')),
        timeoutMs,
      ),
    ),
  ]);
}
