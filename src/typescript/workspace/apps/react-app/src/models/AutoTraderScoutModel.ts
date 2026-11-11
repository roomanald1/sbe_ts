import { BehaviorSubject } from 'rxjs';
import { learningRpc } from '../learning_rsocket';

export type AutoTraderScoutPosition = {
  decisionId: string;
  symbol: string;
  strategy: string;
  source: 'Scout';
  scoutLevelPercent?: number;
  scoutParameterKey?: string;
  scoutParameterValue?: number;
  scoutParameterEntryEligible?: boolean;
  comparisonRole?: 'baseline' | 'variant';
  pairedDeltaPercent?: number;
  pairedDeltaTone?: 'is-positive' | 'is-negative' | 'is-flat';
  exitScenario: 'strategy' | 'tight' | 'wide';
  direction: 'bullish' | 'bearish';
  strength: number;
  openedAt: number;
  durationLabel?: string;
  closedAt?: number;
  entryPrice?: number;
  entryBid?: number;
  entryAsk?: number;
  markPrice?: number;
  takerFeeRate: number;
  status: 'Open' | 'Won' | 'Lost' | 'Flat' | 'Outcome unavailable';
  netReturnPercent?: number;
  openNetReturnPercent?: number;
  displayNetReturnPercent?: number;
  netReturnLabel?: string;
  netVsBreakevenLeft?: number;
  netVsBreakevenWidth?: number;
  netVsBreakevenTone?: 'is-positive' | 'is-negative' | 'is-flat';
  netReturnEstimated?: boolean;
  returnBasis?: 'strategy exit' | 'ticker estimate';
  horizonMarkouts?: Record<
    string,
    {
      netReturnPercent: number;
      grossReturnPercent?: number;
      executionCostPercent?: number;
      bestFavorablePercent?: number;
      worstAdversePercent?: number;
      timeToPositiveNetSeconds?: number;
      postExitNetReturnPercent?: number;
      observedAt?: number;
      takerFeeRate?: number;
    }
  >;
};

export type ScoutLearningSummary = {
  kind: 'Scout summary' | 'Parameter probe' | 'Breakout distance';
  symbol: string;
  strategy: string;
  scenario: 'strategy' | 'tight' | 'wide';
  parameter?: string;
  value?: number;
  entryEligible?: boolean;
  level?: number;
  count: number;
  wins: number;
  winRatePercent?: number;
  winRateColor?: string;
  average?: number;
  averageTone?: 'is-positive' | 'is-negative' | 'is-flat';
  pairedDeltaAverage?: number;
  pairedDeltaTone?: 'is-positive' | 'is-negative' | 'is-flat';
};

type ScoutCounts = {
  total: number;
  open: number;
  resolved: number;
  wins: number;
  losses: number;
  flats: number;
  unavailable: number;
};
export type ScoutSourceFilter = 'live' | 'all';
export type ScoutStatusFilter = 'all' | 'open' | 'closed';
type ScoutOutcomesResponse = {
  rows: AutoTraderScoutPosition[];
  summary: { all: ScoutCounts; live: ScoutCounts };
  learningRows: ScoutLearningSummary[];
  symbols: string[];
  learningSymbol: string | null;
  learningOutcomes: Array<{
    strategy: string;
    netReturnPercent: number;
    parameterKey?: string;
    parameterValue?: number;
  }>;
  updatedAt: number;
  source: ScoutSourceFilter;
  status: ScoutStatusFilter;
  page: number;
  pageNumber: number;
  pageSize: number;
  pageCount: number;
  totalRows: number;
  hasPrevious: boolean;
  hasNext: boolean;
  sortBy: string;
  sortDirection: 'asc' | 'desc';
};

type ScoutComparisonOutcome = {
  stage?: string;
  scout_comparison_id?: string;
  scout_comparison_role?: 'baseline' | 'variant';
  scout_net_return_percent?: number;
  paired_delta_percent?: number;
};

export function pairedScoutDeltas(
  outcomes: ScoutComparisonOutcome[],
): Map<string, number> {
  const returns = new Map<string, { baseline?: number; variant?: number }>();
  const deltas = new Map<string, number>();
  for (const outcome of outcomes) {
    const id = outcome.scout_comparison_id;
    if (outcome.stage !== 'scout-outcome' || !id) continue;
    if (
      typeof outcome.paired_delta_percent === 'number' &&
      Number.isFinite(outcome.paired_delta_percent)
    ) {
      deltas.set(id, outcome.paired_delta_percent);
    }
    if (
      (outcome.scout_comparison_role !== 'baseline' &&
        outcome.scout_comparison_role !== 'variant') ||
      typeof outcome.scout_net_return_percent !== 'number' ||
      !Number.isFinite(outcome.scout_net_return_percent)
    )
      continue;
    const pair = returns.get(id) ?? {};
    pair[outcome.scout_comparison_role] = outcome.scout_net_return_percent;
    returns.set(id, pair);
  }
  for (const [id, pair] of returns) {
    if (
      !deltas.has(id) &&
      typeof pair.baseline === 'number' &&
      typeof pair.variant === 'number'
    ) {
      deltas.set(id, pair.variant - pair.baseline);
    }
  }
  return deltas;
}

export class AutoTraderScoutModel {
  readonly state$ = new BehaviorSubject<
    ScoutOutcomesResponse & { loading: boolean; error?: string }
  >({
    rows: [],
    summary: {
      all: {
        total: 0,
        open: 0,
        resolved: 0,
        wins: 0,
        losses: 0,
        flats: 0,
        unavailable: 0,
      },
      live: {
        total: 0,
        open: 0,
        resolved: 0,
        wins: 0,
        losses: 0,
        flats: 0,
        unavailable: 0,
      },
    },
    learningRows: [],
    symbols: [],
    learningSymbol: null,
    learningOutcomes: [],
    updatedAt: 0,
    source: 'live',
    status: 'all',
    page: 0,
    pageNumber: 1,
    pageSize: 50,
    pageCount: 0,
    totalRows: 0,
    hasPrevious: false,
    hasNext: false,
    sortBy: 'openedAt',
    sortDirection: 'desc',
    loading: true,
  });
  private timer?: ReturnType<typeof setTimeout>;
  private refreshing = false;
  private consumers = 0;
  private retryDelayMs = 2_000;
  private requestId = 0;
  private controller?: AbortController;

  start() {
    this.consumers += 1;
    if (this.consumers > 1) return;
    this.retryDelayMs = 2_000;
    void this.refreshAndSchedule();
  }

  stop() {
    this.consumers = Math.max(0, this.consumers - 1);
    if (this.consumers > 0) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
  }

  retry() {
    if (this.refreshing) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
    this.retryDelayMs = 2_000;
    void this.refreshAndSchedule();
  }

  setFilters(filters: {
    source?: ScoutSourceFilter;
    status?: ScoutStatusFilter;
    page?: number;
    learningSymbol?: string;
    sortBy?: string;
    sortDirection?: 'asc' | 'desc';
  }) {
    const current = this.state$.value;
    const next = {
      source: filters.source ?? current.source,
      status: filters.status ?? current.status,
      page: filters.page ?? 0,
      learningSymbol: filters.learningSymbol ?? current.learningSymbol ?? 'all',
      sortBy: filters.sortBy ?? current.sortBy,
      sortDirection: filters.sortDirection ?? current.sortDirection,
    };
    if (
      next.source === current.source &&
      next.status === current.status &&
      next.page === current.page &&
      next.learningSymbol === (current.learningSymbol ?? 'all') &&
      next.sortBy === current.sortBy &&
      next.sortDirection === current.sortDirection
    )
      return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
    if (
      next.sortBy !== current.sortBy ||
      next.sortDirection !== current.sortDirection
    )
      next.page = 0;
    this.state$.next({
      ...current,
      ...next,
      rows: [],
      loading: true,
      error: undefined,
    });
    void this.refreshAndSchedule();
  }

  private async refreshAndSchedule() {
    await this.refresh();
    if (this.consumers === 0) return;
    const failed = Boolean(this.state$.value.error);
    const delay = failed ? this.retryDelayMs : 60_000;
    this.retryDelayMs = failed
      ? Math.min(30_000, this.retryDelayMs * 2)
      : 2_000;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.refreshAndSchedule(), delay);
  }

  private async refresh() {
    const requestId = ++this.requestId;
    this.controller?.abort();
    const controller = new AbortController();
    this.controller = controller;
    this.refreshing = true;
    try {
      const filters = this.state$.value;
      const query = new URLSearchParams({
        source: filters.source,
        status: filters.status,
        page: String(filters.page),
        page_size: String(filters.pageSize),
        learning_symbol: filters.learningSymbol ?? 'all',
        sort_by: filters.sortBy,
        sort_direction: filters.sortDirection,
      });
      const response = await learningRpc(
        'GET',
        '/outcomes?' + query.toString(),
      );
      if (!response.ok) throw new Error('Scout outcome data unavailable');
      const data = (await response.json()) as ScoutOutcomesResponse;
      if (requestId !== this.requestId) return;
      this.state$.next({ ...data, loading: false, error: undefined });
    } catch (error) {
      if (requestId === this.requestId) {
        this.state$.next({
          ...this.state$.value,
          loading: false,
          error:
            error instanceof Error
              ? error.message
              : 'Could not load scout outcomes',
        });
      }
    } finally {
      if (requestId === this.requestId) {
        this.refreshing = false;
        this.controller = undefined;
      }
    }
  }
}

export const autoTraderScoutModel = new AutoTraderScoutModel();
