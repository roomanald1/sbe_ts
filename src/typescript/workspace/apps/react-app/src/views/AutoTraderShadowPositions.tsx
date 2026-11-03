import { memo, useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, PaginationModule, themeQuartz, type ColDef } from 'ag-grid-community';
import { autoTraderShadowModel, type AutoTraderShadowPosition } from '../models/AutoTraderShadowModel';
import type { SymbolPriceTick } from '../types/symbol_price';
import type { PriceHistorySample } from '../price_change_data';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule, PaginationModule]);
const theme = themeQuartz.withParams({ accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31', foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24' });
type ShadowGridRow = AutoTraderShadowPosition & { markPrice?: number; openNetReturnPercent?: number };
const percent = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? (value > 0 ? '+' : '') + value.toFixed(3) + '%' : '—';
const winRateColor = (rate: number) => {
  const progress = Math.max(0, Math.min(100, rate)) / 100;
  const red = [240, 110, 114];
  const green = [69, 201, 137];
  const rgb = red.map((channel, index) => Math.round(channel + (green[index] - channel) * progress));
  return 'rgb(' + rgb.join(', ') + ')';
};
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
  const [positionFilter, setPositionFilter] = useState<'all' | 'open' | 'closed'>('all');
  const [sourceFilter, setSourceFilter] = useState<'live' | 'all'>('live');
  const [scoutSymbolFilter, setScoutSymbolFilter] = useState('all');
  useEffect(() => { autoTraderShadowModel.start(); return () => autoTraderShadowModel.stop(); }, []);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1_000); return () => clearInterval(timer); }, []);
  const marks = useMemo(() => new Map(ticks.filter((tick) => Number.isFinite(tick.price) && now - tick.receivedAt < 60_000).map((tick) => [tick.symbol, tick.price])), [ticks, now]);
  const rows = useMemo<ShadowGridRow[]>(() => state.rows.map((row) => {
    const latestHistory = histories.get(row.symbol)?.at(-1);
    const markPrice = row.status === 'Open'
      ? marks.get(row.symbol) ?? (latestHistory && now - latestHistory.timestamp < 60_000 ? latestHistory.price : undefined)
      : undefined;
    const grossReturn = row.entryPrice && markPrice
      ? (row.direction === 'bullish' ? markPrice / row.entryPrice - 1 : row.entryPrice / markPrice - 1) * 100
      : undefined;
    const entrySpreadPercent = row.entryBid && row.entryAsk && row.entryPrice
      ? (row.entryAsk - row.entryBid) / row.entryPrice * 100
      : 0;
    const estimatedOpenReturn = grossReturn === undefined
      ? undefined
      : grossReturn - row.takerFeeRate * 2 * 100 - entrySpreadPercent;
    return { ...row, markPrice, openNetReturnPercent: estimatedOpenReturn };
  }), [state.rows, marks, histories, now]);
  const liveStrategyRows = useMemo(() => rows.filter((row) => row.source === 'Scout' && row.strategy.startsWith('live-')), [rows]);
  const sourceRows = sourceFilter === 'live' ? liveStrategyRows : rows;
  const openCount = sourceRows.filter((row) => row.status === 'Open').length;
  const closedCount = sourceRows.length - openCount;
  const wins = sourceRows.filter((row) => row.status === 'Won').length;
  const losses = sourceRows.filter((row) => row.status === 'Lost').length;
  const flats = sourceRows.filter((row) => row.status === 'Flat').length;
  const unavailable = sourceRows.filter((row) => row.status === 'Outcome unavailable').length;
  const sourceCounts = useMemo(() => {
    const counts = {
      Scout: { open: 0, resolved: 0, unavailable: 0 },
      Exploratory: { open: 0, resolved: 0, unavailable: 0 },
    };
    for (const row of rows) {
      const count = counts[row.source];
      if (row.status === 'Open') count.open += 1;
      else if (row.status === 'Outcome unavailable') count.unavailable += 1;
      else count.resolved += 1;
    }
    return counts;
  }, [rows]);
  const scoutParameterSummaries = useMemo(() => {
    const cohorts = new Map<string, { kind: 'Parameter probe'; symbol: string; strategy: string; parameter: string; value: number; entryEligible: boolean; scenario: 'strategy' | 'tight'; rows: AutoTraderShadowPosition[] }>();
    for (const row of rows) {
      if (row.source !== 'Scout' || !row.scoutParameterKey || typeof row.scoutParameterValue !== 'number') continue;
      const entryEligible = row.scoutParameterEntryEligible ?? false;
      const key = [row.symbol, row.strategy, row.scoutParameterKey, row.scoutParameterValue, entryEligible, row.exitScenario].join('|');
      const cohort = cohorts.get(key) ?? { kind: 'Parameter probe', symbol: row.symbol, strategy: row.strategy, parameter: row.scoutParameterKey, value: row.scoutParameterValue, entryEligible, scenario: row.exitScenario, rows: [] };
      cohort.rows.push(row);
      cohorts.set(key, cohort);
    }
    return [...cohorts.values()].map(({ kind, symbol, strategy, parameter, value, entryEligible, scenario, rows: samples }) => {
      const resolved = samples.filter((row) => ['Won', 'Lost', 'Flat'].includes(row.status));
      const wins = resolved.filter((row) => row.status === 'Won').length;
      const returns = resolved.flatMap((row) => typeof row.netReturnPercent === 'number' ? [row.netReturnPercent] : []);
      return { kind, symbol, strategy, parameter, value, entryEligible, scenario, count: resolved.length, wins, average: returns.length ? returns.reduce((sum, current) => sum + current, 0) / returns.length : undefined };
    });
  }, [rows]);
  const scoutLevelSummaries = useMemo(() => {
    const cohorts = new Map<string, { symbol: string; strategy: string; scenario: 'strategy' | 'tight'; level: number; rows: AutoTraderShadowPosition[] }>();
    for (const row of rows) {
      if (row.source !== 'Scout' || typeof row.scoutLevelPercent !== 'number') continue;
      const key = [row.symbol, row.strategy, row.exitScenario, row.scoutLevelPercent].join('|');
      const cohort = cohorts.get(key) ?? { symbol: row.symbol, strategy: row.strategy, scenario: row.exitScenario, level: row.scoutLevelPercent, rows: [] };
      cohort.rows.push(row);
      cohorts.set(key, cohort);
    }
    return [...cohorts.values()].map(({ symbol, strategy, scenario, level, rows: samples }) => {
      const resolved = samples.filter((row) => ['Won', 'Lost', 'Flat'].includes(row.status));
      const wins = resolved.filter((row) => row.status === 'Won').length;
      const returns = resolved.flatMap((row) => typeof row.netReturnPercent === 'number' ? [row.netReturnPercent] : []);
      return { kind: 'Breakout distance' as const, symbol, strategy, scenario, level, count: resolved.length, wins, average: returns.length ? returns.reduce((sum, value) => sum + value, 0) / returns.length : undefined };
    });
  }, [rows]);
  const scoutOutcomeSummaries = useMemo(() => {
    const cohorts = new Map<string, { symbol: string; strategy: string; scenario: 'strategy' | 'tight'; rows: AutoTraderShadowPosition[] }>();
    for (const row of rows) {
      if (row.source !== 'Scout') continue;
      const key = [row.symbol, row.strategy, row.exitScenario].join('|');
      const cohort = cohorts.get(key) ?? { symbol: row.symbol, strategy: row.strategy, scenario: row.exitScenario, rows: [] };
      cohort.rows.push(row);
      cohorts.set(key, cohort);
    }
    return [...cohorts.values()].map(({ symbol, strategy, scenario, rows: samples }) => {
      const resolved = samples.filter((row) => ['Won', 'Lost', 'Flat'].includes(row.status));
      const wins = resolved.filter((row) => row.status === 'Won').length;
      const returns = resolved.flatMap((row) => typeof row.netReturnPercent === 'number' ? [row.netReturnPercent] : []);
      return { kind: 'Scout summary' as const, symbol, strategy, scenario, count: resolved.length, wins, average: returns.length ? returns.reduce((sum, value) => sum + value, 0) / returns.length : undefined };
    });
  }, [rows]);
  const scoutGridRows = useMemo(() => [...scoutOutcomeSummaries, ...scoutParameterSummaries, ...scoutLevelSummaries], [scoutOutcomeSummaries, scoutParameterSummaries, scoutLevelSummaries]);
  const scoutSymbols = useMemo(() => [...new Set(scoutGridRows.map((summary) => summary.symbol))].sort(), [scoutGridRows]);
  const filteredScoutGridRows = useMemo(() => scoutSymbolFilter === 'all' ? scoutGridRows : scoutGridRows.filter((summary) => summary.symbol === scoutSymbolFilter), [scoutGridRows, scoutSymbolFilter]);
  const scoutSummaryColumns = useMemo<ColDef<(typeof scoutGridRows)[number]>[]>(() => [
    { field: 'kind', headerName: 'Grouping', width: 155, sortable: false },
    { field: 'symbol', headerName: 'Symbol', width: 120, sortable: false },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), width: 190, sortable: false },
    { field: 'parameter', headerName: 'Probed parameter', minWidth: 190, flex: 1, sortable: false, valueFormatter: ({ value }) => value ?? '—' },
    { field: 'value', headerName: 'Probed value', valueFormatter: ({ value }) => typeof value === 'number' ? String(Number(value.toPrecision(6))) : '—', width: 130, sortable: false },
    { field: 'entryEligible', headerName: 'Entry condition', valueFormatter: ({ value }) => typeof value === 'boolean' ? value ? 'Passed' : 'Not passed' : '—', width: 145, sortable: false },
    { field: 'level', headerName: 'Entry distance', valueFormatter: ({ value }) => typeof value === 'number' ? value + '% below breakout' : '—', width: 175, sortable: false },
    { field: 'scenario', headerName: 'Exit scenario', valueFormatter: ({ value }) => value === 'tight' ? 'Tighter exits' : 'Strategy exits', width: 145, sortable: false },
    { field: 'count', headerName: 'Outcomes', width: 110, sortable: false },
    { headerName: 'Win rate', width: 110, sortable: false, valueGetter: ({ data }) => data && data.count ? data.wins / data.count * 100 : undefined, valueFormatter: ({ value }) => typeof value === 'number' ? Math.round(value) + '%' : '—', cellStyle: ({ value }) => ({ color: typeof value === 'number' ? winRateColor(value) : '#a0a6ad', fontWeight: 700 }) },
    { field: 'average', headerName: 'Avg net', width: 120, sortable: true, sort: 'desc', sortIndex: 0, valueFormatter: ({ value }) => percent(value), cellClassRules: { 'pnl-positive': ({ value }) => typeof value === 'number' && value > 1e-12, 'pnl-negative': ({ value }) => typeof value === 'number' && value < -1e-12 } },
  ], []);
  const columns = useMemo<ColDef<ShadowGridRow>[]>(() => [
    { field: 'symbol', headerName: 'Symbol', width: 85, minWidth: 75 },
    { field: 'strategy', headerName: 'Strategy', valueFormatter: ({ value }) => strategyLabel(value), width: 130, minWidth: 110 },
    { field: 'status', headerName: 'State', width: 85, minWidth: 75, cellStyle: ({ value }) => value === 'Won' ? { color: '#45c989', fontWeight: 700 } : value === 'Lost' ? { color: '#f06e72', fontWeight: 700 } : undefined },
    { field: 'exitScenario', headerName: 'Exit scenario', valueFormatter: ({ value }) => value === 'tight' ? 'Tighter exits' : 'Strategy exits', width: 105, minWidth: 90 },
    { headerName: 'Scout level', valueGetter: ({ data }) => data?.scoutLevelPercent, valueFormatter: ({ value }) => typeof value === 'number' ? String(value.toFixed(2)) + '% below breakout' : '—', width: 125, minWidth: 105 },
    { headerName: 'Opened at', valueGetter: ({ data }) => data?.openedAt, valueFormatter: ({ value }) => typeof value === 'number' ? new Date(value).toLocaleString() : '—', width: 140, minWidth: 120 },
    { headerName: 'Duration', headerTooltip: 'Elapsed time for open shadow positions.', width: 80, sort: 'asc', sortIndex: 0, valueGetter: ({ data }) => {
      if (!data || data.status !== 'Open') return undefined;
      return Math.max(0, now - data.openedAt);
    }, valueFormatter: ({ value }) => typeof value === 'number' ? durationLabel(value) : '—', minWidth: 80 },
    { headerName: 'Entry', valueGetter: ({ data }) => data?.entryPrice, valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', width: 95, minWidth: 80 },
    { headerName: 'Mark', valueGetter: ({ data }) => data?.markPrice, valueFormatter: ({ value }) => typeof value === 'number' ? value.toPrecision(8) : '—', width: 105, minWidth: 90 },
    { headerName: 'Net vs breakeven', width: 130, headerTooltip: 'Net return after estimated or configured round-trip taker fees and spread. The bar is centred on breakeven and scaled from −2% to +2%; larger returns are clipped visually.', minWidth: 120, cellRenderer: ({ data }: { data?: ShadowGridRow }) => {
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
    { headerName: 'Net', headerTooltip: 'Closed net return after configured taker fees and spread. Open positions show the estimated net return.', valueGetter: ({ data }) => data?.status === 'Open' ? data.openNetReturnPercent : data?.netReturnPercent, valueFormatter: ({ value }) => percent(value), cellClassRules: { 'is-positive': ({ value }) => typeof value === 'number' && value > 1e-12, 'is-negative': ({ value }) => typeof value === 'number' && value < -1e-12 }, minWidth: 75, maxWidth: 90 },
    { field: 'strength', headerName: 'Signal strength', valueFormatter: ({ value }) => typeof value === 'number' ? `${(value * 100).toFixed(0)}%` : '—', minWidth: 115 },
  ], [now]);
  const visibleRows = useMemo(() => sourceRows.filter((row) => positionFilter === 'all' || (positionFilter === 'open' ? row.status === 'Open' : row.status !== 'Open')), [sourceRows, positionFilter]);

  return <section className="feed-section signal-info-panel" aria-label="AutoTrader shadow outcomes">
    <FeedToolbar title="Scout and exploratory shadow outcomes" />
    <p className="signal-info-note">Simulated outcomes from the last 7 days; these do not represent filled trades. A signal can create multiple rows for exit scenarios and tested entry levels. Filled Scout trades are counted separately in the AutoTrader summary.</p>
    <div className="auto-trader-shadow-summary" aria-live="polite">
      <span>{openCount} open</span><span>{wins} wins</span><span>{losses} losses</span><span>{flats} flat</span>{unavailable > 0 && <span>{unavailable} outcome unavailable</span>}
    </div>
    <div className="auto-trader-shadow-summary" aria-label="Shadow outcomes by source">
      <span>Scout shadows · {sourceCounts.Scout.open} open · {sourceCounts.Scout.resolved} resolved{sourceCounts.Scout.unavailable ? ' · ' + sourceCounts.Scout.unavailable + ' unavailable' : ''}</span>
      <span>Live strategy Scouts · {liveStrategyRows.filter((row) => row.status === 'Open').length} open · {liveStrategyRows.filter((row) => row.status !== 'Open').length} resolved</span>
      <span>Exploratory shadows · {sourceCounts.Exploratory.open} open · {sourceCounts.Exploratory.resolved} resolved{sourceCounts.Exploratory.unavailable ? ' · ' + sourceCounts.Exploratory.unavailable + ' unavailable' : ''}</span>
    </div>
    <div className="auto-trader-shadow-position-filters" role="group" aria-label="Filter shadow positions">
      <button type="button" aria-pressed={sourceFilter === 'live'} onClick={() => { setSourceFilter('live'); setPositionFilter('all'); }}>Live strategy Scouts ({liveStrategyRows.length})</button>
      <button type="button" aria-pressed={sourceFilter === 'all'} onClick={() => { setSourceFilter('all'); setPositionFilter('all'); }}>All shadows ({rows.length})</button>
      <button type="button" aria-pressed={positionFilter === 'open'} onClick={() => setPositionFilter('open')}>Open ({openCount})</button>
      <button type="button" aria-pressed={positionFilter === 'closed'} onClick={() => setPositionFilter('closed')}>Closed ({closedCount})</button>
    </div>
    <details className="auto-trader-scout-details">
      <summary>Scout learning results</summary>
      <p>Scout outcome summaries, parameter probes, and breakout entry distances share one grid, grouped by result type, symbol, and strategy. Rows are ordered by average net return; win rate is shown for context. The summary rows include each strategy with recorded Scout outcomes.</p>
      <label className="auto-trader-scout-symbol-filter">SYMBOL<select value={scoutSymbolFilter} onChange={(event) => setScoutSymbolFilter(event.target.value)}><option value="all">All symbols</option>{scoutSymbols.map((symbol) => <option key={symbol} value={symbol}>{symbol}</option>)}</select></label>
      {scoutGridRows.length > 0
        ? <div className="data-grid-wrap auto-trader-scout-summary-grid"><AgGridReact rowData={filteredScoutGridRows} columnDefs={scoutSummaryColumns} getRowId={({ data }) => [data.kind, data.symbol, data.strategy, data.scenario, 'parameter' in data ? data.parameter : '', 'value' in data ? data.value : '', 'entryEligible' in data ? data.entryEligible : '', 'level' in data ? data.level : ''].join(':')} defaultColDef={{ sortable: false, resizable: true }} pagination paginationPageSize={25} theme={theme} domLayout="normal" rowHeight={34} headerHeight={30} /></div>
        : <p className="positions-empty">No Scout learning outcomes recorded in the last 7 days.</p>}
    </details>
    {state.error && <div className="positions-error">{state.error}</div>}
    <div className="data-grid-wrap signal-info-grid auto-trader-shadow-position-grid"><AgGridReact<ShadowGridRow> rowData={visibleRows} columnDefs={columns} getRowId={({ data }) => data.decisionId} defaultColDef={{ sortable: true, resizable: true }} theme={theme} domLayout="normal" rowHeight={34} headerHeight={30} /></div>
    {state.loading && <p className="positions-empty">Loading shadow outcomes…</p>}
    {!state.loading && !state.error && visibleRows.length === 0 && <p className="positions-empty">{sourceFilter === 'live' ? 'No live strategy Scout shadows recorded in the last 7 days.' : positionFilter === 'closed' ? 'No closed shadow positions recorded in the last 7 days.' : positionFilter === 'open' ? 'No open shadow positions.' : 'No scout or exploratory shadow candidates recorded in the last 7 days.'}</p>}
  </section>;
}

export const AutoTraderShadowPositions = memo(AutoTraderShadowPositionsComponent);
