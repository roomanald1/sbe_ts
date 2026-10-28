import { useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { DatedCarryModel, type DatedCarryCandidate, type DatedCarryPosition } from '../models/DatedCarryModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31',
  foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad', rowHoverColor: '#141b24',
});
const pct = (value: number) => (value >= 0 ? '+' : '') + value.toFixed(3) + '%';
const money = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 2 });
const when = (value: number) => new Date(value).toLocaleString();
const candidateColumns: ColDef<DatedCarryCandidate>[] = [
  { field: 'spot_symbol', headerName: 'Spot', minWidth: 110 },
  { field: 'future_symbol', headerName: 'Delivery future', minWidth: 170 },
  { field: 'days_to_expiry', headerName: 'Days left', valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(1) : '', minWidth: 100 },
  { field: 'quantity', headerName: 'Coin qty', valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(3) : '', minWidth: 100 },
  { field: 'basis_percent', headerName: 'Executable basis', valueFormatter: ({ value }) => typeof value === 'number' ? pct(value) : '', minWidth: 145 },
  { field: 'net_after_budget_percent', headerName: 'After 0.5% budget', valueFormatter: ({ value }) => typeof value === 'number' ? pct(value) : '', minWidth: 165 },
  { field: 'reason', headerName: 'Gate', minWidth: 210, flex: 1 },
];
const positionColumns: ColDef<DatedCarryPosition>[] = [
  { field: 'future_symbol', headerName: 'Delivery future', minWidth: 170 },
  { field: 'status', headerName: 'Paper status', minWidth: 140 },
  { field: 'quantity', headerName: 'Coin qty', valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(3) : '', minWidth: 100 },
  { field: 'entry_ms', headerName: 'Entered', valueFormatter: ({ value }) => typeof value === 'number' ? when(value) : '', minWidth: 175 },
  { field: 'expiry_ms', headerName: 'Settlement', valueFormatter: ({ value }) => typeof value === 'number' ? when(value) : '', minWidth: 175 },
  { field: 'settlement_source', headerName: 'Settlement source', minWidth: 150 },
  { field: 'net_pnl_usdt', headerName: 'Net P&L (USDT)', valueFormatter: ({ value }) => typeof value === 'number' ? money(value) : 'Open', minWidth: 145 },
];

export function DatedCarry() {
  const model = useMemo(() => new DatedCarryModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const book = state.book;
  const positions = useMemo(() => (book?.positions ?? []).slice().reverse(), [book?.positions]);
  const realized = useMemo(() => positions.reduce((sum, position) => sum + (position.net_pnl_usdt ?? 0), 0), [positions]);
  return <section className="feed-section dated-carry-section" aria-label="Dated futures carry">
    <FeedToolbar title="Dated futures carry" />
    <div className="dated-carry-content">
      <p>Paper strategy - buy BTC/ETH spot and short the matching quarterly future when the executable premium exceeds the 0.5% round-trip cost budget. Spot exits use the 30-minute settlement window.</p>
      <div className="dated-carry-controls">
        <label>MAX SPOT LEG (USDT)<input type="number" min="100" max="10000" step="100" value={state.notional} onChange={(event) => model.setNotional(event.target.value)} /></label>
        <button type="button" disabled={state.loading || state.saving} onClick={() => void model.saveSize()}>Save size</button>
      </div>
      <p className="dated-carry-status">{state.loading ? 'Connecting...' : book?.enabled ? 'Paper entries enabled' : 'Select this strategy in AutoTrader settings and turn AutoTrader on to enable paper entries'} - {book?.checked_at_ms ? 'Signal received ' + when(book.checked_at_ms) : 'Waiting for market scan'} - Realized paper P&L: {money(realized)} USDT</p>
      {state.error && <p className="auto-trader-error" role="alert">{state.error}</p>}
      {book?.last_error && <p className="auto-trader-error" role="alert">{book.last_error}</p>}
      <strong>Current 58-62 day entry window</strong>
      <div className="data-grid-wrap dated-carry-grid"><AgGridReact<DatedCarryCandidate> rowData={book?.candidates ?? []} columnDefs={candidateColumns} getRowId={({ data }) => data.future_symbol} defaultColDef={{ sortable: true, resizable: true }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={30} /></div>
      <strong>Paired paper positions</strong>
      <div className="data-grid-wrap dated-carry-grid"><AgGridReact<DatedCarryPosition> rowData={positions} columnDefs={positionColumns} getRowId={({ data }) => data.future_symbol} defaultColDef={{ sortable: true, resizable: true }} theme={gridTheme} domLayout="autoHeight" rowHeight={34} headerHeight={30} /></div>
      <small>Premium uses public Binance order books. Insights publishes the settlement-window spot TWAP and an official delivery price or labeled index estimate after expiry. P&L includes modeled fees and 5 bp slippage per leg; it excludes collateral financing and liquidation.</small>
    </div>
  </section>;
}
