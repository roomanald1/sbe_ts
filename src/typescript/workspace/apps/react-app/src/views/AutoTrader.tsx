import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  CellStyleModule,
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
  type ITooltipParams,
} from 'ag-grid-community';
import { formatForSymbol } from '../instrument_precision';
import { AutoTraderModel } from '../models/AutoTraderModel';
import type { AutoTraderOrder } from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { getOrderTypeDescription } from '../order_type_descriptions';

const money = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const pnlDetailMoney = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 4 });
const timestamp = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
ModuleRegistry.registerModules([CellStyleModule, ClientSideRowModelModule]);

const autoTraderGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

const orderColumnDefs: ColDef<AutoTraderOrder>[] = [
  { field: 'timestamp', headerName: 'Time / Trigger', minWidth: 102, valueGetter: ({ data }) => data?.triggered_at ?? data?.timestamp, valueFormatter: ({ value }) => value ? timestamp.format(value) : '' },
  { field: 'symbol', headerName: 'Symbol', minWidth: 78 },
  { field: 'side', headerName: 'Side', minWidth: 56, valueFormatter: ({ value }) => String(value ?? '').toUpperCase() },
  { field: 'order_type', headerName: 'Type', minWidth: 70, tooltipValueGetter: (params: ITooltipParams<AutoTraderOrder>) => getOrderTypeDescription(params.value), cellRenderer: (params: ICellRendererParams<AutoTraderOrder>) => <span title={getOrderTypeDescription(params.value)}>{String(params.value ?? '')}</span> },
  { field: 'purpose', headerName: 'Purpose', minWidth: 76, valueFormatter: ({ value }) => value === 'portfolio-hedge' ? 'HEDGE' : String(value ?? '').toUpperCase() },
  { field: 'status', headerName: 'Status', minWidth: 70 },
  { field: 'filled_quantity', headerName: 'Filled / Qty', minWidth: 96, valueFormatter: ({ data }) => data ? formatForSymbol(data.filled_quantity, data.symbol, 'quantity') + '/' + formatForSymbol(data.quantity, data.symbol, 'quantity') : '' },
  { field: 'average_price', headerName: 'Avg Price', minWidth: 78, valueFormatter: ({ value }) => value == null ? '' : money.format(value) },
  { field: 'realized_pnl', headerName: 'Realized USD', minWidth: 92, cellClass: ({ value }) => value >= 0 ? 'pnl-positive' : 'pnl-negative', valueFormatter: ({ value }) => value == null ? '' : money.format(value) },
];

const defaultOrderColDef: ColDef<AutoTraderOrder> = {
  sortable: true,
  resizable: true,
  flex: 1,
  minWidth: 68,
};
export function AutoTrader() {
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [breakdownOpen, setBreakdownOpen] = useState(false);
  const [decisionDetailsOpen, setDecisionDetailsOpen] = useState(false);
  const currencyBreakdown = useMemo(() => {
    const grouped = new Map<string, { pnl: number; outcomes: number; symbols: Set<string> }>();
    state.pnlBreakdown.forEach((item) => {
      const currency = item.symbol.split('-')[1] || 'Other';
      const current = grouped.get(currency) ?? { pnl: 0, outcomes: 0, symbols: new Set<string>() };
      current.pnl += item.realized_pnl;
      current.outcomes += item.outcomes;
      current.symbols.add(item.symbol);
      grouped.set(currency, current);
    });
    return Array.from(grouped, ([currency, value]) => ({ currency, ...value })).sort((a, b) => Math.abs(b.pnl) - Math.abs(a.pnl));
  }, [state.pnlBreakdown]);
  return (
    <section className={"feed-section auto-trader-section" + (breakdownOpen || settingsOpen ? " has-open-popup" : "")} aria-label="Auto Trader">
      <FeedToolbar title="Auto Trader" />
      <div className="auto-trader-content">
        <div className={"auto-trader-live-banner " + (state.enabled ? "is-live" : "") + (state.loading ? " is-loading" : "")}>
          <span className="auto-trader-live-dot" aria-hidden="true" />
          <strong>{state.loading ? 'Connecting' : state.enabled ? 'LIVE' : 'PAUSED'}</strong>
          <span>{state.loading ? 'Checking status' : state.enabled ? 'Scanning markets · 5s cycle' : 'Auto Trader is off'}</span>
          <div className="auto-trader-live-actions">
            <button type="button" disabled={state.loading || state.busy} onClick={() => void model.setEnabled(!state.enabled)}>{state.busy ? 'Working…' : state.enabled ? 'Pause' : 'Go live'}</button>
          </div>
        </div>
        <div className="auto-trader-settings-anchor">
          <p>Paper · {state.explorationPercent}% explore · SL {state.stopLossPercent.toFixed(2)}% / TP {state.takeProfitPercent.toFixed(2)}%</p>
          <button className="auto-trader-settings-button" type="button" aria-expanded={settingsOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => setSettingsOpen((open) => !open)}>…</button>
          {settingsOpen && <div className="auto-trader-settings-popup" role="dialog" aria-label="Auto Trader settings">
            <div className="auto-trader-settings-heading"><strong>Auto Trader settings</strong><button type="button" aria-label="Close settings" onClick={() => setSettingsOpen(false)}>×</button></div>
            <div className="auto-trader-limits">
              <label>MAX PER ORDER (USD)<input type="number" min="0.01" step="any" value={state.maxOrderNotional} onChange={(event) => model.setLimits(event.target.value, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label title="Maximum USD notional allocated to Auto Trader positions and pending orders that add exposure. Opening positions uses available capacity; closing fills release it. Manual positions are not included.">TOTAL BUDGET (USD)<input type="number" min="0.01" step="any" value={state.dailyNotionalLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, event.target.value, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>DAILY LOSS CAP (USD)<input type="number" min="0.01" step="any" value={state.dailyRealizedLossLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, event.target.value, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>EXPLORATION SIZE (%)<input type="number" min="0" max="100" step="1" value={state.explorationPercent} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, event.target.value, state.maxOpenPositions)} /></label>
              <label title="Maximum number of distinct Auto Trader positions held at once.">MAX OPEN POSITIONS<input type="number" min="1" max="20" step="1" value={state.maxOpenPositions} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, event.target.value)} /></label>
              <label>VWAP / TWAP DURATION (MIN)<input type="number" min="5" max="390" step="1" value={state.vwapDurationMinutes} onChange={(event) => model.setExecutionParameters({ vwapDurationMinutes: event.target.value })} /></label>
              <label>VWAP PARTICIPATION (%)<input type="number" min="1" max="50" step="1" value={state.vwapParticipationPercent} onChange={(event) => model.setExecutionParameters({ vwapParticipationPercent: event.target.value })} /></label>
              <label>LIMIT OFFSET (BPS)<input type="number" min="-10" max="10" step="0.5" value={state.limitOffsetBps} onChange={(event) => model.setExecutionParameters({ limitOffsetBps: event.target.value })} /></label>
              <label>TRIGGER OFFSET (BPS)<input type="number" min="5" max="100" step="1" value={state.triggerOffsetBps} onChange={(event) => model.setExecutionParameters({ triggerOffsetBps: event.target.value })} /></label>
              <label>TRAILING DISTANCE (%)<input type="number" min="0.05" max="1" step="0.05" value={state.trailingDistancePercent} onChange={(event) => model.setExecutionParameters({ trailingDistancePercent: event.target.value })} /></label>
              <button type="button" disabled={state.busy || state.loading} onClick={() => void model.saveLimits()}>Save settings</button>
            </div>
          </div>}
        </div>
        <p className="auto-trader-allocation">Budget used <strong>USD {money.format(state.usedNotionalToday)}</strong> / {money.format(Number(state.dailyNotionalLimit) || 0)} · Available <strong>USD {money.format(Math.max(0, (Number(state.dailyNotionalLimit) || 0) - state.usedNotionalToday))}</strong></p>
        <div className="auto-trader-insights">
          <div className="auto-trader-insight auto-trader-decision" role="status">
            <span className="auto-trader-insight-label">Last decision</span>
            <span className="auto-trader-decision-text">{state.lastDecision || 'Waiting for first cycle.'}</span>
            {state.lastDecisionAt > 0 && <time dateTime={new Date(state.lastDecisionAt).toISOString()}>{timestamp.format(state.lastDecisionAt)}</time>}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Learning</span>
            <strong>{state.learningSamples} closed outcomes</strong>
            <span className="auto-trader-learning-detail">
              {state.learningWinRate == null ? 'Win rate unavailable' : (state.learningWinRate * 100).toFixed(0) + '% profitable'}
              {state.preferredOrderType ? ' · Strongest: ' + state.preferredOrderType : ''}
              {' · Parameter profiles P0/P1/P2: ' + state.parameterProfileSamples.join('/')}
            </span>
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Execution · {state.executionQuality.measured_fill_samples}/{state.executionQuality.filled_order_samples} fills measured</span>
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
          <div className="auto-trader-insight auto-trader-decision-quality">
            <span className="auto-trader-insight-label">Decision quality · {state.decisionQuality.completed_trade_samples}/{state.decisionQuality.filled_entry_samples} outcomes closed</span>
            <strong>{state.decisionQuality.decision_samples} decisions · {state.decisionQuality.unresolved_trade_samples} open</strong>
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
            <span className="auto-trader-insight-label">Fully closed exits · rolling {state.exitLearningSamples}/200</span>
            <strong>{state.closedTradeAverageReturnPercent == null ? 'Average return unavailable' : (state.closedTradeAverageReturnPercent >= 0 ? '+' : '') + state.closedTradeAverageReturnPercent.toFixed(3) + '%'}</strong>
            <span className="auto-trader-learning-detail">{state.closedTradeWins} wins · {state.closedTradeLosses} losses · {state.closedTradeFlat} near flat</span>
          </div>
        </div>
        <div className="auto-trader-pnl-summary" aria-label="Auto Trader profit and loss">
          <span>Realized: <strong className={state.realizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>USD {money.format(state.realizedPnl)}</strong></span>
          <span>Open: <strong className={state.unrealizedPnl == null ? undefined : state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable (missing mark)' : 'USD ' + money.format(state.unrealizedPnl)}</strong></span>
          <span>Total: <strong className={state.unrealizedPnl == null ? undefined : state.realizedPnl + state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable' : 'USD ' + money.format(state.realizedPnl + state.unrealizedPnl)}</strong></span>
        </div>
        <div className="auto-trader-breakdown-anchor">
          <button className="auto-trader-breakdown-button" type="button" aria-expanded={breakdownOpen} aria-haspopup="dialog" onClick={() => setBreakdownOpen((open) => !open)}>Signal P&amp;L by currency <span>{currencyBreakdown.length}</span></button>
          {breakdownOpen && <div className="auto-trader-settings-popup auto-trader-breakdown-popup" role="dialog" aria-label="Signal P&L by currency">
            <div className="auto-trader-settings-heading"><strong>Signal P&amp;L by currency</strong><button type="button" aria-label="Close breakdown" onClick={() => setBreakdownOpen(false)}>×</button></div>
            <small>Net realized by quote currency · fees included.</small>
          {currencyBreakdown.length === 0 ? <small>No closed outcomes yet.</small> : (
            <div className="auto-trader-pnl-breakdown-list">
              {currencyBreakdown.map((item) => <div key={item.currency}>
                <span>{item.currency} · {item.symbols.size} symbols · {item.outcomes} outcomes</span>
                <strong className={item.pnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>USD {pnlDetailMoney.format(item.pnl)}</strong>
              </div>)}
            </div>
          )}
          </div>}
        </div>
        {state.error && <p className="auto-trader-error" role="alert">{state.error}</p>}
        <div className="auto-trader-orders" aria-label="Recent auto trader orders">
          <strong>Recent orders</strong>
          <div className="data-grid-wrap auto-trader-grid-wrap">
            <AgGridReact<AutoTraderOrder>
              rowData={state.orders}
              columnDefs={orderColumnDefs}
              defaultColDef={defaultOrderColDef}
              getRowId={({ data }) => String(data.timestamp) + '-' + data.symbol + '-' + data.side}
              theme={autoTraderGridTheme}
              rowHeight={26}
              headerHeight={28}
              suppressColumnMoveAnimation
              animateRows={false}
              overlayNoRowsTemplate="No auto-trader orders yet."
            />
          </div>
        </div>
        <small className="auto-trader-footnote">Execution summary uses audited signal entries and IOC exits with a recorded decision mid. Later resting-order fills and protective-stop slippage are not included.</small>
      </div>
    </section>
  );
}
