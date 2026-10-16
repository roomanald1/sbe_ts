import { memo, useEffect, useMemo } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import { SignalInfoModel, type SignalPerformanceRow, type SignalWalkForwardRow } from '../models/SignalInfoModel';
import type { SignalSnapshot } from '../models/SignalsModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31',
  foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad', rowHoverColor: '#141b24',
});
const percent = (value?: number | null) => value == null ? 'N/A' : value.toFixed(3) + '%';
const ratio = (value?: number | null) => value == null ? 'N/A' : (value * 100).toFixed(1) + '%';
const horizonLabel = (value: unknown) => ({ 60: '1 minute', 300: '5 minutes', 900: '15 minutes', 3600: '1 hour' } as Record<number, string>)[Number(value)] ?? String(value) + ' seconds';
const signedValueClasses = {
  'signal-value-positive': (params: { value: unknown }) => typeof params.value === 'number' && params.value > 0,
  'signal-value-negative': (params: { value: unknown }) => typeof params.value === 'number' && params.value < 0,
};
const directionBadge = ({ value }: { value: unknown }) => {
  const direction = value === 'bullish' || value === 'bearish' ? value : 'neutral';
  return <span className={'signal-badge signal-badge-' + direction}>{String(value ?? 'neutral').toUpperCase()}</span>;
};
const strengthRenderer = ({ value }: { value: unknown }) => {
  const strength = typeof value === 'number' ? Math.max(0, Math.min(1, value)) : 0;
  const level = strength >= 0.5 ? 'high' : strength >= 0.25 ? 'medium' : 'low';
  return <span className={'signal-strength signal-strength-' + level}><span className="signal-strength-track"><span style={{ width: (strength * 100) + '%' }} /></span><b>{(strength * 100).toFixed(0)}%</b></span>;
};

function useSignalInfo() {
  const model = useMemo(() => new SignalInfoModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => { void model.start(); return () => model.stop(); }, [model]);
  return state;
}

type BestSignalRow = SignalSnapshot & { time: string };
export function BestSignalsView({ onTradeSymbol }: { onTradeSymbol: (symbol: string) => void }) {
  const state = useSignalInfo();
  const rows = useMemo<BestSignalRow[]>(() => state.best.map((item) => ({ ...item, time: new Date(item.timestamp).toLocaleTimeString() })), [state.best]);
  const columns = useMemo<ColDef<BestSignalRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', cellRenderer: ({ value, data }: { value: unknown; data?: BestSignalRow }) => <button type="button" className="signal-symbol-link" title={'Open order ticket for ' + String(value ?? '')} onClick={(event) => { event.stopPropagation(); if (data) onTradeSymbol(data.symbol); }}>{String(value ?? '')}</button>, minWidth: 110 },
    { field: 'direction', headerName: 'Signal', cellRenderer: directionBadge, minWidth: 100 },
    { field: 'strength', headerName: 'Strength', cellRenderer: strengthRenderer, minWidth: 130 },
    { field: 'score', headerName: 'Score', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => Number(value ?? 0).toFixed(2), minWidth: 80 },
    { field: 'components.trend_1m_percent', headerName: 'Trend 1m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_1m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 100 },
    { field: 'components.trend_5m_percent', headerName: 'Trend 5m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 100 },
    { field: 'components.trend_5m_volatility_units', headerName: '5m Trend / Vol', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_volatility_units, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(2) + 'σ', minWidth: 95 },
    { field: 'components.momentum_10s_percent', headerName: 'Momentum 10s', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.momentum_10s_percent, valueFormatter: ({ value }) => percent(value), minWidth: 110 },
    { field: 'components.trade_flow_imbalance', headerName: 'Trade flow', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trade_flow_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 95 },
    { field: 'components.book_imbalance', headerName: 'Book pressure', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.book_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 105 },
    { field: 'time', headerName: 'Updated', minWidth: 95 },
  ], [onTradeSymbol]);
  return <section className="feed-section signal-info-panel" aria-label="Best signals across markets">
    <FeedToolbar title="Best signals across top spot markets" />
    <p className="signal-info-note">Ranked by backend signal strength across the ten highest-turnover spot pairs. The five-minute trend is context only and does not affect the live score. “5m Trend / Vol” shows the move in recent volatility units. Snapshot refreshes every 15 seconds.</p>
    {state.bestError && <div className="positions-error">{state.bestError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<BestSignalRow> rowData={rows} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {rows.length === 0 && <p className="positions-empty">Waiting for fresh signals from the backend.</p>}
  </section>;
}

type HistoryRow = SignalSnapshot & { time: string };
function SignalHistoryViewComponent({ paused = false }: { paused?: boolean }) {
  const state = useSignalInfo();
  const rows = useMemo<HistoryRow[]>(() => state.history.map((item) => ({ ...item, time: new Date(item.timestamp).toLocaleString() })), [state.history]);
  const displayedRowData = usePausedValue(rows, paused);
  const columns = useMemo<ColDef<HistoryRow>[]>(() => [
    { field: 'time', headerName: 'Time', minWidth: 145 },
    { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
    { field: 'direction', headerName: 'Signal', cellRenderer: directionBadge, minWidth: 100 },
    { field: 'score', headerName: 'Score', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => Number(value ?? 0).toFixed(2), minWidth: 85 },
    { field: 'components.trend_1m_percent', headerName: 'Trend 1m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_1m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 105 },
    { field: 'components.trend_5m_percent', headerName: 'Trend 5m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 105 },
    { field: 'components.trend_5m_volatility_units', headerName: '5m Trend / Vol', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_volatility_units, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(2) + 'σ', minWidth: 95 },
    { field: 'components.momentum_10s_percent', headerName: 'Momentum 10s', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.momentum_10s_percent, valueFormatter: ({ value }) => percent(value), minWidth: 115 },
    { field: 'components.trade_flow_imbalance', headerName: 'Trade flow', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trade_flow_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 100 },
    { field: 'components.book_imbalance', headerName: 'Book pressure', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.book_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 110 },
    { field: 'stale', headerName: 'Data', valueFormatter: ({ value }) => value ? 'Stale' : 'Current', minWidth: 85 },
  ], []);
  return <section className="feed-section signal-info-panel" aria-label="Signal history">
    <FeedToolbar title="Recent signal history" />
    {state.historyError && <div className="positions-error">{state.historyError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<HistoryRow> rowData={displayedRowData} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {rows.length === 0 && <p className="positions-empty">No stored signal snapshots yet.</p>}
  </section>;
}

export const SignalHistoryView = memo(SignalHistoryViewComponent);

function SignalPerformanceViewComponent({ paused = false }: { paused?: boolean }) {
  const state = useSignalInfo();
  const displayedRowData = usePausedValue(state.performance, paused);
  const walkForwardRows = usePausedValue(state.walkForward, paused);
  const columns = useMemo<ColDef<SignalPerformanceRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
    { field: 'horizon_seconds', headerName: 'Horizon', valueFormatter: ({ value }) => horizonLabel(value), minWidth: 100 },
    { field: 'sample_count', headerName: 'Total N', minWidth: 80 },
    { field: 'directional_sample_count', headerName: 'Directional N', minWidth: 105 },
    { field: 'neutral_sample_count', headerName: 'Neutral N', minWidth: 90 },
    { field: 'bullish_match_rate', headerName: 'Bullish hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 125 },
    { field: 'bearish_match_rate', headerName: 'Bearish hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 125 },
    { field: 'direction_match_rate', headerName: 'Directional hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 135 },
    { field: 'net_sample_count', headerName: 'Costed N', minWidth: 85 },
    { field: 'average_net_return_percent', headerName: 'Avg net return', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 120 },
  ], []);
  const walkForwardColumns = useMemo<ColDef<SignalWalkForwardRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
    { field: 'horizon_seconds', headerName: 'Horizon', valueFormatter: ({ value }) => horizonLabel(value), minWidth: 95 },
    { field: 'training_samples', headerName: 'Training N', minWidth: 100 },
    { field: 'evaluated_signals', headerName: 'Test N', minWidth: 80 },
    { field: 'selected_signals', headerName: 'Signals taken', minWidth: 105 },
    { field: 'selected_strength_threshold', headerName: 'Learned min strength', valueFormatter: ({ value }) => value == null ? 'N/A' : (value * 100).toFixed(0) + '%', minWidth: 145 },
    { field: 'direction_match_rate', headerName: 'Hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 100 },
    { field: 'average_net_return_percent', headerName: 'Avg net return', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 120 },
    { field: 'positive_net_rate', headerName: 'Positive net', valueFormatter: ({ value }) => ratio(value), minWidth: 105 },
    { field: 'baseline_direction_match_rate', headerName: 'Current hit rate', valueFormatter: ({ value }) => ratio(value), minWidth: 115 },
    { field: 'baseline_average_net_return_percent', headerName: 'Current avg net', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 115 },
    { field: 'five_minute_trend_sample_count', headerName: '5m trend N', minWidth: 90 },
    { field: 'five_minute_trend_match_rate', headerName: '5m trend hit', valueFormatter: ({ value }) => ratio(value), minWidth: 100 },
    { field: 'five_minute_trend_average_net_return_percent', headerName: '5m trend avg net', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 125 },
  ], []);
  return <section className="feed-section signal-info-panel" aria-label="Signal performance">
    <FeedToolbar title="Signal outcome tracking" />
    <p className="signal-info-note">Direction hit rates exclude neutral signals. Older stored outcomes count toward Total N but lack direction and cost data. Net returns use fresh top-of-book entry/exit quotes and the configured taker fee; order size and market impact are not simulated.</p>
    {state.performanceError && <div className="positions-error">{state.performanceError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<SignalPerformanceRow> rowData={displayedRowData} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {state.performance.length === 0 && <p className="positions-empty">Cost-aware outcome samples begin after one minute; 5-minute, 15-minute and 1-hour rows appear as those horizons mature.</p>}
    <h4 className="signal-info-subheading">Walk-forward threshold evaluation</h4>
    <p className="signal-info-note">The learned threshold uses earlier, non-overlapping costed outcomes only. Longer horizons take longer to accumulate independent samples. The current signal and five-minute trend are compared on the same later, costed directional observations; the five-minute trend is context-only and uses a minimum 0.25 volatility-unit move. Evaluation starts after 30 training observations.</p>
    {state.walkForwardError && <div className="positions-error">{state.walkForwardError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<SignalWalkForwardRow> rowData={walkForwardRows} columnDefs={walkForwardColumns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {walkForwardRows.length === 0 && <p className="positions-empty">Collecting enough independent, costed outcomes for walk-forward evaluation.</p>}
  </section>;
}

export const SignalPerformanceView = memo(SignalPerformanceViewComponent);
