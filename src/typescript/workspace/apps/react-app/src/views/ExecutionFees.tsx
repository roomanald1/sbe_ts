import { memo, useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { ExecutionFeesModel, type ExecutionFeeRow } from '../models/ExecutionFeesModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const model = new ExecutionFeesModel();
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
const number = (value: unknown, digits = 4) => typeof value === 'number' && Number.isFinite(value)
  ? value.toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: digits })
  : '—';
const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

export const ExecutionFees = memo(function ExecutionFees() {
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, []);
  const columns = useMemo<ColDef<ExecutionFeeRow>[]>(() => [
    { field: 'source', headerName: 'Source', minWidth: 100 },
    { field: 'liquidity', headerName: 'Liquidity', minWidth: 100, valueFormatter: ({ value }) => value === 'unknown' ? 'Unknown · legacy' : typeof value === 'string' ? value[0].toUpperCase() + value.slice(1) : 'Unknown' },
    { field: 'fee_currency', headerName: 'Fee currency', width: 110 },
    { field: 'fills', headerName: 'Fills', width: 90 },
    { field: 'traded_notional', headerName: 'Traded notional', valueFormatter: ({ value, data }) => number(value, 2) + ' ' + (data?.fee_currency ?? ''), minWidth: 145 },
    { field: 'fees_paid', headerName: 'Fees paid', valueFormatter: ({ value, data }) => number(value, 6) + ' ' + (data?.fee_currency ?? ''), minWidth: 130 },
    { field: 'effective_fee_bps', headerName: 'Effective fee', valueFormatter: ({ value }) => typeof value === 'number' && Number.isFinite(value) ? value.toFixed(2) + ' bps' : '—', minWidth: 120 },
    { field: 'first_fill_ms', headerName: 'First fill', valueFormatter: ({ value }) => typeof value === 'number' ? dateTime.format(value) : '—', minWidth: 150 },
    { field: 'last_fill_ms', headerName: 'Latest fill', valueFormatter: ({ value }) => typeof value === 'number' ? dateTime.format(value) : '—', minWidth: 150 },
  ], []);

  return <section className="feed-section execution-fees-widget" aria-label="Execution fee summary">
    <div className="feed-toolbar"><strong>Execution fees</strong><span>{number(state.fills, 0)} ledger fills · manual and AutoTrader</span><button type="button" onClick={() => void model.refresh()} disabled={state.loading}>Refresh</button></div>
    <p className="muted">All fills recorded in the position ledger, grouped by source, maker/taker classification, and fee currency. Historical fills without liquidity metadata remain included as Unknown. Fees in different currencies are kept separate.</p>
    {state.error && <div className="book-popup-message" role="alert">{state.error}</div>}
    {state.loading && state.rows.length === 0
      ? <div className="book-popup-message">Loading ledger fee summary…</div>
      : state.rows.length === 0
        ? <div className="book-popup-message">No fills are recorded in the position ledger.</div>
        : <div className="data-grid-wrap signal-info-grid"><AgGridReact<ExecutionFeeRow> rowData={state.rows} columnDefs={columns} getRowId={({ data }) => data.source + ':' + data.liquidity + ':' + data.fee_currency} defaultColDef={{ sortable: true, resizable: true, flex: 0, minWidth: 80 }} theme={theme} domLayout="autoHeight" rowHeight={30} headerHeight={30} />
          </div>}
    <small className="muted">{state.first_fill_ms ? 'History: ' + dateTime.format(state.first_fill_ms) + ' – ' + dateTime.format(state.last_fill_ms ?? state.first_fill_ms) : ''}{state.updatedAt ? ' · Updated ' + dateTime.format(state.updatedAt) : ''}</small>
  </section>;
});
