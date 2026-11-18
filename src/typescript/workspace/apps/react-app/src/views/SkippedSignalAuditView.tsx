import { useMemo } from 'react';
import { gridTheme, useSignalInfo } from './SignalInfoViewShared';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { SkippedSignalAuditRow } from '../models/SignalInfoModel';

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
