import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  AutoTraderOutcomesModel,
  type AutoTraderOutcome,
} from './AutoTraderOutcomesModel';
import { positionFetch } from '../position_api';

export const ORDERS_POSITIONS_HISTORY_LIMIT = 200;
export const MAX_ACTIVITY_ONLY_TERMINAL_ORDERS = 100;
const REFRESH_MS = 15_000;

export type LifecycleIdentity = {
  opportunity_id?: string | null;
  episode_id?: string | null;
  trade_id?: string | null;
  profile_id?: string | null;
  profile_version?: number | null;
  execution_mode?: string | null;
  entry_policy?: string | null;
  source?: string | null;
};

export type LifecycleOrder = {
  id?: string | null;
  client_order_id?: string | null;
  parent_order_id?: string | null;
  symbol?: string | null;
  side?: string | null;
  quantity?: number | null;
  remaining_quantity?: number | null;
  limit_price?: number | null;
  timestamp?: number | null;
  expires_at?: number | null;
  time_in_force?: string | null;
  post_only?: boolean | null;
  lifecycle?: LifecycleIdentity | null;
  market_path_completeness?: string | null;
  market_path_dropped_samples?: number | null;
  [field: string]: unknown;
};

export type LifecycleBarrier = {
  id?: string | null;
  client_order_id?: string | null;
  parent_order_id?: string | null;
  symbol?: string | null;
  side?: string | null;
  quantity?: number | null;
  remaining_quantity?: number | null;
  filled_quantity?: number | null;
  trigger_price?: number | null;
  trigger_direction?: string | null;
  order_kind?: string | null;
  execution_type?: string | null;
  created_at?: number | null;
  triggered_at?: number | null;
  retry_after?: number | null;
  status?: string | null;
  average_execution_price?: number | null;
  fee_paid?: number | null;
  error?: string | null;
  source?: string | null;
  lifecycle?: LifecycleIdentity | null;
};

export type TerminalOrderPath = {
  order?: LifecycleOrder | null;
  terminal_reason?: string | null;
  terminal_at_ms?: number | null;
};

export type ActivityOrderIdentity = {
  timestamp?: number | null;
  symbol?: string | null;
  side?: string | null;
  execution_order_id?: string | null;
  client_order_id?: string | null;
  parent_order_id?: string | null;
  decision_id?: string | null;
  strategy?: string | null;
  order_type?: string | null;
  purpose?: string | null;
  quantity?: number | null;
  filled_quantity?: number | null;
  average_price?: number | null;
  status?: string | null;
  failure_reason?: string | null;
  fee_liquidity?: string | null;
};

export type ActivityOnlyTerminalOrder = {
  key: string;
  activity: ActivityOrderIdentity;
};

export type LifecycleHistory = {
  orders: LifecycleOrder[];
  barrierOrders: LifecycleBarrier[];
  activeOrders: LifecycleOrder[];
  activeBarrierOrders: LifecycleBarrier[];
  terminalPaths: TerminalOrderPath[];
  hasMoreOrders: boolean | null;
  hasMoreBarrierOrders: boolean | null;
  nextOrdersCursor: string | null;
  nextBarrierOrdersCursor: string | null;
  executionPathsTruncated: boolean | null;
};

export type OrdersPositionsState = {
  loading: boolean;
  pmAvailable: boolean;
  pmError?: string;
  outcomesLoading: boolean;
  outcomesError?: string;
  positions: AutoTraderOutcome[];
  closedPositions: ClosedPositionRecord[];
  activityOrders: ActivityOrderIdentity[];
  history: LifecycleHistory;
  fetchedAtMs: number | null;
  openSort?: PositionSort;
  closedSort?: ClosedSort;
};

export type PositionSort = {
  key:
    | 'symbol'
    | 'strategy'
    | 'side'
    | 'quantity'
    | 'holding'
    | 'notional'
    | 'net';
  direction: 'asc' | 'desc';
};

export type ClosedSort = {
  key:
    | 'symbol'
    | 'strategy'
    | 'side'
    | 'closedAt'
    | 'duration'
    | 'net'
    | 'fees';
  direction: 'asc' | 'desc';
};

export type ClosedPositionRecord = AutoTraderOutcome & {
  fillDetails: AutoTraderOutcome[];
};

function stableOutcomeTradeId(outcome: AutoTraderOutcome): string | null {
  const value =
    outcome.tradeId ?? outcome.ledgerTrade?.lifecycle?.trade_id ?? null;
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/**
 * A closed ledger summary and its fill rows are one position only when the
 * canonical trade ID matches exactly. Rows without that identity stay separate.
 */
export function groupClosedPositionRows(
  rows: readonly AutoTraderOutcome[],
): ClosedPositionRecord[] {
  const summariesByTradeId = new Map<string, AutoTraderOutcome[]>();
  for (const row of rows) {
    if (!row.rowId.startsWith('ledger-summary:')) continue;
    const tradeId = stableOutcomeTradeId(row);
    if (!tradeId) continue;
    const summaries = summariesByTradeId.get(tradeId) ?? [];
    summaries.push(row);
    summariesByTradeId.set(tradeId, summaries);
  }
  const fillsByTradeId = new Map<string, AutoTraderOutcome[]>();
  const groupedFillIds = new Set<string>();
  for (const row of rows) {
    if (!row.rowId.startsWith('ledger-trade:')) continue;
    const tradeId = stableOutcomeTradeId(row);
    if (!tradeId || summariesByTradeId.get(tradeId)?.length !== 1) continue;
    const fills = fillsByTradeId.get(tradeId) ?? [];
    fills.push(row);
    fillsByTradeId.set(tradeId, fills);
    groupedFillIds.add(row.rowId);
  }
  return rows
    .filter((row) => !groupedFillIds.has(row.rowId))
    .map((row) => ({
      ...row,
      fillDetails: row.rowId.startsWith('ledger-summary:')
        ? (fillsByTradeId.get(stableOutcomeTradeId(row) ?? '') ?? [])
        : [],
    }));
}

function compareNullable(left: unknown, right: unknown): number {
  if (left === right) return 0;
  if (left === undefined || left === null || left === '') return 1;
  if (right === undefined || right === null || right === '') return -1;
  if (typeof left === 'number' && typeof right === 'number')
    return left - right;
  return String(left).localeCompare(String(right), undefined, {
    numeric: true,
    sensitivity: 'base',
  });
}

function compareWithMissingLast(
  left: unknown,
  right: unknown,
  sign: number,
): number {
  const leftMissing = left === undefined || left === null || left === '';
  const rightMissing = right === undefined || right === null || right === '';
  if (leftMissing || rightMissing)
    return leftMissing === rightMissing ? 0 : leftMissing ? 1 : -1;
  return sign * compareNullable(left, right);
}

export function sortOpenPositions(
  rows: readonly AutoTraderOutcome[],
  sort: PositionSort,
): AutoTraderOutcome[] {
  const value = (row: AutoTraderOutcome): unknown => {
    switch (sort.key) {
      case 'symbol':
        return row.symbol;
      case 'strategy':
        return row.strategy;
      case 'side':
        return row.direction || row.side;
      case 'quantity':
        return Math.abs(row.remainingQuantity ?? row.quantity);
      case 'holding':
        return row.openedAt;
      case 'notional':
        return row.entryNotionalUsd;
      case 'net':
        return row.netPnlUsd;
    }
  };
  const sign = sort.direction === 'asc' ? 1 : -1;
  return [...rows].sort((left, right) =>
    compareWithMissingLast(value(left), value(right), sign),
  );
}

export function sortClosedPositions(
  rows: readonly ClosedPositionRecord[],
  sort: ClosedSort,
): ClosedPositionRecord[] {
  const value = (row: ClosedPositionRecord): unknown => {
    switch (sort.key) {
      case 'symbol':
        return row.symbol;
      case 'strategy':
        return row.strategy;
      case 'side':
        return row.direction || row.side;
      case 'closedAt':
        return row.closedAt;
      case 'duration':
        return row.closedAt === undefined
          ? undefined
          : row.closedAt - row.openedAt;
      case 'net':
        return row.netPnlUsd;
      case 'fees':
        return row.totalFees;
    }
  };
  const sign = sort.direction === 'asc' ? 1 : -1;
  return [...rows].sort((left, right) =>
    compareWithMissingLast(value(left), value(right), sign),
  );
}

function nextSort<K extends string>(
  current: { key: K; direction: 'asc' | 'desc' },
  key: K,
) {
  return {
    key,
    direction:
      current.key === key && current.direction === 'asc'
        ? ('desc' as const)
        : ('asc' as const),
  };
}

const emptyHistory: LifecycleHistory = {
  orders: [],
  barrierOrders: [],
  activeOrders: [],
  activeBarrierOrders: [],
  terminalPaths: [],
  hasMoreOrders: null,
  hasMoreBarrierOrders: null,
  nextOrdersCursor: null,
  nextBarrierOrdersCursor: null,
  executionPathsTruncated: null,
};

const emptyState: OrdersPositionsState = {
  loading: true,
  pmAvailable: false,
  outcomesLoading: true,
  positions: [],
  closedPositions: [],
  activityOrders: [],
  history: emptyHistory,
  fetchedAtMs: null,
  openSort: { key: 'symbol', direction: 'asc' },
  closedSort: { key: 'closedAt', direction: 'desc' },
};

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringOrNull(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function arrayOrEmpty<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

export function parseLifecycleHistory(
  payload: unknown,
): LifecycleHistory | null {
  const root = record(payload);
  if (
    !root ||
    !Array.isArray(root.orders) ||
    !Array.isArray(root.barrier_orders) ||
    !Array.isArray(root.active_orders) ||
    !Array.isArray(root.active_barrier_orders)
  )
    return null;
  return {
    orders: root.orders as LifecycleOrder[],
    barrierOrders: root.barrier_orders as LifecycleBarrier[],
    activeOrders: root.active_orders as LifecycleOrder[],
    activeBarrierOrders: root.active_barrier_orders as LifecycleBarrier[],
    terminalPaths: arrayOrEmpty<TerminalOrderPath>(root.execution_paths),
    hasMoreOrders:
      typeof root.has_more_orders === 'boolean' ? root.has_more_orders : null,
    hasMoreBarrierOrders:
      typeof root.has_more_barrier_orders === 'boolean'
        ? root.has_more_barrier_orders
        : null,
    nextOrdersCursor: stringOrNull(root.next_orders_cursor),
    nextBarrierOrdersCursor: stringOrNull(root.next_barrier_orders_cursor),
    executionPathsTruncated:
      typeof root.execution_paths_truncated === 'boolean'
        ? root.execution_paths_truncated
        : null,
  };
}

function idSet(
  values: Array<{ execution?: unknown; client?: unknown; parent?: unknown }>,
): Set<string> {
  const result = new Set<string>();
  for (const value of values) {
    if (typeof value.execution === 'string' && value.execution.trim())
      result.add(`execution:${value.execution.trim()}`);
    if (typeof value.client === 'string' && value.client.trim())
      result.add(`client:${value.client.trim()}`);
    if (typeof value.parent === 'string' && value.parent.trim())
      result.add(`parent:${value.parent.trim()}`);
  }
  return result;
}

function pmOrderIds(order: LifecycleOrder): Set<string> {
  return idSet([
    {
      execution: order.id,
      client: order.client_order_id,
      parent: order.parent_order_id,
    },
  ]);
}

function activityOrderIds(order: ActivityOrderIdentity): Set<string> {
  return idSet([
    {
      execution: order.execution_order_id,
      client: order.client_order_id,
      parent: order.parent_order_id,
    },
  ]);
}

function activityHasUniquePmRecord(
  activity: ActivityOrderIdentity,
  pmOrders: readonly LifecycleOrder[],
): boolean {
  const identities: Array<{ matches: (order: LifecycleOrder) => boolean }> = [];
  if (activity.execution_order_id)
    identities.push({
      matches: (order) =>
        pmOrderIds(order).has(`execution:${activity.execution_order_id}`),
    });
  if (activity.client_order_id)
    identities.push({
      matches: (order) =>
        pmOrderIds(order).has(`client:${activity.client_order_id}`),
    });
  if (activity.parent_order_id)
    identities.push({
      matches: (order) =>
        pmOrderIds(order).has(`parent:${activity.parent_order_id}`),
    });
  for (const identity of identities) {
    const matches = pmOrders.filter(identity.matches);
    if (matches.length === 1) return true;
    if (matches.length > 1) return false;
  }
  return false;
}

export function linkActivityOrder(
  order: LifecycleOrder,
  activityOrders: readonly ActivityOrderIdentity[],
): ActivityOrderIdentity | null {
  const pmIds = pmOrderIds(order);
  if (pmIds.size === 0) return null;
  const matches = activityOrders.filter((activity) =>
    [...pmIds].some((id) => activityOrderIds(activity).has(id)),
  );
  return matches.length === 1 ? matches[0] : null;
}

function hasTerminalActivityStatus(order: ActivityOrderIdentity): boolean {
  const status =
    typeof order.status === 'string' ? order.status.trim().toLowerCase() : '';
  return /^(filled|complete|completed|closed|expired|expired_without_full_fill|cancelled|canceled|rejected|failed|terminated)$/.test(
    status,
  );
}

export function buildActivityOnlyTerminalOrders(
  activityOrders: readonly ActivityOrderIdentity[],
  pmOrders: readonly LifecycleOrder[],
  limit = MAX_ACTIVITY_ONLY_TERMINAL_ORDERS,
): {
  rows: ActivityOnlyTerminalOrder[];
  sourceCount: number;
  truncated: boolean;
} {
  const sourceRows = activityOrders.filter((activity) => {
    if (!hasTerminalActivityStatus(activity)) return false;
    return !activityHasUniquePmRecord(activity, pmOrders);
  });
  const sorted = sourceRows
    .slice()
    .sort((left, right) => (right.timestamp ?? 0) - (left.timestamp ?? 0));
  const rows = sorted.slice(0, Math.max(0, limit)).map((activity, index) => ({
    key:
      activity.execution_order_id ??
      activity.client_order_id ??
      activity.parent_order_id ??
      activity.decision_id ??
      `activity-record-${index}`,
    activity,
  }));
  return {
    rows,
    sourceCount: sorted.length,
    truncated: sorted.length > rows.length,
  };
}

export function terminalPathForOrder(
  order: LifecycleOrder,
  paths: readonly TerminalOrderPath[],
): TerminalOrderPath | null {
  const ids = pmOrderIds(order);
  if (ids.size === 0) return null;
  const matches = paths.filter((path) => {
    const pathOrder = path.order;
    return !!pathOrder && [...ids].some((id) => pmOrderIds(pathOrder).has(id));
  });
  return matches.length === 1 ? matches[0] : null;
}

function outcomeIdentifiers(outcome: AutoTraderOutcome): Set<string> {
  const lifecycle = outcome.ledgerTrade?.lifecycle;
  const result = new Set<string>();
  if (outcome.decisionId && !outcome.decisionId.startsWith('ledger:'))
    result.add(`decision:${outcome.decisionId}`);
  if (outcome.tradeId || lifecycle?.trade_id)
    result.add(`trade:${outcome.tradeId ?? lifecycle?.trade_id}`);
  if (outcome.episodeId || lifecycle?.episode_id)
    result.add(`episode:${outcome.episodeId ?? lifecycle?.episode_id}`);
  if (outcome.opportunityId || lifecycle?.opportunity_id)
    result.add(
      `opportunity:${outcome.opportunityId ?? lifecycle?.opportunity_id}`,
    );
  return result;
}

function barrierIdentifiers(barrier: LifecycleBarrier): Set<string> {
  const result = new Set<string>();
  if (barrier.lifecycle?.trade_id)
    result.add(`trade:${barrier.lifecycle.trade_id}`);
  if (barrier.lifecycle?.episode_id)
    result.add(`episode:${barrier.lifecycle.episode_id}`);
  if (barrier.lifecycle?.opportunity_id)
    result.add(`opportunity:${barrier.lifecycle.opportunity_id}`);
  if (barrier.parent_order_id)
    result.add(`execution:${barrier.parent_order_id}`);
  return result;
}

export function barriersForPosition(
  outcome: AutoTraderOutcome,
  barriers: readonly LifecycleBarrier[],
  activityOrders: readonly ActivityOrderIdentity[],
): LifecycleBarrier[] {
  const positionIds = outcomeIdentifiers(outcome);
  if (positionIds.size === 0) return [];
  const orderDecisionIds = new Map<string, string[]>();
  for (const order of activityOrders) {
    if (
      !order.execution_order_id ||
      !order.decision_id ||
      order.decision_id.startsWith('ledger:')
    )
      continue;
    const decisions = orderDecisionIds.get(order.execution_order_id) ?? [];
    decisions.push(order.decision_id);
    orderDecisionIds.set(order.execution_order_id, decisions);
  }
  return barriers.filter((barrier) => {
    const ids = barrierIdentifiers(barrier);
    const direct = [...ids].some((id) => positionIds.has(id));
    const linkedDecisionIds = barrier.parent_order_id
      ? (orderDecisionIds.get(barrier.parent_order_id) ?? [])
      : [];
    const decisionMatch =
      linkedDecisionIds.length === 1 &&
      positionIds.has(`decision:${linkedDecisionIds[0]}`);
    return direct || decisionMatch;
  });
}

type OutcomeModelState = {
  rows: AutoTraderOutcome[];
  closedLedgerRows: AutoTraderOutcome[];
  strategyOrders: ActivityOrderIdentity[];
  loading: boolean;
  error?: string;
};

export class OrdersPositionsWidgetModel {
  readonly state$ = new BehaviorSubject<OrdersPositionsState>(emptyState);
  private readonly outcomes: AutoTraderOutcomesModel;
  private readonly fetcher: (path: string) => Promise<Response>;
  private readonly now: () => number;
  private outcomesSubscription?: Subscription;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private requestInFlight = false;

  constructor(
    outcomes = new AutoTraderOutcomesModel(),
    fetcher: (path: string) => Promise<Response> = (path) =>
      positionFetch(path),
    now: () => number = Date.now,
  ) {
    this.outcomes = outcomes;
    this.fetcher = fetcher;
    this.now = now;
  }

  start() {
    if (this.outcomesSubscription) return;
    this.outcomes.start();
    this.outcomesSubscription = this.outcomes.state$.subscribe((raw) => {
      const state = raw as OutcomeModelState;
      this.state$.next({
        ...this.state$.value,
        positions: state.rows.filter((row) => row.status === 'Open'),
        closedPositions: groupClosedPositionRows(
          state.closedLedgerRows.filter((row) => row.status === 'Closed'),
        ),
        activityOrders: state.strategyOrders ?? [],
        outcomesLoading: state.loading,
        outcomesError: state.error,
      });
    });
    void this.refreshHistory();
    this.refreshTimer = setInterval(
      () => void this.refreshHistory(),
      REFRESH_MS,
    );
  }

  stop() {
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = undefined;
    this.outcomesSubscription?.unsubscribe();
    this.outcomesSubscription = undefined;
    this.outcomes.stop();
  }

  sortOpenBy(key: PositionSort['key']) {
    const current = this.state$.value.openSort ?? {
      key: 'symbol' as const,
      direction: 'asc' as const,
    };
    this.state$.next({
      ...this.state$.value,
      openSort: nextSort(current, key),
    });
  }

  sortClosedBy(key: ClosedSort['key']) {
    const current = this.state$.value.closedSort ?? {
      key: 'closedAt' as const,
      direction: 'desc' as const,
    };
    this.state$.next({
      ...this.state$.value,
      closedSort: nextSort(current, key),
    });
  }

  async refreshHistory() {
    if (this.requestInFlight) return;
    this.requestInFlight = true;
    this.state$.next({
      ...this.state$.value,
      loading: this.state$.value.fetchedAtMs === null,
      pmError: undefined,
    });
    try {
      const response = await this.fetcher(
        `/positions/orders/history?limit=${ORDERS_POSITIONS_HISTORY_LIMIT}`,
      );
      if (!response.ok)
        throw new Error(`PM order history unavailable (${response.status})`);
      const history = parseLifecycleHistory(await response.json());
      if (!history) throw new Error('PM order history contract unavailable');
      this.state$.next({
        ...this.state$.value,
        loading: false,
        pmAvailable: true,
        pmError: undefined,
        history,
        fetchedAtMs: this.now(),
      });
    } catch (error) {
      this.state$.next({
        ...this.state$.value,
        loading: false,
        pmError:
          error instanceof Error
            ? error.message
            : 'PM order history unavailable',
      });
    } finally {
      this.requestInFlight = false;
    }
  }
}
