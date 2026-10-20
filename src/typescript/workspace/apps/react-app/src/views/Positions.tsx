import { formatForSymbol } from '../instrument_precision';
import { memo, useEffect, useMemo } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  CellStyleModule,
  ModuleRegistry,
  NumberFilterModule,
  TextFilterModule,
  themeQuartz,
  type ColDef,
  type ColGroupDef,
  type ICellRendererParams,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { Connection } from '../connection';
import { OrderManagerModel } from '../models/OrderManagerModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { getOrderTypeDescription } from '../order_type_descriptions';

ModuleRegistry.registerModules([
  ClientSideRowModelModule,
  CellStyleModule,
  NumberFilterModule,
  TextFilterModule,
]);

type TradeSide = 'buy' | 'sell';
const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';

type PositionGridRow = {
  id: string;
  kind: 'order' | 'vwap' | 'barrier';
  orderType: string;
  expiresAt?: number;
  orderTimeInForce?: 'gtc' | 'gtd';
  status: string;
  symbol: string;
  side: TradeSide;
  executed: number;
  remaining: number;
  limitPrice?: number;
  triggerDirection?: 'above' | 'below';
  executionType?: 'market' | 'limit';
  activationLimit?: number;
  estimatedFee?: number;
  current?: number;
  quote: string;
  bookError?: string;
  error?: string;
  progress?: number;
  etaMinutes?: number;
  executionVwap?: number;
  marketVwap?: number;
  slippage?: number;
};

const positionsGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

const defaultPositionColDef: ColDef<PositionGridRow> = {
  sortable: true,
  filter: true,
  resizable: true,
  floatingFilter: false,
  flex: 1,
  minWidth: 68,
};

function CancelOrderCell({
  data,
  model,
}: ICellRendererParams<PositionGridRow> & { model: OrderManagerModel }) {
  if (!data) return null;
  if (data.kind === 'vwap' && data.status !== 'running') return null;
  if (data.kind === 'barrier' && data.status !== 'armed') return null;
  return (
    <button
      className="positions-cancel-button"
      type="button"
      onClick={() =>
        void (data.kind === 'vwap'
          ? model.cancelVwap(data.id)
          : model.cancelOrder(data.id))
      }
    >
      Cancel
    </button>
  );
}

function PositionsComponent({
  connection,
  symbols,
  paused = false,
}: {
  connection: Connection;
  symbols: string[];
  paused?: boolean;
}) {
  const model = useMemo(
    () => new OrderManagerModel({ connection, symbols }),
    [],
  );
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.updateProps({ connection, symbols });
  }, [model, connection, symbols]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const rowData = useMemo<PositionGridRow[]>(
    () => [
      ...state.rows.map(({ order, current: mark, bookError }) => ({
        id: order.id,
        kind: 'order' as const,
        orderType: order.post_only ? 'Post-only limit' : order.time_in_force === 'gtd' ? 'GTD limit' : 'Limit',
        expiresAt: order.expires_at,
        orderTimeInForce: order.time_in_force,
        status: 'Resting',
        symbol: order.symbol,
        side: order.side,
        executed: order.quantity - order.remaining_quantity,
        remaining: order.remaining_quantity,
        limitPrice: order.limit_price,
        estimatedFee: order.estimated_fee,
        current: mark,
        quote: quoteCurrency(order.symbol),
        bookError,
      })),
      ...state.barrierOrders.map((order) => ({
        id: order.id,
        kind: 'barrier' as const,
        orderType: order.order_kind || 'Barrier',
        status: order.status,
        error: order.error,
        symbol: order.symbol,
        side: order.side,
        executed: order.filled_quantity,
        remaining: order.remaining_quantity,
        limitPrice: order.trigger_price,
        triggerDirection: order.trigger_direction,
        executionType: order.execution_type,
        activationLimit: order.limit_price,
        current: state.books[order.symbol]?.bids[0] && state.books[order.symbol]?.asks[0]
          ? ((state.books[order.symbol]!.bids[0].price + state.books[order.symbol]!.asks[0].price) / 2)
          : (state.books[order.symbol]?.bids[0]?.price ?? state.books[order.symbol]?.asks[0]?.price),
        quote: quoteCurrency(order.symbol),
        bookError: state.bookErrors[order.symbol],
      })),
      ...state.vwapRuns.slice(-20).map((run) => ({
          id: run.id,
          kind: 'vwap' as const,
          orderType: run.strategy === 'twap' ? 'TWAP' : 'VWAP',
          status: run.status,
          symbol: run.symbol,
          side: run.side,
          executed: run.executed_quantity,
          remaining: Math.max(run.quantity - run.executed_quantity, 0),
          progress: run.quantity > 0 ? Math.min(run.executed_quantity / run.quantity, 1) * 100 : 0,
          etaMinutes: run.executed_quantity > 0 && run.started_at < Date.now()
            ? (Math.max(run.quantity - run.executed_quantity, 0) / (run.executed_quantity / ((Date.now() - run.started_at) / 60000)))
            : undefined,
          executionVwap: run.executed_quantity > 0 ? run.execution_vwap : undefined,
          marketVwap: run.executed_quantity > 0 ? run.market_vwap : undefined,
          slippage: run.executed_quantity > 0 ? run.slippage : undefined,
          quote: quoteCurrency(run.symbol),
        })),
    ],
    [state.rows, state.barrierOrders, state.vwapRuns, state.books, state.bookErrors],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<PositionGridRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Symbol', minWidth: 90 },
      { field: 'orderType', headerName: 'Type', minWidth: 92, tooltipValueGetter: ({ value }) => getOrderTypeDescription(value) },
      { field: 'status', headerName: 'Status', minWidth: 105, tooltipField: 'error' },
      {
        field: 'side',
        headerName: 'Side',
        minWidth: 60,
        filter: 'agTextColumnFilter',
        cellClass: ({ value }) =>
          value === 'buy' ? 'pnl-positive' : 'pnl-negative',
        valueFormatter: ({ value }) => String(value ?? '').toUpperCase(),
      },
      {
        field: 'executed',
        headerName: 'Executed',
        minWidth: 68,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value === undefined ? '' : formatForSymbol(value, data?.symbol ?? '', 'quantity'),
      },
      {
        field: 'remaining',
        headerName: 'Remaining',
        minWidth: 68,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value === undefined ? '' : formatForSymbol(value, data?.symbol ?? '', 'quantity'),
      },
      ...(state.vwapRuns.length > 0 ? [{
        headerName: 'VWAP',
        marryChildren: true,
        children: [
          {
            field: 'progress',
            headerName: 'Filled',
            minWidth: 88,
            valueFormatter: ({ value, data }: { value: number | undefined; data: PositionGridRow | undefined }) => data?.kind === 'vwap' && value != null ? value.toFixed(1) + '%' : '',
          },
          {
            field: 'etaMinutes',
            headerName: 'Pace ETA',
            minWidth: 82,
            valueFormatter: ({ value, data }: { value: number | undefined; data: PositionGridRow | undefined }) => data?.kind === 'vwap' && value != null && Number.isFinite(value) ? Math.ceil(value) + ' min' : data?.kind === 'vwap' ? '—' : '',
          },
          {
            field: 'executionVwap',
            headerName: 'Exec VWAP',
            minWidth: 92,
            valueFormatter: ({ value, data }: { value: number | undefined; data: PositionGridRow | undefined }) => data?.kind === 'vwap' && value != null ? formatForSymbol(value, data.symbol, 'price') + ' ' + data.quote : '',
          },
          {
            field: 'marketVwap',
            headerName: 'Market VWAP',
            minWidth: 92,
            valueFormatter: ({ value, data }: { value: number | undefined; data: PositionGridRow | undefined }) => data?.kind === 'vwap' && value != null ? formatForSymbol(value, data.symbol, 'price') + ' ' + data.quote : '',
          },
          {
            field: 'slippage',
            headerName: 'VWAP delta',
            minWidth: 92,
            valueFormatter: ({ value, data }: { value: number | undefined; data: PositionGridRow | undefined }) => data?.kind === 'vwap' && value != null ? formatForSymbol(value, data.symbol, 'price') + ' ' + data.quote : '',
          },
        ],
      } satisfies ColGroupDef<PositionGridRow>] : []),
      {
        field: 'expiresAt',
        headerName: 'Expires',
        minWidth: 105,
        valueFormatter: ({ value, data }) => data?.kind === 'order' && value ? new Date(value).toLocaleString() : '',
      },
      {
        field: 'limitPrice',
        headerName: 'Limit / barrier',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value === undefined
            ? ''
            : (data?.kind === 'barrier' ? (data.triggerDirection === 'above' ? 'Above ' : 'Below ') : '') + formatForSymbol(value, data?.symbol ?? '', 'price') + (data?.kind === 'barrier' && data.executionType === 'limit' && data.activationLimit ? ' → limit ' + formatForSymbol(data.activationLimit, data.symbol, 'price') : '') + ' ' + (data?.quote ?? ''),
      },
      {
        field: 'estimatedFee',
        headerName: 'Est. fee',
        minWidth: 80,
        valueFormatter: ({ value, data }) => value == null || data?.kind !== 'order' ? '' : formatForSymbol(value, data.symbol, 'price') + ' ' + data.quote,
      },
      {
        field: 'current',
        headerName: 'Current',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          !data || data.kind === 'vwap'
            ? ''
            : value === undefined
              ? data.bookError
                ? 'Unavailable'
                : 'Loading...'
              : formatForSymbol(value, data.symbol, 'price') + ' ' + (data.quote ?? ''),
      },
      {
        colId: 'actions',
        headerName: '',
        cellRenderer: CancelOrderCell,
        cellRendererParams: { model },
        sortable: false,
        filter: false,
        floatingFilter: false,
        flex: 0,
        width: 64,
        minWidth: 64,
        maxWidth: 64,
      },
    ],
    [model, state.vwapRuns.length],
  );
  return (
    <section className="feed-section positions-page" aria-label="Orders">
      <FeedToolbar title="Orders" />
      {state.error && (
        <div className="positions-error" role="alert">
          {state.error}
        </div>
      )}
      <div className="data-grid-wrap positions-grid-wrap">
        <AgGridReact<PositionGridRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={defaultPositionColDef}
          className="orders-grid"
          getRowId={({ data }) => data.id}
          theme={positionsGridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={26}
          groupHeaderHeight={18}
          suppressColumnMoveAnimation
        />
        {!state.loading &&
          state.orders.length === 0 &&
          state.barrierOrders.length === 0 &&
          state.vwapRuns.length === 0 && (
            <p className="positions-empty">
              No open orders or VWAP executions.
            </p>
          )}
        {state.loading &&
          state.orders.length === 0 &&
          state.vwapRuns.length === 0 && (
            <p className="positions-empty">Loading orders...</p>
          )}
      </div>
    </section>
  );
}

export const Positions = memo(PositionsComponent);
