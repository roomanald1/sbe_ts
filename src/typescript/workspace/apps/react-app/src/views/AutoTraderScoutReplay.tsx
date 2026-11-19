import { memo, useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  PaginationModule,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import {
  autoTraderScoutReplayModel,
  type ScoutReplayOutcome,
} from '../models/AutoTraderScoutReplayModel';

ModuleRegistry.registerModules([ClientSideRowModelModule, PaginationModule]);

const replayTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

const timestamp = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value)
    ? new Date(value).toLocaleString()
    : '—';

const percent = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value)
    ? `${value > 0 ? '+' : ''}${value.toFixed(4)}%`
    : '—';

const outcomeColumns: ColDef<ScoutReplayOutcome>[] = [
  {
    field: 'evaluated_timestamp_ms',
    headerName: 'Evaluated',
    valueFormatter: ({ value }) => timestamp(value),
    width: 170,
    sort: 'asc',
  },
  { field: 'symbol', headerName: 'Symbol', width: 100 },
  { field: 'strategy', headerName: 'Strategy', width: 160 },
  { field: 'market_regime', headerName: 'Regime', width: 100 },
  {
    field: 'net_return_percent',
    headerName: 'Net return',
    valueFormatter: ({ value }) => percent(value),
    width: 110,
  },
  {
    field: 'simulated_net_pnl_usd',
    headerName: 'Net P&L (USD)',
    valueFormatter: ({ value }) =>
      typeof value === 'number' && Number.isFinite(value)
        ? value.toFixed(4)
        : '—',
    width: 120,
  },
  {
    field: 'taker_fee_rate',
    headerName: 'Taker fee',
    valueFormatter: ({ value }) =>
      typeof value === 'number' && Number.isFinite(value)
        ? `${(value * 100).toFixed(4)}%`
        : '—',
    width: 100,
  },
  { field: 'exit_scenario', headerName: 'Exit scenario', width: 120 },
  {
    field: 'signal_timestamp_ms',
    headerName: 'Signal time',
    valueFormatter: ({ value }) => timestamp(value),
    width: 170,
  },
  { field: 'sample_group_id', headerName: 'Outcome group', width: 180 },
  { field: 'decision_id', headerName: 'Decision ID', width: 220 },
  {
    colId: 'parameters',
    headerName: 'Parameters',
    valueGetter: ({ data }) => JSON.stringify(data?.strategy_parameters ?? {}),
    width: 220,
  },
];

function AutoTraderScoutReplayComponent() {
  const state = useBehaviorSnapshot(autoTraderScoutReplayModel.state$);
  useEffect(() => {
    autoTraderScoutReplayModel.start();
    return () => autoTraderScoutReplayModel.stop();
  }, []);

  const defaultColDef = useMemo<ColDef>(
    () => ({ sortable: true, resizable: true, minWidth: 85 }),
    [],
  );

  return (
    <section
      className="feed-section scout-replay-history"
      aria-label="Scout replay history"
    >
      <FeedToolbar title="Scout replay history">
        <button
          type="button"
          className="control-button"
          onClick={() => autoTraderScoutReplayModel.refreshNow()}
          disabled={state.loading}
        >
          {state.loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </FeedToolbar>

      <div
        className={`scout-replay-coverage ${state.coverage.complete ? 'is-complete' : 'is-incomplete'}`}
        role="status"
        aria-live="polite"
      >
        <strong>
          {state.coverage.complete ? 'History complete' : 'History incomplete'}
        </strong>
        <span>
          {state.coverage.returnedRecords.toLocaleString()} of{' '}
          {state.coverage.totalDurableRecords.toLocaleString()} durable records
          loaded
        </span>
        {state.coverage.malformedRecords > 0 && (
          <span>
            {state.coverage.malformedRecords.toLocaleString()} malformed records
            omitted
          </span>
        )}
        <span>
          Runtime cache limit:{' '}
          {state.coverage.runtimeHydrationLimit.toLocaleString()}
        </span>
        {state.updatedAt > 0 && (
          <span>Updated {timestamp(state.updatedAt)}</span>
        )}
      </div>

      {state.coverage.hasMore && (
        <button
          type="button"
          className="control-button"
          onClick={() => autoTraderScoutReplayModel.loadMore()}
          disabled={state.loading}
        >
          {state.loading ? 'Loading…' : 'Load more history'}
        </button>
      )}

      {state.error && (
        <div className="book-popup-message" role="alert">
          {state.error}
        </div>
      )}
      {state.workerErrors && state.workerErrors.length > 0 && (
        <div className="book-popup-message" role="alert">
          Replay history is partial. Failed Scout workers:{' '}
          {state.workerErrors
            .map(({ strategy, error }) => `${strategy}: ${error}`)
            .join('; ')}
        </div>
      )}
      {!state.loading && !state.error && state.outcomes.length === 0 && (
        <div className="book-popup-message">
          No durable Scout outcomes are recorded.
        </div>
      )}
      {state.outcomes.length > 0 && (
        <div className="data-grid-wrap scout-replay-grid">
          <AgGridReact<ScoutReplayOutcome>
            rowData={state.outcomes}
            columnDefs={outcomeColumns}
            defaultColDef={defaultColDef}
            getRowId={({ data }) =>
              `${data.decision_id}:${data.evaluated_timestamp_ms}:${data.exit_scenario}`
            }
            pagination
            paginationPageSize={100}
            paginationPageSizeSelector={[50, 100, 250]}
            theme={replayTheme}
            domLayout="normal"
            rowHeight={26}
            headerHeight={28}
          />
        </div>
      )}
    </section>
  );
}

export const AutoTraderScoutReplay = memo(AutoTraderScoutReplayComponent);
