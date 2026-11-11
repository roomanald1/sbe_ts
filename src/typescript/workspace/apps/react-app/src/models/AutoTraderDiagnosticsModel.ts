import { BehaviorSubject, type Subscription } from 'rxjs';
import { AutoTraderModel, type AutoTraderState } from './AutoTraderModel';

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
    return {
      ...state,
      diagnosticRows,
      symbolGroupCount: diagnosticRows.filter((row) => row.kind === 'symbol')
        .length,
    };
  }
}
