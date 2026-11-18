import { useMemo } from 'react';
import { memo } from 'react';
import {
  gridTheme,
  percent,
  ratio,
  signedValueClasses,
  directionBadge,
  useSignalInfo,
} from './SignalInfoViewShared';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { SignalSnapshot } from '../models/SignalsModel';

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
