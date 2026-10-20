import { formatForSymbol } from '../instrument_precision';
import { memo, useEffect, useMemo, useState } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  CellStyleModule,
  ClientSideRowModelModule,
  ModuleRegistry,
  NumberFilterModule,
  TextFilterModule,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import { CloseInsightsModel, type CloseInsight } from '../models/CloseInsightsModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import type { PriceHistorySample } from '../price_change_data';
import { PriceHistoryPopup } from './PriceHistoryPopup';

ModuleRegistry.registerModules([ClientSideRowModelModule, CellStyleModule, NumberFilterModule, TextFilterModule]);
const money = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
type CloseInsightRow = CloseInsight & { currency: string };
type CloseActionParams = ICellRendererParams<CloseInsightRow> & { model: CloseInsightsModel; closingSymbol?: string };
type ProfitBarrierParams = ICellRendererParams<CloseInsightRow> & { model: CloseInsightsModel };
function CloseActionCell({ data, model, closingSymbol }: CloseActionParams) {
  if (!data) return null;
  const closing = closingSymbol === data.position.symbol;
  const noLiquidity = data.closeDepthQuantity <= 0;
  return <button className="pnl-close-button" type="button" disabled={closingSymbol !== undefined || noLiquidity} title={noLiquidity ? 'No executable quantity is available in the live book' : 'Close available quantity with an IOC order'} onClick={() => model.requestClose(data)}>{closing ? 'Closing...' : 'Close IOC'}</button>;
}

type CloseSymbolParams = ICellRendererParams<CloseInsightRow> & { onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void };
function ProfitBarrierCell({ data, model }: ProfitBarrierParams) {
  const state = useBehaviorSnapshot(model.state$);
  if (!data) return null;
  const armedBarrier = state.barriers.find((barrier) => barrier.symbol === data.position.symbol && barrier.status === 'armed');
  const pending = state.pendingBarrierSymbols.includes(data.position.symbol);
  const coversPosition = armedBarrier !== undefined && armedBarrier.remaining_quantity + 1e-12 >= Math.abs(data.position.quantity);
  return armedBarrier
    ? <button className={"pnl-close-button pnl-barrier-cancel " + (coversPosition ? "pnl-barrier-covered" : "pnl-barrier-partial")} type="button" disabled={pending} title={coversPosition ? 'Cancel the armed barrier; it covers the full position' : 'Warning: this barrier covers only part of the position. Cancel barrier'} onClick={() => void model.cancelBarrier(armedBarrier)}>{pending ? 'Cancelling...' : 'Cancel barrier'}</button>
    : <button className="pnl-close-button" type="button" disabled={pending} title="Create a server managed barrier at the selected net profit target" onClick={() => void model.createProfitBarrier(data)}>{pending ? 'Setting...' : 'Profit barrier'}</button>;
}

function CloseSymbolCell({ data, onTradeSymbol }: CloseSymbolParams) {
  if (!data) return null;
  return <button className="symbol-tag symbol-book-button" type="button" title={'Open ' + data.closeSide + ' order for ' + data.position.symbol} aria-label={'Open ' + data.closeSide + ' order for ' + data.position.symbol} onClick={(event) => { event.stopPropagation(); onTradeSymbol(data.position.symbol, data.closeSide); }}>{data.position.symbol}</button>;
}

function CloseTrendCell({ data, histories, onShowTrend }: ICellRendererParams<CloseInsightRow> & { histories: Map<string, PriceHistorySample[]>; onShowTrend: (symbol: string) => void }) {
  const symbol = data?.position.symbol;
  if (!symbol) return null;
  const prices = (histories.get(symbol) ?? []).slice(-60).map((sample) => sample.price);
  if (prices.length < 2) return <span className="sparkline-empty">N/A</span>;
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const range = max - min || 1;
  const points = prices.map((price, index) => `${(index / (prices.length - 1)) * 64},${18 - ((price - min) / range) * 16}`).join(' ');
  const direction = prices.at(-1)! >= prices[0] ? 'up' : 'down';
  return <button className="trend-cell-button" type="button" aria-label={"View " + symbol + " price chart"} title="View price chart" onClick={(event) => { event.stopPropagation(); onShowTrend(symbol); }}><svg className={"pair-sparkline sparkline-" + direction} viewBox="0 0 64 20" aria-hidden="true"><polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" /></svg></button>;
}

function CloseInsightsComponent({ histories = new Map<string, PriceHistorySample[]>(), paused = false, onTradeSymbol }: { histories?: Map<string, PriceHistorySample[]>; paused?: boolean; onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void }) {
  const model = useMemo(() => new CloseInsightsModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const [trendSymbol, setTrendSymbol] = useState<string>();
  const rowData = useMemo<CloseInsightRow[]>(
    () => state.insights.map((insight) => ({ ...insight, currency: insight.position.symbol.split('-')[1] ?? '' })),
    [state.insights],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<CloseInsightRow>[]>(
    () => [
      { field: 'position.symbol', headerName: 'Instrument', minWidth: 95, cellRenderer: CloseSymbolCell, cellRendererParams: { onTradeSymbol } },
      { field: 'closeSide', headerName: 'Side', minWidth: 65, valueFormatter: ({ value }) => String(value ?? '').toUpperCase() },
      { field: 'closeQuantity', headerName: 'Qty', minWidth: 85, valueFormatter: ({ value, data }) => value == null ? '' : formatForSymbol(value, data?.position.symbol ?? '', 'quantity') },
      { field: 'position.symbol', headerName: 'TREND', colId: 'trend', minWidth: 76, maxWidth: 88, sortable: false, filter: false, cellRenderer: CloseTrendCell, cellRendererParams: { histories, onShowTrend: setTrendSymbol } },
      { field: 'closePrice', headerName: 'Est. close', minWidth: 105, valueFormatter: ({ value, data }) => value == null ? 'Unavailable' : formatForSymbol(value, data?.position.symbol ?? '', 'price') + ' ' + (data?.currency ?? '') },
      { field: 'breakevenPrice', headerName: 'Breakeven price', minWidth: 115, valueFormatter: ({ value, data }) => value == null ? 'N/A' : formatForSymbol(value, data?.position.symbol ?? '', 'price') + ' ' + (data?.currency ?? '') },
      { field: 'currentPrice', headerName: 'Current price', minWidth: 110, valueFormatter: ({ value, data }) => value == null ? 'N/A' : formatForSymbol(value, data?.position.symbol ?? '', 'price') + ' ' + (data?.currency ?? '') },
      { field: 'closeFeeUsd', headerName: 'Est. fee (USD)', minWidth: 105, valueFormatter: ({ value }) => value == null ? 'N/A' : money.format(value) + ' USD' },
      { field: 'closePnlUsd', headerName: 'Net P&L (USD)', minWidth: 115, cellClass: ({ value }) => value == null || value === 0 ? undefined : value > 0 ? 'pnl-positive' : 'pnl-negative', valueFormatter: ({ value }) => value == null ? 'Unavailable' : money.format(value) + ' USD' },
      { field: 'profitableCloseProbability5m', headerName: 'Profitable in 5m', minWidth: 110, valueFormatter: ({ value, data }) => value == null ? 'Sampling ' + (data?.probabilitySamples ?? 0) + '/31' : (value * 100).toFixed(1) + '%' },
      { field: 'depthCoversPosition', headerName: 'Book covers', minWidth: 90, valueFormatter: ({ value, data }) => value ? 'Yes' : (data?.closeDepthQuantity ?? 0) > 0 ? 'Partial' : 'No' },
      { colId: 'profit-barrier', headerName: 'Profit target', cellRenderer: ProfitBarrierCell, cellRendererParams: { model }, sortable: false, filter: false, flex: 0, width: 120, minWidth: 120, maxWidth: 120 },
      { colId: 'actions', headerName: 'Action', cellRenderer: CloseActionCell, cellRendererParams: { model, closingSymbol: state.closingSymbol }, sortable: false, filter: false, flex: 0, width: 100, minWidth: 100, maxWidth: 100 },
    ],
    [model, state.closingSymbol, histories, onTradeSymbol],
  );
  const confirmation = state.confirmingInsight;

  return (
    <section className="feed-section close-insights-panel" aria-label="Close-out insights">
      <FeedToolbar title="Close-out insights" />
      <label className="close-profit-target">Net profit target (USD) <input type="number" min="0.01" step="0.01" value={state.barrierTargetUsd} onChange={(event) => model.setBarrierTargetUsd(event.target.value)} /></label>
      <p className="barrier-order-note">The server estimates a fee adjusted trigger. Reaching it submits an IOC; execution price, USD conversion, and available liquidity can change the realized profit.</p>
      {state.error && <div className="positions-error" role="alert">{state.error}</div>}
      <div className="data-grid-wrap positions-grid-wrap close-insights-grid">
        <AgGridReact<CloseInsightRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={{ sortable: true, filter: true, resizable: true, floatingFilter: false, flex: 1, minWidth: 75 }}
          getRowId={({ data }) => data.position.symbol}
          theme={gridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={28}
          suppressColumnMoveAnimation
        />
        {!state.loading && rowData.length === 0 && <p className="positions-empty">No open positions.</p>}
        {state.loading && rowData.length === 0 && <p className="positions-empty">Loading...</p>}
      </div>
      {trendSymbol && <PriceHistoryPopup symbol={trendSymbol} history={histories.get(trendSymbol) ?? []} onClose={() => setTrendSymbol(undefined)} />}
      {confirmation && (
        <div className="trade-popup-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) model.cancelClose(); }}>
          <section className="trade-popup close-confirm-modal" role="dialog" aria-modal="true" aria-labelledby="close-confirm-title">
            <header>
              <div><span>CONFIRM IOC CLOSE</span><h2 id="close-confirm-title">{confirmation.position.symbol}</h2></div>
              <button type="button" aria-label="Cancel close" onClick={() => model.cancelClose()}>x</button>
            </header>
            <div className="close-confirm-summary">
              <span>Side / quantity</span><strong>{confirmation.closeSide.toUpperCase()} {formatForSymbol(confirmation.closeQuantity, confirmation.position.symbol, 'quantity')}</strong>
              <span>Book quantity available</span><strong>{formatForSymbol(confirmation.closeDepthQuantity, confirmation.position.symbol, 'quantity')}</strong>
              <span>Estimated close</span><strong>{confirmation.closePrice === undefined ? 'Unavailable' : formatForSymbol(confirmation.closePrice, confirmation.position.symbol, 'price') + ' ' + (confirmation.position.symbol.split('-')[1] ?? '')}</strong>
              <span>Breakeven price</span><strong>{confirmation.breakevenPrice === undefined ? 'Unavailable' : formatForSymbol(confirmation.breakevenPrice, confirmation.position.symbol, 'price') + ' ' + (confirmation.position.symbol.split('-')[1] ?? '')}</strong>
              <span>Current price</span><strong>{confirmation.currentPrice === undefined ? 'Unavailable' : formatForSymbol(confirmation.currentPrice, confirmation.position.symbol, 'price') + ' ' + (confirmation.position.symbol.split('-')[1] ?? '')}</strong>
              <span>Estimated fee</span><strong>{confirmation.closeFeeUsd === undefined ? 'Unavailable' : money.format(confirmation.closeFeeUsd) + ' USD'}</strong>
              <span>Net P&L</span><strong className={(confirmation.closePnl ?? 0) >= 0 ? 'pnl-positive' : 'pnl-negative'}>{confirmation.closePnlUsd === undefined ? 'Unavailable' : money.format(confirmation.closePnlUsd) + ' USD'}</strong>
            </div>
            <p>The IOC may fill only the quantity available in the live book. Any unfilled quantity is canceled.</p>
            <div className="close-confirm-actions">
              <button type="button" className="close-confirm-cancel" onClick={() => model.cancelClose()}>Cancel</button>
              <button type="button" className="paper-trade-submit is-sell" onClick={() => void model.confirmClose()}>Confirm close</button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}

export const CloseInsights = memo(CloseInsightsComponent);
