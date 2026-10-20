import { formatForSymbol } from '../instrument_precision';
import { memo, useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  CellStyleModule,
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
} from 'ag-grid-community';
import {
  SymbolPriceTableModel,
  type SymbolPriceTableProps,
} from '../models/SymbolPriceTableModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import type { SymbolPriceTick } from '../types/symbol_price';
import type { PriceHistorySample } from '../price_change_data';
import { PriceHistoryPopup } from './PriceHistoryPopup';

ModuleRegistry.registerModules([ClientSideRowModelModule, CellStyleModule]);

const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});
const tickGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

type TickGridRow = SymbolPriceTick & { receivedAt: number };

function SymbolCellRenderer({
  value,
  data,
  model,
}: ICellRendererParams<TickGridRow, string> & {
  model: SymbolPriceTableModel;
}) {
  if (!value) return null;
  return (
    <button
      className="symbol-tag symbol-book-button"
      type="button"
      onClick={() => model.selectSymbol(value, data?.direction === 'down' ? 'sell' : 'buy')}
      disabled={!model.getSnapshot().onSelectSymbol}
      aria-label={'Open ' + (data?.direction === 'down' ? 'sell' : 'buy') + ' order for ' + value}
    >
      {value}
    </button>
  );
}

function SparklineCellRenderer({
  value,
  model,
  onShowTrend,
}: ICellRendererParams<TickGridRow, string> & {
  model: SymbolPriceTableModel;
  onShowTrend: (symbol: string) => void;
}) {
  if (!value) return null;
  const history = model.getSnapshot().histories.get(value)?.slice(-60) ?? [];
  if (history.length < 2) return <span className="sparkline-empty">N/A</span>;

  const prices = history.map((sample: PriceHistorySample) => sample.price);
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const range = max - min || 1;
  const points = prices
    .map((price, index) => `${(index / (prices.length - 1)) * 64},${18 - ((price - min) / range) * 16}`)
    .join(' ');
  const direction = prices.at(-1)! >= prices[0] ? 'up' : 'down';

  return (
    <button className="trend-cell-button" type="button" aria-label={'View ' + value + ' price chart'} title="View price chart" onClick={() => onShowTrend(value)}>
    <svg className={`pair-sparkline sparkline-${direction}`} viewBox="0 0 64 20" role="img" aria-label={`${value} recent price trend`}>
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
    </button>
  );
}
export const SymbolPriceTable = memo(function SymbolPriceTable(props: SymbolPriceTableProps) {
  const model = useMemo(() => new SymbolPriceTableModel(props), []);
  useEffect(() => {
    model.setProps(props);
  }, [model, props.ticks, props.state, props.onSelectSymbol, props.maxRows, props.histories]);
  const view = useBehaviorSnapshot(model.state$);
  const [trendSymbol, setTrendSymbol] = useState<string>();
  const rowData = view.visibleTicks as TickGridRow[];
  const columnDefs = useMemo<ColDef<TickGridRow>[]>(
    () => [
      {
        field: 'symbol',
        headerName: 'PAIR',
        minWidth: 90,
        cellRenderer: SymbolCellRenderer,
        cellRendererParams: { model },
      },
      {
        field: 'price',
        headerName: 'PRICE',
        minWidth: 90,
        cellClass: ({ data }) => data ? 'price-cell price-' + data.direction : undefined,
        valueFormatter: ({ value, data }) => value == null ? '' : formatForSymbol(value, data?.symbol ?? '', 'price'),
      },
      {
        field: 'symbol',
        headerName: 'TREND',
        colId: 'trend',
        minWidth: 76,
        maxWidth: 88,
        sortable: false,
        cellRenderer: SparklineCellRenderer,
        cellRendererParams: { model, onShowTrend: setTrendSymbol },
      },
      {
        field: 'volatility1m',
        headerName: 'VOL 1M',
        headerTooltip: 'Blue bar fills at 0.5% realized volatility over one minute.',
        minWidth: 100,
        cellStyle: ({ value }) => {
          if (value == null) return undefined;
          const fill = Math.max(0, Math.min(100, (Number(value) / 0.5) * 100));
          return { background: `linear-gradient(90deg, rgba(101, 174, 230, 0.18) ${fill}%, transparent ${fill}%)` };
        },
        valueFormatter: ({ value }) => value == null ? 'N/A' : Number(value).toFixed(3) + '%',
      },
      {
        field: 'volatility5m',
        headerName: 'VOL 5M',
        headerTooltip: 'Orange bar fills at 1.0% realized volatility over five minutes.',
        minWidth: 100,
        sort: 'desc',
        cellStyle: ({ value }) => {
          if (value == null) return undefined;
          const fill = Math.max(0, Math.min(100, Number(value) * 100));
          return { background: `linear-gradient(90deg, rgba(245, 166, 85, 0.2) ${fill}%, transparent ${fill}%)` };
        },
        valueFormatter: ({ value }) => value == null ? 'N/A' : Number(value).toFixed(3) + '%',
      },
      {
        field: 'receivedAt',
        headerName: 'UPDATED',
        minWidth: 112,
        valueFormatter: ({ value }) => value == null ? '' : timeFormatter.format(value),
      },
    ],
    [model],
  );
  return (
    <section
      className={
        view.maxRows === undefined
          ? 'feed-section'
          : 'feed-section compact-market-panel'
      }
      aria-label="Pair volatility ranking"
    >
      <FeedToolbar title="Pair volatility">
        <span className="toolbar-meta">RANKED BY 5M REALIZED VOL</span>
      </FeedToolbar>
      <div className="data-grid-wrap tick-grid-wrap">
        <AgGridReact<TickGridRow>
          rowData={rowData}
          columnDefs={columnDefs}
          defaultColDef={{ sortable: true, resizable: true, flex: 1, minWidth: 68 }}
          getRowId={({ data }) => data.symbol}
          theme={tickGridTheme}
          domLayout="normal"
          rowHeight={view.maxRows === undefined ? 28 : 24}
          headerHeight={28}
          suppressColumnMoveAnimation
        />
        {rowData.length === 0 && (
          <p className="positions-empty">{view.emptyMessage}</p>
        )}
      </div>
      {trendSymbol && <PriceHistoryPopup symbol={trendSymbol} history={view.histories.get(trendSymbol) ?? []} onClose={() => setTrendSymbol(undefined)} />}
    </section>
  );
});
