import { BehaviorSubject, type Subscription } from 'rxjs';
import { AutoTraderModel, type AutoTraderState } from './AutoTraderModel';
import type { LatestEntryDiagnostic } from './AutoTraderModel';

export type CandidateCoverage = {
  state: string;
  retained: number;
  observed?: number | null;
  truncated: boolean;
};

export type FunnelCandidate = NonNullable<
  LatestEntryDiagnostic['candidateDetails']
>[number] & {
  selectionMode?: string | null;
  strictEligibility?: boolean | null;
  exploratoryEligibility?: boolean | null;
};

export type FunnelCycle = LatestEntryDiagnostic & {
  candidateCoverage?: CandidateCoverage | null;
  candidateDetails?: FunnelCandidate[];
};

export type CandidateFunnelRecord = {
  cycleId: string;
  cycleTimestampMs: number;
  candidate: FunnelCandidate;
};

export type CandidateFunnelGroup = {
  key: string;
  symbol: string | null;
  strategy: string | null;
  records: CandidateFunnelRecord[];
};

export const MAX_FUNNEL_CYCLES = 35;

export function boundedFunnelHistory(
  history: readonly LatestEntryDiagnostic[],
  latest: LatestEntryDiagnostic | null,
): FunnelCycle[] {
  const byCycle = new Map<string, FunnelCycle>();
  for (const raw of history) byCycle.set(raw.cycleId, raw as FunnelCycle);
  if (latest) byCycle.set(latest.cycleId, latest as FunnelCycle);
  return Array.from(byCycle.values())
    .sort((left, right) => left.timestampMs - right.timestampMs)
    .slice(-MAX_FUNNEL_CYCLES);
}

export function candidateFunnelRecords(
  cycles: readonly FunnelCycle[],
): CandidateFunnelRecord[] {
  return cycles.flatMap((cycle) =>
    (cycle.candidateDetails ?? []).map((candidate) => ({
      cycleId: cycle.cycleId,
      cycleTimestampMs: cycle.timestampMs,
      candidate,
    })),
  );
}

export function groupCandidateFunnelRecords(
  records: readonly CandidateFunnelRecord[],
  dimension: 'symbol' | 'strategy' | 'symbol_strategy',
): CandidateFunnelGroup[] {
  const groups = new Map<string, CandidateFunnelGroup>();
  for (const record of records) {
    const symbol = record.candidate.symbol || null;
    const strategy = record.candidate.strategy ?? null;
    const key =
      dimension === 'symbol'
        ? (symbol ?? 'Unknown symbol')
        : dimension === 'strategy'
          ? (strategy ?? 'Unknown strategy')
          : `${symbol ?? 'Unknown symbol'} / ${strategy ?? 'Unknown strategy'}`;
    const group = groups.get(key) ?? {
      key,
      symbol,
      strategy,
      records: [],
    };
    group.records.push(record);
    groups.set(key, group);
  }
  return Array.from(groups.values()).sort((left, right) =>
    left.key.localeCompare(right.key),
  );
}

type SymbolGateHit = AutoTraderState['symbolGateHits'][number];
export type GateHit = {
  gate: string;
  count: number;
  detail: string;
  distance: number | null;
};
export type SymbolDiagnosticRow = {
  kind: 'symbol';
  symbol: string;
  direction: string;
  gates: GateHit[];
  distance: number | null;
  readinessScore: number;
  expanded: boolean;
};
export type GateDetailRow = {
  kind: 'gate';
  symbol: string;
  direction: string;
  distance: number | null;
  gate: string;
  count: number;
  detail: string;
};
export type DiagnosticsRow = SymbolDiagnosticRow | GateDetailRow;
export type AutoTraderDiagnosticsState = AutoTraderState & {
  diagnosticRows: DiagnosticsRow[];
  symbolGroupCount: number;
  funnelHistory: FunnelCycle[];
  availableFromApiCount: number;
  displayedCycleCount: number;
};

export function buildAutoTraderDiagnosticRows(
  hits: SymbolGateHit[],
  expandedGroups: ReadonlySet<string>,
): DiagnosticsRow[] {
  const grouped = new Map<
    string,
    Omit<
      SymbolDiagnosticRow,
      'kind' | 'distance' | 'readinessScore' | 'expanded'
    >
  >();
  for (const item of hits) {
    const key = item.symbol + ':' + item.direction;
    const group = grouped.get(key) ?? {
      symbol: item.symbol,
      direction: item.direction,
      gates: [],
    };
    group.gates.push({
      gate: item.gate,
      count: item.count,
      detail: item.last_detail,
      distance: item.distance_to_pass,
    });
    grouped.set(key, group);
  }

  const symbols: SymbolDiagnosticRow[] = Array.from(grouped.values())
    .map((group) => ({
      ...group,
      kind: 'symbol' as const,
      expanded: expandedGroups.has(group.symbol + ':' + group.direction),
      distance: group.gates.reduce<number | null>(
        (max, item) =>
          item.distance == null ? max : Math.max(max ?? 0, item.distance),
        null,
      ),
      readinessScore: group.gates.some((item) => item.gate === 'Readiness')
        ? 100
        : Math.round(
            100 /
              (1 +
                group.gates.reduce(
                  (penalty, item) =>
                    penalty +
                    (item.distance == null ? 1 : Math.max(item.distance, 0.05)),
                  0,
                )),
          ),
    }))
    .sort(
      (a, b) =>
        a.symbol.localeCompare(b.symbol) ||
        a.direction.localeCompare(b.direction),
    );

  return symbols.flatMap((group) => [
    group,
    ...(group.expanded
      ? group.gates.map(
          (gate): GateDetailRow => ({
            kind: 'gate',
            symbol: group.symbol,
            direction: group.direction,
            distance: group.distance,
            gate: gate.gate,
            count: gate.count,
            detail: gate.detail,
          }),
        )
      : []),
  ]);
}

export class AutoTraderDiagnosticsModel {
  readonly state$: BehaviorSubject<AutoTraderDiagnosticsState>;
  private readonly expandedGroups = new Set<string>();
  private sourceSubscription?: Subscription;
  private readonly source: AutoTraderModel;

  constructor(source: AutoTraderModel = new AutoTraderModel()) {
    this.source = source;
    this.state$ = new BehaviorSubject(this.project(source.state$.value));
  }

  start() {
    if (this.sourceSubscription) return;
    this.source.start();
    this.sourceSubscription = this.source.state$.subscribe((state) =>
      this.state$.next(this.project(state)),
    );
  }

  stop() {
    this.sourceSubscription?.unsubscribe();
    this.sourceSubscription = undefined;
    this.source.stop();
  }

  toggleGroup(symbol: string, direction: string) {
    const key = symbol + ':' + direction;
    if (this.expandedGroups.has(key)) this.expandedGroups.delete(key);
    else this.expandedGroups.add(key);
    this.state$.next(this.project(this.source.state$.value));
  }

  private project(state: AutoTraderState): AutoTraderDiagnosticsState {
    const diagnosticRows = buildAutoTraderDiagnosticRows(
      state.symbolGateHits,
      this.expandedGroups,
    );
    const funnelHistory = boundedFunnelHistory(
      state.entryDiagnosticHistory,
      state.latestEntryDiagnostic,
    );
    return {
      ...state,
      diagnosticRows,
      symbolGroupCount: diagnosticRows.filter((row) => row.kind === 'symbol')
        .length,
      funnelHistory,
      availableFromApiCount: state.entryDiagnosticHistory.length,
      displayedCycleCount: funnelHistory.length,
    };
  }
}
