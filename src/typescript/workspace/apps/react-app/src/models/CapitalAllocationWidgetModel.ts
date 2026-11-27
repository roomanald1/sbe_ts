import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  AutoTraderModel,
  type AutoTraderState,
  type CapitalAllocation,
} from './AutoTraderModel';

export type CapitalAllocationWidgetState = {
  active: boolean;
  loading: boolean;
  capitalAllocation: CapitalAllocation | undefined;
  availableNotional: number;
  dailyNotionalLimit: number | null;
  usedNotionalToday: number | null;
  dailyNotionalRemaining: number | null;
  maxOpenPositions: number | null;
  explorationLimitPercent: number | null;
  tab: CapitalAllocationTab;
  groupBy: CapitalAllocationGroup;
  displayedRows: CapitalAllocationDisplayRow[];
  reportedReasonRows: Array<{ reason: string; rowCount: number }>;
  poolSegments: CapitalAllocationSegment[];
  poolReconciles: boolean | null;
  utilizationPercent: number | null;
  error: string | null;
};

export type CapitalAllocationTab = 'allocation' | 'candidates' | 'history';
export type CapitalAllocationGroup = 'strategy' | 'symbol' | 'horizon';
export type CapitalAllocationDisplayRow = {
  key: string;
  label: string;
  rowCount: number;
  effectiveAllocation: number;
  used: number;
  reserved: number;
  sharePercent: number | null;
  utilizationPercent: number | null;
  evidenceEligible: boolean | null;
  states: string[];
  reasons: string[];
};
export type CapitalAllocationSegment = {
  id: 'used' | 'reserved' | 'available';
  value: number;
  sharePercent: number;
};

type AllocationSource = Pick<AutoTraderModel, 'state$' | 'start' | 'stop'>;

function project(
  source: AutoTraderState,
  active: boolean,
  tab: CapitalAllocationTab,
  groupBy: CapitalAllocationGroup,
): CapitalAllocationWidgetState {
  const loading = source.loading;
  const dailyNotionalLimit = loading
    ? null
    : Number.isFinite(source.serverDailyNotionalLimit)
      ? source.serverDailyNotionalLimit
      : null;
  const usedNotionalToday =
    loading || !Number.isFinite(source.usedNotionalToday)
      ? null
      : source.usedNotionalToday;
  const dailyNotionalRemaining =
    dailyNotionalLimit === null || usedNotionalToday === null
      ? null
      : dailyNotionalLimit - usedNotionalToday;
  const parsedMaxPositions = Number(source.maxOpenPositions);
  const maxOpenPositions =
    loading || !Number.isFinite(parsedMaxPositions) ? null : parsedMaxPositions;
  const explorationLimitPercent =
    loading || !Number.isFinite(source.serverExplorationPercent)
      ? null
      : source.serverExplorationPercent;
  const allocation = source.capitalAllocation;
  const validPoolAmounts =
    allocation !== undefined &&
    [
      allocation.poolLimit,
      allocation.used,
      allocation.reserved,
      allocation.available,
    ].every((value) => Number.isFinite(value) && value >= 0);
  const poolReconciles =
    allocation && validPoolAmounts
      ? Math.abs(
          allocation.used +
            allocation.reserved +
            allocation.available -
            allocation.poolLimit,
        ) <= Math.max(0.01, allocation.poolLimit * 1e-6)
      : null;
  const utilizationPercent =
    allocation && allocation.poolLimit > 0
      ? ((allocation.used + allocation.reserved) / allocation.poolLimit) * 100
      : null;
  const poolSegments =
    allocation &&
    allocation.poolLimit > 0 &&
    [allocation.used, allocation.reserved, allocation.available].every(
      (value) => Number.isFinite(value) && value >= 0,
    )
      ? (
          [
            ['used', allocation.used],
            ['reserved', allocation.reserved],
            ['available', allocation.available],
          ] as const
        ).map(([id, value]) => ({
          id,
          value,
          sharePercent: (value / allocation.poolLimit) * 100,
        }))
      : [];
  const reasonCounts = new Map<string, number>();
  for (const row of allocation?.rows ?? []) {
    const reason = row.reason.trim();
    if (reason) reasonCounts.set(reason, (reasonCounts.get(reason) ?? 0) + 1);
  }
  return {
    active,
    loading,
    capitalAllocation: allocation,
    availableNotional: source.availableNotional,
    dailyNotionalLimit,
    usedNotionalToday,
    dailyNotionalRemaining,
    maxOpenPositions,
    explorationLimitPercent,
    tab,
    groupBy,
    displayedRows: allocation
      ? groupCapitalAllocationRows(allocation, groupBy)
      : [],
    reportedReasonRows: [...reasonCounts]
      .map(([reason, rowCount]) => ({ reason, rowCount }))
      .sort(
        (left, right) =>
          right.rowCount - left.rowCount ||
          left.reason.localeCompare(right.reason),
      ),
    poolSegments,
    poolReconciles,
    utilizationPercent,
    error: source.error ?? null,
  };
}

export function groupCapitalAllocationRows(
  allocation: CapitalAllocation,
  groupBy: CapitalAllocationGroup,
): CapitalAllocationDisplayRow[] {
  const groups = new Map<string, CapitalAllocation['rows']>();
  for (const row of allocation.rows) {
    const label = row[groupBy] || 'Unknown';
    const current = groups.get(label) ?? [];
    current.push(row);
    groups.set(label, current);
  }
  return [...groups.entries()]
    .map(([label, rows]) => {
      const effectiveAllocation = rows.reduce(
        (total, row) => total + row.effectiveAllocation,
        0,
      );
      const used = rows.reduce((total, row) => total + row.used, 0);
      const reserved = rows.reduce((total, row) => total + row.reserved, 0);
      const eligibility = rows.map((row) => row.evidenceEligible);
      const evidenceEligible = eligibility.every((value) => value === true)
        ? true
        : eligibility.every((value) => value === false)
          ? false
          : null;
      return {
        key: label,
        label,
        rowCount: rows.length,
        effectiveAllocation,
        used,
        reserved,
        sharePercent:
          allocation.poolLimit > 0
            ? (effectiveAllocation / allocation.poolLimit) * 100
            : null,
        utilizationPercent:
          effectiveAllocation > 0 ? (used / effectiveAllocation) * 100 : null,
        evidenceEligible,
        states: [...new Set(rows.map((row) => row.state).filter(Boolean))],
        reasons: [...new Set(rows.map((row) => row.reason).filter(Boolean))],
      };
    })
    .sort(
      (left, right) =>
        right.effectiveAllocation - left.effectiveAllocation ||
        left.label.localeCompare(right.label),
    );
}

/** Per-widget lifecycle boundary over the authoritative shared AutoTrader status source. */
export class CapitalAllocationWidgetModel {
  readonly state$: BehaviorSubject<CapitalAllocationWidgetState>;
  private subscription?: Subscription;
  private running = false;
  private generation = 0;
  readonly instanceId: string;
  private readonly source: AllocationSource;
  private tab: CapitalAllocationTab = 'allocation';
  private groupBy: CapitalAllocationGroup = 'strategy';

  constructor(
    instanceId: string,
    source: AllocationSource = new AutoTraderModel(),
  ) {
    this.instanceId = instanceId;
    this.source = source;
    this.state$ = new BehaviorSubject(
      project(source.state$.value, false, this.tab, this.groupBy),
    );
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    const generation = ++this.generation;
    this.subscription = this.source.state$.subscribe((state) => {
      if (this.running && generation === this.generation)
        this.state$.next(project(state, true, this.tab, this.groupBy));
    });
    this.source.start();
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    this.generation += 1;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.source.stop();
    this.state$.next(
      project(this.source.state$.value, false, this.tab, this.groupBy),
    );
  }

  setTab(tab: CapitalAllocationTab): void {
    this.tab = tab;
    this.state$.next(
      project(this.source.state$.value, this.running, this.tab, this.groupBy),
    );
  }

  setGroupBy(groupBy: CapitalAllocationGroup): void {
    this.groupBy = groupBy;
    this.state$.next(
      project(this.source.state$.value, this.running, this.tab, this.groupBy),
    );
  }

  dispose(): void {
    this.stop();
    this.state$.complete();
  }
}
