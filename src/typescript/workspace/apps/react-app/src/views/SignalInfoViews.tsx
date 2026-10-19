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
  const direction = value === 'bullish' || value === 'bearish' || value === 'ranging' ? value : 'neutral';
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
export function BestSignalsView({ onTradeSymbol, onRangeOrders }: { onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void; onRangeOrders: (request: { symbol: string; lower: number; upper: number }) => void }) {
  const state = useSignalInfo();
  const rows = useMemo<BestSignalRow[]>(() => state.best.map((item) => ({ ...item, time: new Date(item.timestamp).toLocaleTimeString() })), [state.best]);
  const columns = useMemo<ColDef<BestSignalRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', cellRenderer: ({ value, data }: { value: unknown; data?: BestSignalRow }) => <button type="button" className="signal-symbol-link" title={'Open ' + (data?.direction === 'bearish' ? 'sell' : 'buy') + ' order for ' + String(value ?? '')} onClick={(event) => { event.stopPropagation(); if (data) onTradeSymbol(data.symbol, data.direction === 'bearish' ? 'sell' : 'buy'); }}>{String(value ?? '')}</button>, minWidth: 110 },
    { field: 'direction', headerName: 'Signal', minWidth: 145, tooltipValueGetter: ({ data }) => data?.reasons.find((reason) => reason.startsWith('Breakout watch:')) ?? undefined, cellRenderer: ({ value, data }: { value: unknown; data?: BestSignalRow }) => {
      const breakout = data?.reasons.find((reason) => reason.startsWith('Breakout watch:'));
      const breakoutUp = breakout?.startsWith('Breakout watch: UP');
      const rangeStatus = value === 'ranging' && data?.range_lower_bound != null && data.range_upper_bound != null
        ? <button type="button" className="signal-badge signal-badge-ranging signal-range-trigger" title="Set buy and sell limit orders at the range boundaries" onClick={(event) => { event.stopPropagation(); onRangeOrders({ symbol: data.symbol, lower: data.range_lower_bound!, upper: data.range_upper_bound! }); }}>RANGING</button>
        : directionBadge({ value });
      return <span className="signal-status-group">{rangeStatus}{breakout && <span className={'signal-badge ' + (breakoutUp ? 'signal-badge-bullish' : 'signal-badge-bearish')}>BREAKOUT {breakoutUp ? '↑' : '↓'}</span>}</span>;
    } },
    { field: 'ranging_probability', headerName: 'Range setup', valueFormatter: ({ value }) => ratio(value), minWidth: 115, tooltipValueGetter: () => 'Heuristic score from the prior five minutes of containment and boundary visits. It is not an accuracy estimate.' },
    { headerName: 'Range boundaries', valueGetter: ({ data }) => data?.range_lower_bound != null && data?.range_upper_bound != null ? String(data.range_lower_bound) + ' – ' + String(data.range_upper_bound) : 'N/A', minWidth: 180, tooltipValueGetter: () => 'Observed low and high prices over the five-minute analysis window.' },
    { field: 'strength', headerName: 'Est. accuracy', cellRenderer: strengthRenderer, minWidth: 130, tooltipValueGetter: () => 'Smoothed historical hit rate for similar signals. Ranging calls are checked against the next five minutes staying inside the forecast bounds; estimates use a neutral prior when samples are sparse.' },
    { field: 'score', headerName: 'Score', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => Number(value ?? 0).toFixed(2), minWidth: 80 },
    { field: 'components.trend_1m_percent', headerName: 'Trend 1m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_1m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 100 },
    { field: 'components.trend_5m_percent', headerName: 'Trend 5m', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_percent, valueFormatter: ({ value }) => percent(value), minWidth: 100 },
    { field: 'components.trend_5m_volatility_units', headerName: '5m Trend / Vol', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trend_5m_volatility_units, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(2) + 'σ', minWidth: 95 },
    { field: 'components.momentum_10s_percent', headerName: 'Momentum 10s', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.momentum_10s_percent, valueFormatter: ({ value }) => percent(value), minWidth: 110 },
    { field: 'components.trade_flow_imbalance', headerName: 'Trade flow', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.trade_flow_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 95 },
    { field: 'components.book_imbalance', headerName: 'Book pressure', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.book_imbalance, valueFormatter: ({ value }) => ratio(value), minWidth: 105 },
    { field: 'components.mispricing_vs_vwap_bps', headerName: 'VWAP mispricing', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.mispricing_vs_vwap_bps, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(1) + ' bps', minWidth: 125, tooltipValueGetter: () => 'Current price vs recent trade VWAP. Positive means above VWAP.' },
    { field: 'components.pressure_without_price_move', headerName: 'Pressure / no move', cellClassRules: signedValueClasses, valueGetter: ({ data }) => data?.components.pressure_without_price_move, valueFormatter: ({ value }) => value == null ? 'N/A' : (value * 100).toFixed(0) + '%', minWidth: 135, tooltipValueGetter: () => 'Trade pressure imbalance detected while price stayed nearly flat. Signed by buy/sell pressure.' },
    { field: 'components.aggressor_cost_bps', headerName: 'Taker cost', valueGetter: ({ data }) => data?.components.aggressor_cost_bps, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(1) + ' bps', minWidth: 105, tooltipValueGetter: () => 'Average aggressive trade execution cost vs midpoint; higher values mean takers paid more.' },
    { field: 'time', headerName: 'Updated', minWidth: 95 },
  ], [onTradeSymbol, onRangeOrders]);
  return <section className="feed-section signal-info-panel" aria-label="Best signals across markets">
    <FeedToolbar title="Best signals across top spot markets" />
    <p className="signal-info-note">Ranked by backend signal strength across the ten highest-turnover spot pairs. The five-minute trend is context only and does not affect the live score. “5m Trend / Vol” shows the move in recent volatility units. Snapshot refreshes every 15 seconds.</p>
    {state.bestError && <div className="positions-error">{state.bestError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<BestSignalRow> rowData={rows} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {rows.length === 0 && <p className="positions-empty">Waiting for fresh signals from the backend.</p>}
  </section>;
}

type HistoryRow = SignalSnapshot & { time: string };
function SignalHistoryViewComponent({ paused = false, onTradeSymbol }: { paused?: boolean; onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void }) {
  const state = useSignalInfo();
  const rows = useMemo<HistoryRow[]>(() => state.history.map((item) => ({ ...item, time: new Date(item.timestamp).toLocaleString() })), [state.history]);
  const displayedRowData = usePausedValue(rows, paused);
  const columns = useMemo<ColDef<HistoryRow>[]>(() => [
    { field: 'time', headerName: 'Time', minWidth: 145 },
    { field: 'symbol', headerName: 'Symbol', minWidth: 100, cellRenderer: ({ value, data }: { value: unknown; data?: HistoryRow }) => <button type="button" className="signal-symbol-link" onClick={() => data && onTradeSymbol(data.symbol, data.direction === 'bearish' ? 'sell' : 'buy')}>{String(value ?? '')}</button> },
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

function SignalPerformanceViewComponent({ paused = false, onTradeSymbol }: { paused?: boolean; onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void }) {
  const state = useSignalInfo();
  const displayedRowData = usePausedValue(state.performance, paused);
  const columns = useMemo<ColDef<SignalPerformanceRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 100, cellRenderer: ({ value }: { value: unknown }) => <button type="button" className="signal-symbol-link" onClick={() => onTradeSymbol(String(value ?? ''), 'buy')}>{String(value ?? '')}</button> },
    { field: 'horizon_seconds', headerName: 'Horizon', valueFormatter: ({ value }) => horizonLabel(value), minWidth: 100 },
    { field: 'sample_count', headerName: 'Total N', minWidth: 80 },
    { field: 'directional_sample_count', headerName: 'Directional N', minWidth: 105 },
    { field: 'neutral_sample_count', headerName: 'Neutral N', minWidth: 90 },
    { field: 'bullish_match_rate', headerName: 'Bullish hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 125 },
    { field: 'bearish_match_rate', headerName: 'Bearish hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 125 },
    { field: 'direction_match_rate', headerName: 'Directional hit rate', cellClassRules: { 'signal-performance-good': ({ value }) => typeof value === 'number' && value >= 0.5, 'signal-performance-poor': ({ value }) => typeof value === 'number' && value < 0.5 }, valueFormatter: ({ value }) => ratio(value), minWidth: 135 },
    { field: 'net_sample_count', headerName: 'Costed N', minWidth: 85 },
    { field: 'average_net_return_percent', headerName: 'Avg net return', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 120 },
  ], [onTradeSymbol]);
  return <section className="feed-section signal-info-panel" aria-label="Signal performance">
    <FeedToolbar title="Signal outcome tracking" />
    <p className="signal-info-note">Direction hit rates exclude neutral signals. Older stored outcomes count toward Total N but lack direction and cost data. Net returns use fresh top-of-book entry/exit quotes and the configured taker fee; order size and market impact are not simulated.</p>
    {state.performanceError && <div className="positions-error">{state.performanceError}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<SignalPerformanceRow> rowData={displayedRowData} columnDefs={columns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {state.performance.length === 0 && <p className="positions-empty">Cost-aware outcome samples begin after one minute; 5-minute, 15-minute and 1-hour rows appear as those horizons mature.</p>}

  </section>;
}


function WalkForwardViewComponent({ paused = false, onTradeSymbol }: { paused?: boolean; onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void }) {
  const state = useSignalInfo();
  const displayedRows = usePausedValue(state.walkForward, paused);
  const walkForwardColumns = useMemo<ColDef<SignalWalkForwardRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 100, cellRenderer: ({ value }: { value: unknown }) => <button type="button" className="signal-symbol-link" onClick={() => onTradeSymbol(String(value ?? ''), 'buy')}>{String(value ?? '')}</button> },
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
    { field: 'logistic_selected_signals', headerName: 'Logistic N', minWidth: 90 },
    { field: 'logistic_direction_match_rate', headerName: 'Logistic hit', valueFormatter: ({ value }) => ratio(value), minWidth: 100 },
    { field: 'logistic_average_net_return_percent', headerName: 'Logistic avg net', cellClassRules: signedValueClasses, valueFormatter: ({ value }) => percent(value), minWidth: 120 },
    { field: 'logistic_positive_net_rate', headerName: 'Logistic positive net', valueFormatter: ({ value }) => ratio(value), minWidth: 135 },
    ...(['signal_long', 'signal_short', 'logistic_long', 'logistic_short'] as const).flatMap((side) => {
      const label = side === 'signal_long' ? 'Signal long' : side === 'signal_short' ? 'Signal short' : side === 'logistic_long' ? 'Logistic long' : 'Logistic short';
      return [
        { headerName: label + ' N', valueGetter: ({ data }: { data?: SignalWalkForwardRow }) => data?.[side].sample_count ?? 0, minWidth: 100 },
        { headerName: label + ' hit', valueGetter: ({ data }: { data?: SignalWalkForwardRow }) => data?.[side].direction_match_rate, valueFormatter: ({ value }: { value: number | null | undefined }) => ratio(value), minWidth: 100 },
        { headerName: label + ' avg net', valueGetter: ({ data }: { data?: SignalWalkForwardRow }) => data?.[side].average_net_return_percent, cellClassRules: signedValueClasses, valueFormatter: ({ value }: { value: number | null | undefined }) => percent(value), minWidth: 120 },
        { headerName: label + ' positive net', valueGetter: ({ data }: { data?: SignalWalkForwardRow }) => data?.[side].positive_net_rate, valueFormatter: ({ value }: { value: number | null | undefined }) => ratio(value), minWidth: 125 },
        { headerName: label + ' 95% lower', valueGetter: ({ data }: { data?: SignalWalkForwardRow }) => data?.[side].positive_net_rate_lower_95, valueFormatter: ({ value }: { value: number | null | undefined }) => ratio(value), minWidth: 125 },
      ];
    }),
  ], [onTradeSymbol]);
  return <section className="feed-section signal-info-panel walk-forward-panel" aria-label="Walk-forward model evaluation">
    <FeedToolbar title="Walk-forward model evaluation" />
    <p className="signal-info-note">Each test prediction is evaluated against later, non-overlapping outcomes. Long and short results are split for the strength filter and logistic model. The 95% lower bound estimates uncertainty in the positive-net rate; side samples under 30 are preliminary. Logistic regression learns from 1m trend, 10s momentum, trade flow and book imbalance. Returns use recorded execution costs; order size and market impact are not included.</p>
    {state.walkForwardError && <div className="positions-error">{state.walkForwardError}</div>}
    <div className="data-grid-wrap signal-info-grid walk-forward-grid"><AgGridReact<SignalWalkForwardRow> rowData={displayedRows} columnDefs={walkForwardColumns} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={38} /></div>
    {displayedRows.length === 0 && <p className="positions-empty">Walk-forward rows appear after at least 30 eligible, non-overlapping costed outcomes are available for a symbol and horizon. This can take 30 minutes for the 1-minute horizon and much longer for longer horizons.</p>}
  </section>;
}

export const WalkForwardView = memo(WalkForwardViewComponent);

export const SignalPerformanceView = memo(SignalPerformanceViewComponent);
