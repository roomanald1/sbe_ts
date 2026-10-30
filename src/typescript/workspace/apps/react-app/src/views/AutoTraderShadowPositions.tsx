import { memo, useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { autoTraderShadowModel, type AutoTraderShadowPosition } from '../models/AutoTraderShadowModel';
import type { SymbolPriceTick } from '../types/symbol_price';
import type { PriceHistorySample } from '../price_change_data';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
type ShadowGridRow = AutoTraderShadowPosition & { markPrice?: number; openNetReturnPercent?: number };
const percent = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? (value > 0 ? '+' : '') + value.toFixed(3) + '%' : '—';
const durationLabel = (milliseconds: number) => {
  const seconds = Math.max(0, Math.floor(milliseconds / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainderSeconds = seconds % 60;
  if (minutes < 60) return remainderSeconds ? `${minutes}m ${remainderSeconds}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const remainderMinutes = minutes % 60;
  return remainderMinutes ? `${hours}h ${remainderMinutes}m` : `${hours}h`;
};

function AutoTraderShadowPositionsComponent({ ticks = [], histories = new Map() }: { ticks?: SymbolPriceTick[]; histories?: Map<string, PriceHistorySample[]> }) {
  const state = useBehaviorSnapshot(autoTraderShadowModel.state$);
  const [now, setNow] = useState(Date.now);
  useEffect(() => { autoTraderShadowModel.start(); return () => autoTraderShadowModel.stop(); }, []);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 5_000); return () => clearInterval(timer); }, []);
  const marks = useMemo(() => new Map(ticks.filter((tick) => Number.isFinite(tick.price) && now - tick.receivedAt < 60_000).map((tick) => [tick.symbol, tick.price])), [ticks, now]);
  const rows = useMemo<ShadowGridRow[]>(() => state.rows.map((row) => {
    const markPrice = row.status === 'Open' ? marks.get(row.symbol) : undefined;
    const grossReturn = row.entryPrice && markPrice
      ? (row.direction === 'bullish' ? markPrice / row.entryPrice - 1 : row.entryPrice / markPrice - 1) * 100
      : undefined;
    const entrySpreadPercent = row.entryBid && row.entryAsk && row.entryPrice
      ? (row.entryAsk - row.entryBid) / row.entryPrice * 100
      : 0;
    const estimatedOpenReturn = grossReturn === undefined
      ? undefined
      : grossReturn - row.takerFeeRate * 2 * 100 - entrySpreadPercent;
    if (row.status !== 'Outcome unavailable' || !row.entryPrice) {
      return { ...row, markPrice, openNetReturnPercent: estimatedOpenReturn };
    }
    const targetTime = row.openedAt + 300_000;
    const exitSample = histories.get(row.symbol)?.find((sample) => sample.timestamp >= targetTime && sample.timestamp <= targetTime + 60_000);
    if (!exitSample) return { ...row, markPrice, openNetReturnPercent: estimatedOpenReturn };
    const grossAtHorizon = (row.direction === 'bullish'
      ? exitSample.price / row.entryPrice - 1
      : row.entryPrice / exitSample.price - 1) * 100;
    const netReturnPercent = grossAtHorizon - row.takerFeeRate * 2 * 100 - entrySpreadPercent;
    const status: AutoTraderShadowPosition['status'] = netReturnPercent > 1e-12 ? 'Won' : netReturnPercent < -1e-12 ? 'Lost' : 'Flat';
    return { ...row, status, closedAt: exitSample.timestamp, markPrice: exitSample.price, netReturnPercent, returnBasis: 'ticker estimate' };
  }), [state.rows, marks, histories]);
  const openCount = rows.filter((row) => row.status === 'Open').length;
  const wins = rows.filter((row) => row.status === 'Won').length;
  const losses = rows.filter((row) => row.status === 'Lost').length;
  const flats = rows.filter((row) => row.status === 'Flat').length;
  const unavailable = rows.filter((row) => row.status === 'Outcome unavailable').length;
  const columns = useMemo<ColDef<ShadowGridRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', minWidth: 110 },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), minWidth: 165 },
    { field: 'source', headerName: 'Source', minWidth: 105 },
    { headerName: 'Opened at', valueGetter: ({ data }) => data?.openedAt, valueFormatter: ({ value }) => typeof value === 'number' ? new Date(value).toLocaleString() : '—', minWidth: 165 },
    { headerName: 'Duration', headerTooltip: 'Elapsed time for open shadows; holding time until the five-minute outcome for resolved shadows.', valueGetter: ({ data }) => {
      if (!data) return undefined;
      if (data.status === 'Open') return Math.max(0, now - data.openedAt);
      if (typeof data.closedAt === 'number') return Math.max(0, data.closedAt - data.openedAt);
      return Math.min(300_000, Math.max(0, now - data.openedAt));
    }, valueFormatter: ({ value }) => typeof value === 'number' ? durationLabel(value) : '—', minWidth: 95 },
    { headerName: 'Entry', valueGetter: ({ data }) => data?.entryPrice, valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', minWidth: 115 },
    { headerName: 'Mark', valueGetter: ({ data }) => data?.markPrice, valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', minWidth: 115 },
    { headerName: 'Net vs breakeven', headerTooltip: 'Net return after estimated or configured round-trip taker fees and spread. The bar is centred on breakeven and scaled from −2% to +2%; larger returns are clipped visually.', minWidth: 290, cellRenderer: ({ data }: { data?: ShadowGridRow }) => {
      if (!data) return null;
      const value = data.status === 'Open' ? data.openNetReturnPercent : data.netReturnPercent;
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return <span className="auto-trader-pnl-bar-unavailable">Mark unavailable</span>;
      }
      const center = 50;
      const current = Math.max(10, Math.min(90, center + value / 2 * 40));
      const left = Math.min(center, current);
      const width = Math.abs(current - center);
      const tone = value > 1e-12 ? 'is-positive' : value < -1e-12 ? 'is-negative' : 'is-flat';
      const estimated = data.returnBasis === 'ticker estimate' || data.status === 'Open';
      const label = percent(value) + (estimated ? ' est.' : '');
      return <div className="auto-trader-pnl-bar-cell" title={label + ' net of estimated/configured fees and spread'}>
        <div className="auto-trader-pnl-bar-track" aria-label={'Net return ' + label + ', breakeven at centre'}>
          <span className={'auto-trader-pnl-bar-fill ' + tone} style={{ left: left + '%', width: width + '%' }} />
          <span className="auto-trader-pnl-marker is-breakeven" style={{ left: center + '%' }} title="Net breakeven" />
        </div>
        <span className={'auto-trader-pnl-bar-label ' + tone}>{label}</span>
      </div>;
    } },
    { field: 'status', headerName: 'State', minWidth: 145 },
    { field: 'strength', headerName: 'Signal strength', valueFormatter: ({ value }) => typeof value === 'number' ? `${(value * 100).toFixed(0)}%` : '—', minWidth: 115 },
  ], [now]);

  return <section className="feed-section signal-info-panel" aria-label="AutoTrader shadow positions">
    <FeedToolbar title="AutoTrader shadow positions" />
    <p className="signal-info-note">No orders are placed for these candidates. Concurrency is controlled by AutoTrader settings; repeated refreshes are coalesced into one five-minute shadow per symbol, strategy, and direction. Open marks are indicative and net out estimated round-trip taker fees and observed entry spread. Completed results use the five-minute bid/ask evaluation when available, otherwise a labelled ticker estimate. Shadow outcomes are separate from real position winners and losers.</p>
    <div className="auto-trader-shadow-summary" aria-live="polite">
      <span>{openCount} open</span><span>{wins} wins</span><span>{losses} losses</span><span>{flats} flat</span>{unavailable > 0 && <span>{unavailable} outcome unavailable</span>}
    </div>
    {state.error && <div className="positions-error">{state.error}</div>}
    <div className="data-grid-wrap signal-info-grid"><AgGridReact<ShadowGridRow> rowData={rows} columnDefs={columns} getRowId={({ data }) => data.decisionId} defaultColDef={{ sortable: true, resizable: true, flex: 1 }} theme={theme} domLayout="autoHeight" rowHeight={34} headerHeight={30} /></div>
    {state.loading && <p className="positions-empty">Loading shadow outcomes…</p>}
    {!state.loading && !state.error && rows.length === 0 && <p className="positions-empty">No scout or exploratory shadow candidates recorded in the last 15 minutes.</p>}
  </section>;
}

export const AutoTraderShadowPositions = memo(AutoTraderShadowPositionsComponent);
