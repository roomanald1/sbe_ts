import { memo, useEffect, useMemo } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  CellStyleModule,
  DateFilterModule,
  ModuleRegistry,
  NumberFilterModule,
  TextFilterModule,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { FillsModel } from '../models/FillsModel';
import type { PositionTrade } from '../models/PositionTypes';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

ModuleRegistry.registerModules([
  ClientSideRowModelModule,
  CellStyleModule,
  DateFilterModule,
  NumberFilterModule,
  TextFilterModule,
]);

const numberFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 8,
});
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'short',
  timeStyle: 'short',
});
const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';

const fillsGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

type FillGridRow = PositionTrade & {
  time: Date;
  value: number;
  quote: string;
};

const defaultFillColDef: ColDef<FillGridRow> = {
  sortable: true,
  filter: true,
  floatingFilter: false,
  resizable: true,
  flex: 1,
  minWidth: 60,
};

function FillsComponent({ paused = false }: { paused?: boolean }) {
  const model = useMemo(() => new FillsModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const { fills, error, loading } = useBehaviorSnapshot(model.state$);
  const rowData = useMemo<FillGridRow[]>(
    () =>
      fills.map((fill) => ({
        ...fill,
        time: new Date(fill.timestamp),
        value: fill.quantity * fill.execution_price,
        quote: quoteCurrency(fill.symbol),
      })),
    [fills],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<FillGridRow>[]>(
    () => [
      {
        field: 'time',
        headerName: 'Time',
        minWidth: 90,
        filter: 'agDateColumnFilter',
        valueFormatter: ({ value }) =>
          value ? timeFormatter.format(value) : '',
      },
      { field: 'symbol', headerName: 'Symbol', minWidth: 90 },
      {
        field: 'side',
        headerName: 'Side',
        minWidth: 56,
        cellClass: ({ value }) =>
          value === 'buy' ? 'pnl-positive' : 'pnl-negative',
        valueFormatter: ({ value }) => String(value ?? '').toUpperCase(),
      },
      {
        field: 'quantity',
        headerName: 'Qty',
        headerTooltip: 'Quantity',
        minWidth: 60,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value }) =>
          value == null ? '' : numberFormatter.format(value),
      },
      {
        field: 'execution_price',
        headerName: 'Price',
        headerTooltip: 'Execution price',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value == null
            ? ''
            : numberFormatter.format(value) + ' ' + (data?.quote ?? ''),
      },
      {
        field: 'fee',
        headerName: 'Fee',
        minWidth: 80,
        valueFormatter: ({ value, data }) => value == null ? 'N/A' : numberFormatter.format(value) + ' ' + (data?.fee_currency || data?.quote || ''),
      },
      {
        field: 'fee_liquidity',
        headerName: 'Liquidity',
        minWidth: 78,
        valueFormatter: ({ value }) => value ? String(value).toUpperCase() : 'N/A',
      },
      {
        field: 'value',
        headerName: 'Value',
        headerTooltip: 'Quantity multiplied by execution price',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value == null
            ? ''
            : numberFormatter.format(value) + ' ' + (data?.quote ?? ''),
      },
    ],
    [],
  );

  return (
    <section className="feed-section fills-panel" aria-label="Order fills">
      <FeedToolbar title="Fills">
        <span className="toolbar-meta">{fills.length} FILLS</span>
      </FeedToolbar>
      {error && (
        <div className="positions-error" role="alert">
          {error}
        </div>
      )}
      <div className="data-grid-wrap fills-grid-wrap fills-table-wrap">
        <AgGridReact<FillGridRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={defaultFillColDef}
          getRowId={({ data }) => data.id}
          theme={fillsGridTheme}
          domLayout="autoHeight"
          rowHeight={32}
          headerHeight={38}
          suppressColumnMoveAnimation
        />
        {!loading && fills.length === 0 && (
          <p className="positions-empty">No fills recorded.</p>
        )}
        {loading && fills.length === 0 && (
          <p className="positions-empty">Loading fills…</p>
        )}
      </div>
    </section>
  );
}

export const Fills = memo(FillsComponent);
