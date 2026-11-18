import { useMemo, useState } from 'react';
import {
  gridTheme,
  percent,
  ratio,
  signedValueClasses,
  useSignalInfo,
} from './SignalInfoViewShared';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type {
  MissedOpportunityBand,
  PostExitOpportunityRow,
} from '../models/SignalInfoModel';

export function MissedOpportunityBandsView({
  paused = false,
}: {
  paused?: boolean;
}) {
  const state = useSignalInfo();
  const rows = usePausedValue(state.missedOpportunityBands, paused);
  const [symbol, setSymbol] = useState('');
  const [direction, setDirection] = useState('');
  const [strategy, setStrategy] = useState('');
  const [marketContext, setMarketContext] = useState('');
  const dimensions = useMemo(
    () => ({
      symbols: [
        ...new Set(rows.flatMap((row) => (row.symbol ? [row.symbol] : []))),
      ].sort(),
      directions: [
        ...new Set(
          rows.flatMap((row) => (row.direction ? [row.direction] : [])),
        ),
      ].sort(),
      strategies: [
        ...new Set(rows.flatMap((row) => (row.strategy ? [row.strategy] : []))),
      ].sort(),
      contexts: [
        ...new Set(
          rows.flatMap((row) => (row.marketContext ? [row.marketContext] : [])),
        ),
      ].sort(),
    }),
    [rows],
  );
  const filteredRows = useMemo(() => {
    const filtering = Boolean(symbol || direction || strategy || marketContext);
    return rows.filter((row) =>
      filtering
        ? (!symbol || row.symbol === symbol) &&
          (!direction || row.direction === direction) &&
          (!strategy || row.strategy === strategy) &&
          (!marketContext || row.marketContext === marketContext)
        : !row.symbol && !row.direction && !row.strategy && !row.marketContext,
    );
  }, [rows, symbol, direction, strategy, marketContext]);
  const thresholdSuggestions = useMemo(
    () =>
      (['score', 'strength'] as const).flatMap((gate) => {
        const supported = filteredRows.filter(
          (row) =>
            row.gate === gate &&
            row.sampleCount >= 20 &&
            row.averageNetReturnPercent != null &&
            row.averageNetReturnPercent > 0 &&
            row.stopTargetSampleCount >= 20 &&
            row.stopTargetAverageNetReturnPercent != null &&
            row.stopTargetAverageNetReturnPercent > 0 &&
            row.stopTargetPositiveNetRate != null &&
            row.stopTargetPositiveNetRate >= 0.55,
        );
        const fiveMinute = supported.filter(
          (row) => row.horizonSeconds === 300,
        );
        const evidence = fiveMinute.length
          ? fiveMinute
          : supported.filter(
              (row) =>
                (row.horizonSeconds === 900 || row.horizonSeconds === 3600) &&
                row.timeToProfitSampleCount >= 20 &&
                row.timeToProfitRate != null &&
                row.timeToProfitRate >= 0.5 &&
                row.averageTimeToProfitSeconds != null &&
                row.averageTimeToProfitSeconds > 300 &&
                row.averageTimeToProfitSeconds <= row.horizonSeconds,
            );
        const best = evidence.sort(
          (left, right) => left.threshold - right.threshold,
        )[0];
        return best
          ? [
              {
                gate,
                threshold: best.threshold,
                horizonSeconds: best.horizonSeconds,
                sampleCount: best.stopTargetSampleCount,
              },
            ]
          : [];
      }),
    [filteredRows],
  );
  const columns = useMemo<ColDef<MissedOpportunityBand>[]>(
    () => [
      {
        field: 'symbol',
        headerName: 'Symbol',
        valueFormatter: ({ value }) => value || 'All',
        minWidth: 105,
      },
      {
        field: 'direction',
        headerName: 'Direction',
        valueFormatter: ({ value }) => value || 'All',
        minWidth: 100,
      },
      {
        field: 'strategy',
        headerName: 'Strategy',
        valueFormatter: ({ value }) => value || 'All',
        minWidth: 150,
      },
      {
        field: 'marketContext',
        headerName: 'Market context',
        valueFormatter: ({ value }) => value || 'All',
        minWidth: 130,
      },
      { field: 'gate', headerName: 'Gate', minWidth: 120 },
      {
        field: 'threshold',
        headerName: 'Threshold',
        valueFormatter: ({ value, data }) =>
          data?.gate === 'score'
            ? Number(value).toFixed(2)
            : (Number(value) * 100).toFixed(0) + '%',
        minWidth: 105,
      },
      {
        field: 'horizonSeconds',
        headerName: 'Horizon',
        valueFormatter: ({ value }) => (Number(value) / 60).toFixed(0) + 'm',
        minWidth: 90,
      },
      { field: 'sampleCount', headerName: 'Missed signals', minWidth: 110 },
      {
        field: 'averageNetReturnPercent',
        headerName: 'Avg net at horizon',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 140,
      },
      {
        field: 'positiveNetRate',
        headerName: 'Net positive at end',
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 140,
      },
      {
        field: 'stopTargetSampleCount',
        headerName: 'SL/TP samples',
        minWidth: 105,
      },
      {
        field: 'stopTargetAverageNetReturnPercent',
        headerName: 'SL/TP simulated net',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 145,
      },
      {
        field: 'stopTargetPositiveNetRate',
        headerName: 'SL/TP win rate',
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 120,
      },
      {
        field: 'strategyExitSampleCount',
        headerName: 'Strategy exits',
        minWidth: 110,
      },
      {
        field: 'strategyExitAverageNetReturnPercent',
        headerName: 'Strategy-exit net',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 140,
      },
      {
        field: 'strategyExitPositiveNetRate',
        headerName: 'Strategy-exit win rate',
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 150,
      },
      {
        field: 'timeToProfitRate',
        headerName: 'Became net positive',
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 145,
      },
      {
        field: 'averageTimeToProfitSeconds',
        headerName: 'Avg time to profit',
        valueFormatter: ({ value }) =>
          value == null ? '—' : Number(value).toFixed(0) + 's',
        minWidth: 135,
      },
      {
        field: 'medianTimeToProfitSeconds',
        headerName: 'Median time to profit',
        valueFormatter: ({ value }) =>
          value == null ? '—' : Number(value).toFixed(0) + 's',
        minWidth: 145,
      },
    ],
    [],
  );
  const postExitRows = usePausedValue(state.postExitOpportunities, paused);
  const postExitColumns = useMemo<ColDef<PostExitOpportunityRow>[]>(
    () => [
      {
        field: 'timestamp',
        headerName: 'Observed',
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? new Date(value).toLocaleString() : '—',
        minWidth: 165,
      },
      { field: 'symbol', headerName: 'Symbol', minWidth: 110 },
      { field: 'side', headerName: 'Side', minWidth: 90 },
      { field: 'strategy', headerName: 'Strategy', minWidth: 140 },
      {
        field: 'missedReturnPercent',
        headerName: 'Return after exit',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 135,
      },
      {
        field: 'bestPostExitReturnPercent',
        headerName: 'Best post-exit',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 125,
      },
      {
        field: 'worstPostExitReturnPercent',
        headerName: 'Worst post-exit',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 125,
      },
      {
        field: 'missedPnlQuote',
        headerName: 'Missed P&L (quote)',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) =>
          value == null ? '—' : Number(value).toFixed(2),
        minWidth: 145,
      },
    ],
    [],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Missed opportunity bands"
    >
      <FeedToolbar title="Missed opportunity bands" />
      <p className="signal-info-note">
        Skipped-signal outcomes at 1m, 5m, 15m, and 60m, after entry/exit spread
        and taker fees. “Became net positive” is the share that crossed above
        zero during the horizon; average and median times use those samples
        only. SL/TP simulation uses the thresholds recorded at the skipped
        decision and the entry spread as an exit-spread proxy. If both barriers
        were touched, sampled path-extreme timestamps determine order; ambiguous
        ordering remains stop-first. Five-minute outcomes are preferred for gate
        adjustments, with longer windows used when short-horizon evidence is
        insufficient.
      </p>
      <div
        className="signal-info-filters"
        aria-label="Missed opportunity filters"
      >
        <label className="chart-symbol-control">
          Symbol
          <select
            aria-label="Filter missed opportunities by symbol"
            value={symbol}
            onChange={(event) => setSymbol(event.target.value)}
          >
            <option value="">All</option>
            {dimensions.symbols.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="chart-symbol-control">
          Direction
          <select
            aria-label="Filter missed opportunities by direction"
            value={direction}
            onChange={(event) => setDirection(event.target.value)}
          >
            <option value="">All</option>
            {dimensions.directions.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="chart-symbol-control">
          Strategy
          <select
            aria-label="Filter missed opportunities by strategy"
            value={strategy}
            onChange={(event) => setStrategy(event.target.value)}
          >
            <option value="">All</option>
            {dimensions.strategies.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="chart-symbol-control">
          Market context
          <select
            aria-label="Filter missed opportunities by market context"
            value={marketContext}
            onChange={(event) => setMarketContext(event.target.value)}
          >
            <option value="">All</option>
            {dimensions.contexts.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="signal-info-note">
        {thresholdSuggestions.length
          ? `Evidence-backed candidate floors: ${thresholdSuggestions
              .map(
                (item) =>
                  `${item.gate} ≤ ${
                    item.gate === 'score'
                      ? item.threshold.toFixed(2)
                      : `${(item.threshold * 100).toFixed(0)}%`
                  } (${item.sampleCount} SL/TP samples, ${item.horizonSeconds / 60}m)`,
              )
              .join(
                '; ',
              )}. These are diagnostics; live gates remain managed by the configured adaptive policy.`
          : 'No statistically supported threshold relaxation is available for this filter.'}
      </p>
      {state.missedOpportunityBandsError && (
        <div className="positions-error">
          {state.missedOpportunityBandsError}
        </div>
      )}
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<MissedOpportunityBand>
          rowData={filteredRows}
          columnDefs={columns}
          defaultColDef={{
            sortable: true,
            resizable: true,
            flex: 1,
            minWidth: 80,
          }}
          theme={gridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={28}
        />
      </div>
      {filteredRows.every(
        (row) => row.sampleCount === 0 && row.stopTargetSampleCount === 0,
      ) &&
        !state.missedOpportunityBandsError && (
          <p className="positions-empty">
            No missed opportunity samples are available for this filter yet.
            Candidate outcomes populate after skipped signals have a completed
            1-minute evaluation.
          </p>
        )}
      <h3 className="signal-info-subheading">
        Post-exit opportunity observations
      </h3>
      <p className="signal-info-note">
        Closed trades are observed for up to 30 minutes after exit. These
        results remain separate from skipped-entry bands because they describe
        realized positions, and are also sent to Learning for exit-timing
        feedback.
      </p>
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<PostExitOpportunityRow>
          rowData={postExitRows}
          columnDefs={postExitColumns}
          defaultColDef={{
            sortable: true,
            resizable: true,
            flex: 1,
            minWidth: 80,
          }}
          theme={gridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={28}
        />
      </div>
      {postExitRows.length === 0 && (
        <p className="positions-empty">
          No post-exit observations are available yet.
        </p>
      )}
    </section>
  );
}
