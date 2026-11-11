import { BehaviorSubject } from 'rxjs';
import { autoTraderRequestStream } from './auto_trader_rsocket';

export type AutoTraderUiSnapshot = {
  status: Record<string, unknown>;
  activity: Record<string, unknown>;
  parameters: Record<string, unknown>;
  dated_carry: Record<string, unknown>;
  execution_audit: {
    items: Array<Record<string, unknown>>;
    nextCursor?: number | null;
  };
};

const snapshot$ = new BehaviorSubject<AutoTraderUiSnapshot | undefined>(
  undefined,
);
let stopStream: (() => void) | undefined;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let retryDelay = 1000;
let subscribers = 0;
let firstSnapshot: Promise<AutoTraderUiSnapshot> | undefined;
let resolveFirstSnapshot:
  | ((snapshot: AutoTraderUiSnapshot) => void)
  | undefined;

function connect() {
  if (stopStream || subscribers === 0) return;
  let cancelled = false;
  let cancelRequest: (() => void) | undefined;
  stopStream = () => {
    cancelled = true;
    cancelRequest?.();
  };
  void autoTraderRequestStream(
    'autotrader:snapshot',
    (data) => {
      if (cancelled) return;
      try {
        const message = JSON.parse(Buffer.from(data).toString()) as
          | { type: 'snapshot'; state: AutoTraderUiSnapshot }
          | { type: 'delta'; changes: Partial<AutoTraderUiSnapshot> };
        const value =
          message.type === 'snapshot'
            ? message.state
            : message.type === 'delta' && snapshot$.value
              ? { ...snapshot$.value, ...message.changes }
              : undefined;
        if (
          value &&
          value.status &&
          value.activity &&
          value.parameters &&
          value.dated_carry &&
          Array.isArray(value.execution_audit?.items)
        ) {
          snapshot$.next(value);
          retryDelay = 1000;
          resolveFirstSnapshot?.(value);
          resolveFirstSnapshot = undefined;
        }
      } catch {
        /* Keep the last valid snapshot. */
      }
    },
    () => {
      if (cancelled) return;
      cancelled = true;
      stopStream = undefined;
      if (subscribers && !retryTimer) {
        retryTimer = setTimeout(() => {
          retryTimer = undefined;
          connect();
        }, retryDelay);
        retryDelay = Math.min(retryDelay * 2, 30_000);
      }
    },
  ).then((stream) => {
    cancelRequest = () => stream.cancel();
    if (cancelled) stream.cancel();
  });
}

export function subscribeAutoTraderStream(
  observer: (snapshot: AutoTraderUiSnapshot) => void,
): () => void {
  subscribers += 1;
  const subscription = snapshot$.subscribe((snapshot) => {
    if (snapshot) observer(snapshot);
  });
  connect();
  return () => {
    subscription.unsubscribe();
    subscribers = Math.max(0, subscribers - 1);
    if (!subscribers) {
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = undefined;
      stopStream?.();
      stopStream = undefined;
    }
  };
}

export function currentAutoTraderSnapshot(): AutoTraderUiSnapshot | undefined {
  return snapshot$.value;
}

/** Wait for the shared stream before requesting an RSocket snapshot. */
export function waitForAutoTraderSnapshot(
  timeoutMs = 5000,
): Promise<AutoTraderUiSnapshot> {
  if (snapshot$.value) return Promise.resolve(snapshot$.value);
  if (!firstSnapshot) {
    firstSnapshot = new Promise<AutoTraderUiSnapshot>((resolve) => {
      resolveFirstSnapshot = resolve;
    });
  }
  return Promise.race([
    firstSnapshot,
    new Promise<AutoTraderUiSnapshot>((_, reject) =>
      setTimeout(
        () => reject(new Error('Auto-trader stream unavailable')),
        timeoutMs,
      ),
    ),
  ]);
}
