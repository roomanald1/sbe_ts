import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  AutoTraderModel,
  type AutoTraderExecutionQuality,
  type AutoTraderOrder,
} from './AutoTraderModel';
import { positionFetch } from '../position_api';

export const MAX_EXECUTION_QUALITY_ORDERS = 100;

export type ExecutionOrderEvidence = AutoTraderOrder & {
  execution_order_id?: string | null;
  decision_bid?: number | null;
  decision_ask?: number | null;
  decision_mid?: number | null;
  submitted_limit_price?: number | null;
  expires_at?: number | null;
  reserved_notional?: number | null;
  fees_paid?: number | null;
  fill_rate?: number | null;
  triggered_at?: number | null;
  id?: string | null;
  order_id?: string | null;
  client_order_id?: string | null;
  parent_order_id?: string | null;
};

export type MarketPathSample = {
  source_system?: string | null;
  source_epoch?: string | null;
  source_cursor?: number | null;
  source_cursor_kind?: string | null;
  source_capture_complete?: boolean | null;
  request_fingerprint?: string | null;
  attempt_order_id?: string | null;
  parent_order_id?: string | null;
  recorded_at_utc_ms?: number | null;
  book_updated_at_ms?: number | null;
  best_bid?: number | null;
  best_ask?: number | null;
  reference_price?: number | null;
};

export type RestingOrderPath = {
  id?: string | null;
  client_order_id?: string | null;
  parent_order_id?: string | null;
  market_path?: MarketPathSample[] | null;
  market_path_completeness?: string | null;
  market_path_dropped_samples?: number | null;
  remaining_quantity?: number | null;
  quantity?: number | null;
  limit_price?: number | null;
  status?: string | null;
};

export type ExecutionPathRecord = {
  order: RestingOrderPath;
  terminal_reason?: string | null;
  terminal_at_ms?: number | null;
};

export type MatchedMarketPath = {
  key: string;
  source: 'active_order' | 'terminal_history';
  order: RestingOrderPath;
  terminalReason: string | null;
  terminalAtMs: number | null;
};

export type UnlinkedMarketPath = {
  path: MatchedMarketPath;
  reason:
    | 'no_stable_order_identity'
    | 'no_activity_order_match'
    | 'ambiguous_activity_order_match';
};

export type ExecutionPathData = {
  activeOrders: RestingOrderPath[];
  terminalPaths: ExecutionPathRecord[];
  activeContractAvailable: boolean;
  historyContractAvailable: boolean;
  historyTruncated: boolean | null;
  historyHasMoreOrders: boolean | null;
};

export type ExecutionOutcome =
  | 'filled'
  | 'partially_filled'
  | 'pending'
  | 'post_only_cross_rejection'
  | 'passive_expiry'
  | 'cancelled'
  | 'rejected'
  | 'terminal_no_fill'
  | 'unknown';

export type ExecutionOrderRow = {
  key: string;
  order: ExecutionOrderEvidence;
  outcome: ExecutionOutcome;
  outcomeLabel: string;
  stableOrderId: string | null;
  decisionId: string | null;
  decisionTouchDistanceBps: number | null;
  decisionCrossingQuoteDistanceBps: number | null;
  configuredLifetimeMs: number | null;
  marketPaths: MatchedMarketPath[];
};

export type ExecutionQualitySnapshot = {
  rows: ExecutionOrderRow[];
  visibleRows: ExecutionOrderRow[];
  symbols: string[];
  filters: { symbol: string; outcome: ExecutionOutcome | '' };
  availableOrderCount: number;
  displayedOrderCount: number;
  summary: AutoTraderExecutionQuality;
  loading: boolean;
  nowMs: number;
  pathEvidence: ExecutionPathData;
  unlinkedMarketPaths: UnlinkedMarketPath[];
  pathEvidenceStatus: 'loading' | 'available' | 'partial' | 'unavailable';
  pathEvidenceLoading: boolean;
  pathEvidenceError: string | null;
  pathEvidenceStale: boolean;
};

export type ExecutionQualityState = ExecutionQualitySnapshot;

const emptyPathData: ExecutionPathData = {
  activeOrders: [],
  terminalPaths: [],
  activeContractAvailable: false,
  historyContractAvailable: false,
  historyTruncated: null,
  historyHasMoreOrders: null,
};

function safeArray<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

function pathSamples(order: RestingOrderPath): MarketPathSample[] {
  return safeArray<MarketPathSample>(order.market_path);
}

function identityKeys(order: Record<string, unknown>): Set<string> {
  const keys = new Set<string>();
  const add = (kind: string, value: unknown) => {
    if (typeof value === 'string' && value.trim())
      keys.add(`${kind}:${value.trim()}`);
  };
  add('execution', order.execution_order_id);
  add('execution', order.order_id);
  add('execution', order.id);
  add('client', order.client_order_id);
  add('parent', order.parent_order_id);
  return keys;
}

function pathIdentityKeys(order: RestingOrderPath): Set<string> {
  const keys = identityKeys(order as Record<string, unknown>);
  for (const sample of pathSamples(order)) {
    if (
      typeof sample.attempt_order_id === 'string' &&
      sample.attempt_order_id.trim()
    )
      keys.add(`execution:${sample.attempt_order_id.trim()}`);
    if (
      typeof sample.parent_order_id === 'string' &&
      sample.parent_order_id.trim()
    )
      keys.add(`parent:${sample.parent_order_id.trim()}`);
  }
  return keys;
}

export function projectMarketPathRecords(
  data: ExecutionPathData,
): MatchedMarketPath[] {
  const active = data.activeOrders
    .filter(
      (order) =>
        typeof order.remaining_quantity === 'number' &&
        order.remaining_quantity > 0,
    )
    .map(
      (order, index): MatchedMarketPath => ({
        key: `active:${order.id ?? order.client_order_id ?? `record-${index}`}`,
        source: 'active_order',
        order,
        terminalReason: null,
        terminalAtMs: null,
      }),
    );
  const terminal = data.terminalPaths.map(
    (record, index): MatchedMarketPath => ({
      key: `terminal:${record.order.id ?? record.order.client_order_id ?? `record-${index}`}`,
      source: 'terminal_history',
      order: record.order,
      terminalReason:
        typeof record.terminal_reason === 'string'
          ? record.terminal_reason
          : null,
      terminalAtMs:
        typeof record.terminal_at_ms === 'number' &&
        Number.isFinite(record.terminal_at_ms)
          ? record.terminal_at_ms
          : null,
    }),
  );
  return [...active, ...terminal];
}

export function joinMarketPathsToOrders(
  orders: readonly ExecutionOrderEvidence[],
  data: ExecutionPathData,
): { rows: ExecutionOrderRow[]; unlinked: UnlinkedMarketPath[] } {
  const baseRows = buildExecutionOrderRows(orders);
  const paths = projectMarketPathRecords(data);
  const rowsByKey = new Map(baseRows.map((row) => [row.key, row]));
  const orderKeys = baseRows.map((row) => ({
    row,
    keys: identityKeys(row.order as Record<string, unknown>),
  }));
  const unlinked: UnlinkedMarketPath[] = [];
  for (const path of paths) {
    const keys = pathIdentityKeys(path.order);
    if (keys.size === 0) {
      unlinked.push({ path, reason: 'no_stable_order_identity' });
      continue;
    }
    const matches = orderKeys.filter(({ keys: activityKeys }) =>
      [...keys].some((key) => activityKeys.has(key)),
    );
    if (matches.length !== 1) {
      unlinked.push({
        path,
        reason:
          matches.length === 0
            ? 'no_activity_order_match'
            : 'ambiguous_activity_order_match',
      });
      continue;
    }
    const matched = matches[0].row;
    rowsByKey.set(matched.key, {
      ...matched,
      marketPaths: [...matched.marketPaths, path],
    });
    const index = orderKeys.findIndex(({ row }) => row.key === matched.key);
    if (index >= 0) orderKeys[index].row = rowsByKey.get(matched.key)!;
  }
  return {
    rows: baseRows.map((row) => rowsByKey.get(row.key) ?? row),
    unlinked,
  };
}

function derivePathEvidenceStatus(
  data: ExecutionPathData,
): ExecutionQualitySnapshot['pathEvidenceStatus'] {
  if (!data.activeContractAvailable && !data.historyContractAvailable)
    return 'unavailable';
  if (
    !data.activeContractAvailable ||
    !data.historyContractAvailable ||
    data.historyTruncated === null ||
    data.historyHasMoreOrders === null ||
    data.historyTruncated === true
  )
    return 'partial';
  return 'available';
}

export function parseExecutionPathData(
  activePayload: unknown,
  historyPayload: unknown,
): ExecutionPathData {
  const active =
    activePayload !== null &&
    typeof activePayload === 'object' &&
    !Array.isArray(activePayload)
      ? (activePayload as Record<string, unknown>)
      : null;
  const history =
    historyPayload !== null &&
    typeof historyPayload === 'object' &&
    !Array.isArray(historyPayload)
      ? (historyPayload as Record<string, unknown>)
      : null;
  const historyOrders =
    history && Array.isArray(history.execution_paths)
      ? (history.execution_paths as ExecutionPathRecord[])
      : [];
  const validHistory = !!history && Array.isArray(history.execution_paths);
  const validActive = !!active && Array.isArray(active.orders);
  return {
    activeOrders: validActive ? (active!.orders as RestingOrderPath[]) : [],
    terminalPaths: historyOrders.filter(
      (record) =>
        record && typeof record.order === 'object' && record.order !== null,
    ),
    activeContractAvailable: validActive,
    historyContractAvailable: validHistory,
    historyTruncated:
      history && typeof history.execution_paths_truncated === 'boolean'
        ? history.execution_paths_truncated
        : null,
    historyHasMoreOrders:
      history && typeof history.has_more_orders === 'boolean'
        ? history.has_more_orders
        : null,
  };
}

function positiveFinite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0;
}

function bpsFrom(reference: unknown, value: unknown): number | null {
  if (!positiveFinite(reference) || !positiveFinite(value)) return null;
  return (value / reference - 1) * 10_000;
}

function executionOutcome(order: ExecutionOrderEvidence): {
  outcome: ExecutionOutcome;
  label: string;
} {
  const status = order.status.trim().toLowerCase();
  const reason = (order.failure_reason ?? '').toLowerCase();
  const kind = order.order_type.toLowerCase();
  const makerOrder = /post.?only|maker/.test(kind);
  if (
    makerOrder &&
    /(cross|would take|marketable|immediate match)/.test(reason)
  )
    return {
      outcome: 'post_only_cross_rejection',
      label: 'Post-only crossing rejection',
    };
  if (/expir|expired/.test(status + ' ' + reason))
    return {
      outcome: makerOrder ? 'passive_expiry' : 'terminal_no_fill',
      label: makerOrder ? 'Passive order expired' : 'Order expired',
    };
  if (/cancel|canceled|cancelled/.test(status + ' ' + reason))
    return { outcome: 'cancelled', label: 'Cancelled' };
  if (/reject/.test(status + ' ' + reason))
    return { outcome: 'rejected', label: 'Rejected' };

  const filled = Number.isFinite(order.filled_quantity)
    ? Math.max(order.filled_quantity, 0)
    : 0;
  const quantity = Number.isFinite(order.quantity)
    ? Math.max(order.quantity, 0)
    : 0;
  if (filled > 0 && quantity > 0 && filled >= quantity - 1e-12)
    return { outcome: 'filled', label: 'Filled' };
  if (filled > 0)
    return { outcome: 'partially_filled', label: 'Partially filled' };
  if (/open|pending|working|submitted|accepted|resting/.test(status))
    return { outcome: 'pending', label: 'Pending' };
  if (/filled|complete|closed/.test(status))
    return {
      outcome: 'terminal_no_fill',
      label: 'Terminal; fill detail unavailable',
    };
  return { outcome: 'unknown', label: 'Outcome unavailable' };
}

export function buildExecutionOrderRows(
  orders: readonly ExecutionOrderEvidence[],
): ExecutionOrderRow[] {
  return orders
    .slice()
    .sort((left, right) => right.timestamp - left.timestamp)
    .slice(0, MAX_EXECUTION_QUALITY_ORDERS)
    .map((order, index) => {
      const { outcome, label } = executionOutcome(order);
      const buy = order.side.toLowerCase() === 'buy';
      const touch = buy ? order.decision_bid : order.decision_ask;
      const crossingQuote = buy ? order.decision_ask : order.decision_bid;
      const submittedLimit = order.submitted_limit_price;
      const stableOrderId = order.execution_order_id ?? null;
      const decisionId = order.decision_id ?? null;
      return {
        key:
          stableOrderId ??
          (decisionId
            ? `${decisionId}:${order.timestamp}`
            : `record:${order.timestamp}:${index}`),
        order,
        outcome,
        outcomeLabel: label,
        stableOrderId,
        decisionId,
        decisionTouchDistanceBps: bpsFrom(touch, submittedLimit),
        decisionCrossingQuoteDistanceBps: bpsFrom(
          crossingQuote,
          submittedLimit,
        ),
        configuredLifetimeMs:
          typeof order.expires_at === 'number' &&
          Number.isFinite(order.expires_at) &&
          order.expires_at >= order.timestamp
            ? order.expires_at - order.timestamp
            : null,
        marketPaths: [],
      };
    });
}

export function buildExecutionQualitySnapshot(
  orders: readonly ExecutionOrderEvidence[],
  summary: AutoTraderExecutionQuality,
  loading: boolean,
  pathEvidence: ExecutionPathData = emptyPathData,
): ExecutionQualitySnapshot {
  const joined = joinMarketPathsToOrders(orders, pathEvidence);
  return {
    rows: joined.rows,
    visibleRows: joined.rows,
    symbols: [...new Set(joined.rows.map(({ order }) => order.symbol))].sort(),
    filters: { symbol: '', outcome: '' },
    availableOrderCount: orders.length,
    displayedOrderCount: joined.rows.length,
    summary,
    loading,
    nowMs: Date.now(),
    pathEvidence,
    unlinkedMarketPaths: joined.unlinked,
    pathEvidenceStatus: derivePathEvidenceStatus(pathEvidence),
    pathEvidenceLoading: false,
    pathEvidenceError: null,
    pathEvidenceStale: false,
  };
}

export class AutoTraderExecutionQualityModel {
  readonly state$: BehaviorSubject<ExecutionQualityState>;
  private readonly source: AutoTraderModel;
  private readonly fetchPath: (path: string) => Promise<Response>;
  private sourceSubscription?: Subscription;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private clockTimer?: ReturnType<typeof setInterval>;
  private pathData: ExecutionPathData = emptyPathData;
  private pathRequest?: Promise<void>;
  private pathRequestGeneration = 0;
  private pathEvidenceLoaded = false;
  private pathEvidenceLoading = false;
  private pathEvidenceError: string | null = null;
  private pathEvidenceStale = false;
  private started = false;
  private filters: ExecutionQualityState['filters'] = {
    symbol: '',
    outcome: '',
  };
  private nowMs = Date.now();

  constructor(
    source: AutoTraderModel = new AutoTraderModel(),
    fetchPath: (path: string) => Promise<Response> = (path) =>
      positionFetch(path),
  ) {
    this.source = source;
    this.fetchPath = fetchPath;
    this.state$ = new BehaviorSubject(this.project());
  }

  start() {
    if (this.started) return;
    this.started = true;
    this.source.start();
    this.sourceSubscription = this.source.state$.subscribe(() =>
      this.state$.next(this.project()),
    );
    void this.refresh();
    this.refreshTimer = setInterval(() => void this.refresh(), 15_000);
    this.clockTimer = setInterval(() => {
      this.nowMs = Date.now();
      this.state$.next(this.project());
    }, 5_000);
  }

  stop() {
    if (!this.started) return;
    this.started = false;
    this.pathRequestGeneration += 1;
    this.pathRequest = undefined;
    this.pathEvidenceLoading = false;
    this.pathEvidenceStale = this.pathEvidenceLoaded;
    this.sourceSubscription?.unsubscribe();
    this.sourceSubscription = undefined;
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = undefined;
    if (this.clockTimer) clearInterval(this.clockTimer);
    this.clockTimer = undefined;
    this.source.stop();
    this.state$.next(this.project());
  }

  setFilters(filters: Partial<ExecutionQualityState['filters']>) {
    this.filters = {
      symbol: filters.symbol ?? this.filters.symbol,
      outcome: filters.outcome ?? this.filters.outcome,
    };
    this.state$.next(this.project());
  }

  async refresh(): Promise<void> {
    if (!this.started) return;
    if (this.pathRequest) return this.pathRequest;
    const requestId = ++this.pathRequestGeneration;
    this.pathEvidenceLoading = true;
    this.pathEvidenceStale = this.pathEvidenceLoaded;
    this.state$.next(this.project());
    const request = this.loadPathEvidence(requestId);
    this.pathRequest = request;
    try {
      await request;
    } finally {
      if (this.pathRequest === request) this.pathRequest = undefined;
    }
  }

  private project(): ExecutionQualityState {
    const state = this.source.state$.value;
    const base = buildExecutionQualitySnapshot(
      state.orders as ExecutionOrderEvidence[],
      state.executionQuality,
      state.loading,
      this.pathData,
    );
    const visibleRows = base.rows.filter(
      ({ order, outcome }) =>
        (!this.filters.symbol || order.symbol === this.filters.symbol) &&
        (!this.filters.outcome || outcome === this.filters.outcome),
    );
    return {
      ...base,
      visibleRows,
      filters: this.filters,
      nowMs: this.nowMs,
      pathEvidenceStatus: this.pathEvidenceLoaded
        ? base.pathEvidenceStatus
        : 'loading',
      pathEvidenceLoading: this.pathEvidenceLoading,
      pathEvidenceError: this.pathEvidenceError,
      pathEvidenceStale: this.pathEvidenceStale,
    };
  }

  private async loadPathEvidence(requestId: number): Promise<void> {
    const previousData = this.pathData;
    const previousLoaded = this.pathEvidenceLoaded;
    const isCurrent = () =>
      this.started && requestId === this.pathRequestGeneration;
    try {
      const [activeResult, historyResult] = await Promise.all([
        this.readPathResponse('/positions/orders'),
        this.readPathResponse('/positions/orders/history?limit=200'),
      ]);
      if (!isCurrent()) return;
      const failures = [
        activeResult.error && `Active order paths: ${activeResult.error}`,
        historyResult.error && `Terminal path history: ${historyResult.error}`,
      ].filter((error): error is string => Boolean(error));
      const parsed = parseExecutionPathData(
        activeResult.payload,
        historyResult.payload,
      );
      this.pathData = {
        activeOrders: activeResult.error
          ? previousData.activeOrders
          : parsed.activeOrders,
        terminalPaths: historyResult.error
          ? previousData.terminalPaths
          : parsed.terminalPaths,
        activeContractAvailable: activeResult.error
          ? previousData.activeContractAvailable
          : parsed.activeContractAvailable,
        historyContractAvailable: historyResult.error
          ? previousData.historyContractAvailable
          : parsed.historyContractAvailable,
        historyTruncated: historyResult.error
          ? previousData.historyTruncated
          : parsed.historyTruncated,
        historyHasMoreOrders: historyResult.error
          ? previousData.historyHasMoreOrders
          : parsed.historyHasMoreOrders,
      };
      this.pathEvidenceLoaded = true;
      this.pathEvidenceError = failures.length ? failures.join('; ') : null;
      this.pathEvidenceStale = failures.length > 0 && previousLoaded;
    } catch (error) {
      if (!isCurrent()) return;
      this.pathEvidenceError =
        error instanceof Error
          ? error.message
          : 'Position Management path evidence unavailable';
      this.pathEvidenceStale = previousLoaded;
      this.pathEvidenceLoaded = true;
    } finally {
      if (isCurrent()) {
        this.pathEvidenceLoading = false;
        this.state$.next(this.project());
      }
    }
  }

  private async readPathResponse(
    path: string,
  ): Promise<{ payload: unknown; error?: string }> {
    try {
      const response = await this.fetchPath(path);
      if (!response.ok)
        return {
          payload: null,
          error: `Position Management request failed (${response.status})`,
        };
      try {
        return { payload: await response.json() };
      } catch (error) {
        return {
          payload: null,
          error:
            error instanceof Error
              ? error.message
              : 'Invalid Position Management response',
        };
      }
    } catch (error) {
      return {
        payload: null,
        error:
          error instanceof Error
            ? error.message
            : 'Position Management path evidence unavailable',
      };
    }
  }
}

export function decisionPriceDistanceLabel(distanceBps: number | null): string {
  if (distanceBps == null || !Number.isFinite(distanceBps))
    return 'Unavailable';
  return `${distanceBps >= 0 ? '+' : ''}${distanceBps.toFixed(2)} bps`;
}

export function executionFreshnessLabel(
  updatedAt: number | undefined,
  nowMs: number,
): { label: string; stale: boolean } {
  if (!Number.isFinite(updatedAt) || !updatedAt || updatedAt > nowMs)
    return { label: 'Update timestamp unavailable', stale: true };
  const ageMs = nowMs - updatedAt;
  if (ageMs > 60_000)
    return {
      label: `Stale · updated ${Math.floor(ageMs / 1000)}s ago`,
      stale: true,
    };
  return {
    label:
      ageMs < 1_000
        ? 'Updated just now'
        : `Updated ${Math.floor(ageMs / 1000)}s ago`,
    stale: false,
  };
}
