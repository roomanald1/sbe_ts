import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  CellStyleModule,
  ClientSideRowModelModule,
  TextFilterModule,
  ModuleRegistry,
  NumberFilterModule,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
  type ITooltipParams,
} from 'ag-grid-community';
import { formatForSymbol } from '../instrument_precision';
import type { SymbolPriceTick } from '../types/symbol_price';
import { AutoTraderModel } from '../models/AutoTraderModel';
import type { AutoTraderOrder, ResearchStrategy } from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { getOrderTypeDescription } from '../order_type_descriptions';

const money = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const timestamp = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

function explainAutoTraderReason(reason: string): string {
  const lower = reason.toLowerCase();
  if (lower.startsWith('no tradeable bias')) {
    const fresh = reason.match(/fresh directional (\d+)\/(\d+)/i);
    const strength = reason.match(/strength floors passed (\d+)/i);
    const exploration = reason.match(/raw-score floors passed (\d+)/i);
    const usable = fresh ? Number(fresh[1]) : 0;
    const total = fresh ? Number(fresh[2]) : 0;
    if (total === 0) return 'No insights available';
    if (usable === 0) {
      const filters = reason.match(/filtered: (\d+) stale, (\d+) non-directional, (\d+) blocked by existing positions/i);
      const causes = filters
        ? Number(filters[1]) + ' stale · ' + Number(filters[2]) + ' no direction · ' + Number(filters[3]) + ' position blocked'
        : 'filtered before strength or history checks';
      return 'No usable buy or sell signal\n' + total + ' insight checked · ' + causes;
    }
    if (strength?.[1] === '0' && exploration?.[1] === '0') return 'Signal too weak\n' + usable + ' usable · 0 passed normal strength · 0 passed trial score';
    const history = reason.match(/(\d+) candidate\(s\) blocked by negative walk-forward history/i);
    const blocked = history ? Number(history[1]) : 0;
    if (blocked > 0) return 'Past-performance check blocked ' + blocked + ' candidate' + (blocked === 1 ? '' : 's');
    return 'No signal passed all entry checks\n' + usable + ' usable signals checked';
  }
  if (lower.includes('no insights available')) return 'No insights available';
  if (lower.includes('no usable buy or sell signal') || lower.includes('no usable directional signal') || lower.includes('no fresh directional signal')) return 'No usable buy or sell signal';
  if (lower.includes('signals were stale')) return 'Insights were stale';
  if (lower.includes('no directional signal')) return 'Insights had no clear direction';
  if (lower.includes('position blocked')) return 'Existing position blocked another entry';
  if (lower.includes('signal too weak') || lower.includes('strength and score floors')) return 'Signal too weak for entry';
  if (lower.includes('daily realized-loss limit')) return 'Daily loss limit reached';
  if (lower.includes('budget') || lower.includes('capacity')) return 'Not enough budget or position capacity';
  if (lower.includes('walk-forward') || lower.includes('historical')) return 'Past-performance check blocked the trade';
  if (lower.includes('stale local order book')) return 'Market data was out of date';
  if (lower.includes('no cached local order book')) return 'Market data was unavailable';
  if (lower.includes('position') || lower.includes('cooldown')) return 'Existing position or wait period';
  if (lower.includes('cycle error')) return 'Decision cycle error';
  return reason;
}
ModuleRegistry.registerModules([CellStyleModule, ClientSideRowModelModule, NumberFilterModule, TextFilterModule]);

const autoTraderGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

type AutoTraderOrderRow = AutoTraderOrder & { current_price?: number; position_status?: string; protection_order?: boolean; strategy_group?: string; group_header?: boolean; group_label?: string };

const orderColumnDefs: ColDef<AutoTraderOrderRow>[] = [
  { field: 'timestamp', headerName: 'Time / Trigger', minWidth: 102, colSpan: ({ data }) => data?.group_header ? orderColumnDefs.length : 1, cellRenderer: ({ data, value }: ICellRendererParams<AutoTraderOrderRow>) => data?.group_header ? <strong>{data.group_label}</strong> : value ? timestamp.format(value) : '', valueGetter: ({ data }) => data?.group_header ? undefined : data?.triggered_at ?? data?.timestamp },
  { field: 'symbol', headerName: 'Symbol', minWidth: 78 },
  { field: 'side', headerName: 'Side', minWidth: 56, valueFormatter: ({ value }) => String(value ?? '').toUpperCase() },
  { field: 'order_type', headerName: 'Type', minWidth: 70, tooltipValueGetter: (params: ITooltipParams<AutoTraderOrderRow>) => getOrderTypeDescription(params.value), cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => <span title={getOrderTypeDescription(params.value)}>{String(params.value ?? '')}</span> },
  { field: 'strategy', headerName: 'Strategy', minWidth: 154, cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => <span>{params.data?.protection_order ? '↳ ' : ''}{strategyLabel(params.value)}</span> },
  { field: 'purpose', headerName: 'Purpose', minWidth: 76, valueFormatter: ({ value }) => value === 'portfolio-hedge' ? 'HEDGE' : String(value ?? '').toUpperCase() },
  { field: 'status', headerName: 'Order Status', minWidth: 82, filter: 'agTextColumnFilter' },
  { field: 'failure_reason', headerName: 'Failure reason', minWidth: 210, cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => { const reason = String(params.value ?? ''); return reason ? <span title={reason}>{reason}</span> : ''; } },
  { field: 'position_status', headerName: 'Position', minWidth: 75 },
  { field: 'filled_quantity', headerName: 'Filled / Qty', minWidth: 96, valueFormatter: ({ data }) => data ? formatForSymbol(data.filled_quantity, data.symbol, 'quantity') + '/' + formatForSymbol(data.quantity, data.symbol, 'quantity') : '' },
  { field: 'submitted_limit_price', headerName: 'Limit / Stop Price', minWidth: 112, valueGetter: ({ data }) => data?.submitted_limit_price ?? data?.target_price, valueFormatter: ({ value, data }) => value == null || !data ? '' : formatForSymbol(value, data.symbol, 'price') },
  { field: 'current_price', headerName: 'Current Price', minWidth: 92, valueFormatter: ({ value, data }) => value == null || !data ? '' : formatForSymbol(value, data.symbol, 'price') },
  { field: 'average_price', headerName: 'Avg Price', minWidth: 78, valueFormatter: ({ value }) => value == null ? '' : money.format(value) },
  { field: 'realized_pnl_usd', headerName: 'Realized USD', minWidth: 92, cellClass: ({ value }) => value >= 0 ? 'pnl-positive' : 'pnl-negative', valueFormatter: ({ value }) => value == null || Math.abs(value) < 1e-9 ? '—' : money.format(value) },
];

function strategyLabel(strategy?: string): string {
  const labels: Record<string, string> = {
    'donchian-55-btc200': 'Donchian 55 + BTC SMA200',
    'donchian-20-sma200': 'Donchian 20 + asset SMA200',
    'trend-sleeves-btc200': 'BTC/ETH/BNB trend sleeves',
    'trend-50-200': 'SMA 50/200 trend',
    'relative-strength-90-sma200': '90-day momentum rotation',
    'rsi2-oversold-rebound': 'RSI(2) rebound',
    'generic-insight-signals': 'Live insight signals',
    'portfolio-hedge': 'Portfolio hedge',
  };
  return strategy ? labels[strategy] ?? strategy : 'Legacy / unknown';
}

const defaultOrderColDef: ColDef<AutoTraderOrderRow> = {
  sortable: true,
  resizable: true,
  filter: true,
  flex: 1,
  minWidth: 68,
};
export function AutoTrader({ ticks }: { ticks: SymbolPriceTick[] }) {
  const currentPrices = useMemo(() => new Map(ticks.map((tick) => [tick.symbol, tick.price])), [ticks]);
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const ledgerOpenPositionSymbols = useMemo(() => new Set(state.ledgerOpenPositionSymbols), [state.ledgerOpenPositionSymbols]);
  const orderRows = useMemo(() => state.orders.map((order) => {
    const protectionOrder = order.purpose?.toLowerCase().includes('stop') ?? false;
    const positionIsOpen = ledgerOpenPositionSymbols.has(order.symbol);
    const failureReason = order.failure_reason ?? (protectionOrder && order.status === 'unfilled'
      ? positionIsOpen
        ? 'Stop triggered, but the position manager reported no fill.'
        : 'Stop triggered after the position was closed; no open quantity remained to reduce.'
      : undefined);
    return {
      ...order,
      failure_reason: failureReason,
      current_price: positionIsOpen ? currentPrices.get(order.symbol) : undefined,
      position_status: protectionOrder ? positionIsOpen ? 'Open' : 'Closed' : order.filled_quantity > 0 ? positionIsOpen ? 'Open' : 'Closed' : '',
      protection_order: protectionOrder,
      strategy_group: order.decision_id ?? ('legacy:' + (order.strategy ?? 'unknown') + ':' + order.symbol),
    };
  }), [state.orders, ledgerOpenPositionSymbols, currentPrices]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [parametersOpen, setParametersOpen] = useState(false);
  const [decisionDetailsOpen, setDecisionDetailsOpen] = useState(false);
  const [showOnlyLiveOrders, setShowOnlyLiveOrders] = useState(false);
  const visibleOrderRows = useMemo(() => {
    const filtered = showOnlyLiveOrders
      ? orderRows.filter((order) => ['submitting', 'accepted', 'resting', 'partially_filled', 'armed', 'triggering'].includes(order.status.toLowerCase()))
      : orderRows;
    const groups = new Map<string, AutoTraderOrderRow[]>();
    for (const order of filtered) groups.set(order.strategy_group!, [...(groups.get(order.strategy_group!) ?? []), order]);
    const orderedGroups = [...groups.entries()].sort(([, a], [, b]) => Math.max(...b.map((row) => row.triggered_at ?? row.timestamp)) - Math.max(...a.map((row) => row.triggered_at ?? row.timestamp)));
    return orderedGroups.flatMap(([key, rows]) => {
      rows.sort((a, b) => (b.triggered_at ?? b.timestamp) - (a.triggered_at ?? a.timestamp));
      const first = rows[0];
      const label = strategyLabel(first.strategy) + ' · ' + first.symbol + ' · ' + rows.length + (rows.length === 1 ? ' order' : ' orders');
      return [{ timestamp: 0, symbol: '', side: '', order_type: '', quantity: 0, filled_quantity: 0, status: '', realized_pnl: 0, strategy_group: key, group_header: true, group_label: label }, ...rows];
    });
  }, [orderRows, showOnlyLiveOrders]);
  useEffect(() => {
    if (!parametersOpen) return;
    void model.refreshDecisionParameters();
    const timer = setInterval(() => void model.refreshDecisionParameters(), 5000);
    return () => clearInterval(timer);
  }, [model, parametersOpen]);
  const scoutOrderCounts = useMemo(() => {
    const scouts = state.orders.filter((order) => order.purpose === 'scout');
    return {
      total: scouts.length,
      filled: scouts.filter((order) => order.filled_quantity > 0).length,
      pending: scouts.filter((order) => ['accepted', 'resting', 'armed', 'submitting'].includes(order.status)).length,
      failed: scouts.filter((order) => order.status === 'failed').length,
    };
  }, [state.orders]);
  const scoutOutcomeCohort = state.decisionQuality.cohorts.find((cohort) => cohort.dimension === 'selection' && cohort.value === 'scout');
  return (
    <section className={"feed-section auto-trader-section" + (settingsOpen || parametersOpen ? " has-open-popup" : "")} aria-label="Auto Trader">
      <FeedToolbar title="Auto Trader" />
      <div className="auto-trader-content">
        <div className={"auto-trader-live-banner " + (state.enabled ? "is-live" : "") + (state.loading ? " is-loading" : "")}>
          <span className="auto-trader-live-dot" aria-hidden="true" />
          <strong>{state.loading ? 'Connecting' : state.enabled ? 'LIVE' : 'PAUSED'}</strong>
          <span>{state.loading ? 'Checking status' : state.enabled ? 'Auto Trader is active' : 'Auto Trader is off'}</span>
          <div className="auto-trader-live-actions">
            <button type="button" disabled={state.loading || state.busy} onClick={() => void model.setEnabled(!state.enabled)}>{state.busy ? 'Working…' : state.enabled ? 'Pause' : 'Go live'}</button>
          </div>
        </div>
        <div className="auto-trader-settings-anchor">
          <p>Paper · {state.serverExplorationPercent}% explore · SL {state.stopLossPercent.toFixed(2)}% / TP {state.takeProfitPercent.toFixed(2)}%</p>
          <div className="auto-trader-settings-actions">
            <button className="auto-trader-parameters-button" type="button" aria-expanded={parametersOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => { setParametersOpen((open) => !open); setSettingsOpen(false); }}>View parameters</button>
            <button className="auto-trader-settings-button" type="button" aria-expanded={settingsOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => { setSettingsOpen((open) => !open); setParametersOpen(false); }}>…</button>
          </div>
          {parametersOpen && <div className="auto-trader-settings-popup auto-trader-parameters-popup" role="dialog" aria-label="Current Auto Trader parameters">
            <div className="auto-trader-settings-heading"><strong>Current Auto Trader parameters</strong><button type="button" aria-label="Close parameters" onClick={() => setParametersOpen(false)}>×</button></div>
            <small aria-live="polite">Live parameters from the AutoTrader service · refreshed {state.decisionParameters ? timestamp.format(state.decisionParameters.refreshed_at) : 'on open and every 5s'}</small>
            {state.decisionParametersError && <p className="auto-trader-error" role="alert">{state.decisionParametersError}</p>}
            {!state.decisionParameters && !state.decisionParametersError && <p>Loading live decision parameters…</p>}
            {state.decisionParameters && <div className="auto-trader-parameter-sections">
              <section><strong>Shared risk controls</strong><dl>
                <div><dt>Risk appetite</dt><dd>{Number(state.riskAppetite)} · {Number(state.riskAppetite) < 34 ? 'Safe' : Number(state.riskAppetite) > 66 ? 'Risky' : 'Balanced'}</dd></div>
                <div><dt>Maximum per order</dt><dd>{Number(state.maxOrderNotional).toLocaleString()}</dd></div>
                <div><dt>Maximum allocated exposure</dt><dd>{Number(state.dailyNotionalLimit).toLocaleString()}</dd></div>
                <div><dt>Daily loss cap</dt><dd>{Number(state.dailyRealizedLossLimit).toLocaleString()}</dd></div>
                <div><dt>Maximum open positions</dt><dd>{state.maxOpenPositions}</dd></div>
                <div><dt>Maximum entries per position</dt><dd>{state.maxEntriesPerPosition}</dd></div>
                <div><dt>Exploration allocation</dt><dd>{state.explorationPercent}%</dd></div>
                <div><dt>Protective stop / target</dt><dd>{state.stopLossPercent.toFixed(2)}% / {state.takeProfitPercent.toFixed(2)}%</dd></div>
              </dl></section>
            </div>}
          </div>}
          {settingsOpen && <div className="auto-trader-settings-popup" role="dialog" aria-label="Auto Trader settings">
            <div className="auto-trader-settings-heading"><strong>Auto Trader settings</strong><button type="button" aria-label="Close settings" onClick={() => setSettingsOpen(false)}>×</button></div>
            <div className="auto-trader-limits">
              <fieldset className="auto-trader-strategy-options" title="Enabled daily signal families share the same AutoTrader portfolio exposure, daily loss, and open-position limits."><legend>STRATEGIES · SHARED PORTFOLIO LIMITS</legend>
                <label className="auto-trader-generic-strategy"><input type="checkbox" checked={state.genericSignalsEnabled} onChange={(event) => model.setGenericSignalsEnabled(event.target.checked)} /><span><strong>Live insight signals</strong><small>Use the existing directional signal feed, with walk-forward evidence and AutoTrader risk checks.</small></span></label>
                {([{ value: 'donchian-55-btc200', label: '55-day Donchian breakout · trade highs only while BTC is above its SMA200' }, { value: 'donchian-20-sma200', label: '20-day Donchian breakout · buy assets above their own SMA200' }, { value: 'trend-sleeves-btc200', label: 'Trend sleeves · BTC/ETH SMA50 + BNB SMA100 · BTC SMA200 risk gate' }, { value: 'trend-50-200', label: 'Generic trend following · hold each asset above its SMA50 and SMA200' }, { value: 'relative-strength-90-sma200', label: 'Generic momentum rotation · hold the strongest 90-day asset above its SMA200' }, { value: 'rsi2-oversold-rebound', label: 'Generic mean reversion · RSI(2) ≤ 10 above SMA200 · five-day max hold' }] as Array<{ value: ResearchStrategy; label: string }>).map(({ value, label }) => <label key={value}><input type="checkbox" checked={state.researchStrategies.includes(value)} onChange={(event) => model.setResearchStrategies(event.target.checked ? [...state.researchStrategies, value] : state.researchStrategies.filter((strategy) => strategy !== value))} />{label}</label>)}
                <small>20-day volatility breakout uses the same entry rule as the 20-day Donchian option. Buy-and-hold remains research-only because it needs a no-exit policy. RSI rebound positions retain their simulated five-day maximum hold, alongside the shared protective stop and target.</small>
                {state.researchStrategies.length === 0 && !state.genericSignalsEnabled && <small className="auto-trader-strategy-validation">Select at least one strategy before saving.</small>}
              </fieldset>
              <label>MAX PER ORDER (USD)<input type="number" min="0.01" step="any" value={state.maxOrderNotional} onChange={(event) => model.setLimits(event.target.value, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label title="Maximum USD notional allocated to Auto Trader positions and pending orders that add exposure. Opening positions uses available capacity; closing fills release it. Manual positions are not included.">MAX ALLOCATED EXPOSURE (USD)<input type="number" min="0.01" step="any" value={state.dailyNotionalLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, event.target.value, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>DAILY LOSS CAP (USD)<input type="number" min="0.01" step="any" value={state.dailyRealizedLossLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, event.target.value, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>EXPLORATION SIZE (%)<input type="number" min="0" max="100" step="1" value={state.explorationPercent} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, event.target.value, state.maxOpenPositions)} /></label>
              <label className="auto-trader-risk-slider" title="Shifts strength and score floors and adjusts take-profit. Riskier settings lower entry floors and move take-profit toward a 0.05% net win after fees; safer settings raise floors and preserve larger targets.">RISK APPETITE <span className="auto-trader-risk-value">{Number(state.riskAppetite)} · {Number(state.riskAppetite) < 34 ? 'Safe' : Number(state.riskAppetite) > 66 ? 'Risky' : 'Balanced'}</span><input aria-label="Auto Trader risk appetite" type="range" min="0" max="100" step="1" value={state.riskAppetite} onChange={(event) => model.setRiskAppetite(event.target.value)} /><span className="auto-trader-risk-scale"><span>Safe · higher floors</span><span>Risky · lower floors</span></span></label>
              <label title="Maximum number of distinct Auto Trader positions held at once.">MAX OPEN POSITIONS<input type="number" min="1" max="20" step="1" value={state.maxOpenPositions} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, event.target.value)} /></label>
              <label title="Maximum number of entries Auto Trader can make while a position remains open, including the initial entry.">ENTRIES PER POSITION<input type="number" min="1" max="10" step="1" value={state.maxEntriesPerPosition} onChange={(event) => model.setMaxEntriesPerPosition(event.target.value)} /></label>
              <label>VWAP / TWAP DURATION (MIN)<input type="number" min="5" max="390" step="1" value={state.vwapDurationMinutes} onChange={(event) => model.setExecutionParameters({ vwapDurationMinutes: event.target.value })} /></label>
              <label>VWAP PARTICIPATION (%)<input type="number" min="1" max="50" step="1" value={state.vwapParticipationPercent} onChange={(event) => model.setExecutionParameters({ vwapParticipationPercent: event.target.value })} /></label>
              <label>LIMIT OFFSET (BPS)<input type="number" min="-10" max="10" step="0.5" value={state.limitOffsetBps} onChange={(event) => model.setExecutionParameters({ limitOffsetBps: event.target.value })} /></label>
              <label>TRIGGER OFFSET (BPS)<input type="number" min="5" max="100" step="1" value={state.triggerOffsetBps} onChange={(event) => model.setExecutionParameters({ triggerOffsetBps: event.target.value })} /></label>
              <label>TRAILING DISTANCE (%)<input type="number" min="0.05" max="1" step="0.05" value={state.trailingDistancePercent} onChange={(event) => model.setExecutionParameters({ trailingDistancePercent: event.target.value })} /></label>
              <label title="Number of most recent fully closed exits used for rolling exit statistics. Range: 1–5000.">ROLLING CLOSED EXITS<input type="number" min="1" max="5000" step="1" value={state.closedTradeWindow} onChange={(event) => model.setClosedTradeWindow(event.target.value)} /></label>
              <button type="button" disabled={state.busy || state.loading || (state.researchStrategies.length === 0 && !state.genericSignalsEnabled)} onClick={() => void model.saveLimits()}>Save settings</button>
            </div>
          </div>}
        </div>
        <p className="auto-trader-allocation">Reserved exposure (incl. scheduled stop headroom) <strong>USD {money.format(state.usedNotionalToday)}</strong> / {money.format(state.serverDailyNotionalLimit)} cap · New exposure available <strong>USD {money.format(state.availableNotional)}</strong>{state.usedNotionalToday > state.serverDailyNotionalLimit && <span> · Existing positions or pending orders exceed the cap; new exposure is blocked.</span>}</p>
        <div className="auto-trader-insights">
          <div className="auto-trader-insight auto-trader-decision" role="status">
            <span className="auto-trader-insight-label">Why no trades? · {state.decisionsSinceTrade.toLocaleString()} scans since last order</span>
            {state.gapReasons.length > 0 ? <ol className="auto-trader-gap-reasons">
              {state.gapReasons.slice().sort((a, b) => b.count - a.count).slice(0, 3).map((item) => <li key={item.reason}>
                <span>{explainAutoTraderReason(item.reason)}</span><strong>{item.count.toLocaleString()} scans</strong>
              </li>)}
            </ol> : <span className="auto-trader-decision-text">{state.lastDecision ? explainAutoTraderReason(state.lastDecision) : 'Waiting for the first decision.'}</span>}

            {state.lastDecisionAt > 0 && <time dateTime={new Date(state.lastDecisionAt).toISOString()}>{timestamp.format(state.lastDecisionAt)}</time>}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Learning observations · today UTC</span>
            <strong>{state.learningSamples} learning samples</strong>
            <span className="auto-trader-learning-detail">
              {state.learningWinRate == null ? 'Positive rate unavailable' : (state.learningWinRate * 100).toFixed(0) + '% positive'}
              {state.preferredOrderType ? ' · Strongest: ' + state.preferredOrderType : ''}
              {' · Profile observations P0/P1/P2: ' + state.parameterProfileSamples.join('/')}
            </span>
            <span className="auto-trader-learning-detail">Includes realized fills and some unfilled post-only orders removed from the book. Cancelled protective stops with no fill do not count; partial stop fills do.</span>
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Execution · today UTC · {state.executionQuality.measured_fill_samples}/{state.executionQuality.filled_order_samples} fills measured</span>
            <strong>{state.executionQuality.average_all_in_cost_bps == null ? 'Cost unavailable' : (state.executionQuality.average_all_in_cost_bps >= 0 ? '+' : '') + state.executionQuality.average_all_in_cost_bps.toFixed(2) + ' bps all-in'}</strong>
            <span className="auto-trader-learning-detail">
              {state.executionQuality.average_implementation_shortfall_bps == null ? 'No reference price' : (state.executionQuality.average_implementation_shortfall_bps >= 0 ? '+' : '') + state.executionQuality.average_implementation_shortfall_bps.toFixed(2) + ' bps vs mid'}
              {' · '}{state.executionQuality.average_fee_bps == null ? 'Fees unavailable' : state.executionQuality.average_fee_bps.toFixed(2) + ' bps fees'}
              {' · '}{state.executionQuality.average_fill_rate == null ? 'Fill rate unavailable' : (state.executionQuality.average_fill_rate * 100).toFixed(1) + '% filled'}
              {' · '}{state.executionQuality.partial_fill_samples} partial · {state.executionQuality.unfilled_order_samples} unfilled
              {' · '}{state.executionQuality.reference_price_coverage_percent == null ? 'Reference coverage unavailable' : state.executionQuality.reference_price_coverage_percent.toFixed(0) + '% reference'}
              {state.executionQuality.average_submit_response_ms == null ? '' : ' · ' + state.executionQuality.average_submit_response_ms.toFixed(0) + 'ms paper submit round-trip'}
              {state.executionQuality.history_truncated ? ' · last 50k audit updates' : ''}
            </span>
          </div>
          <div className="auto-trader-insight auto-trader-scout-summary" aria-label="Today scout summary">
            <span className="auto-trader-insight-label">Scout summary · today UTC</span>
            <strong>{state.scoutOpenPositions} positions open now · {state.decisionQuality.scout_completed_trade_samples} trades closed today</strong>
            <span className="auto-trader-learning-detail">{state.decisionQuality.scout_filled_entry_samples} filled entries · {state.decisionQuality.scout_unresolved_trade_samples} unresolved entries · {state.decisionQuality.scout_unpriced_closed_trade_samples} closed without an entry price</span>
            <span className="auto-trader-learning-detail">Today's scout orders: {scoutOrderCounts.filled} filled · {scoutOrderCounts.pending} pending · {scoutOrderCounts.failed} failed</span>
            {scoutOutcomeCohort ? <span className="auto-trader-learning-detail">Closed cohort: {scoutOutcomeCohort.completed_trade_samples} trades · {scoutOutcomeCohort.average_net_return_percent >= 0 ? '+' : ''}{scoutOutcomeCohort.average_net_return_percent.toFixed(3)}% average · {(scoutOutcomeCohort.win_rate * 100).toFixed(0)}% wins · USD {money.format(scoutOutcomeCohort.total_net_pnl)}</span> : <span className="auto-trader-learning-detail">{state.scoutLearningSamples > 0 ? state.scoutLearningSamples + ' realized scout outcomes today · thresholds use full history · ' + (state.scoutLearningWinRate == null ? 'win rate unavailable' : (state.scoutLearningWinRate * 100).toFixed(0) + '% positive') : 'No scout outcomes closed today.'}</span>}
          </div>
          <div className="auto-trader-insight auto-trader-decision-quality">
            <span className="auto-trader-insight-label">Decision quality · today UTC · {state.decisionQuality.completed_trade_samples}/{state.decisionQuality.filled_entry_samples} outcomes closed</span>
            <strong>{state.decisionQuality.decision_samples} decisions · {state.decisionQuality.unresolved_trade_samples} open</strong>
            <span className="auto-trader-learning-detail">Tracks filled Auto Trader entries after the full position closes; skipped signals and open trades are not completed outcomes.</span>
            <button className="auto-trader-cohort-toggle" type="button" aria-expanded={decisionDetailsOpen} onClick={() => setDecisionDetailsOpen((open) => !open)}>
              {decisionDetailsOpen ? 'Hide breakdown' : 'View breakdown · ' + state.decisionQuality.cohorts.length + ' signals'}
            </button>
            {decisionDetailsOpen && <div className="auto-trader-learning-detail auto-trader-cohort-list">
              {state.decisionQuality.cohorts.map((cohort) => <span key={cohort.dimension + ':' + cohort.value}>
                {cohort.dimension === 'entry_time_utc' ? 'Entry time (UTC)' : cohort.dimension === 'market_regime' ? 'Market regime' : cohort.dimension.split('_').join(' ')}: {cohort.value} · {cohort.completed_trade_samples} trades · {cohort.average_net_return_percent >= 0 ? '+' : ''}{cohort.average_net_return_percent.toFixed(2)}% avg · {(cohort.win_rate * 100).toFixed(0)}% wins{cohort.preliminary ? ' · preliminary' : ''}
              </span>)}
              {state.decisionQuality.cohorts.length === 0 && <span>Completed, linked outcomes will appear here as trades close.</span>}
              {state.decisionQuality.unpriced_closed_trade_samples > 0 && <span>{state.decisionQuality.unpriced_closed_trade_samples} closed outcomes lack an entry price and are excluded.</span>}
              {state.decisionQuality.history_truncated && <span>Based on the latest 50,000 audit updates.</span>}
            </div>}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Fully closed exits · today UTC ({state.exitLearningSamples})</span>
            <strong>{state.closedTradeAverageReturnPercent == null ? 'Average return unavailable' : (state.closedTradeAverageReturnPercent >= 0 ? '+' : '') + state.closedTradeAverageReturnPercent.toFixed(3) + '%'}</strong>
            <span className="auto-trader-learning-detail">{state.closedTradeWins} wins · {state.closedTradeLosses} losses · {state.closedTradeFlat} near flat</span>
            <span className="auto-trader-learning-detail">Wins and losses count positions closed to flat; partial stop fills can realize P&amp;L while the position remains open.</span>
          </div>
        </div>
        <div className="auto-trader-pnl-summary" aria-label="Auto Trader profit and loss">
          <span>Realized today: <strong className={state.realizedPnl == null ? undefined : state.realizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.realizedPnl == null ? 'Unavailable (position ledger or FX rate)' : 'USD ' + money.format(state.realizedPnl)}</strong></span>
          <span>Open now: <strong className={state.unrealizedPnl == null ? undefined : state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable (missing mark or FX rate)' : 'USD ' + money.format(state.unrealizedPnl)}</strong></span>
          <span>Today realized + current open: <strong className={state.realizedPnl == null || state.unrealizedPnl == null ? undefined : state.realizedPnl + state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.realizedPnl == null || state.unrealizedPnl == null ? 'Unavailable' : 'USD ' + money.format(state.realizedPnl + state.unrealizedPnl)}</strong></span>
        </div>
        {state.error && <p className="auto-trader-error" role="alert">{state.error}</p>}
        <div className="auto-trader-orders" aria-label="Recent auto trader orders">
          <div className="auto-trader-orders-heading">
            <strong>Today’s trades &amp; protection ({visibleOrderRows.filter((row) => !row.group_header).length}{showOnlyLiveOrders ? ' of ' + orderRows.length : ''}) · newest first within each strategy and symbol · UTC</strong>
            <label className="auto-trader-live-filter">
              <span>Live orders only</span>
              <input type="checkbox" role="switch" checked={showOnlyLiveOrders} onChange={(event) => setShowOnlyLiveOrders(event.target.checked)} />
            </label>
          </div>
          <div className="data-grid-wrap auto-trader-grid-wrap">
            <AgGridReact<AutoTraderOrderRow>
              rowData={visibleOrderRows}
              columnDefs={orderColumnDefs}
              defaultColDef={defaultOrderColDef}
              getRowId={({ data }) => data.group_header ? 'group-' + data.strategy_group : String(data.timestamp) + '-' + data.symbol + '-' + data.side}
              theme={autoTraderGridTheme}
              rowHeight={26}
              getRowClass={({ data }) => data?.group_header ? 'auto-trader-strategy-group' : data?.protection_order ? 'auto-trader-protection-order' : ''}
              headerHeight={28}
              suppressColumnMoveAnimation
              animateRows={false}
            />
          </div>
        </div>
        <small className="auto-trader-footnote">Execution summary uses audited signal entries and IOC exits with a recorded decision mid. Later resting-order fills and protective-stop slippage are not included.</small>
      </div>
    </section>
  );
}
