import { useMemo } from 'react';
import {
  gridTheme,
  percent,
  ratio,
  signedValueClasses,
  directionBadge,
  strengthRenderer,
  useSignalInfo,
} from './SignalInfoViewShared';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { SignalSnapshot } from '../models/SignalsModel';

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
