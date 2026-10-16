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
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import { PnlModel } from '../models/PnlModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import type { Connection } from '../connection';

ModuleRegistry.registerModules([
  ClientSideRowModelModule,
  CellStyleModule,
  NumberFilterModule,
  TextFilterModule,
]);

const moneyFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 8,
});
const quantityFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 0,
  maximumFractionDigits: 8,
});
const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';

const pnlGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

type PnlGridRow = {
  symbol: string;
  quantity: number;
  averageEntry?: number;
  mark?: number;
  realized?: number;
  unrealized?: number;
  feesPaid?: number;
  returnPct?: number;
  lastTrade: number;
  markTimestamp?: number;
  quote: string;
  bookError?: string;
};

const defaultPnlColDef: ColDef<PnlGridRow> = {
  sortable: true,
  filter: true,
  floatingFilter: false,
  resizable: true,
  flex: 1,
  minWidth: 68,
};

function PnlComponent({ connection, paused = false }: { connection: Connection; paused?: boolean }) {
  const model = useMemo(() => new PnlModel(connection), []);
  useEffect(() => {
    model.setConnection(connection);
  }, [model, connection]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const { rows, totals, currencies, loading, error } = useBehaviorSnapshot(
    model.state$,
  );
  const rowData = useMemo<PnlGridRow[]>(
    () => rows.map(({ position, mark, unrealized, realizedUsd, unrealizedUsd, feesPaidUsd, averageEntryUsd, markTimestamp, bookError }) => ({
        symbol: position.symbol,
        quantity: position.quantity,
        averageEntry:
          position.quantity === 0 ? undefined : averageEntryUsd,
        mark,
        realized: realizedUsd,
        unrealized: unrealizedUsd,
        feesPaid: feesPaidUsd,
        returnPct:
          position.average_entry_price > 0 && unrealized !== undefined
            ? (unrealized / (Math.abs(position.quantity) * position.average_entry_price)) * 100
            : undefined,
        lastTrade: position.last_trade_timestamp,
        markTimestamp,
        quote: quoteCurrency(position.symbol),
        bookError,
      })),
    [rows],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<PnlGridRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Instrument', minWidth: 90 },
      { field: 'feesPaid', headerName: 'Fees paid', minWidth: 90, valueFormatter: ({ value }) => value == null ? 'N/A' : moneyFormatter.format(value) + ' USD' },
      { field: 'returnPct', headerName: 'Return %', minWidth: 75, valueFormatter: ({ value }) => value == null ? '—' : value.toFixed(2) + '%' },
      { field: 'lastTrade', headerName: 'Last trade', minWidth: 75, valueFormatter: ({ value }) => value ? Math.max(0, Math.floor((Date.now() - value) / 86400000)) + 'd' : '—' },
      { field: 'markTimestamp', headerName: 'Mark age', minWidth: 85, valueFormatter: ({ value }) => value ? Math.max(0, Math.floor((Date.now() - value) / 1000)) + 's' : '—' },
      {
        field: 'quantity',
        headerName: 'Net qty',
        minWidth: 70,
        filter: 'agNumberColumnFilter',
        cellClass: ({ value }) =>
          value == null || value === 0
            ? undefined
            : value > 0
              ? 'pnl-positive'
              : 'pnl-negative',
        valueFormatter: ({ value }) =>
          value == null ? '' : quantityFormatter.format(value),
      },
      {
        field: 'averageEntry',
        headerName: 'Avg entry (USD)',
        minWidth: 85,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value == null
            ? '—'
            : moneyFormatter.format(value) + ' USD / ' + (data?.symbol.split('-')[0] ?? 'unit'),
      },
      {
        field: 'mark',
        headerName: 'Mark',
        minWidth: 75,
        filter: 'agNumberColumnFilter',
        valueFormatter: ({ value, data }) =>
          value == null
            ? data?.quantity === 0
              ? '—'
              : data?.bookError
                ? 'Unavailable'
                : 'Loading…'
            : moneyFormatter.format(value) + ' ' + (data?.quote ?? ''),
      },
      {
        field: 'realized',
        headerName: 'Realized P&L',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        cellClass: ({ value }) =>
          value == null || value === 0
            ? undefined
            : value > 0
              ? 'pnl-positive'
              : 'pnl-negative',
        valueFormatter: ({ value }) =>
          value == null
            ? ''
            : moneyFormatter.format(value) + ' USD',
      },
      {
        field: 'unrealized',
        headerName: 'Unrealized P&L',
        minWidth: 90,
        filter: 'agNumberColumnFilter',
        cellClass: ({ value }) =>
          value == null || value === 0
            ? undefined
            : value > 0
              ? 'pnl-positive'
              : 'pnl-negative',
        valueFormatter: ({ value }) =>
          value == null
            ? '—'
            : moneyFormatter.format(value) + ' USD',
      },
    ],
    [],
  );

  return (
    <section className="feed-section pnl-panel" aria-label="Profit and loss">
      <FeedToolbar title="P&L" />
      {error && (
        <div className="positions-error" role="alert">
          {error}
        </div>
      )}
      {currencies.length > 0 && (
        <div className="positions-totals">
          {currencies.map((currency) => {
            const total = totals[currency];
            const combined = total.realized + total.unrealized;
            return (
              <div className="position-total-card" key={currency}>
                <span>USD P&amp;L</span>
                <strong
                  className={combined >= 0 ? 'pnl-positive' : 'pnl-negative'}
                >
                  {total.hasUnconvertedPnl ? 'Unavailable' : moneyFormatter.format(combined) + ' USD'}
                </strong>
                <small>
                  Fees {moneyFormatter.format(total.feesPaid)} USD - Realized {moneyFormatter.format(total.realized)} USD · Unrealized{' '}
                  {total.hasUnmarkedExposure
                    ? 'Loading mark…'
                    : moneyFormatter.format(total.unrealized) + ' USD'}
                  {total.hasUnconvertedPnl && ' · Missing OKX USD rate: ' + total.missingCurrencies.join(', ')}
                </small>
              </div>
            );
          })}
        </div>
      )}
      <div className="data-grid-wrap pnl-grid-wrap pnl-exposure-wrap">
        <AgGridReact<PnlGridRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={defaultPnlColDef}
          getRowId={({ data }) => data.symbol}
          theme={pnlGridTheme}
          domLayout="autoHeight"
          rowHeight={32}
          headerHeight={38}
          suppressColumnMoveAnimation
        />
        {!loading && rows.length === 0 && (
          <p className="positions-empty">
            No filled exposure or realized P&amp;L yet.
          </p>
        )}
        {loading && (
          <p className="positions-empty">Loading fill-based P&amp;L…</p>
        )}
      </div>
    </section>
  );
}

export const Pnl = memo(PnlComponent);
