import { useEffect, useMemo } from 'react';
import {
  SymbolPriceGridModel,
  type SymbolPriceGridProps,
  type SymbolGridCell,
  type SymbolGridRow,
} from '../models/SymbolPriceGridModel';
import { FeedToolbar } from './FeedToolbar';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
} from 'ag-grid-community';

ModuleRegistry.registerModules([ClientSideRowModelModule]);

const priceFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 10,
});

const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

function SymbolGridCellRenderer({
  value,
  model,
}: ICellRendererParams<SymbolGridRow, SymbolGridCell> & {
  model: SymbolPriceGridModel;
}) {
  if (!value) return null;

  return (
    <div className="symbol-grid-cell-wrap">
      <button
        className="symbol-grid-cell-button"
        type="button"
        onClick={() => model.selectSymbol(value.symbol)}
        disabled={!model.getSnapshot().onSelectSymbol}
        aria-label={`Show ${value.symbol} order book`}
      >
        <span className="grid-symbol" title={value.symbol}>
          {value.symbol}
        </span>
        <span className={`grid-price price-${value.direction}`}>
          {priceFormatter.format(value.price)}
        </span>
      </button>
      {model.getSnapshot().onTradeSymbol && (
        <span className="grid-trade-actions">
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              model.tradeSymbol(value.symbol);
            }}
          >
            Buy
          </button>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              model.tradeSymbol(value.symbol);
            }}
          >
            Sell
          </button>
        </span>
      )}
    </div>
  );
}

export function SymbolPriceGrid(props: SymbolPriceGridProps) {
  const model = useMemo(
    () =>
      new SymbolPriceGridModel({ ...props, compact: props.compact ?? false }),
    [],
  );
  useEffect(() => {
    model.setProps({ ...props, compact: props.compact ?? false });
  }, [
    model,
    props.ticks,
    props.feedSource,
    props.compact,
    props.onSelectSymbol,
    props.onTradeSymbol,
  ]);
  const state = useBehaviorSnapshot(model.state$);
  const { rowData, compact } = { rowData: state.rows, compact: state.compact };
  const columnDefs = useMemo<ColDef<SymbolGridRow, SymbolGridCell>[]>(
    () =>
      Array.from({ length: 10 }, (_, columnIndex) => ({
        headerName: columnIndex * 10 + '-' + (columnIndex * 10 + 9),
        colId: 'symbols-' + columnIndex,
        valueGetter: (params) => params.data?.cells[columnIndex],
        cellRenderer: SymbolGridCellRenderer,
        cellRendererParams: { model },
        equals: (previous, next) =>
          previous?.symbol === next?.symbol && previous?.price === next?.price,
        sortable: false,
        resizable: false,
        flex: 1,
        minWidth: 0,
      })),
    [model],
  );

  return (
    <section
      className={compact ? 'feed-section compact-market-panel' : 'feed-section'}
      aria-label="Live symbol price grid"
    >
      <FeedToolbar title="Market overview">
        <span className="toolbar-meta">
          {state.feedSource === 'okx'
            ? 'Top 100 · 24h volume'
            : '100 synthetic symbols · fast'}
        </span>
      </FeedToolbar>
      <div className="symbol-grid">
        <AgGridReact<SymbolGridRow>
          suppressColumnMoveAnimation={true}
          suppressHorizontalScroll
          rowData={rowData}
          columnDefs={columnDefs}
          getRowId={(params) => String(params.data.rowIndex)}
          theme={gridTheme}
          domLayout="autoHeight"
          rowHeight={compact ? 24 : 42}
          headerHeight={0}
          cellFlashDuration={500}
          cellFadeDuration={900}
          suppressCellFocus
        />
      </div>
    </section>
  );
}
