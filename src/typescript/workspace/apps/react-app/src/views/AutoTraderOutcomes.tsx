import { memo, useEffect, useMemo, useRef, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef, type ColGroupDef, type GridApi } from 'ag-grid-community';
import { AutoTraderOutcomesModel, type AutoTraderOutcome } from '../models/AutoTraderOutcomesModel';
import type { Strategy } from '../models/AutoTraderModel';
import type { SymbolPriceTick } from '../types/symbol_price';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { money, strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
const pct = (v: unknown) => typeof v === 'number' && Number.isFinite(v) ? v.toFixed(3) + '%' : '—';
const usd = new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD', minimumFractionDigits: 3, maximumFractionDigits: 3 });
const signed = { 'pnl-positive': (p: { value: unknown }) => typeof p.value === 'number' && p.value > 0, 'pnl-negative': (p: { value: unknown }) => typeof p.value === 'number' && p.value < 0 };
type OutcomeGridRow = AutoTraderOutcome & { markPrice?: number };
type StrategyWinRateRow = { strategy: string; enabled?: boolean; runStatus: string; runReason: string; trades: number; wins: number; losses: number; flat: number; feesUsd?: number; netPnlUsd?: number; todayRealizedUsd?: number; winRate?: number; averageReturn?: number; netExpectancyUsd?: number; averageNetWinUsd?: number; averageNetWinPercent?: number; averageNetLossUsd?: number; averageNetLossPercent?: number; profitFactor?: number; medianHoldingMs?: number; averageHoldingMs?: number; periodStart: number; periodEnd: number; averageEntrySizeUsd?: number; lowSample: boolean; learningMultiplier?: number; learningSamples?: number; learningPositiveRate?: number };
const allStrategies: Strategy[] = ['live-momentum', 'live-trade-flow', 'live-order-book', 'live-vwap-dislocation', 'live-absorption-pressure', 'donchian-55-btc200', 'donchian-20-sma200', 'donchian-20-btc200', 'trend-sleeves-btc200', 'trend-50-200', 'relative-strength-90-sma200', 'risk-adjusted-monthly-momentum', 'rsi2-oversold-rebound', 'dated-futures-carry'];
const strategyCellTints = { status: 'rgba(148, 157, 170, 0.06)', learning: 'rgba(74, 127, 151, 0.075)', sample: 'rgba(161, 139, 75, 0.075)', performance: 'rgba(77, 132, 91, 0.075)', profile: 'rgba(125, 100, 157, 0.075)' } as const;
const colorStrategyColumns = (columns: ColDef<StrategyWinRateRow>[], group: keyof typeof strategyCellTints) => columns.map((column, index) => ({ ...column, headerClass: ['strategy-column-' + group + '-header', index === 0 && 'strategy-group-start', index === columns.length - 1 && 'strategy-group-end'].filter(Boolean).join(' '), cellClass: ['strategy-column-' + group + '-cell', index === 0 && 'strategy-group-start', index === columns.length - 1 && 'strategy-group-end'].filter(Boolean).join(' '), cellStyle: { backgroundColor: strategyCellTints[group] } }));
const dateLabel = (value: number) => new Intl.DateTimeFormat(undefined, { year: '2-digit', month: 'short', day: 'numeric' }).format(value);
const median = (values: number[]) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
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
function AutoTraderOutcomesComponent({ embedded = false, ticks = [], liveOnly = false, closedOnly = false, strategyRatesOnly = false }: { embedded?: boolean; ticks?: SymbolPriceTick[]; liveOnly?: boolean; closedOnly?: boolean; strategyRatesOnly?: boolean }) {
  const model = useMemo(() => new AutoTraderOutcomesModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  const outcomesGridApi = useRef<GridApi<OutcomeGridRow> | null>(null);
  useEffect(() => { void model.start(); return () => model.stop(); }, [model]);
  useEffect(() => { outcomesGridApi.current?.resetRowHeights(); }, [state.confirmingCloseSymbols, state.closingSymbols]);
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 15_000); return () => clearInterval(timer); }, []);
  const marks = useMemo(() => new Map(ticks.filter((tick) => Number.isFinite(tick.price) && now - tick.receivedAt < 60_000).map((tick) => [tick.symbol, tick.price])), [ticks, now]);
  const rows = useMemo<OutcomeGridRow[]>(() => state.rows.map((row) => ({ ...row, markPrice: row.status === 'Open' ? marks.get(row.symbol) : undefined })).filter((row) => liveOnly ? row.status === 'Open' : closedOnly ? row.status === 'Closed' || row.status === 'Partially closed' : true).sort((a, b) => closedOnly ? (b.closedAt ?? b.openedAt) - (a.closedAt ?? a.openedAt) : 0), [state.rows, marks, liveOnly, closedOnly]);
  const closedRows = rows.filter((row) => row.status === 'Closed');
  const openRows = rows.filter((row) => row.status === 'Open');
  const strategyWinRates = useMemo<StrategyWinRateRow[]>(() => {
    const grouped = new Map<string, AutoTraderOutcome[]>();
    for (const row of closedRows) {
      if (row.strategy === 'live-insights') continue;
      grouped.set(row.strategy, [...(grouped.get(row.strategy) ?? []), row]);
    }
    const strategyNames = [...new Set([...allStrategies, ...grouped.keys(), ...Object.keys(state.todayStrategyRealizedUsd ?? {})])];
    return strategyNames.map((strategy) => {
      const trades = grouped.get(strategy) ?? [];
      const enabled = state.autoTraderStatusAvailable ? state.enabledStrategies.includes(strategy) : undefined;
      const diagnostic = state.strategyDiagnostics.find((item) => item.strategy === strategy);
      const openPosition = state.openStrategyPositions.find((position) => position.strategy === strategy) ?? openRows.find((position) => position.strategy === strategy);
      const orders = state.strategyOrders.filter((order) => order.strategy === strategy).sort((a, b) => b.timestamp - a.timestamp);
      const workingOrder = orders.find((order) => order.purpose !== 'stop-loss' && ['submitting', 'accepted', 'resting', 'partially_filled', 'armed'].includes(order.status ?? ''));
      const protectiveStop = openPosition
        ? orders.find((order) => order.purpose === 'stop-loss' && ['armed', 'triggering'].includes(order.status ?? ''))
        : undefined;
      const latestOrder = orders[0];
      let runStatus = 'No diagnostic';
      let runReason = diagnostic ? 'No current signal reported' : 'No strategy-specific run or block detail is available';
      if (strategy === 'unattributed') {
        runStatus = 'Ledger adjustment';
        runReason = 'Realized P&L in the position ledger could not be linked to a strategy order';
      } else if (!state.autoTraderStatusAvailable) {
        runStatus = 'Unavailable';
        runReason = 'Could not load AutoTrader configuration';
      } else if (!enabled) {
        runStatus = 'Disabled';
        runReason = 'Not enabled in the current strategy selection';
      } else if (!state.autoTraderEnabled) {
        runStatus = 'Paused';
        runReason = 'Auto Trader is paused';
      } else if (strategy === 'dated-futures-carry' && state.datedCarryDiagnostics) {
        const carry = state.datedCarryDiagnostics;
        const carryPositions = (carry.positions ?? []).filter((position) => !['settled', 'closed', 'complete'].includes(position.status.toLowerCase()));
        const eligibleCarry = (carry.candidates ?? []).filter((candidate) => candidate.eligible);
        const checkedAt = carry.checked_at_ms ? new Date(carry.checked_at_ms).toLocaleString() : undefined;
        if (carry.last_error) {
          runStatus = 'Scan error';
          runReason = carry.last_error;
        } else if (carryPositions.length > 0) {
          runStatus = 'Position open';
          runReason = carryPositions.map((position) => position.future_symbol + ' · ' + position.status + ' · settles ' + new Date(position.expiry_ms).toLocaleDateString()).join('; ');
        } else if (!checkedAt) {
          runStatus = 'Waiting for scan';
          runReason = 'No dated futures carry market scan has completed yet';
        } else if (eligibleCarry.length > 0) {
          runStatus = 'Signal active';
          runReason = eligibleCarry.map((candidate) => candidate.spot_symbol + '/' + candidate.future_symbol + ' · basis +' + candidate.basis_percent.toFixed(3) + '% · after budget +' + candidate.net_after_budget_percent.toFixed(3) + '% · ' + candidate.days_to_expiry.toFixed(1) + ' days to expiry').join('; ');
        } else {
          runStatus = 'No qualifying signal';
          runReason = (carry.candidates ?? []).map((candidate) => candidate.spot_symbol + '/' + candidate.future_symbol + ' · ' + candidate.reason + ' · basis ' + candidate.basis_percent.toFixed(3) + '%').join('; ') || 'No BTC or ETH delivery contracts were found in the 58–62 day entry window';
        }
        if (checkedAt) runReason += ' · scanned ' + checkedAt;
      } else if (openPosition) {
        runStatus = 'Position open';
        runReason = protectiveStop ? openPosition.symbol + ' · protective stop ' + protectiveStop.status : openPosition.symbol;
      } else if (workingOrder) {
        runStatus = 'Order working';
        runReason = [workingOrder.symbol, workingOrder.order_type, workingOrder.status].filter(Boolean).join(' · ');
      } else if (latestOrder?.status === 'failed' && latestOrder.failure_reason) {
        runStatus = 'Blocked';
        runReason = latestOrder.failure_reason;
      } else if (diagnostic?.btc_gate_required && diagnostic.btc_gate_passed === false) {
        runStatus = 'Blocked';
        runReason = 'BTC trend gate not passed' + (diagnostic.btc_distance_percent != null ? ' (' + diagnostic.btc_distance_percent.toFixed(2) + '% vs SMA' + (diagnostic.btc_window_days ?? '') + ')' : '');
      } else if (diagnostic?.symbols.some((item) => item.signal_active)) {
        runStatus = 'Signal active';
        runReason = diagnostic.symbols.filter((item) => item.signal_active).map((item) => item.symbol).join(', ');
      } else if (diagnostic && diagnostic.symbols.length > 0) {
        runStatus = 'No qualifying signal';
        const blocked = diagnostic.symbols.find((item) => !item.eligible || !item.primary_passed || item.secondary_passed === false);
        const primaryDetail = blocked?.primary_metric_unit === 'RSI' && typeof blocked.primary_metric_value === 'number'
          ? 'RSI ' + blocked.primary_metric_value.toFixed(2)
          : typeof blocked?.primary_distance_percent === 'number' ? blocked.primary_distance_percent.toFixed(2) + '%' : '';
        runReason = blocked
          ? blocked.symbol + ' · ' + (!blocked.eligible ? 'not eligible' : !blocked.primary_passed ? blocked.primary_rule + ' not met (' + primaryDetail + ')' : (blocked.secondary_rule ?? 'secondary rule') + ' not met' + (blocked.secondary_distance_percent != null ? ' (' + blocked.secondary_distance_percent.toFixed(2) + '%)' : ''))
          : 'Evaluated; no signal is active';
      } else if (latestOrder) {
        runStatus = latestOrder.status === 'filled' ? 'Recently traded' : 'Last order';
        runReason = [latestOrder.symbol, latestOrder.status, latestOrder.failure_reason].filter(Boolean).join(' · ') || 'No block reason recorded';
      }
      const fees = trades.map((row) => row.feesUsd);
      const netValues = trades.map((row) => row.netPnlUsd);
      const grossValues = trades.map((row) => typeof row.quoteUsdRate === 'number' && Number.isFinite(row.quoteUsdRate) && typeof row.feesUsd === 'number' && Number.isFinite(row.feesUsd) ? row.pnl * row.quoteUsdRate + row.feesUsd : undefined);
      const netComplete = trades.length > 0 && netValues.every((value): value is number => typeof value === 'number' && Number.isFinite(value));
      const grossComplete = trades.length > 0 && grossValues.every((value): value is number => typeof value === 'number' && Number.isFinite(value));
      const outcomes = netComplete ? netValues : trades.map((row) => row.pnl);
      const wins = outcomes.filter((value) => value > 0).length;
      const losses = outcomes.filter((value) => value < 0).length;
      const netSamples = netComplete ? trades.flatMap((trade, index) => {
        const value = netValues[index];
        const capital = trade.quantity * trade.entryPrice * (trade.quoteUsdRate ?? 1);
        return typeof value === 'number' && Number.isFinite(capital) && capital > 0
          ? [{ value, returnPercent: value / capital * 100 }]
          : [];
      }) : [];
      const netWins = netSamples.filter((sample) => sample.value > 0);
      const netLosses = netSamples.filter((sample) => sample.value < 0);
      const profitTotal = grossComplete ? grossValues.filter((value) => value > 0).reduce((sum, value) => sum + value, 0) : undefined;
      const lossTotal = grossComplete ? Math.abs(grossValues.filter((value) => value < 0).reduce((sum, value) => sum + value, 0)) : undefined;
      const holdingTimes = trades.map((row) => (row.closedAt ?? 0) - row.openedAt).filter((value) => Number.isFinite(value) && value >= 0);
      const entrySizes = trades.map((row) => typeof row.quoteUsdRate === 'number' && Number.isFinite(row.quoteUsdRate) ? Math.abs(row.quantity * row.entryPrice * row.quoteUsdRate) : undefined);
      const validEntrySizes = entrySizes.filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
      const averageReturn = trades.length ? (netComplete
        ? netSamples.reduce((sum, sample) => sum + sample.returnPercent, 0) / trades.length
        : trades.reduce((sum, row) => sum + row.pnl / (row.quantity * row.entryPrice) * 100, 0) / trades.length) : undefined;
      const closedTimes = trades.map((row) => row.closedAt ?? row.openedAt).filter((value) => value > 0 && Number.isFinite(value));
      const firstClosed = closedTimes.length ? Math.min(...closedTimes) : 0;
      const lastClosed = closedTimes.length ? Math.max(...closedTimes) : 0;
      const totalNet = netComplete ? netValues.reduce((sum, value) => sum + value, 0) : undefined;
      const mean = (values: number[]) => values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : undefined;
      return { strategy, trades: trades.length, wins, losses, flat: trades.length - wins - losses,
        enabled, runStatus, runReason,
        todayRealizedUsd: state.todayStrategyRealizedUsd?.[strategy] ?? (state.todayStrategyRealizedUsd ? 0 : undefined),
        feesUsd: trades.length > 0 && fees.every((value): value is number => typeof value === 'number' && Number.isFinite(value))
          ? fees.reduce((sum, value) => sum + value, 0) : undefined,
        netPnlUsd: totalNet,
        winRate: trades.length ? wins / trades.length * 100 : undefined,
        averageReturn,
        netExpectancyUsd: totalNet !== undefined && trades.length ? totalNet / trades.length : undefined,
        averageNetWinUsd: netComplete ? mean(netWins.map((sample) => sample.value)) : undefined,
        averageNetWinPercent: netComplete ? mean(netWins.map((sample) => sample.returnPercent)) : undefined,
        averageNetLossUsd: netComplete ? mean(netLosses.map((sample) => sample.value)) : undefined,
        averageNetLossPercent: netComplete ? mean(netLosses.map((sample) => sample.returnPercent)) : undefined,
        profitFactor: profitTotal !== undefined && lossTotal !== undefined ? (lossTotal > 0 ? profitTotal / lossTotal : profitTotal > 0 ? Number.POSITIVE_INFINITY : undefined) : undefined,
        medianHoldingMs: holdingTimes.length ? median(holdingTimes) : undefined,
        averageHoldingMs: mean(holdingTimes),
        periodStart: firstClosed,
        periodEnd: lastClosed,
        averageEntrySizeUsd: validEntrySizes.length === trades.length ? mean(validEntrySizes) : undefined,
        lowSample: trades.length > 0 && trades.length < 30,
        learningMultiplier: state.strategyLearning[strategy]?.learning_multiplier,
        learningSamples: state.strategyLearning[strategy]?.learning_samples,
        learningPositiveRate: state.strategyLearning[strategy]?.positive_rate ?? undefined };
    }).sort((a, b) => b.trades - a.trades || a.strategy.localeCompare(b.strategy));
  }, [closedRows, state.strategyLearning, state.enabledStrategies, state.autoTraderEnabled, state.strategyDiagnostics, state.strategyOrders, state.todayStrategyRealizedUsd, state.openStrategyPositions, state.autoTraderStatusAvailable, openRows]);
  const strategyColumns = useMemo<(ColDef<StrategyWinRateRow> | ColGroupDef<StrategyWinRateRow>)[]>(() => [
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), headerClass: 'strategy-column-strategy-header strategy-group-end', cellClass: 'strategy-column-strategy-cell strategy-group-end', cellStyle: { backgroundColor: '#15181d' }, minWidth: 170 },
    { headerName: 'Performance', headerClass: 'strategy-group-performance', children: colorStrategyColumns([
      { field: 'feesUsd', headerName: 'Fees', headerTooltip: 'Entry and exit fees for fully closed trades, converted using the latest quote currency rate.', cellClassRules: { 'pnl-negative': (p) => typeof p.value === 'number' && p.value > 0 }, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', width: 64, minWidth: 64, maxWidth: 72, flex: 0 },
      { field: 'todayRealizedUsd', headerName: 'Today USD', headerTooltip: 'Realized P&L today for this strategy, grouped from the same position ledger used by the Realized today summary. Converted using the latest quote currency rates.', cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', width: 78, minWidth: 78, maxWidth: 86, flex: 0 },
      { field: 'netPnlUsd', headerName: 'Sample net', headerTooltip: 'Realized P&L after fees for fully closed trades in the available strategy sample, converted using the latest quote currency rate.', cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', width: 78, minWidth: 78, maxWidth: 86, flex: 0 },
      { field: 'netExpectancyUsd', headerName: 'Exp/trade', headerTooltip: 'Average audited realized net P&L per closed trade after fees; unavailable if any trade lacks quote conversion data.', cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', width: 82, minWidth: 82, maxWidth: 90, flex: 0 },
      { field: 'averageNetWinUsd', headerName: 'Avg win', headerTooltip: 'Mean net P&L for winning trades, shown as USD and return percent.', cellClassRules: signed, valueFormatter: ({ value, data }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) + ' (' + pct(data?.averageNetWinPercent) + ')' : '—', width: 95, minWidth: 95, maxWidth: 103, flex: 0 },
      { field: 'averageNetLossUsd', headerName: 'Avg loss', headerTooltip: 'Mean net P&L for losing trades, shown as USD and return percent.', cellClassRules: signed, valueFormatter: ({ value, data }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) + ' (' + pct(data?.averageNetLossPercent) + ')' : '—', width: 125, minWidth: 125, maxWidth: 133, flex: 0 },
      { field: 'profitFactor', headerName: 'PF', headerTooltip: 'Gross realized profits divided by gross realized losses before fees; requires fee data for every trade.', valueFormatter: ({ value }) => typeof value === 'number' && value === Number.POSITIVE_INFINITY ? '∞' : typeof value === 'number' && Number.isFinite(value) ? value.toFixed(2) : '—', width: 48, minWidth: 48, maxWidth: 54, flex: 0 },
      { field: 'winRate', headerName: 'Win%', sort: 'desc', valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(1) + '%' : '—', width: 55, minWidth: 55, maxWidth: 62, flex: 0 },
      { field: 'averageReturn', headerName: 'Ret%', cellClassRules: signed, valueFormatter: ({ value }) => pct(value), width: 58, minWidth: 58, maxWidth: 65, flex: 0 },
    ], 'performance') },
    { headerName: 'Closed sample', headerClass: 'strategy-group-sample', children: colorStrategyColumns([
      { field: 'trades', headerName: 'Trades', headerTooltip: 'Trade count is flagged as low sample below 30 closed trades.', cellRenderer: ({ data }: { data?: StrategyWinRateRow }) => data ? <span>{data.trades}{data.lowSample && <small title="Fewer than 30 closed trades; treat the statistics as preliminary"> · low sample</small>}</span> : '—', width: 112, minWidth: 112, maxWidth: 128, flex: 0 },
      { field: 'wins', headerName: 'Wins', width: 46, minWidth: 46, maxWidth: 52, flex: 0 },
      { field: 'losses', headerName: 'Losses', width: 50, minWidth: 50, maxWidth: 56, flex: 0 },
      { field: 'periodStart', headerName: 'Period', headerTooltip: 'Date range from the oldest to newest fully closed trade in this strategy.', valueFormatter: ({ value, data }) => typeof value === 'number' && value > 0 && typeof data?.periodEnd === 'number' && data.periodEnd > 0 ? dateLabel(value) + (dateLabel(value) === dateLabel(data.periodEnd) ? '' : ' – ' + dateLabel(data.periodEnd)) : '—', width: 100, minWidth: 100, maxWidth: 130, flex: 0 },
    ], 'sample') },
    { headerName: 'Strategy status', headerClass: 'strategy-group-status', children: colorStrategyColumns([
      { field: 'enabled', headerName: 'On', valueFormatter: ({ value }) => value === true ? 'Yes' : value === false ? 'No' : '—', width: 42, minWidth: 42, maxWidth: 48, flex: 0 },
      { field: 'runStatus', headerName: 'Run state', tooltipField: 'runStatus', width: 148, minWidth: 148, maxWidth: 148, flex: 0 },
      { field: 'runReason', headerName: 'Latest reason', tooltipField: 'runReason', minWidth: 140 },
    ], 'status') },
    { headerName: 'Trade profile', headerClass: 'strategy-group-profile', children: colorStrategyColumns([
      { field: 'medianHoldingMs', headerName: 'Hold M/A', headerTooltip: 'Median and average time from entry to final exit.', valueFormatter: ({ value, data }) => typeof value === 'number' && typeof data?.averageHoldingMs === 'number' ? durationLabel(value) + ' / ' + durationLabel(data.averageHoldingMs) : '—', width: 82, minWidth: 82, maxWidth: 92, flex: 0 },
      { field: 'averageEntrySizeUsd', headerName: 'Entry USD', headerTooltip: 'Mean USD notional at entry for fully closed trades; uses entry price and the latest quote currency conversion rate.', valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', width: 72, minWidth: 72, maxWidth: 82, flex: 0 },
    ], 'profile') },
    { headerName: 'Learning · all symbols', headerClass: 'strategy-group-learning', children: colorStrategyColumns([
      { field: 'learningMultiplier', headerName: 'Mult.', headerTooltip: 'Current strategy size multiplier learned from closed outcomes pooled across symbols.', valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? '×' + value.toFixed(2) : '—', width: 56, minWidth: 56, maxWidth: 64, flex: 0 },
      { field: 'learningSamples', headerName: 'Learn n', headerTooltip: 'Number of closed outcomes used by the pooled learning model across all symbols for this strategy.', valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? value.toLocaleString() : '—', width: 60, minWidth: 60, maxWidth: 68, flex: 0 },
      { field: 'learningPositiveRate', headerName: 'Positive', headerTooltip: 'Share of pooled learning outcomes with positive net realized P&L across all symbols.', valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? (value * 100).toFixed(1) + '%' : '—', width: 62, minWidth: 62, maxWidth: 70, flex: 0 },
    ], 'learning') },
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
    { colId: 'net-vs-breakeven', headerName: 'Net vs breakeven', cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
      if (!data || data.status !== 'Open') return data?.status ?? '—';
      const netQuote = openPnl(data);
      if (netQuote === undefined) return <span className="auto-trader-pnl-bar-unavailable">Mark unavailable</span>;
      if (typeof data.quoteUsdRate !== 'number' || !Number.isFinite(data.quoteUsdRate)) return <span className="auto-trader-pnl-bar-unavailable">USD conversion unavailable</span>;
      const netUsd = netQuote * data.quoteUsdRate;
      const notional = data.quantity * data.entryPrice;
      const returnPct = notional > 0 ? netQuote / notional * 100 : 0;
      const positive = netUsd > 0;
      const label = (netUsd >= 0 ? '+' : '') + 'USD ' + netUsd.toFixed(3) + ' · ' + (returnPct >= 0 ? '+' : '') + returnPct.toFixed(2) + '%';
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
          <span className={'auto-trader-pnl-bar-fill ' + (positive ? 'is-positive' : netUsd < 0 ? 'is-negative' : 'is-flat')} style={{ left: fillLeft + '%', width: fillWidth + '%' }} />
          <span className="auto-trader-pnl-marker is-entry" style={{ left: entryPosition + '%' }} title="Entry" />
          <span className="auto-trader-pnl-marker is-breakeven" style={{ left: breakEvenPosition + '%' }} title="Net breakeven" />
          {stopPosition !== undefined && <span className="auto-trader-pnl-marker is-stop" style={{ left: stopPosition + '%' }} title={'Stop loss −' + data.stopLossPercent?.toFixed(2) + '%'} />}
          {targetPosition !== undefined && <span className="auto-trader-pnl-marker is-target" style={{ left: targetPosition + '%' }} title={'Take profit +' + data.takeProfitPercent?.toFixed(2) + '%'} />}
        </div>
        <span className={'auto-trader-pnl-bar-label ' + (positive ? 'is-positive' : netUsd < 0 ? 'is-negative' : 'is-flat')}>{label}</span>
        <div className="auto-trader-pnl-exit-percentages" title="Net exit thresholds as a percentage of entry notional">
          {stopPosition !== undefined && <span className="is-stop" style={{ left: stopPosition + '%' }}>SL −{data.stopLossPercent!.toFixed(2)}%</span>}
          {targetPosition !== undefined && <span className="is-target" style={{ left: targetPosition + '%' }}>TP +{data.takeProfitPercent!.toFixed(2)}%</span>}
        </div>
      </div>;
    }, minWidth: 320, flex: 2 },
    { field: 'status', headerName: 'State', minWidth: 75 },
    { field: 'openedAt', headerName: 'Opened', valueFormatter: ({ value }) => value ? new Date(value).toLocaleString() : '—', minWidth: 150 },
    { field: 'closedAt', headerName: 'Closed', valueFormatter: ({ value, data }) => value ? new Date(value).toLocaleString() : data?.status === 'Partially closed' ? '' : '—', minWidth: 150 },
    { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
    { headerName: 'Open for', valueGetter: ({ data }) => data?.status === 'Open' ? Math.max(0, now - data.openedAt) : undefined, valueFormatter: ({ value }) => durationLabel(value), minWidth: 95, maxWidth: 115, flex: 0 },
    { headerName: 'Manual close', colId: 'manual-close', sortable: false, filter: false, minWidth: 180, maxWidth: 190, flex: 0, suppressMouseEventHandling: ({ event }) => event.target instanceof Element && Boolean(event.target.closest('button')), cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
      if (!liveOnly || !data || data.status !== 'Open') return null;
      const confirming = state.confirmingCloseSymbols.has(data.symbol);
      const closing = state.closingSymbols.has(data.symbol);
      const firstForSymbol = rows.find((row) => row.status === 'Open' && row.symbol === data.symbol);
      if (firstForSymbol && firstForSymbol.rowId !== data.rowId) {
        return <span title={'The close action for ' + data.symbol + ' closes all strategy lots for this symbol.'}>Same-symbol close above</span>;
      }
      if (closing) return <span className="auto-trader-close-progress">Closing…</span>;
      if (confirming) return <div className="auto-trader-position-close-actions">
        <span>Close all {data.symbol} strategy lots?</span>
        <button className="is-confirm" type="button" disabled={closing} onClick={(event) => { event.stopPropagation(); void model.confirmClose(data.symbol); }}>Confirm</button>
        <button className="is-cancel" type="button" disabled={closing} onClick={(event) => { event.stopPropagation(); model.cancelClose(data.symbol); }}>Cancel</button>
      </div>;
      return <button className="auto-trader-position-close-button" type="button" disabled={closing} title="Submit an IOC order to close the combined AutoTrader position for this symbol at available market liquidity">Close symbol</button>;
    } },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), minWidth: 170 },
    { field: 'side', headerName: 'Side', valueFormatter: ({ value }) => typeof value === 'string' ? value.toUpperCase() : '—', minWidth: 75 },
    { field: 'quantity', headerName: 'Quantity', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(6) : '—', minWidth: 100 },
    { headerName: 'Remaining', valueGetter: ({ data }) => data?.status === 'Partially closed' ? data.remainingQuantity : undefined, valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(6) : '—', minWidth: 100 },
    { field: 'entryPrice', headerName: 'Entry', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(7) : '—', minWidth: 100 },
    { field: 'markPrice', headerName: 'Mark', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(7) : '—', minWidth: 100 },
    { headerName: 'Size USD', headerTooltip: 'Open quantity at the latest mark, converted to USD; uses entry price when no mark is available.', valueGetter: ({ data }) => {
      if (data?.status !== 'Open' || typeof data.quoteUsdRate !== 'number' || !Number.isFinite(data.quoteUsdRate)) return undefined;
      const price = data.markPrice ?? data.entryPrice;
      return Math.abs(data.quantity * price) * data.quoteUsdRate;
    }, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', minWidth: 90, maxWidth: 125 },
    { headerName: 'At stop (USD)', headerTooltip: 'Estimated loss if the stop-loss threshold is reached, based on entry notional and converted to USD. Excludes fees and slippage.', valueGetter: ({ data }) => {
      if (data?.status !== 'Open' || typeof data.stopLossPercent !== 'number' || !Number.isFinite(data.stopLossPercent) || data.stopLossPercent <= 0 || typeof data.quoteUsdRate !== 'number' || !Number.isFinite(data.quoteUsdRate)) return undefined;
      return -Math.abs(data.quantity * data.entryPrice) * data.stopLossPercent / 100 * data.quoteUsdRate;
    }, cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', minWidth: 112, maxWidth: 135 },
    { headerName: 'At target (USD)', headerTooltip: 'Estimated gain if the take-profit threshold is reached, based on entry notional and converted to USD. Excludes fees and slippage.', valueGetter: ({ data }) => {
      if (data?.status !== 'Open' || typeof data.takeProfitPercent !== 'number' || !Number.isFinite(data.takeProfitPercent) || data.takeProfitPercent <= 0 || typeof data.quoteUsdRate !== 'number' || !Number.isFinite(data.quoteUsdRate)) return undefined;
      return Math.abs(data.quantity * data.entryPrice) * data.takeProfitPercent / 100 * data.quoteUsdRate;
    }, cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? usd.format(value) : '—', minWidth: 120, maxWidth: 145 },
    { headerName: 'Unrealized P&L (USD)', headerTooltip: 'Current open P&L net of entry fees and estimated exit fees and adverse slippage.', valueGetter: ({ data }) => {
      const pnl = data?.status === 'Open' ? openPnl(data) : undefined;
      return pnl !== undefined && typeof data?.quoteUsdRate === 'number' && Number.isFinite(data.quoteUsdRate) ? pnl * data.quoteUsdRate : undefined;
    }, cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? money.format(value) : '—', minWidth: 150 },
    { headerName: 'Entry fees', valueGetter: ({ data }) => data?.status === 'Open' ? Math.abs(data.entryFees) : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 110 },
    { headerName: 'Est. exit fee', valueGetter: ({ data }) => data?.status === 'Open' && data.markPrice !== undefined ? Math.abs(data.signedQuantity * data.markPrice) * data.estimatedExitFeeRate : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 120 },
    { headerName: 'Est. exit slippage', valueGetter: ({ data }) => data?.status === 'Open' && data.markPrice !== undefined ? Math.abs(data.signedQuantity * data.markPrice) * data.estimatedExitSlippageBps / 10_000 : undefined, valueFormatter: ({ value, data }) => typeof value === 'number' ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4) : '—', minWidth: 135 },
    { headerName: 'Realized / open P&L (USD)', headerTooltip: 'Realized closed-trade P&L or estimated open P&L converted from quote currency to USD.', valueGetter: ({ data }) => data?.status === 'Open' ? (openPnl(data) !== undefined && typeof data.quoteUsdRate === 'number' && Number.isFinite(data.quoteUsdRate) ? openPnl(data)! * data.quoteUsdRate : undefined) : data?.netPnlUsd, cellClassRules: signed, valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? money.format(value) : '—', minWidth: 170 },
    { field: 'direction', headerName: 'Signal direction', minWidth: 110 },
    { field: 'strength', headerName: 'Strength', valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 85 },
    { field: 'scout', headerName: 'Scout', valueFormatter: ({ value }) => value === true ? 'Yes' : value === false ? 'No' : '—', minWidth: 75 },
    { headerName: 'Trend 1m', valueGetter: ({ data }) => data?.components.trend_1m_percent, valueFormatter: ({ value }) => pct(value), minWidth: 95 },
    { headerName: 'Trend 5m / vol', valueGetter: ({ data }) => data?.components.trend_5m_volatility_units, valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(2) + 'σ' : '—', minWidth: 110 },
    { headerName: 'Momentum 10s', valueGetter: ({ data }) => data?.components.momentum_10s_percent, valueFormatter: ({ value }) => pct(value), minWidth: 105 },
    { headerName: 'Trade flow', valueGetter: ({ data }) => data?.components.trade_flow_imbalance, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 95 },
    { headerName: 'Book pressure', valueGetter: ({ data }) => data?.components.book_imbalance, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 105 },
    { headerName: 'VWAP mispricing', valueGetter: ({ data }) => data?.components.mispricing_vs_vwap_bps, valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(1) + ' bps' : '—', minWidth: 120 },
    { headerName: 'Pressure / no move', valueGetter: ({ data }) => data?.components.pressure_without_price_move, valueFormatter: ({ value }) => typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—', minWidth: 130 },
  ], [now, liveOnly, model, rows, state.confirmingCloseSymbols, state.closingSymbols, state.datedCarryDiagnostics]);
  const completedOnlyColumns = new Set(['Open for', 'Manual close', 'Mark', 'Size USD', 'At stop (USD)', 'At target (USD)', 'Unrealized P&L (USD)', 'Entry fees', 'Est. exit fee', 'Est. exit slippage', 'Net vs breakeven', 'State']);
  const visibleColumns = liveOnly
    ? ['Symbol', 'Strategy', 'Open for', 'Size USD', 'At stop (USD)', 'At target (USD)', 'Unrealized P&L (USD)', 'Net vs breakeven', 'Manual close'].flatMap((headerName) => columns
      .filter((column) => column.headerName === headerName)
      .map((column) => headerName === 'Net vs breakeven'
        ? { ...column, flex: 1, minWidth: 320 }
        : { ...column, flex: 0 }))
    : closedOnly ? columns.filter((column) => !completedOnlyColumns.has(String(column.headerName ?? '')))
      : columns;
  return <section className={embedded ? 'auto-trader-outcomes-embedded' : 'feed-section signal-info-panel'} aria-label={strategyRatesOnly ? 'AutoTrader strategy win rates' : liveOnly ? 'Live AutoTrader positions' : 'AutoTrader position outcomes'}>
    {!embedded && <FeedToolbar title={liveOnly ? 'AutoTrader live positions' : closedOnly ? 'AutoTrader closed & partial positions' : 'AutoTrader positions & outcomes'} />}
    {!liveOnly && !closedOnly && !strategyRatesOnly && <p className="signal-info-note">{openRows.length} open positions · {summary.markedOpenCount}/{openRows.length} marked · indicative open P&amp;L {summary.markedOpenCount ? Object.entries(summary.openPnlByQuote).map(([quote, value]) => quote + ' ' + (value >= 0 ? '+' : '') + value.toFixed(4)).join(' · ') : 'unavailable'} · {summary.wins} closed winners · {summary.losses} closed losers · {summary.avg.toFixed(3)}% average closed return. Open marks use the latest price tick and net out signed entry fees, estimated exit fees, and estimated adverse exit slippage based on AutoTrader fills for that symbol; closed P&amp;L uses audited realized values. Signal values show the recorded entry decision, not proof of causation.</p>}
    {!strategyRatesOnly && state.error && <div className="positions-error">{state.error}</div>}
    {!closedOnly && !strategyRatesOnly && <div className="auto-trader-pnl-marker-legend" aria-label="Position bar marker key">
      <span className="is-stop">SL</span><span><i className="is-entry" />Entry</span>
      <span><i className="is-breakeven" />BE</span><span className="is-target">TP</span>
    </div>}
    {(strategyRatesOnly || (!liveOnly && !closedOnly)) && <><strong className="auto-trader-strategy-winrate-title">Strategy performance and today’s realized P&L</strong>
    <div className="data-grid-wrap signal-info-grid auto-trader-strategy-winrate-grid"><AgGridReact<StrategyWinRateRow> rowData={strategyWinRates} columnDefs={strategyColumns} getRowId={({ data }) => data.strategy} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 48 }} theme={theme} domLayout="autoHeight" rowHeight={26} headerHeight={28} /></div>
    {strategyWinRates.length === 0 && !state.loading && <p className="positions-empty">No closed trades to calculate strategy win rates yet.</p>}</>}
    {!strategyRatesOnly && <div className="data-grid-wrap signal-info-grid"><AgGridReact<OutcomeGridRow> rowData={rows} columnDefs={visibleColumns} onGridReady={(event) => { outcomesGridApi.current = event.api; }} getRowHeight={({ data }) => data && state.confirmingCloseSymbols.has(data.symbol) ? 72 : 48} onCellClicked={(event) => {
      if (!liveOnly || event.column.getColId() !== 'manual-close' || !event.data || event.data.status !== 'Open') return;
      const target = event.event?.target;
      if (!(target instanceof Element)) return;
      if (target.closest('.is-confirm')) void model.confirmClose(event.data.symbol);
      else if (target.closest('.is-cancel')) model.cancelClose(event.data.symbol);
      else if (target.closest('.auto-trader-position-close-button')) model.requestClose(event.data.symbol);
    }} onFirstDataRendered={liveOnly ? (event) => {
      const columnIds = (event.api.getColumns() ?? []).map((column) => column.getColId()).filter((columnId) => columnId !== 'net-vs-breakeven');
      if (columnIds.length) event.api.autoSizeColumns(columnIds);
    } : undefined} getRowId={({ data }) => data.rowId} getRowClass={({ data }) => data?.status === 'Open' ? 'auto-trader-open-position-row' : 'auto-trader-closed-position-row'} defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 80 }} theme={theme} domLayout="autoHeight" rowHeight={48} headerHeight={28} /></div>}
    {!strategyRatesOnly && !state.loading && rows.length === 0 && !state.error && <p className="positions-empty">{liveOnly ? 'No live AutoTrader positions are open.' : closedOnly ? 'No fully or partially closed AutoTrader positions are available yet.' : 'No open AutoTrader positions or fully closed audited trades are available yet.'}</p>}
  </section>;
}
export const AutoTraderOutcomes = memo(AutoTraderOutcomesComponent);
