import { useMemo } from 'react';
import { memo } from 'react';
import {
  gridTheme,
  percent,
  ratio,
  horizonLabel,
  signedValueClasses,
  useSignalInfo,
} from './SignalInfoViewShared';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { SignalPerformanceRow } from '../models/SignalInfoModel';

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
