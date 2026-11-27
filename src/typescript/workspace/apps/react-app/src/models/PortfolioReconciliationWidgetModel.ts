import { BehaviorSubject, type Subscription } from 'rxjs';
import { AutoTraderModel, type CapitalAllocation } from './AutoTraderModel';
import { positionFetch } from '../position_api';

const REFRESH_MS = 30_000;
const INTERVAL_LIMIT = 250;

type JsonObject = Record<string, unknown>;
export type EvidenceStatus =
  | 'complete'
  | 'provisional'
  | 'not_evaluated'
  | 'incomplete'
  | 'stale'
  | 'unavailable';

export type PortfolioInterval = {
  raw: JsonObject;
  evidenceId: string | null;
  endMs: number | null;
  completeness: EvidenceStatus;
  independentStatus: EvidenceStatus;
  returnPercent: number | null;
  netPnlUsd: number | null;
  reasons: string[];
};

export type PortfolioReconciliationState = {
  loading: boolean;
  error?: string;
  intervalError?: string;
  stale: boolean;
  dataStatus: 'inactive' | 'loading' | 'available' | 'partial' | 'unavailable';
  tab: 'summary' | 'balances' | 'intervals';
  intervalFilter: 'all' | 'complete' | 'incomplete';
  snapshot: JsonObject | null;
  intervalCoverage: string | null;
  intervals: PortfolioInterval[];
  truncated: boolean | null;
  allocation: CapitalAllocation | null;
  refreshedAtMs: number | null;
};

export type PortfolioFetcher = (
  path: string,
  signal?: AbortSignal,
) => Promise<Response>;

export function visiblePortfolioIntervals(
  state: PortfolioReconciliationState,
): PortfolioInterval[] {
  if (state.intervalFilter === 'complete')
    return state.intervals.filter(
      (interval) => interval.completeness === 'complete',
    );
  if (state.intervalFilter === 'incomplete')
    return state.intervals.filter(
      (interval) => interval.completeness !== 'complete',
    );
  return state.intervals;
}

function object(value: unknown): JsonObject | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as JsonObject)
    : null;
}

function finite(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null;
}

function status(
  value: unknown,
  fallback: EvidenceStatus = 'unavailable',
): EvidenceStatus {
  switch (value) {
    case 'complete':
    case 'pm_internal_complete':
      return 'complete';
    case 'provisional':
      return 'provisional';
    case 'not_evaluated':
    case 'not_evaluated_yet':
      return 'not_evaluated';
    case 'incomplete':
    case 'incomplete_or_unknown':
    case 'truncated':
      return 'incomplete';
    case 'stale':
      return 'stale';
    case 'unavailable':
      return 'unavailable';
    default:
      return fallback;
  }
}

function independentStatus(raw: JsonObject): EvidenceStatus {
  const eligibility = text(raw.quantitative_eligibility);
  if (eligibility?.includes('blocked_pending_independent_reconciliation'))
    return 'not_evaluated';
  if (raw.independent_execution_ledger_reconciled === true) return 'complete';
  if (raw.independent_execution_ledger_reconciled === false)
    return 'incomplete';
  return 'unavailable';
}

export function projectPortfolioIntervals(payload: unknown): {
  coverage: string | null;
  intervals: PortfolioInterval[];
  truncated: boolean | null;
} {
  const root = object(payload);
  if (!root || !Array.isArray(root.intervals))
    return { coverage: null, intervals: [], truncated: null };
  const intervals = root.intervals.flatMap((item): PortfolioInterval[] => {
    const raw = object(item);
    if (!raw) return [];
    const complete = raw.complete === true;
    const reasons = Array.isArray(raw.completeness_reasons)
      ? raw.completeness_reasons.filter(
          (reason): reason is string => typeof reason === 'string',
        )
      : [];
    const returnComplete = raw.portfolio_return_complete === true;
    const rawStatus = status(
      raw.valuation_status,
      complete ? 'complete' : 'incomplete',
    );
    return [
      {
        raw,
        evidenceId: text(raw.evidence_id),
        endMs: finite(raw.interval_end_utc_ms),
        completeness: complete
          ? rawStatus === 'stale'
            ? 'stale'
            : 'complete'
          : 'incomplete',
        independentStatus: independentStatus(raw),
        returnPercent: returnComplete
          ? finite(raw.flat_cash_period_return_pct)
          : null,
        netPnlUsd: returnComplete
          ? finite(raw.flat_cash_period_net_pnl_usd)
          : null,
        reasons,
      },
    ];
  });
  const coverage = text(root.coverage);
  return {
    coverage,
    intervals,
    truncated: typeof root.truncated === 'boolean' ? root.truncated : null,
  };
}

export function financialSnapshotValue(
  snapshot: unknown,
  ...path: string[]
): unknown {
  let current: unknown = snapshot;
  for (const key of path) {
    const record = object(current);
    if (!record || !(key in record)) return undefined;
    current = record[key];
  }
  return current;
}

export class PortfolioReconciliationWidgetModel {
  readonly state$ = new BehaviorSubject<PortfolioReconciliationState>({
    loading: true,
    stale: false,
    dataStatus: 'loading',
    tab: 'summary',
    intervalFilter: 'all',
    snapshot: null,
    intervalCoverage: null,
    intervals: [],
    truncated: null,
    allocation: null,
    refreshedAtMs: null,
  });
  private timer?: ReturnType<typeof setInterval>;
  private allocationSubscription?: Subscription;
  private readonly fetcher: PortfolioFetcher;
  private readonly autoTrader: AutoTraderModel;
  private readonly now: () => number;
  private running = false;
  private generation = 0;
  private request?: AbortController;

  constructor(
    fetcher: PortfolioFetcher = (path, signal) =>
      positionFetch(path, { signal }),
    autoTrader: AutoTraderModel = new AutoTraderModel(),
    now: () => number = Date.now,
  ) {
    this.fetcher = fetcher;
    this.autoTrader = autoTrader;
    this.now = now;
  }

  start() {
    if (this.running) return;
    this.running = true;
    const generation = ++this.generation;
    this.allocationSubscription = this.autoTrader.state$.subscribe((state) => {
      if (this.running && generation === this.generation)
        this.state$.next({
          ...this.state$.value,
          allocation: state.capitalAllocation ?? null,
        });
    });
    this.autoTrader.start();
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), REFRESH_MS);
  }

  stop() {
    this.running = false;
    this.generation += 1;
    this.request?.abort();
    this.request = undefined;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.allocationSubscription?.unsubscribe();
    this.allocationSubscription = undefined;
    this.autoTrader.stop();
    const current = this.state$.value;
    this.state$.next({
      ...current,
      loading: false,
      dataStatus: current.snapshot ? current.dataStatus : 'inactive',
    });
  }

  setTab(tab: PortfolioReconciliationState['tab']) {
    this.state$.next({ ...this.state$.value, tab });
  }

  setIntervalFilter(
    intervalFilter: PortfolioReconciliationState['intervalFilter'],
  ) {
    this.state$.next({ ...this.state$.value, intervalFilter });
  }

  async refresh() {
    const generation = this.generation;
    const controller = new AbortController();
    this.request?.abort();
    this.request = controller;
    const previous = this.state$.value;
    this.state$.next({
      ...previous,
      loading: previous.refreshedAtMs === null,
      stale: previous.refreshedAtMs !== null,
      error: undefined,
      intervalError: undefined,
      dataStatus:
        previous.refreshedAtMs === null ? 'loading' : previous.dataStatus,
    });
    try {
      const [snapshotResult, intervalsResult] = await Promise.allSettled([
        this.fetcher('/positions/financial-snapshot', controller.signal),
        this.fetcher(
          `/positions/portfolio-evidence/intervals?limit=${INTERVAL_LIMIT}`,
          controller.signal,
        ),
      ]);
      if (generation !== this.generation || controller.signal.aborted) return;
      if (snapshotResult.status === 'rejected') throw snapshotResult.reason;
      const snapshotResponse = snapshotResult.value;
      if (!snapshotResponse.ok)
        throw new Error(
          `Financial snapshot unavailable (${snapshotResponse.status})`,
        );
      const snapshot = object(await snapshotResponse.json());
      if (!snapshot) throw new Error('Financial snapshot contract unavailable');
      const intervalsResponse =
        intervalsResult.status === 'fulfilled' ? intervalsResult.value : null;
      const intervalPayload = intervalsResponse?.ok
        ? await intervalsResponse.json()
        : null;
      if (generation !== this.generation || controller.signal.aborted) return;
      const projected = projectPortfolioIntervals(intervalPayload);
      const intervalError =
        intervalsResult.status === 'rejected'
          ? `Portfolio intervals unavailable: ${intervalsResult.reason instanceof Error ? intervalsResult.reason.message : 'request failed'}`
          : !intervalsResponse?.ok
            ? `Portfolio intervals unavailable (${intervalsResponse?.status ?? 'unknown status'})`
            : projected.coverage === null && projected.truncated === null
              ? 'Portfolio interval evidence contract unavailable'
              : undefined;
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: undefined,
        intervalError,
        snapshot,
        intervalCoverage: projected.coverage,
        intervals: projected.intervals,
        truncated: projected.truncated,
        refreshedAtMs: this.now(),
        stale: false,
        dataStatus: intervalError ? 'partial' : 'available',
      });
    } catch (error) {
      if (generation !== this.generation || controller.signal.aborted) return;
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error:
          error instanceof Error
            ? error.message
            : 'Portfolio evidence unavailable',
        stale: this.state$.value.refreshedAtMs !== null,
        dataStatus: this.state$.value.snapshot ? 'partial' : 'unavailable',
      });
    } finally {
      if (this.request === controller) this.request = undefined;
    }
  }
}
