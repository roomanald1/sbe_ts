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
  return (
    <section className="feed-section auto-trader-section" aria-label="Auto Trader">
      <FeedToolbar title="Auto Trader" />
      <div className="auto-trader-content">
        <p className="auto-trader-status">{state.loading ? 'Loading status…' : state.enabled ? 'Enabled · evaluates signals every 5 seconds' : 'Disabled'}</p>
        <div className="auto-trader-settings-anchor">
          <p>Paper · {state.explorationPercent}% exploration · stop {state.stopLossPercent.toFixed(2)}% / target {state.takeProfitPercent.toFixed(2)}%</p>
          <button className="auto-trader-settings-button" type="button" aria-expanded={settingsOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => setSettingsOpen((open) => !open)}>…</button>
          {settingsOpen && <div className="auto-trader-settings-popup" role="dialog" aria-label="Auto Trader settings">
            <div className="auto-trader-settings-heading"><strong>Auto Trader settings</strong><button type="button" aria-label="Close settings" onClick={() => setSettingsOpen(false)}>×</button></div>
            <div className="auto-trader-limits">
              <label>MAX PER ORDER (USD)<input type="number" min="0.01" step="any" value={state.maxOrderNotional} onChange={(event) => model.setLimits(event.target.value, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label title="Maximum USD notional allocated to Auto Trader positions and pending orders that add exposure. Opening positions uses available capacity; closing fills release it. Manual positions are not included.">TOTAL BUDGET (USD)<input type="number" min="0.01" step="any" value={state.dailyNotionalLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, event.target.value, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>DAILY LOSS CAP (USD)<input type="number" min="0.01" step="any" value={state.dailyRealizedLossLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, event.target.value, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>EXPLORATION SIZE (%)<input type="number" min="0" max="100" step="1" value={state.explorationPercent} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, event.target.value, state.maxOpenPositions)} /></label>
              <label title="Maximum number of distinct Auto Trader positions held at once.">MAX OPEN POSITIONS<input type="number" min="1" max="20" step="1" value={state.maxOpenPositions} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, event.target.value)} /></label>
              <button type="button" disabled={state.busy || state.loading} onClick={() => void model.saveLimits()}>Save settings</button>
            </div>
          </div>}
        </div>
        <p>Allocated: USD {money.format(state.usedNotionalToday)} / USD {money.format(Number(state.dailyNotionalLimit) || 0)} · Available: USD {money.format(Math.max(0, (Number(state.dailyNotionalLimit) || 0) - state.usedNotionalToday))}</p>
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
            </span>
          </div>
        </div>
        <div className="auto-trader-pnl-summary" aria-label="Auto Trader profit and loss">
          <span>Realized: <strong className={state.realizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>USD {money.format(state.realizedPnl)}</strong></span>
          <span>Open: <strong className={state.unrealizedPnl == null ? undefined : state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable (missing mark)' : 'USD ' + money.format(state.unrealizedPnl)}</strong></span>
          <span>Total: <strong className={state.unrealizedPnl == null ? undefined : state.realizedPnl + state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable' : 'USD ' + money.format(state.realizedPnl + state.unrealizedPnl)}</strong></span>
        </div>
        <div className="auto-trader-pnl-breakdown" aria-label="Realized P&L breakdown">
          <strong>Net realized P&amp;L by symbol, position direction, and order purpose · fees included</strong>
          <small>Signal means that signal order reduced or closed exposure and booked the P&amp;L.</small>
          {state.pnlBreakdown.length === 0 ? <small>No closed Auto Trader outcomes yet.</small> : (
            <div className="auto-trader-pnl-breakdown-list">
              {state.pnlBreakdown.map((item) => <div key={item.symbol + '-' + item.direction + '-' + item.purpose}>
                <span>{item.symbol} · {item.direction} · {item.purpose} · {item.outcomes} outcome{item.outcomes === 1 ? '' : 's'}</span>
                <strong className={item.realized_pnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>USD {pnlDetailMoney.format(item.realized_pnl)}</strong>
              </div>)}
            </div>
          )}
        </div>
        <div className="auto-trader-actions">
          <button type="button" disabled={state.loading || state.busy} onClick={() => void model.setEnabled(!state.enabled)}>{state.busy ? 'Working…' : state.enabled ? 'Disable' : 'Enable'}</button>
          <button type="button" disabled={!state.enabled || state.busy} onClick={() => void model.runOnce()}>Run once</button>
        </div>
        {state.lastResult && <p role="status">{state.lastResult}</p>}
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
              rowHeight={32}
              headerHeight={38}
              suppressColumnMoveAnimation
              animateRows={false}
              overlayNoRowsTemplate="No auto-trader orders yet."
            />
          </div>
        </div>
        <small>Protective stop fills are reconciled into Auto Trader P&amp;L. Later fills on other resting or scheduled orders may not be tracked.</small>
      </div>
    </section>
  );
}
