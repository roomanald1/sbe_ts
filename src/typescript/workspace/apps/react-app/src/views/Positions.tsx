import { useEffect, useMemo } from 'react';
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
  symbol: string;
  side: TradeSide;
  remaining: number;
  limitPrice: number;
  current?: number;
  quote: string;
  bookError?: string;
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
  return (
    <button
      className="positions-cancel-button"
      type="button"
      onClick={() => void model.cancelOrder(data.id)}
    >
      Cancel
    </button>
  );
}

export function Positions({
  connection,
  symbols,
}: {
  connection: Connection;
  symbols: string[];
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
    () =>
      state.rows.map(({ order, current: mark, bookError }) => ({
        id: order.id,
        symbol: order.symbol,
        side: order.side,
        remaining: order.remaining_quantity,
        limitPrice: order.limit_price,
        current: mark,
        quote: quoteCurrency(order.symbol),
        bookError,
      })),
    [state.rows],
  );
  const columnDefs = useMemo<ColDef<PositionGridRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Symbol', minWidth: 90 },
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
        field: 'remaining',
        headerName: 'Remaining',
        minWidth: 68,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value }) =>
          value === undefined ? '' : quantityFormatter.format(value),
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
        field: 'current',
        headerName: 'Current',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value === undefined
            ? data?.bookError
              ? 'Unavailable'
              : 'Loading…'
            : moneyFormatter.format(value) + ' ' + (data?.quote ?? ''),
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
              model.setOrderType(event.target.value as 'ioc' | 'limit')
            }
          >
            <option value="ioc">Immediate or cancel</option>
            <option value="limit">Limit</option>
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
        {state.orderType === 'limit' && (
          <label>
            <span className="limit-price-label">
              <span>LIMIT PRICE</span>
              <small className="current-instrument-price">
                MARK{' '}
                {current === undefined
                  ? state.bookErrors[selectedSymbol]
                    ? 'Unavailable'
                    : 'Loading…'
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
          </label>
        )}
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
            ? 'Submitting…'
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
          rowData={rowData}
          columnDefs={columnDefs}
          defaultColDef={defaultPositionColDef}
          getRowId={({ data }) => data.id}
          theme={positionsGridTheme}
          domLayout="autoHeight"
          rowHeight={34}
          headerHeight={38}
          suppressColumnMoveAnimation
        />
        {!state.loading && state.orders.length === 0 && (
          <p className="positions-empty">No resting orders.</p>
        )}
        {state.loading && state.orders.length === 0 && (
          <p className="positions-empty">Loading resting orders…</p>
        )}
      </div>
    </section>
  );
}
