import { memo, useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { AutoTraderOutcomesModel, type AutoTraderOutcome } from '../models/AutoTraderOutcomesModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
const pct = (v: unknown) => typeof v === 'number' ? v.toFixed(3) + '%' : '—';
const signed = { 'pnl-positive': (p: { value: unknown }) => typeof p.value === 'number' && p.value > 0, 'pnl-negative': (p: { value: unknown }) => typeof p.value === 'number' && p.value < 0 };
function AutoTraderOutcomesComponent() {
  const model = useMemo(() => new AutoTraderOutcomesModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => { void model.start(); return () => model.stop(); }, [model]);
  const rows = state.rows;
  const summary = useMemo(() => ({ wins: rows.filter((r) => r.pnl > 0).length, losses: rows.filter((r) => r.pnl < 0).length, avg: rows.length ? rows.reduce((s, r) => s + r.pnl / (r.quantity * r.entryPrice) * 100, 0) / rows.length : 0 }), [rows]);
  const columns = useMemo<ColDef<AutoTraderOutcome>[]>(() => [
    { field: 'closedAt', headerName: 'Closed', valueFormatter: ({ value }) => value ? new Date(value).toLocaleString() : '—', minWidth: 150 },
    { field: 'symbol', minWidth: 100 }, { field: 'side', headerName: 'Side', minWidth: 75 },
    { field: 'pnl', headerName: 'Realized P&L', cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(4) : '—', minWidth: 115 },
    { headerName: 'Return', valueGetter: ({ data }) => data ? data.pnl / (data.quantity * data.entryPrice) * 100 : null, cellClassRules: signed, valueFormatter: ({ value }) => pct(value), minWidth: 95 },
    { field: 'direction', headerName: 'Signal direction', minWidth: 110 },
    { field: 'strength', headerName: 'Strength', valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 85 },
    { field: 'scout', headerName: 'Scout', valueFormatter: ({ value }) => value ? 'Yes' : 'No', minWidth: 75 },
    { headerName: 'Trend 1m', valueGetter: ({ data }) => data?.components.trend_1m_percent, valueFormatter: ({ value }) => pct(value), minWidth: 95 },
    { headerName: 'Trend 5m / vol', valueGetter: ({ data }) => data?.components.trend_5m_volatility_units, valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(2) + 'σ' : '—', minWidth: 110 },
    { headerName: 'Momentum 10s', valueGetter: ({ data }) => data?.components.momentum_10s_percent, valueFormatter: ({ value }) => pct(value), minWidth: 105 },
    { headerName: 'Trade flow', valueGetter: ({ data }) => data?.components.trade_flow_imbalance, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 95 },
    { headerName: 'Book pressure', valueGetter: ({ data }) => data?.components.book_imbalance, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 105 },
    { headerName: 'VWAP mispricing', valueGetter: ({ data }) => data?.components.mispricing_vs_vwap_bps, valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(1) + ' bps' : '—', minWidth: 120 },
    { headerName: 'Pressure / no move', valueGetter: ({ data }) => data?.components.pressure_without_price_move, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 130 },
  ], []);
  return <section className="feed-section signal-info-panel" aria-label="AutoTrader closed trade outcomes">
    <FeedToolbar title="AutoTrader winners & losers" />
    <p className="signal-info-note">Completed audited trades matched to the signal snapshot recorded at entry. {summary.wins} winners · {summary.losses} losers · {summary.avg.toFixed(3)}% average return. Signal values show recorded inputs, not proof of causation; sort or filter outcomes to compare patterns.</p>
    {state.error && <div className="positions-error">{state.error}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<AutoTraderOutcome> rowData={rows} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={theme} domLayout="autoHeight" rowHeight={26} headerHeight={28} /></div>
    {!state.loading && rows.length === 0 && !state.error && <p className="positions-empty">No fully closed AutoTrader trades with entry signal snapshots are available yet.</p>}
  </section>;
}
export const AutoTraderOutcomes = memo(AutoTraderOutcomesComponent);
