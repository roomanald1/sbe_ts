import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef, type ICellRendererParams } from 'ag-grid-community';
import { AutoTraderModel } from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const diagnosticsGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31',
  foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24',
});
type GateHit = { gate: string; count: number; detail: string; distance: number | null };
type SymbolGroup = {
  kind: 'symbol'; symbol: string; direction: string; gates: GateHit[];
  distance: number | null; readinessScore: number; expanded: boolean;
};
type GateDetailRow = {
  kind: 'gate'; symbol: string; direction: string; distance: number | null;
  gate: string; count: number; detail: string;
};
type DiagnosticsRow = SymbolGroup | GateDetailRow;
type GateColumnParams = ICellRendererParams<DiagnosticsRow> & { onToggle: (key: string) => void };
function GateColumnCell({ data, onToggle }: GateColumnParams) {
  if (!data) return null;
  if (data.kind === 'gate') return <span className="auto-trader-detail-gate">{data.gate}</span>;
  const key = data.symbol + ':' + data.direction;
  return <button className="auto-trader-detail-toggle" type="button" onClick={(event) => { event.stopPropagation(); onToggle(key); }}>{data.expanded ? 'Hide gates' : 'Show ' + data.gates.length + ' gate' + (data.gates.length === 1 ? '' : 's')}</button>;
}

export function AutoTraderDiagnostics() {
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(() => new Set());
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const symbolGroups = useMemo(() => {
    const grouped = new Map<string, { symbol: string; direction: string; gates: Array<{ gate: string; count: number; detail: string; distance: number | null }> }>();
    state.symbolGateHits.forEach((item) => {
      const key = item.symbol + ':' + item.direction;
      const group = grouped.get(key) ?? { symbol: item.symbol, direction: item.direction, gates: [] };
      group.gates.push({ gate: item.gate, count: item.count, detail: item.last_detail, distance: item.distance_to_pass });
      grouped.set(key, group);
    });
    return Array.from(grouped.values()).map((group) => ({
      ...group,
      kind: 'symbol' as const,
      expanded: expandedGroups.has(group.symbol + ':' + group.direction),
      distance: group.gates.reduce<number | null>((max, item) => item.distance === null || item.distance === undefined ? max : Math.max(max ?? 0, item.distance), null),
      readinessScore: group.gates.some((item) => item.gate === 'Readiness') ? 100 : Math.round(100 / (1 + group.gates.reduce((penalty, item) => penalty + (item.distance === null || item.distance === undefined ? 1 : Math.max(item.distance, 0.05)), 0))),
    })).sort((a, b) => a.symbol.localeCompare(b.symbol) || a.direction.localeCompare(b.direction));
  }, [state.symbolGateHits, expandedGroups]);
  const toggleGroup = (key: string) => setExpandedGroups((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  const gridRows = useMemo<DiagnosticsRow[]>(() => symbolGroups.flatMap((group) => [
    group,
    ...(group.expanded ? group.gates.map((gate): GateDetailRow => ({
      kind: 'gate', symbol: group.symbol, direction: group.direction, distance: group.distance,
      gate: gate.gate, count: gate.count, detail: gate.detail,
    })) : []),
  ]), [symbolGroups]);
  const symbolColumns = useMemo<ColDef<DiagnosticsRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 120, flex: 1, sort: 'asc', sortIndex: 0, comparator: (a, b, left, right) => { const bySymbol = String(a ?? '').localeCompare(String(b ?? '')); if (bySymbol) return bySymbol; const leftRow = left.data; const rightRow = right.data; if (!leftRow || !rightRow) return 0; const byDirection = leftRow.direction.localeCompare(rightRow.direction); if (byDirection) return byDirection; if (leftRow.kind !== rightRow.kind) return leftRow.kind === 'symbol' ? -1 : 1; return leftRow.kind === 'gate' && rightRow.kind === 'gate' ? leftRow.gate.localeCompare(rightRow.gate) : 0; }, cellRenderer: (params: ICellRendererParams<DiagnosticsRow>) => params.data?.kind === 'symbol' ? params.value : '' },
    { field: 'direction', headerName: 'Direction', minWidth: 100, valueFormatter: ({ value, data }) => data?.kind === 'symbol' ? String(value ?? '').toUpperCase() : '' },
    { field: 'distance', headerName: 'Distance to pass', minWidth: 145, valueFormatter: ({ value, data }) => data?.kind !== 'symbol' ? '' : value == null ? 'Unavailable' : Number(value).toFixed(2) + ' gap' },
    { field: 'readinessScore', headerName: 'Readiness', minWidth: 110, valueFormatter: ({ value, data }) => data?.kind === 'symbol' ? String(value ?? 0) + '/100' : '' },
    { field: 'gate', colId: 'gate', headerName: 'Gate', minWidth: 170, sortable: false, filter: false, cellRenderer: GateColumnCell, cellRendererParams: { onToggle: toggleGroup } },
    { field: 'count', headerName: 'Blocked', minWidth: 90, sortable: false, filter: false, valueFormatter: ({ value }) => value == null ? '' : Number(value).toLocaleString() },
    { field: 'detail', headerName: 'Latest detail', minWidth: 280, flex: 2, sortable: false, filter: false, tooltipField: 'detail', cellClass: 'auto-trader-nested-detail', wrapText: true, autoHeight: true },
  ], []);

  return (
    <section className="feed-section auto-trader-diagnostics" aria-label="Auto Trader diagnostics">
      <FeedToolbar title="Auto Trader diagnostics" />
      <div className="auto-trader-diagnostics-content">
        <p className="auto-trader-diagnostics-note">
          Readiness is a ranking index: numeric blockers add their normalized gap (at least 0.05), and qualitative blockers add one unit. A submitted entry is 100/100 after passing the gates. Distance separately shows the largest normalized numeric shortfall.
        </p>
        <details className="auto-trader-diagnostics-block auto-trader-gate-hits">
          <summary className="auto-trader-diagnostics-heading">
            <strong>Gate hits</strong>
            <span>{state.decisionsSinceTrade.toLocaleString()} scans</span>
          </summary>
          {state.gateHits.length > 0 ? (
            <ul className="auto-trader-gate-list">
              {state.gateHits.slice().sort((a, b) => b.count - a.count).map((item) => (
                <li key={item.gate}><span>{item.gate}</span><strong>{item.count.toLocaleString()} scans</strong></li>
              ))}
            </ul>
          ) : <p className="auto-trader-diagnostics-empty">{state.loading ? 'Loading current gate counts…' : 'No blocked gates recorded for this running session.'}</p>}
        </details>

        <section className="auto-trader-diagnostics-block" aria-label="Gate counts by symbol">
          <div className="auto-trader-diagnostics-heading">
            <strong>By symbol and direction</strong>
            <span>{symbolGroups.length} groups</span>
          </div>
          {symbolGroups.length > 0 ? (
            <div className="data-grid-wrap auto-trader-symbol-grid">
              <AgGridReact<DiagnosticsRow> animateRows={false} rowData={gridRows} columnDefs={symbolColumns} defaultColDef={{ sortable: true, resizable: true, filter: true, minWidth: 85, cellStyle: ({ data }) => {
                if (data?.distance == null) return { backgroundColor: '#20252c', color: '#c1cad4' };
                const hue = Math.round(120 * (1 - Math.min(Math.max(data.distance, 0), 1)));
                return { backgroundColor: 'hsl(' + hue + ' 58% 19%)', color: 'hsl(' + hue + ' 75% 83%)' };
              } }} theme={diagnosticsGridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={29} getRowId={({ data }) => data.kind === 'symbol' ? 'symbol:' + data.symbol + ':' + data.direction : 'gate:' + data.symbol + ':' + data.direction + ':' + data.gate} getRowClass={({ data }) => data?.kind === 'gate' ? 'auto-trader-gate-detail-row' : undefined} />
            </div>
          ) : <p className="auto-trader-diagnostics-empty">Symbol-level counts cover signal quality and freshness checks plus post-selection market-data, position/cooldown, capacity, order-type, and deep-book gates when a candidate was selected.</p>}
        </section>

        <section className="auto-trader-diagnostics-block" aria-label="Latest signal gate checks">
          <div className="auto-trader-diagnostics-heading"><strong>Latest signal check</strong></div>
          {state.latestSignalDiagnostic ? (
            <pre className="auto-trader-latest-check">{state.latestSignalDiagnostic}</pre>
          ) : <p className="auto-trader-diagnostics-empty">Waiting for a completed decision cycle.</p>}
        </section>

        {state.lastDecisionAt > 0 && <time className="auto-trader-diagnostics-time" dateTime={new Date(state.lastDecisionAt).toISOString()}>
          Last decision {new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(state.lastDecisionAt)} · {state.lastDecision}
        </time>}
      </div>
    </section>
  );
}
