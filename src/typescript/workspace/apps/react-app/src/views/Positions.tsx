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
  type ICellRendererParams,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { Connection } from '../connection';
import { OrderManagerModel } from '../models/OrderManagerModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([
  ClientSideRowModelModule,
  CellStyleModule,
  NumberFilterModule,
  TextFilterModule,
]);

type TradeSide = 'buy' | 'sell';
const moneyFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 8,
});
const quantityFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 0,
  maximumFractionDigits: 8,
});
const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';

type PositionGridRow = {
  id: string;
  kind: 'order' | 'vwap';
  status: string;
  symbol: string;
  side: TradeSide;
  executed: number;
  remaining: number;
  limitPrice?: number;
  estimatedFee?: number;
  current?: number;
  quote: string;
  bookError?: string;
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
  const selectedSymbol = state.selectedSymbol;
  const current = state.selectedCurrentPrice;
  const rowData = useMemo<PositionGridRow[]>(
    () => [
      ...state.rows.map(({ order, current: mark, bookError }) => ({
        id: order.id,
        kind: 'order' as const,
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
      ...state.vwapRuns.slice(-20).map((run) => ({
          id: run.id,
          kind: 'vwap' as const,
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
    [state.rows, state.vwapRuns],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<PositionGridRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Symbol', minWidth: 90 },
      { field: 'status', headerName: 'Status', minWidth: 105 },
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
        valueFormatter: ({ value }) =>
          value === undefined ? '' : quantityFormatter.format(value),
      },
      {
        field: 'remaining',
        headerName: 'Remaining',
        minWidth: 68,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value }) =>
          value === undefined ? '' : quantityFormatter.format(value),
      },
      {
        field: 'progress',
        headerName: 'Filled',
        minWidth: 88,
        valueFormatter: ({ value, data }) => data?.kind === 'vwap' && value != null ? value.toFixed(1) + '%' : '',
      },
      {
        field: 'etaMinutes',
        headerName: 'Pace ETA',
        minWidth: 82,
        valueFormatter: ({ value, data }) => data?.kind === 'vwap' && value != null && Number.isFinite(value) ? Math.ceil(value) + ' min' : data?.kind === 'vwap' ? '—' : '',
      },
      {
        field: 'executionVwap',
        headerName: 'Exec VWAP',
        minWidth: 92,
        valueFormatter: ({ value, data }) => data?.kind === 'vwap' && value != null ? moneyFormatter.format(value) + ' ' + data.quote : '',
      },
      {
        field: 'marketVwap',
        headerName: 'Market VWAP',
        minWidth: 92,
        valueFormatter: ({ value, data }) => data?.kind === 'vwap' && value != null ? moneyFormatter.format(value) + ' ' + data.quote : '',
      },
      {
        field: 'slippage',
        headerName: 'VWAP delta',
        minWidth: 92,
        valueFormatter: ({ value, data }) => data?.kind === 'vwap' && value != null ? moneyFormatter.format(value) + ' ' + data.quote : '',
      },
      {
        field: 'limitPrice',
        headerName: 'Limit',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value === undefined
            ? ''
            : moneyFormatter.format(value) + ' ' + (data?.quote ?? ''),
      },
      {
        field: 'estimatedFee',
        headerName: 'Est. fee',
        minWidth: 80,
        valueFormatter: ({ value, data }) => value == null || data?.kind !== 'order' ? '' : moneyFormatter.format(value) + ' ' + data.quote,
      },      {
        field: 'current',
        headerName: 'Current',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          data?.kind !== 'order'
            ? ''
            : value === undefined
              ? data.bookError
                ? 'Unavailable'
                : 'Loading...'
              : moneyFormatter.format(value) + ' ' + (data.quote ?? ''),
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
    [model],
  );
  return (
    <section className="feed-section positions-page" aria-label="Orders">
      <FeedToolbar title="Orders" />
      <form
        className="paper-trade-form"
        aria-label="Place order"
        onSubmit={(event) => {
          event.preventDefault();
          void model.submit();
        }}
      >
        <label>
          <span>INSTRUMENT</span>
          <select
            value={selectedSymbol}
            onChange={(event) => model.setSymbol(event.target.value)}
            disabled={!state.symbols.length}
          >
            {state.symbols.map((symbol) => (
              <option key={symbol} value={symbol}>
                {symbol}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>SIDE</span>
          <select
            value={state.side}
            onChange={(event) => model.setSide(event.target.value as TradeSide)}
          >
            <option value="buy">Buy</option>
            <option value="sell">Sell</option>
          </select>
        </label>
        <label>
          <span>ORDER TYPE</span>
          <select
            value={state.orderType}
            onChange={(event) =>
              model.setOrderType(event.target.value as 'ioc' | 'limit' | 'vwap')
            }
          >
            <option value="ioc">Immediate or cancel</option>
            <option value="limit">Limit</option>
            <option value="vwap">VWAP execution</option>
          </select>
        </label>
        <label>
          <span>QUANTITY</span>
          <input
            type="number"
            min="0.00000001"
            step="any"
            required
            value={state.quantity}
            onChange={(event) => model.setQuantity(event.target.value)}
            placeholder="0.00"
          />
        </label>
        {state.orderType === 'vwap' && (
          <>
            <label>
              <span>DURATION (MINUTES)</span>
              <input
                type="number"
                min="5"
                max="390"
                step="1"
                required
                value={state.durationMinutes}
                onChange={(event) =>
                  model.setDurationMinutes(event.target.value)
                }
              />
            </label>
            <label>
              <span>MAX MARKET PARTICIPATION (%)</span>
              <input
                type="number"
                min="1"
                max="100"
                step="1"
                required
                value={state.participationRate}
                onChange={(event) =>
                  model.setParticipationRate(event.target.value)
                }
              />
            </label>
          </>
        )}
        {state.orderType === 'limit' && (
          <label>
            <span className="limit-price-label">
              <span>LIMIT PRICE</span>
              <small className="current-instrument-price">
                MARK{' '}
                {current === undefined
                  ? state.bookErrors[selectedSymbol]
                    ? 'Unavailable'
                    : 'Loading...'
                  : moneyFormatter.format(current) +
                    ' ' +
                    quoteCurrency(selectedSymbol)}
              </small>
            </span>
            <input
              type="number"
              min="0.00000001"
              step="any"
              required
              value={state.limitPrice}
              onChange={(event) => model.setLimitPrice(event.target.value)}
              placeholder="0.00"
            />
            {state.limitPriceWarning && (
              <small className="limit-price-warning" role="alert">
                {state.limitPriceWarning}
              </small>
            )}
          </label>
        )}
        <p className="order-fee-estimate">Estimated fee ({((state.orderType === "limit" ? state.makerFeeRate : state.takerFeeRate) * 100).toFixed(3)}%): {state.estimatedFee === undefined ? "Waiting for price" : moneyFormatter.format(state.estimatedFee) + " " + quoteCurrency(selectedSymbol)}</p>
        <button
          className={'paper-trade-submit is-' + state.side}
          type="submit"
          disabled={
            state.submitting ||
            !state.quantity ||
            (state.orderType === 'limit' && !state.limitPrice)
          }
        >
          {state.submitting
            ? 'Submitting...'
            : state.limitPriceWarning &&
                state.confirmedLimitPrice === state.limitPrice
              ? 'Confirm price and ' + state.side
              : state.side === 'buy'
                ? 'Buy'
                : 'Sell'}
        </button>
      </form>
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
          getRowId={({ data }) => data.id}
          theme={positionsGridTheme}
          domLayout="autoHeight"
          rowHeight={34}
          headerHeight={38}
          suppressColumnMoveAnimation
        />
        {!state.loading &&
          state.orders.length === 0 &&
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
