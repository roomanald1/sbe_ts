import { memo, useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, PaginationModule, themeQuartz, type ColDef, type ICellRendererParams } from 'ag-grid-community';
import { autoTraderScoutModel, type AutoTraderScoutPosition, type ScoutLearningSummary } from '../models/AutoTraderScoutModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import { strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule, PaginationModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
const percent = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? (value > 0 ? '+' : '') + value.toFixed(3) + '%' : '—';

function AutoTraderScoutsComponent() {
  const state = useBehaviorSnapshot(autoTraderScoutModel.state$);
  useEffect(() => { autoTraderScoutModel.start(); return () => autoTraderScoutModel.stop(); }, []);

  const emptyWhileLoading = state.loading && state.rows.length === 0;
  const emptyAfterFailure = Boolean(state.error) && state.rows.length === 0;
  const displayCount = (count: number) => emptyWhileLoading ? '…' : emptyAfterFailure ? '—' : count.toLocaleString();
  const learningSymbol = state.learningSymbol ?? 'all';
  const learningRows = state.learningRows;

  const columns = useMemo<ColDef<AutoTraderScoutPosition>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', width: 85, minWidth: 75 },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), width: 130, minWidth: 110 },
    { field: 'status', headerName: 'State', width: 85, minWidth: 75, cellStyle: ({ value }) => value === 'Won' ? { color: '#45c989', fontWeight: 700 } : value === 'Lost' ? { color: '#f06e72', fontWeight: 700 } : undefined },
    { field: 'exitScenario', headerName: 'Exit scenario', valueFormatter: ({ value }) => value === 'tight' ? 'Tighter exits' : 'Strategy exits', width: 105, minWidth: 90 },
    { field: 'scoutLevelPercent', headerName: 'Scout level', valueFormatter: ({ value }) => typeof value === 'number' ? String(value.toFixed(2)) + '% below breakout' : '—', width: 125, minWidth: 105 },
    { field: 'openedAt', headerName: 'Opened at', valueFormatter: ({ value }) => typeof value === 'number' ? new Date(value).toLocaleString() : '—', width: 140, minWidth: 120 },
    { field: 'durationLabel', headerName: 'Duration', headerTooltip: 'Elapsed time for open scout positions.', width: 80, minWidth: 80, sort: 'asc', sortIndex: 0 },
    { field: 'entryPrice', headerName: 'Entry', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', width: 95, minWidth: 80 },
    { field: 'markPrice', headerName: 'Mark', valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', width: 105, minWidth: 90 },
    { field: 'netReturnLabel', headerName: 'Net vs breakeven', width: 160, minWidth: 135, headerTooltip: 'Backend estimate of net return after configured fees and spread.', cellRenderer: ({ data }: ICellRendererParams<AutoTraderScoutPosition>) => {
      if (!data?.netReturnLabel || typeof data.netVsBreakevenLeft !== 'number' || typeof data.netVsBreakevenWidth !== 'number') return <span className="auto-trader-pnl-bar-unavailable">Mark unavailable</span>;
      const tone = data.netVsBreakevenTone ?? 'is-flat';
      return <div className="auto-trader-pnl-bar-cell" title={data.netReturnLabel + ' net of estimated/configured fees and spread'}>
        <div className="auto-trader-pnl-bar-track" aria-label={'Net return ' + data.netReturnLabel + ', breakeven at centre'}>
          <span className={'auto-trader-pnl-bar-fill ' + tone} style={{ left: data.netVsBreakevenLeft + '%', width: data.netVsBreakevenWidth + '%' }} />
          <span className="auto-trader-pnl-marker is-breakeven" style={{ left: '50%' }} title="Net breakeven" />
        </div>
        <span className={'auto-trader-pnl-bar-label ' + tone}>{data.netReturnLabel}</span>
      </div>;
    } },
    { field: 'displayNetReturnPercent', headerName: 'Net', headerTooltip: 'Backend-computed return after fees and spread.', valueFormatter: ({ value }) => percent(value), cellClass: ({ data }) => data?.netVsBreakevenTone, minWidth: 75, maxWidth: 90 },
    { field: 'comparisonRole', headerName: 'Probe', headerTooltip: 'Baseline and variant are evaluated on the same signal. Parameter promotion uses the paired delta.', valueFormatter: ({ value }) => value === 'baseline' ? 'Baseline' : value === 'variant' ? 'Variant' : '—', minWidth: 78, maxWidth: 88 },
    { field: 'pairedDeltaPercent', headerName: 'Δ vs base', headerTooltip: 'Backend-computed variant return minus its same-signal baseline.', valueFormatter: ({ value }) => percent(value), cellClass: ({ data }) => data?.pairedDeltaTone, minWidth: 90, maxWidth: 105 },
    { field: 'strength', headerName: 'Signal strength', valueFormatter: ({ value }) => typeof value === 'number' ? `${(value * 100).toFixed(0)}%` : '—', minWidth: 115 },
  ], []);

  const learningColumns = useMemo<ColDef<ScoutLearningSummary>[]>(() => [
    { field: 'kind', headerName: 'Grouping', width: 155, sortable: false },
    { field: 'symbol', headerName: 'Symbol', width: 120, sortable: false },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), width: 190, sortable: false },
    { field: 'parameter', headerName: 'Probed parameter', minWidth: 190, flex: 1, sortable: false, valueFormatter: ({ value }) => value ?? '—' },
    { field: 'value', headerName: 'Probed value', valueFormatter: ({ value }) => typeof value === 'number' ? String(Number(value.toPrecision(6))) : '—', width: 130, sortable: false },
    { field: 'entryEligible', headerName: 'Entry condition', valueFormatter: ({ value }) => typeof value === 'boolean' ? value ? 'Passed' : 'Not passed' : '—', width: 145, sortable: false },
    { field: 'level', headerName: 'Entry distance', valueFormatter: ({ value }) => typeof value === 'number' ? value + '% below breakout' : '—', width: 175, sortable: false },
    { field: 'scenario', headerName: 'Exit scenario', valueFormatter: ({ value }) => value === 'tight' ? 'Tighter exits' : 'Strategy exits', width: 145, sortable: false },
    { field: 'count', headerName: 'Outcomes', width: 110, sortable: false },
    { field: 'winRatePercent', headerName: 'Win rate', width: 110, sortable: false, valueFormatter: ({ value }) => typeof value === 'number' ? Math.round(value) + '%' : '—', cellStyle: ({ data }) => ({ color: data?.winRateColor ?? '#a0a6ad', fontWeight: 700 }) },
    { field: 'average', headerName: 'Avg net', width: 120, sortable: true, sort: 'desc', sortIndex: 0, valueFormatter: ({ value }) => percent(value), cellClass: ({ data }) => data?.averageTone },
    { field: 'pairedDeltaAverage', headerName: 'Variant Δ', headerTooltip: 'Backend-computed mean variant return minus its same-signal baseline.', width: 120, sortable: true, valueFormatter: ({ value }) => percent(value), cellClass: ({ data }) => data?.pairedDeltaTone },
  ], []);

  const all = state.summary.all;
  const live = state.summary.live;
  const summary = state.source === 'live' ? live : all;
  const scoutSymbols = state.symbols;
  const showGrid = !state.loading && state.rows.length > 0;

  return <section className="feed-section signal-info-panel" aria-label="AutoTrader scout outcomes">
    <FeedToolbar title="Scout scout outcomes" />
    <p className="signal-info-note">Simulated outcomes from the last 7 days; these do not represent filled trades. Paired baseline/variant rows show the variant return delta against the same signal. A variant can be a raw loss and still improve on its baseline. Parameter promotion uses that paired delta; unpaired Scout outcomes remain separate.</p>
    {emptyWhileLoading && <p className="positions-empty" role="status">Loading Scout outcomes…</p>}
    {state.error && <div className="positions-error" role="alert">
      {state.rows.length > 0 ? 'Could not refresh Scout outcomes; showing the last loaded data. ' : 'Could not load Scout outcomes. '}
      {state.error} <button type="button" className="auto-trader-audit-more" onClick={() => autoTraderScoutModel.retry()} disabled={state.loading}>Retry now</button>
    </div>}
    <div className="auto-trader-scout-summary" aria-live="polite">
      <span>{displayCount(summary.open)} open</span><span>{displayCount(summary.wins)} wins</span><span>{displayCount(summary.losses)} losses</span><span>{displayCount(summary.flats)} flat</span>{summary.unavailable > 0 && <span>{summary.unavailable} outcome unavailable</span>}
    </div>
    <div className="auto-trader-scout-summary" aria-label="Scout outcomes by source">
      <span>Scouts · {displayCount(all.open)} open · {displayCount(all.resolved)} resolved{all.unavailable ? ' · ' + all.unavailable + ' unavailable' : ''}</span>
      <span>Live strategy Scouts · {displayCount(live.open)} open · {displayCount(live.resolved)} resolved</span>
    </div>
    <div className="auto-trader-scout-position-filters" role="group" aria-label="Filter scout positions">
      <button type="button" aria-pressed={state.source === 'live'} onClick={() => autoTraderScoutModel.setFilters({ source: 'live' })}>Live strategy Scouts ({displayCount(live.total)})</button>
      <button type="button" aria-pressed={state.source === 'all'} onClick={() => autoTraderScoutModel.setFilters({ source: 'all' })}>All scouts ({displayCount(all.total)})</button>
      <button type="button" aria-pressed={state.status === 'open'} onClick={() => autoTraderScoutModel.setFilters({ status: state.status === 'open' ? 'all' : 'open' })}>Open ({displayCount(summary.open)})</button>
      <button type="button" aria-pressed={state.status === 'closed'} onClick={() => autoTraderScoutModel.setFilters({ status: state.status === 'closed' ? 'all' : 'closed' })}>Closed ({displayCount(summary.resolved + summary.unavailable)})</button>
    </div>
    {state.totalRows > 0 && <div className="auto-trader-scout-summary" aria-label="Scout results pages">
      <button type="button" className="auto-trader-audit-more" disabled={!state.hasPrevious || state.loading} onClick={() => autoTraderScoutModel.setFilters({ page: state.page - 1 })}>Previous</button>
      <span>Page {state.pageNumber} of {state.pageCount} · {state.totalRows.toLocaleString()} rows</span>
      <button type="button" className="auto-trader-audit-more" disabled={!state.hasNext || state.loading} onClick={() => autoTraderScoutModel.setFilters({ page: state.page + 1 })}>Next</button>
    </div>}
    <details className="auto-trader-scout-details">
      <summary>Scout learning results</summary>
      <p>Backend summaries grouped by result type, symbol, and strategy. Raw average net and paired variant delta are reported separately.</p>
      <label className="auto-trader-scout-symbol-filter">SYMBOL<select value={learningSymbol} onChange={(event) => autoTraderScoutModel.setFilters({ learningSymbol: event.target.value })}><option value="all">All symbols</option>{scoutSymbols.map((symbol) => <option key={symbol} value={symbol}>{symbol}</option>)}</select></label>
      {learningRows.length > 0
        ? <div className="data-grid-wrap auto-trader-scout-summary-grid"><AgGridReact<ScoutLearningSummary> rowData={learningRows} columnDefs={learningColumns} getRowId={({ data }) => [data.kind, data.symbol, data.strategy, data.scenario, data.parameter ?? '', data.value ?? '', data.entryEligible ?? '', data.level ?? ''].join(':')} defaultColDef={{ sortable: false, resizable: true }} pagination paginationPageSize={25} paginationPageSizeSelector={[25, 50, 100]} theme={theme} domLayout="normal" rowHeight={34} headerHeight={30} /></div>
        : <p className="positions-empty">No Scout learning outcomes recorded in the last 7 days.</p>}
    </details>
    {showGrid && <div className="data-grid-wrap signal-info-grid auto-trader-scout-position-grid"><AgGridReact<AutoTraderScoutPosition> rowData={state.rows} columnDefs={columns} getRowId={({ data }) => data.decisionId} defaultColDef={{ sortable: true, resizable: true }} theme={theme} domLayout="normal" rowHeight={34} headerHeight={30} /></div>}
    {!state.loading && !state.error && state.totalRows === 0 && <p className="positions-empty">No Scout records match these filters in the last 7 days.</p>}
  </section>;
}

export const AutoTraderScouts = memo(AutoTraderScoutsComponent);
