import { memo, useEffect, useMemo, useState } from 'react';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import {
  SignalInfoModel,
  type MissedOpportunityBand,
  type PostExitOpportunityRow,
  type SignalPerformanceRow,
  type SkippedSignalAuditRow,
} from '../models/SignalInfoModel';
import type { SignalSnapshot } from '../models/SignalsModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
const percent = (value?: number | null) =>
  value == null ? 'N/A' : value.toFixed(3) + '%';
const ratio = (value?: number | null) =>
  value == null ? 'N/A' : (value * 100).toFixed(1) + '%';
const horizonLabel = (value: unknown) =>
  (
    ({
      60: '1 minute',
      300: '5 minutes',
      900: '15 minutes',
      3600: '1 hour',
    }) as Record<number, string>
  )[Number(value)] ?? String(value) + ' seconds';
const signedValueClasses = {
  'signal-value-positive': (params: { value: unknown }) =>
    typeof params.value === 'number' && params.value > 0,
  'signal-value-negative': (params: { value: unknown }) =>
    typeof params.value === 'number' && params.value < 0,
};
const directionBadge = ({ value }: { value: unknown }) => {
  const direction =
    value === 'bullish' || value === 'bearish' || value === 'ranging'
      ? value
      : 'neutral';
  return (
    <span className={'signal-badge signal-badge-' + direction}>
      {String(value ?? 'neutral').toUpperCase()}
    </span>
  );
};
const strengthRenderer = ({ value }: { value: unknown }) => {
  const strength =
    typeof value === 'number' ? Math.max(0, Math.min(1, value)) : 0;
  const level = strength >= 0.5 ? 'high' : strength >= 0.25 ? 'medium' : 'low';
  return (
    <span className={'signal-strength signal-strength-' + level}>
      <span className="signal-strength-track">
        <span style={{ width: strength * 100 + '%' }} />
      </span>
      <b>{(strength * 100).toFixed(0)}%</b>
    </span>
  );
};

function useSignalInfo() {
  const model = useMemo(() => new SignalInfoModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    void model.start();
    return () => model.stop();
  }, [model]);
  return state;
}

type BestSignalRow = SignalSnapshot & { time: string };
export function BestSignalsView({
  onTradeSymbol,
  onRangeOrders,
}: {
  onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void;
  onRangeOrders: (request: {
    symbol: string;
    lower: number;
    upper: number;
  }) => void;
}) {
  const state = useSignalInfo();
  const rows = useMemo<BestSignalRow[]>(
    () =>
      state.best.map((item) => ({
        ...item,
        time: new Date(item.timestamp).toLocaleTimeString(),
      })),
    [state.best],
  );
  const columns = useMemo<ColDef<BestSignalRow>[]>(
    () => [
      {
        field: 'symbol',
        headerName: 'Symbol',
        cellRenderer: ({
          value,
          data,
        }: {
          value: unknown;
          data?: BestSignalRow;
        }) => (
          <button
            type="button"
            className="signal-symbol-link"
            title={
              'Open ' +
              (data?.direction === 'bearish' ? 'sell' : 'buy') +
              ' order for ' +
              String(value ?? '')
            }
            onClick={(event) => {
              event.stopPropagation();
              if (data)
                onTradeSymbol(
                  data.symbol,
                  data.direction === 'bearish' ? 'sell' : 'buy',
                );
            }}
          >
            {String(value ?? '')}
          </button>
        ),
        minWidth: 110,
      },
      {
        field: 'direction',
        headerName: 'Signal',
        minWidth: 145,
        tooltipValueGetter: ({ data }) =>
          data?.reasons.find((reason) =>
            reason.startsWith('Breakout watch:'),
          ) ?? undefined,
        cellRenderer: ({
          value,
          data,
        }: {
          value: unknown;
          data?: BestSignalRow;
        }) => {
          const breakout = data?.reasons.find((reason) =>
            reason.startsWith('Breakout watch:'),
          );
          const breakoutUp = breakout?.startsWith('Breakout watch: UP');
          const rangeStatus =
            value === 'ranging' &&
            data?.range_lower_bound != null &&
            data.range_upper_bound != null ? (
              <button
                type="button"
                className="signal-badge signal-badge-ranging signal-range-trigger"
                title="Set buy and sell limit orders at the range boundaries"
                onClick={(event) => {
                  event.stopPropagation();
                  onRangeOrders({
                    symbol: data.symbol,
                    lower: data.range_lower_bound!,
                    upper: data.range_upper_bound!,
                  });
                }}
              >
                RANGING
              </button>
            ) : (
              directionBadge({ value })
            );
          return (
            <span className="signal-status-group">
              {rangeStatus}
              {breakout && (
                <span
                  className={
                    'signal-badge ' +
                    (breakoutUp
                      ? 'signal-badge-bullish'
                      : 'signal-badge-bearish')
                  }
                >
                  BREAKOUT {breakoutUp ? '↑' : '↓'}
                </span>
              )}
            </span>
          );
        },
      },
      {
        field: 'ranging_probability',
        headerName: 'Range setup',
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 115,
        tooltipValueGetter: () =>
          'Heuristic score from the prior five minutes of containment and boundary visits. It is not an accuracy estimate.',
      },
      {
        headerName: 'Range boundaries',
        valueGetter: ({ data }) =>
          data?.range_lower_bound != null && data?.range_upper_bound != null
            ? String(data.range_lower_bound) +
              ' – ' +
              String(data.range_upper_bound)
            : 'N/A',
        minWidth: 180,
        tooltipValueGetter: () =>
          'Observed low and high prices over the five-minute analysis window.',
      },
      {
        field: 'strength',
        headerName: 'Signal strength',
        cellRenderer: strengthRenderer,
        minWidth: 130,
        tooltipValueGetter: () =>
          'Raw conviction: absolute score multiplied by the fraction of available directional inputs. This is not an accuracy estimate.',
      },
      {
        field: 'estimated_accuracy',
        headerName: 'Est. accuracy',
        cellRenderer: strengthRenderer,
        minWidth: 130,
        tooltipValueGetter: () =>
          'Smoothed historical hit rate for similar signals. Ranging calls are checked against the next five minutes staying inside the forecast bounds; estimates use a neutral prior when samples are sparse.',
      },
      {
        field: 'score',
        headerName: 'Score',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => Number(value ?? 0).toFixed(2),
        minWidth: 80,
      },
      {
        field: 'components.trend_1m_percent',
        headerName: 'Trend 1m',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_1m_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 100,
      },
      {
        field: 'components.trend_5m_percent',
        headerName: 'Trend 5m',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_5m_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 100,
      },
      {
        field: 'components.trend_5m_volatility_units',
        headerName: '5m Trend / Vol',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_5m_volatility_units,
        valueFormatter: ({ value }) =>
          value == null ? 'N/A' : value.toFixed(2) + 'σ',
        minWidth: 95,
      },
      {
        field: 'components.momentum_10s_percent',
        headerName: 'Momentum 10s',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.momentum_10s_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 110,
      },
      {
        field: 'components.trade_flow_imbalance',
        headerName: 'Trade flow',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trade_flow_imbalance,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 95,
      },
      {
        field: 'components.book_imbalance',
        headerName: 'Book pressure',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.book_imbalance,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 105,
      },
      {
        field: 'components.mispricing_vs_vwap_bps',
        headerName: 'VWAP mispricing',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.mispricing_vs_vwap_bps,
        valueFormatter: ({ value }) =>
          value == null ? 'N/A' : value.toFixed(1) + ' bps',
        minWidth: 125,
        tooltipValueGetter: () =>
          'Current price vs recent trade VWAP. Positive means above VWAP.',
      },
      {
        field: 'components.pressure_without_price_move',
        headerName: 'Pressure / no move',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.pressure_without_price_move,
        valueFormatter: ({ value }) =>
          value == null ? 'N/A' : (value * 100).toFixed(0) + '%',
        minWidth: 135,
        tooltipValueGetter: () =>
          'Trade pressure imbalance detected while price stayed nearly flat. Signed by buy/sell pressure.',
      },
      {
        field: 'components.aggressor_cost_bps',
        headerName: 'Taker cost',
        valueGetter: ({ data }) => data?.components.aggressor_cost_bps,
        valueFormatter: ({ value }) =>
          value == null ? 'N/A' : value.toFixed(1) + ' bps',
        minWidth: 105,
        tooltipValueGetter: () =>
          'Average aggressive trade execution cost vs midpoint; higher values mean takers paid more.',
      },
      { field: 'time', headerName: 'Updated', minWidth: 95 },
    ],
    [onTradeSymbol, onRangeOrders],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Best signals across markets"
    >
      <FeedToolbar title="Best signals across top spot markets" />
      <p className="signal-info-note">
        Listed alphabetically across the ten highest-turnover spot pairs. The
        five-minute trend is context only and does not affect the live score.
        “5m Trend / Vol” shows the move in recent volatility units. Snapshot
        refreshes every 15 seconds.
      </p>
      {state.bestError && (
        <div className="positions-error">{state.bestError}</div>
      )}
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<BestSignalRow>
          rowData={rows}
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
      {rows.length === 0 && (
        <p className="positions-empty">
          Waiting for fresh signals from the backend.
        </p>
      )}
    </section>
  );
}

type HistoryRow = SignalSnapshot & { time: string };
function SignalHistoryViewComponent({
  paused = false,
  onTradeSymbol,
}: {
  paused?: boolean;
  onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void;
}) {
  const state = useSignalInfo();
  const rows = useMemo<HistoryRow[]>(
    () =>
      state.history.map((item) => ({
        ...item,
        time: new Date(item.timestamp).toLocaleString(),
      })),
    [state.history],
  );
  const displayedRowData = usePausedValue(rows, paused);
  const columns = useMemo<ColDef<HistoryRow>[]>(
    () => [
      { field: 'time', headerName: 'Time', minWidth: 145 },
      {
        field: 'symbol',
        headerName: 'Symbol',
        minWidth: 100,
        cellRenderer: ({
          value,
          data,
        }: {
          value: unknown;
          data?: HistoryRow;
        }) => (
          <button
            type="button"
            className="signal-symbol-link"
            onClick={() =>
              data &&
              onTradeSymbol(
                data.symbol,
                data.direction === 'bearish' ? 'sell' : 'buy',
              )
            }
          >
            {String(value ?? '')}
          </button>
        ),
      },
      {
        field: 'direction',
        headerName: 'Signal',
        cellRenderer: directionBadge,
        minWidth: 100,
      },
      {
        field: 'score',
        headerName: 'Score',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => Number(value ?? 0).toFixed(2),
        minWidth: 85,
      },
      {
        field: 'components.trend_1m_percent',
        headerName: 'Trend 1m',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_1m_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 105,
      },
      {
        field: 'components.trend_5m_percent',
        headerName: 'Trend 5m',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_5m_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 105,
      },
      {
        field: 'components.trend_5m_volatility_units',
        headerName: '5m Trend / Vol',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trend_5m_volatility_units,
        valueFormatter: ({ value }) =>
          value == null ? 'N/A' : value.toFixed(2) + 'σ',
        minWidth: 95,
      },
      {
        field: 'components.momentum_10s_percent',
        headerName: 'Momentum 10s',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.momentum_10s_percent,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 115,
      },
      {
        field: 'components.trade_flow_imbalance',
        headerName: 'Trade flow',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.trade_flow_imbalance,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 100,
      },
      {
        field: 'components.book_imbalance',
        headerName: 'Book pressure',
        cellClassRules: signedValueClasses,
        valueGetter: ({ data }) => data?.components.book_imbalance,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 110,
      },
      {
        field: 'stale',
        headerName: 'Data',
        valueFormatter: ({ value }) => (value ? 'Stale' : 'Current'),
        minWidth: 85,
      },
    ],
    [],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Signal history"
    >
      <FeedToolbar title="Recent signal history" />
      {state.historyError && (
        <div className="positions-error">{state.historyError}</div>
      )}
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<HistoryRow>
          rowData={displayedRowData}
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
      {rows.length === 0 && (
        <p className="positions-empty">No stored signal snapshots yet.</p>
      )}
    </section>
  );
}

export const SignalHistoryView = memo(SignalHistoryViewComponent);

function SignalPerformanceViewComponent({
  paused = false,
  onTradeSymbol,
}: {
  paused?: boolean;
  onTradeSymbol: (symbol: string, side: 'buy' | 'sell') => void;
}) {
  const state = useSignalInfo();
  const displayedRowData = usePausedValue(state.performance, paused);
  const columns = useMemo<ColDef<SignalPerformanceRow>[]>(
    () => [
      {
        field: 'symbol',
        headerName: 'Symbol',
        minWidth: 100,
        cellRenderer: ({ value }: { value: unknown }) => (
          <button
            type="button"
            className="signal-symbol-link"
            onClick={() => onTradeSymbol(String(value ?? ''), 'buy')}
          >
            {String(value ?? '')}
          </button>
        ),
      },
      {
        field: 'horizon_seconds',
        headerName: 'Horizon',
        valueFormatter: ({ value }) => horizonLabel(value),
        minWidth: 100,
      },
      { field: 'sample_count', headerName: 'Total N', minWidth: 80 },
      {
        field: 'directional_sample_count',
        headerName: 'Directional N',
        minWidth: 105,
      },
      { field: 'neutral_sample_count', headerName: 'Neutral N', minWidth: 90 },
      {
        field: 'bullish_match_rate',
        headerName: 'Bullish hit rate',
        cellClassRules: {
          'signal-performance-good': ({ value }) =>
            typeof value === 'number' && value >= 0.5,
          'signal-performance-poor': ({ value }) =>
            typeof value === 'number' && value < 0.5,
        },
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 125,
      },
      {
        field: 'bearish_match_rate',
        headerName: 'Bearish hit rate',
        cellClassRules: {
          'signal-performance-good': ({ value }) =>
            typeof value === 'number' && value >= 0.5,
          'signal-performance-poor': ({ value }) =>
            typeof value === 'number' && value < 0.5,
        },
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 125,
      },
      {
        field: 'direction_match_rate',
        headerName: 'Directional hit rate',
        cellClassRules: {
          'signal-performance-good': ({ value }) =>
            typeof value === 'number' && value >= 0.5,
          'signal-performance-poor': ({ value }) =>
            typeof value === 'number' && value < 0.5,
        },
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 135,
      },
      { field: 'net_sample_count', headerName: 'Costed N', minWidth: 85 },
      {
        field: 'average_net_return_percent',
        headerName: 'Avg net return',
        cellClassRules: signedValueClasses,
        valueFormatter: ({ value }) => percent(value),
        minWidth: 120,
      },
    ],
    [onTradeSymbol],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Signal performance"
    >
      <FeedToolbar title="Signal outcome tracking" />
      <p className="signal-info-note">
        Direction hit rates exclude neutral signals. Older stored outcomes count
        toward Total N but lack direction and cost data. Net returns use fresh
        top-of-book entry/exit quotes and the configured taker fee; order size
        and market impact are not simulated.
      </p>
      {state.performanceError && (
        <div className="positions-error">{state.performanceError}</div>
      )}
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<SignalPerformanceRow>
          rowData={displayedRowData}
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
      {state.performance.length === 0 && (
        <p className="positions-empty">
          Cost-aware outcome samples begin after one minute; 5-minute, 15-minute
          and 1-hour rows appear as those horizons mature.
        </p>
      )}
    </section>
  );
}

export const SignalPerformanceView = memo(SignalPerformanceViewComponent);

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

export function SkippedSignalAuditView({
  paused = false,
}: {
  paused?: boolean;
}) {
  const state = useSignalInfo();
  const rows = usePausedValue(state.skippedSignalAudit, paused);
  const columns = useMemo<ColDef<SkippedSignalAuditRow>[]>(
    () => [
      {
        field: 'timestamp',
        headerName: 'Time',
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? new Date(value).toLocaleString() : '—',
        minWidth: 165,
      },
      { field: 'symbol', headerName: 'Symbol', minWidth: 110 },
      { field: 'direction', headerName: 'Direction', minWidth: 100 },
      { field: 'strategy', headerName: 'Strategy', minWidth: 140 },
      {
        field: 'score',
        headerName: 'Score',
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toFixed(3) : '—',
        minWidth: 90,
      },
      {
        field: 'strength',
        headerName: 'Strength',
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(1) + '%' : '—',
        minWidth: 100,
      },
      {
        headerName: 'Strength floor',
        valueGetter: ({ data }) => data?.entry_gate?.thresholds?.strength_floor,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(1) + '%' : '—',
        minWidth: 120,
      },
      {
        headerName: 'Score floor',
        valueGetter: ({ data }) =>
          data?.entry_gate?.thresholds?.exploration_score_absolute_floor,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toFixed(3) : '—',
        minWidth: 105,
      },
      {
        headerName: 'Market context',
        valueGetter: ({ data }) => data?.entry_gate?.market_context,
        minWidth: 125,
      },
      {
        field: 'reason',
        headerName: 'Rejection reason',
        minWidth: 250,
        flex: 2,
      },
    ],
    [],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Skipped signal audit"
    >
      <FeedToolbar title="Recent skipped signal audit" />
      <p className="signal-info-note">
        Recent skipped and rejected entry decisions, including the observed
        signal, gate floors, market context, and recorded reason.
      </p>
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<SkippedSignalAuditRow>
          rowData={rows}
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
      {rows.length === 0 && (
        <p className="positions-empty">
          No skipped or rejected decision records are available yet.
        </p>
      )}
    </section>
  );
}
