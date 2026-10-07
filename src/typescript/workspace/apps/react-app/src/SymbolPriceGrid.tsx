import React from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
    ClientSideRowModelModule,
    ModuleRegistry,
    themeQuartz,
    type ColDef,
    type ICellRendererParams,
} from 'ag-grid-community';
import type { SymbolPriceTick } from './types/symbol_price';
import type { FeedSource } from './connection';

ModuleRegistry.registerModules([ClientSideRowModelModule]);

type SymbolGridCell = SymbolPriceTick | undefined;

type SymbolGridRow = {
    rowIndex: number;
    cells: SymbolGridCell[];
};

function createSymbolGridRows(ticks: SymbolPriceTick[]): SymbolGridRow[] {
    const rows = Array.from({ length: 10 }, (_, rowIndex) => ({
        rowIndex,
        cells: Array<SymbolGridCell>(10).fill(undefined),
    }));

    ticks.slice(0, 100).forEach((tick, index) => {
        rows[Math.floor(index / 10)].cells[index % 10] = tick;
    });

    return rows;
}

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

function SymbolGridCellRenderer({ value }: ICellRendererParams<SymbolGridRow, SymbolGridCell>) {
    if (!value) return null;

    return (
        <div className="symbol-grid-cell">
            <span className="grid-symbol" title={value.symbol}>{value.symbol}</span>
            <span className={`grid-price price-${value.direction}`}>{priceFormatter.format(value.price)}</span>
        </div>
    );
}

type SymbolPriceGridProps = {
    ticks: SymbolPriceTick[];
    feedSource: FeedSource;
};

export function SymbolPriceGrid({ ticks, feedSource }: SymbolPriceGridProps) {
    const rowData = createSymbolGridRows(ticks);
    const columnDefs = React.useMemo<ColDef<SymbolGridRow, SymbolGridCell>[]>(() =>
        Array.from({ length: 10 }, (_, columnIndex) => ({
            headerName: `${columnIndex * 10}-${columnIndex * 10 + 9}`,
            colId: `symbols-${columnIndex}`,
            valueGetter: params => params.data?.cells[columnIndex],
            cellRenderer: SymbolGridCellRenderer,
            equals: (previous, next) =>
                previous?.symbol === next?.symbol && previous?.price === next?.price,
            sortable: false,
            resizable: false,
            flex: 1,
            minWidth: 0,
        })),
        []);

    return (
        <section className="feed-section" aria-label="Live symbol price grid">
            <div className="feed-toolbar">
                <div>
                    <span className="live-indicator" />
                    <span className="toolbar-title">Market overview</span>
                </div>
                <span className="toolbar-meta">
                    {feedSource === 'okx' ? 'Top 100 · 24h volume' : '100 synthetic symbols · fast'}
                </span>
            </div>
            <div className="symbol-grid">
                <AgGridReact<SymbolGridRow>
                    suppressColumnMoveAnimation={true}
                    suppressHorizontalScroll
                    rowData={rowData}
                    columnDefs={columnDefs}
                    getRowId={params => String(params.data.rowIndex)}
                    theme={gridTheme}
                    domLayout="autoHeight"
                    rowHeight={42}
                    headerHeight={0}
                    cellFlashDuration={500}
                    cellFadeDuration={900}
                    suppressCellFocus
                />
            </div>
        </section>
    );
}