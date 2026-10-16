import { memo, useEffect, useId, useMemo, useRef } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  CellStyleModule,
  ClientSideRowModelModule,
  DateFilterModule,
  ModuleRegistry,
  NumberFilterModule,
  TextFilterModule,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { Connection } from '../connection';
import { TradesModel } from '../models/TradesModel';
import type { RecentTrade } from '../models/TradesModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([
  CellStyleModule,
  ClientSideRowModelModule,
  DateFilterModule,
  NumberFilterModule,
  TextFilterModule,
]);

type TradesViewProps = {
  symbol: string;
  connection: Connection;
  symbols?: string[];
  onSymbolChange?: (symbol: string) => void;
  streamKey?: string;
  paused?: boolean;
};

type TradeGridRow = Omit<RecentTrade, 'timestamp'> & { time: Date };

const numberFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 10,
});
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'short',
  timeStyle: 'medium',
});
const tradesGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

const defaultTradeColDef: ColDef<TradeGridRow> = {
  sortable: true,
  filter: true,
  floatingFilter: false,
  resizable: true,
  flex: 1,
  minWidth: 68,
};

export function TradesViewComponent({
  symbol,
  connection,
  symbols = [],
  onSymbolChange,
  streamKey,
  paused = false,
}: TradesViewProps) {
  const generatedKey = useId();
  const key = streamKey ?? generatedKey;
  const model = useMemo(
    () => new TradesModel({ connection, symbol, streamKey: key }),
    [],
  );
  const state = useBehaviorSnapshot(model.state$);
  const gridRowsByTrade = useRef(new WeakMap<RecentTrade, TradeGridRow>());

  useEffect(() => {
    model.updateProps({ connection, symbol, streamKey: key });
  }, [model, connection, symbol, key]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);

  const rowData = useMemo<TradeGridRow[]>(
    () => state.trades.map((trade) => {
      const existingRow = gridRowsByTrade.current.get(trade);
      if (existingRow) return existingRow;

      const { timestamp, ...tradeData } = trade;
      const row = { ...tradeData, time: new Date(timestamp) };
      gridRowsByTrade.current.set(trade, row);
      return row;
    }),
    [state.trades],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<TradeGridRow>[]>(
    () => [
      {
        field: 'time',
        headerName: 'Time',
        minWidth: 88,
        filter: 'agDateColumnFilter',
        valueFormatter: ({ value }) =>
          value instanceof Date ? timeFormatter.format(value) : '',
      },
      {
        field: 'side',
        headerName: 'Side',
        minWidth: 58,
        cellClass: ({ value }) =>
          value === 'buy' ? 'pnl-positive' : 'pnl-negative',
        valueFormatter: ({ value }) => String(value ?? '').toUpperCase(),
      },
      {
        field: 'price',
        headerName: 'Price',
        filter: 'agNumberColumnFilter',
        cellClass: ({ data }) =>
          data?.side === 'buy' ? 'pnl-positive' : 'pnl-negative',
        valueFormatter: ({ value }) =>
          value == null ? '' : numberFormatter.format(value),
      },
      {
        field: 'size',
        headerName: 'Size',
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value }) =>
          value == null ? '' : numberFormatter.format(value),
      },
    ],
    [],
  );

  return (
    <section
      className="feed-section trades-view-wrap"
      aria-label="Recent trades"
    >
      <FeedToolbar title="Recent trades">
        <label className="chart-symbol-control">
          <select
            aria-label="Trades symbol"
            value={symbol}
            onChange={(event) => onSymbolChange?.(event.target.value)}
          >
            {symbols.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </select>
        </label>
      </FeedToolbar>
      {state.error && (
        <div className="positions-error" role="alert">
          {state.error}
        </div>
      )}
      <div className="data-grid-wrap fills-grid-wrap trades-grid-wrap">
        <AgGridReact<TradeGridRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={defaultTradeColDef}
          getRowId={({ data }) => data.tradeId}
          theme={tradesGridTheme}
          domLayout="autoHeight"
          rowHeight={32}
          headerHeight={38}
          suppressColumnMoveAnimation
          animateRows={false}
          suppressHorizontalScroll
          overlayNoRowsTemplate="Waiting for the first trade…"
        />
      </div>
    </section>
  );
}

export const TradesView = memo(TradesViewComponent);
