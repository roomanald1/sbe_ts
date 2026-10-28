import { memo, useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { AutoTraderOutcomesModel, type AutoTraderOutcome } from '../models/AutoTraderOutcomesModel';
import type { SymbolPriceTick } from '../types/symbol_price';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
const pct = (v: unknown) => typeof v === 'number' && Number.isFinite(v) ? v.toFixed(3) + '%' : '—';
const signed = { 'pnl-positive': (p: { value: unknown }) => typeof p.value === 'number' && p.value > 0, 'pnl-negative': (p: { value: unknown }) => typeof p.value === 'number' && p.value < 0 };
type OutcomeGridRow = AutoTraderOutcome & { markPrice?: number };
type StrategyWinRateRow = { strategy: string; trades: number; wins: number; losses: number; flat: number; winRate: number; averageReturn: number };
const durationLabel = (value: unknown) => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '—';
  const totalMinutes = Math.floor(value / 60_000);
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return days + 'd ' + hours + 'h';
  if (hours > 0) return hours + 'h ' + minutes + 'm';
  if (totalMinutes > 0) return totalMinutes + 'm';
  return Math.floor(value / 1_000) + 's';
};
const openPnl = (row?: OutcomeGridRow) => {
  if (!row || row.status !== 'Open' || row.markPrice === undefined) return undefined;
  const exitNotional = Math.abs(row.signedQuantity * row.markPrice);
  const grossPnl = (row.markPrice - row.entryPrice) * row.signedQuantity;
  const estimatedExitFee = exitNotional * row.estimatedExitFeeRate;
  const estimatedExitSlippage = exitNotional * row.estimatedExitSlippageBps / 10_000;
  return grossPnl + row.entryFees - estimatedExitFee - estimatedExitSlippage;
};
function AutoTraderOutcomesComponent({ embedded = false, ticks = [], liveOnly = false, closedOnly = false }: { embedded?: boolean; ticks?: SymbolPriceTick[]; liveOnly?: boolean; closedOnly?: boolean }) {
  const model = useMemo(() => new AutoTraderOutcomesModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => { void model.start(); return () => model.stop(); }, [model]);
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 15_000); return () => clearInterval(timer); }, []);
  const marks = useMemo(() => new Map(ticks.filter((tick) => Number.isFinite(tick.price) && now - tick.receivedAt < 60_000).map((tick) => [tick.symbol, tick.price])), [ticks, now]);
  const rows = useMemo<OutcomeGridRow[]>(() => state.rows.map((row) => ({ ...row, markPrice: row.status === 'Open' ? marks.get(row.symbol) : undefined })).filter((row) => liveOnly ? row.status === 'Open' : closedOnly ? row.status === 'Closed' : true).sort((a, b) => closedOnly ? (b.closedAt ?? 0) - (a.closedAt ?? 0) : 0), [state.rows, marks, liveOnly, closedOnly]);
  const closedRows = rows.filter((row) => row.status === 'Closed');
  const openRows = rows.filter((row) => row.status === 'Open');
  const strategyWinRates = useMemo<StrategyWinRateRow[]>(() => {
    const grouped = new Map<string, AutoTraderOutcome[]>();
    for (const row of closedRows) grouped.set(row.strategy, [...(grouped.get(row.strategy) ?? []), row]);
    return [...grouped.entries()].map(([strategy, trades]) => {
      const wins = trades.filter((row) => row.pnl > 0).length;
      const losses = trades.filter((row) => row.pnl < 0).length;
      return { strategy, trades: trades.length, wins, losses, flat: trades.length - wins - losses,
        winRate: wins / trades.length * 100,
        averageReturn: trades.reduce((sum, row) => sum + row.pnl / (row.quantity * row.entryPrice) * 100, 0) / trades.length };
    }).sort((a, b) => b.trades - a.trades || a.strategy.localeCompare(b.strategy));
  }, [closedRows]);
  const strategyColumns = useMemo<ColDef<StrategyWinRateRow>[]>(() => [
    { field: 'strategy', headerName: 'Strategy', minWidth: 190 },
    { field: 'trades', headerName: 'Closed trades', minWidth: 100 },
    { field: 'wins', headerName: 'Wins', minWidth: 75 },
    { field: 'losses', headerName: 'Losses', minWidth: 75 },
    { field: 'flat', headerName: 'Flat', minWidth: 70 },
    { field: 'winRate', headerName: 'Win rate', valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(1) + '%' : '—', minWidth: 95 },
    { field: 'averageReturn', headerName: 'Avg return', cellClassRules: signed, valueFormatter: ({ value }) => pct(value), minWidth: 105 },
  ], []);
  const summary = useMemo(() => ({
    wins: closedRows.filter((row) => row.pnl > 0).length,
    losses: closedRows.filter((row) => row.pnl < 0).length,
    avg: closedRows.length ? closedRows.reduce((sum, row) => sum + row.pnl / (row.quantity * row.entryPrice) * 100, 0) / closedRows.length : 0,
    openPnlByQuote: openRows.reduce<Record<string, number>>((totals, row) => {
      const pnl = openPnl(row);
      if (pnl !== undefined) totals[row.pnlQuoteCurrency] = (totals[row.pnlQuoteCurrency] ?? 0) + pnl;
      return totals;
    }, {}),
    markedOpenCount: openRows.filter((row) => openPnl(row) !== undefined).length,
  }), [closedRows, openRows]);
  const columns = useMemo<ColDef<OutcomeGridRow>[]>(() => [
    { headerName: 'Net vs breakeven', cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
      if (!data || data.status !== 'Open') return data?.status ?? '—';
      const net = openPnl(data);
      if (net === undefined) return <span className="auto-trader-pnl-bar-unavailable">Mark unavailable</span>;
      const notional = data.quantity * data.entryPrice;
      const returnPct = notional > 0 ? net / notional * 100 : 0;
      const positive = net > 0;
      const label = (net >= 0 ? '+' : '') + data.pnlQuoteCurrency + ' ' + net.toFixed(4) + ' · ' + (returnPct >= 0 ? '+' : '') + returnPct.toFixed(2) + '%';
      const hasStop = typeof data.stopLossPercent === 'number' && Number.isFinite(data.stopLossPercent) && data.stopLossPercent > 0;
      const hasTarget = typeof data.takeProfitPercent === 'number' && Number.isFinite(data.takeProfitPercent) && data.takeProfitPercent > 0;
      const minReturn = hasStop ? -data.stopLossPercent! : -5;
      const maxReturn = hasTarget ? data.takeProfitPercent! : 5;
      const positionForReturn = (value: number) => Math.max(10, Math.min(90, 10 + (value - minReturn) / (maxReturn - minReturn) * 80));
      const breakEvenPosition = positionForReturn(0);
      const currentPosition = positionForReturn(returnPct);
      const fillLeft = Math.min(breakEvenPosition, currentPosition);
      const fillWidth = Math.abs(currentPosition - breakEvenPosition);
      const entryValue = data.entryFees - notional * data.estimatedExitFeeRate - notional * data.estimatedExitSlippageBps / 10_000;
      const entryPosition = positionForReturn(notional > 0 ? entryValue / notional * 100 : 0);
      const stopPosition = hasStop ? positionForReturn(-data.stopLossPercent!) : undefined;
      const targetPosition = hasTarget ? positionForReturn(data.takeProfitPercent!) : undefined;
      return <div className="auto-trader-pnl-bar-cell" title={label + ' net of entry fees, estimated exit fees, and estimated adverse exit slippage'}>
        <div className="auto-trader-pnl-bar-track" aria-label={'Net return ' + returnPct.toFixed(2) + '%; entry, breakeven, stop loss, and take profit shown on a shared scale'}>
          <span className={'auto-trader-pnl-bar-fill ' + (positive ? 'is-positive' : net < 0 ? 'is-negative' : 'is-flat')} style={{ left: fillLeft + '%', width: fillWidth + '%' }} />
          <span className="auto-trader-pnl-marker is-entry" style={{ left: entryPosition + '%' }} title="Entry" />
          <span className="auto-trader-pnl-marker is-breakeven" style={{ left: breakEvenPosition + '%' }} title="Net breakeven" />
          {stopPosition !== undefined && <span className="auto-trader-pnl-marker is-stop" style={{ left: stopPosition + '%' }} title={'Stop loss −' + data.stopLossPercent?.toFixed(2) + '%'} />}
          {targetPosition !== undefined && <span className="auto-trader-pnl-marker is-target" style={{ left: targetPosition + '%' }} title={'Take profit +' + data.takeProfitPercent?.toFixed(2) + '%'} />}
        </div>
        <span className={'auto-trader-pnl-bar-label ' + (positive ? 'is-positive' : net < 0 ? 'is-negative' : 'is-flat')}>{label}</span>
        <div className="auto-trader-pnl-exit-percentages" title="Net exit thresholds as a percentage of entry notional">
          {stopPosition !== undefined && <span className="is-stop" style={{ left: stopPosition + '%' }}>SL −{data.stopLossPercent!.toFixed(2)}%</span>}
          {targetPosition !== undefined && <span className="is-target" style={{ left: targetPosition + '%' }}>TP +{data.takeProfitPercent!.toFixed(2)}%</span>}
        </div>
      </div>;
    }, minWidth: 320, flex: 2 },
    { field: 'status', headerName: 'State', minWidth: 75 },
    { field: 'openedAt', headerName: 'Opened', valueFormatter: ({ value }) => value ? new Date(value).toLocaleString() : '—', minWidth: 150 },
    { field: 'closedAt', headerName: 'Closed', valueFormatter: ({ value }) => value ? new Date(value).toLocaleString() : '—', minWidth: 150 },
    { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
    { headerName: 'Open for', valueGetter: ({ data }) => data?.status === 'Open' ? Math.max(0, now - data.openedAt) : undefined, valueFormatter: ({ value }) => durationLabel(value), minWidth: 95, maxWidth: 115, flex: 0 },
    { headerName: 'Manual close', colId: 'manual-close', sortable: false, filter: false, minWidth: 180, maxWidth: 190, flex: 0, cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
      if (!liveOnly || !data || data.status !== 'Open') return null;
      const confirming = state.confirmingCloseSymbols.has(data.symbol);
      const closing = state.closingSymbols.has(data.symbol);
      if (closing) return <span className="auto-trader-close-progress">Closing…</span>;
      if (confirming) return <div className="auto-trader-position-close-actions">
        <span>Close at market?</span>
        <button className="is-confirm" type="button" disabled={closing} onClick={() => void model.confirmClose(data.symbol)}>Confirm</button>
        <button className="is-cancel" type="button" disabled={closing} onClick={() => model.cancelClose(data.symbol)}>Cancel</button>
      </div>;
      return <button className="auto-trader-position-close-button" type="button" disabled={closing} title="Submit an IOC order to close this AutoTrader position at available market liquidity" onClick={() => model.requestClose(data.symbol)}>Close</button>;
    } },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), minWidth: 170 },
    { field: 'side', headerName: 'Side', valueFormatter: ({ value }) => typeof value === 'string' ? value.toUpperCase() : '—', minWidth: 75 },
    { field: 'quantity', headerName: 'Quantity', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(6) : '—', minWidth: 100 },
    { field: 'entryPrice', headerName: 'Entry', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(7) : '—', minWidth: 100 },
    { field: 'markPrice', headerName: 'Mark', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(7) : '—', minWidth: 100 },
    { headerName: 'Entry fees', valueGetter: ({ data }) => data?.status === 'Open' ? Math.abs(data.entryFees) : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 110 },
    { headerName: 'Est. exit fee', valueGetter: ({ data }) => data?.status === 'Open' && data.markPrice !== undefined ? Math.abs(data.signedQuantity * data.markPrice) * data.estimatedExitFeeRate : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 120 },
    { headerName: 'Est. exit slippage', valueGetter: ({ data }) => data?.status === 'Open' && data.markPrice !== undefined ? Math.abs(data.signedQuantity * data.markPrice) * data.estimatedExitSlippageBps / 10_000 : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 135 },
    { headerName: 'Realized / open P&L', valueGetter: ({ data }) => data?.status === 'Open' ? openPnl(data) : data?.pnl, cellClassRules: signed, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 150 },
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
  ], [now, liveOnly, model, state.confirmingCloseSymbols, state.closingSymbols]);
  const visibleColumns = liveOnly
    ? ['Symbol', 'Strategy', 'Open for', 'Net vs breakeven', 'Manual close'].flatMap((headerName) => columns.filter((column) => column.headerName === headerName).map((column) => headerName === 'Symbol' ? { ...column, width: 70, minWidth: 45, maxWidth: 180, flex: 0 } : headerName === 'Strategy' ? { ...column, width: 190, minWidth: 140, flex: 0 } : headerName === 'Open for' || headerName === 'Manual close' ? { ...column, flex: 0 } : { ...column, minWidth: 320, flex: 1 }))
    : columns;
  return <section className={embedded ? 'auto-trader-outcomes-embedded' : 'feed-section signal-info-panel'} aria-label={liveOnly ? 'Live AutoTrader positions' : 'AutoTrader position outcomes'}>
    {!embedded && <FeedToolbar title={liveOnly ? 'AutoTrader live positions' : closedOnly ? 'AutoTrader completed positions' : 'AutoTrader positions & outcomes'} />}
    {!liveOnly && <p className="signal-info-note">{openRows.length} open positions · {summary.markedOpenCount}/{openRows.length} marked · indicative open P&amp;L {summary.markedOpenCount ? Object.entries(summary.openPnlByQuote).map(([quote, value]) => quote + ' ' + (value >= 0 ? '+' : '') + value.toFixed(4)).join(' · ') : 'unavailable'} · {summary.wins} closed winners · {summary.losses} closed losers · {summary.avg.toFixed(3)}% average closed return. Open marks use the latest price tick and net out signed entry fees, estimated exit fees, and estimated adverse exit slippage based on AutoTrader fills for that symbol; closed P&amp;L uses audited realized values. Signal values show the recorded entry decision, not proof of causation.</p>}
    {state.error && <div className="positions-error">{state.error}</div>}
    <div className="auto-trader-pnl-marker-legend" aria-label="Position bar marker key">
      <span className="is-stop">SL</span><span><i className="is-entry" />Entry</span>
      <span><i className="is-breakeven" />BE</span><span className="is-target">TP</span>
    </div>
    {!liveOnly && <><strong className="auto-trader-strategy-winrate-title">Closed-trade win rate by strategy</strong>
    <div className="data-grid-wrap signal-info-grid auto-trader-strategy-winrate-grid"><AgGridReact<StrategyWinRateRow> rowData={strategyWinRates} columnDefs={strategyColumns} getRowId={({ data }) => data.strategy} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 70 }} theme={theme} domLayout="autoHeight" rowHeight={26} headerHeight={28} /></div>
    {strategyWinRates.length === 0 && !state.loading && <p className="positions-empty">No closed trades to calculate strategy win rates yet.</p>}</>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<OutcomeGridRow> rowData={rows} columnDefs={visibleColumns} onFirstDataRendered={liveOnly ? (event) => event.api.autoSizeColumns(['symbol']) : undefined} getRowId={({ data }) => data.rowId} getRowClass={({ data }) => data?.status === 'Open' ? 'auto-trader-open-position-row' : 'auto-trader-closed-position-row'} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={theme} domLayout="autoHeight" rowHeight={48} headerHeight={28} /></div>
    {!state.loading && rows.length === 0 && !state.error && <p className="positions-empty">{liveOnly ? 'No live AutoTrader positions are open.' : closedOnly ? 'No completed AutoTrader positions are available yet.' : 'No open AutoTrader positions or fully closed audited trades are available yet.'}</p>}
  </section>;
}
export const AutoTraderOutcomes = memo(AutoTraderOutcomesComponent);
