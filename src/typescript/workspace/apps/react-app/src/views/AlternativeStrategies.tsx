import { useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { AlternativeStrategiesModel, alternativeLabels, alternativeDataNeeds } from '../models/AlternativeStrategiesModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({
  accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31',
  foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad',
});
type Row = ReturnType<AlternativeStrategiesModel['rows']>[number];
type Position = ReturnType<AlternativeStrategiesModel['positions']>[number];
type Event = ReturnType<AlternativeStrategiesModel['events']>[number];
const usd = ({ value }: { value: unknown }) => typeof value === 'number'
  ? value.toLocaleString(undefined, { style: 'currency', currency: 'USD' }) : '—';
const columns: ColDef<Row>[] = [
  { field: 'name', headerName: 'Strategy', minWidth: 220 },
  { field: 'data_status', headerName: 'Data', minWidth: 160 },
  { field: 'insight', headerName: 'Latest insight', minWidth: 350 },
  { field: 'equity_usd', headerName: 'Paper equity', valueFormatter: usd, minWidth: 125 },
  { field: 'realized_pnl_usd', headerName: 'Realized net P&L', valueFormatter: usd, minWidth: 140 },
  { field: 'fees_usd', headerName: 'Costs paid', valueFormatter: usd, minWidth: 120 },
  { field: 'max_drawdown_percent', headerName: 'Max drawdown', minWidth: 130,
    valueFormatter: ({ value }) => typeof value === 'number' ? value.toFixed(2) + '%' : '—' },
  { field: 'open_positions', headerName: 'Open positions', minWidth: 125 },
  { field: 'status', headerName: 'Latest decision', minWidth: 320, flex: 1 },
];
const positionColumns: ColDef<Position>[] = [
  { field: 'strategy', headerName: 'Strategy', minWidth: 200 },
  { field: 'instrument', headerName: 'Instrument / pool / claim', minWidth: 200 },
  { field: 'kind', headerName: 'Position type', minWidth: 130 },
  { field: 'quantity', headerName: 'Quantity / liquidity units', minWidth: 170 },
  { field: 'detail', headerName: 'Position state', minWidth: 350, flex: 1 },
];
const eventColumns: ColDef<Event>[] = [
  { field: 'timestamp_ms', headerName: 'Time', minWidth: 170,
    valueFormatter: ({ value }) => typeof value === 'number' ? new Date(value).toLocaleString() : '' },
  { field: 'strategy', headerName: 'Strategy', minWidth: 200 },
  { field: 'action', headerName: 'Action', minWidth: 120 },
  { field: 'detail', headerName: 'Decision', minWidth: 300, flex: 1 },
  { field: 'equity_usd', headerName: 'Equity at event', valueFormatter: usd, minWidth: 140 },
];
export function AlternativeStrategies() {
  const model = useMemo(() => new AlternativeStrategiesModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const rows = model.rows();
  const positions = model.positions();
  const events = model.events();
  return <details className="alternative-strategies">
    <summary>Five alternative paper strategies</summary>
    <p>Each strategy has an independent $1,000 simulated account, separate from the spot portfolio.
      Entries require AutoTrader to be on and the strategy enabled below. No real orders are sent.</p>
    <p>{state.loading ? 'Connecting…' : state.snapshot?.auto_trader_enabled
      ? 'AutoTrader on · paper entries permitted' : 'AutoTrader paused · marks and settlements continue'}</p>
    {state.error && <p className="auto-trader-error" role="alert">{state.error}</p>}
    {state.snapshot?.load_error && <p className="auto-trader-error" role="alert">{state.snapshot.load_error}</p>}
    <div className="auto-trader-limits alternative-strategy-settings">
      {Object.entries(alternativeLabels).map(([id, label]) => {
        const sleeve = state.snapshot?.book.sleeves[id];
        const config = state.drafts[id] ?? sleeve?.config;
        if (!config) return null;
        return <fieldset className="auto-trader-strategy-options" key={id} disabled={state.saving || !!state.snapshot?.load_error}>
          <legend>{label}</legend>
          <label><input type="checkbox" checked={config.enabled} disabled={state.saving || !!state.snapshot?.load_error || (!config.enabled && !model.feedReady(id))}
            onChange={(event) => model.edit(id, { enabled: event.target.checked })} />Enable paper entries</label>
          <label>MAX TRADE (USD)<input type="number" min="0.01" max="1000" step="any"
            value={config.max_trade_usd} onChange={(event) => model.edit(id, { max_trade_usd: event.target.valueAsNumber })} /></label>
          <label>DAILY REALIZED LOSS CAP (USD)<input type="number" min="0.01" max="1000" step="any"
            value={config.daily_loss_limit_usd} onChange={(event) => model.edit(id, { daily_loss_limit_usd: event.target.valueAsNumber })} /></label>
          {id === 'residual-momentum' && <>
            <label>SPOT FEE (BP / SIDE)<input type="number" min="0" max="100" step="any" value={config.spot_fee_bps}
              onChange={(event) => model.edit(id, { spot_fee_bps: event.target.valueAsNumber })} /></label>
            <label>SHORT BORROW APR (%)<input type="number" min="0" max="500" step="any" value={config.short_borrow_apr * 100}
              onChange={(event) => model.edit(id, { short_borrow_apr: event.target.valueAsNumber / 100 })} /></label>
          </>}
          {['staking-redemption-arbitrage', 'liquidation-keeper'].includes(id) &&
            <label>MIN NET EDGE (BP)<input type="number" min="0" max="1000" step="any" value={config.min_edge_bps}
              onChange={(event) => model.edit(id, { min_edge_bps: event.target.valueAsNumber })} /></label>}
          <small>{alternativeDataNeeds[id]}</small>
          <small>{sleeve?.feed?.summary ?? 'Awaiting native Insights'}</small>
          <button type="button" disabled={state.loading || state.saving || !!state.snapshot?.load_error}
            onClick={() => void model.save(id)}>{state.saving ? 'Saving…' : 'Save strategy'}</button>
        </fieldset>;
      })}
    </div>
    <div className="data-grid-wrap alternative-strategy-grid"><AgGridReact<Row> rowData={rows} columnDefs={columns}
      getRowId={({ data }) => data.id} theme={theme} domLayout="autoHeight"
      defaultColDef={{ sortable: true, resizable: true }} rowHeight={34} headerHeight={30} /></div>
    <strong>Alternative paper positions</strong>
    <div className="data-grid-wrap alternative-strategy-grid"><AgGridReact<Position> rowData={positions}
      columnDefs={positionColumns} getRowId={({ data }) => data.id} theme={theme} domLayout="autoHeight"
      defaultColDef={{ sortable: true, resizable: true }} rowHeight={34} headerHeight={30} /></div>
    <strong>Recent decisions</strong>
    <div className="data-grid-wrap alternative-strategy-events"><AgGridReact<Event> rowData={events}
      columnDefs={eventColumns} getRowId={({ data }) => data.key} theme={theme}
      defaultColDef={{ sortable: true, resizable: true }} rowHeight={34} headerHeight={30} /></div>
    <small>Historical research is not a forecast. Redemption claims are carried at cost until settlement.
      Native feeds pass through Insights. A current, healthy feed is required to enable entries; missing data never creates a fill.</small>
  </details>;
}
