import { BehaviorSubject } from 'rxjs';
import type { Position } from './models/PositionTypes';
import { autoTraderRpc } from './auto_trader_rsocket';

export type HedgeCandidate = {
  symbol: string;
  correlation: number | null;
  paired_samples: number;
  latest_price: number | null;
};

export type HedgePosition = Position & {
  latest_price: number | null;
  candidates: HedgeCandidate[];
};

export type HedgeCandidatesState = {
  positions: HedgePosition[];
  pair_correlations?: {
    symbol_a: string;
    symbol_b: string;
    correlation: number | null;
  }[];
  error?: string;
};

const state$ = new BehaviorSubject<HedgeCandidatesState>({ positions: [] });
let subscribers = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let refreshing = false;

async function refresh() {
  if (refreshing || subscribers === 0) return;
  refreshing = true;
  try {
    const response = await autoTraderRpc(
      'GET',
      '/positions/auto-trader/hedge-candidates',
    );
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const result = (await response.json()) as {
      positions?: HedgePosition[];
      pair_correlations?: HedgeCandidatesState['pair_correlations'];
    };
    state$.next({
      positions: (result.positions ?? []).filter(
        (position) => Math.abs(position.quantity) > 1e-12,
      ),
      pair_correlations: result.pair_correlations,
    });
  } catch {
    if (state$.value.positions.length === 0) {
      state$.next({
        positions: [],
        error: 'Hedge candidates are temporarily unavailable.',
      });
    }
  } finally {
    refreshing = false;
    if (subscribers > 0) timer = setTimeout(() => void refresh(), 15_000);
  }
}

export function subscribeHedgeCandidates(
  observer: (state: HedgeCandidatesState) => void,
) {
  subscribers += 1;
  const subscription = state$.subscribe(observer);
  if (subscribers === 1) void refresh();
  return () => {
    subscription.unsubscribe();
    subscribers = Math.max(0, subscribers - 1);
    if (subscribers === 0 && timer) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
}

export function refreshHedgeCandidates() {
  if (timer) clearTimeout(timer);
  timer = undefined;
  void refresh();
}
