import { BehaviorSubject } from 'rxjs';

export type AutoTraderUiSnapshot = {
  status: Record<string, unknown>;
  activity: Record<string, unknown>;
  parameters: Record<string, unknown>;
  dated_carry: Record<string, unknown>;
  execution_audit: { items: Array<Record<string, unknown>>; nextCursor?: number | null };
};

const snapshot$ = new BehaviorSubject<AutoTraderUiSnapshot | undefined>(undefined);
let socket: WebSocket | undefined;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let retryDelay = 1000;
let subscribers = 0;

function connect() {
  if (socket || subscribers === 0) return;
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const next = new WebSocket(`${protocol}//${window.location.host}/positions/auto-trader/stream`);
  socket = next;
  next.onopen = () => { retryDelay = 1000; };
  next.onmessage = (event) => {
    try {
      const message = JSON.parse(String(event.data)) as
        | { type: 'snapshot'; state: AutoTraderUiSnapshot }
        | { type: 'delta'; changes: Partial<AutoTraderUiSnapshot> };
      const value = message.type === 'snapshot'
        ? message.state
        : message.type === 'delta' && snapshot$.value
          ? { ...snapshot$.value, ...message.changes }
          : undefined;
      if (value && value.status && value.activity && value.parameters && value.dated_carry && Array.isArray(value.execution_audit?.items)) snapshot$.next(value);
    } catch { /* Keep the last valid snapshot. */ }
  };
  next.onclose = () => {
    if (socket === next) socket = undefined;
    if (subscribers && !retryTimer) {
      retryTimer = setTimeout(() => { retryTimer = undefined; connect(); }, retryDelay);
      retryDelay = Math.min(retryDelay * 2, 30_000);
    }
  };
  next.onerror = () => next.close();
}

export function subscribeAutoTraderStream(observer: (snapshot: AutoTraderUiSnapshot) => void): () => void {
  subscribers += 1;
  const subscription = snapshot$.subscribe((snapshot) => { if (snapshot) observer(snapshot); });
  connect();
  return () => {
    subscription.unsubscribe();
    subscribers = Math.max(0, subscribers - 1);
    if (!subscribers) {
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = undefined;
      socket?.close();
      socket = undefined;
    }
  };
}

export function currentAutoTraderSnapshot(): AutoTraderUiSnapshot | undefined { return snapshot$.value; }
