import { BehaviorSubject } from 'rxjs';
import { insightsRequestStream } from './insights_rsocket';

export type InsightsUiSnapshot = {
  signal_dashboard?: Record<string, unknown>;
  auto_trader_dashboard?: Record<string, unknown>;
  execution_audit?: {
    items: Array<Record<string, unknown>>;
    nextCursor?: number | null;
  };
  decision_audit?: {
    items: Array<Record<string, unknown>>;
    nextCursor?: number | null;
  };
  execution_quality?: Record<string, unknown>;
  decision_quality?: Record<string, unknown>;
  close_insights?: { insights?: Array<Record<string, unknown>> };
};

const snapshot$ = new BehaviorSubject<InsightsUiSnapshot | undefined>(
  undefined,
);
let stream: { cancel: () => void } | undefined;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let retryDelay = 1_000;
let subscribers = 0;
let generation = 0;
let requestStream = insightsRequestStream;

export function setInsightsStreamRequesterForTests(
  requester: typeof insightsRequestStream,
) {
  requestStream = requester;
}

function connect() {
  if (stream || subscribers === 0) return;
  const currentGeneration = ++generation;
  void requestStream(
    'ui.snapshot',
    (data) => {
      if (currentGeneration !== generation) return;
      try {
        const message = JSON.parse(Buffer.from(data).toString()) as {
          type: 'snapshot' | 'delta';
          state?: InsightsUiSnapshot;
          changes?: Partial<InsightsUiSnapshot>;
        };
        if (message.type === 'snapshot' && message.state) {
          snapshot$.next(message.state);
          retryDelay = 1_000;
        } else if (message.type === 'delta' && snapshot$.value) {
          snapshot$.next({ ...snapshot$.value, ...message.changes });
        }
      } catch {
        /* Keep the last valid snapshot. */
      }
    },
    () => {
      if (currentGeneration !== generation || !subscribers || retryTimer)
        return;
      stream = undefined;
      const delay = retryDelay;
      retryDelay = Math.min(retryDelay * 2, 30_000);
      retryTimer = setTimeout(() => {
        retryTimer = undefined;
        connect();
      }, delay);
    },
  ).then((subscription) => {
    if (currentGeneration !== generation || !subscribers) {
      subscription.cancel();
      return;
    }
    stream = subscription;
  });
}

export function subscribeInsightsStream(
  observer: (snapshot: InsightsUiSnapshot) => void,
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
      generation += 1;
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = undefined;
      stream?.cancel();
      stream = undefined;
    }
  };
}

export function currentInsightsSnapshot(): InsightsUiSnapshot | undefined {
  return snapshot$.value;
}
