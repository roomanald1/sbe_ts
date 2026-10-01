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

ModuleRegistry.registerModules([ClientSideRowModelModule]);

type SymbolGridCell = SymbolPriceTick | undefined;

type SymbolGridRow = {
    rowIndex: number;
    cells: SymbolGridCell[];
};

type IndexedSymbolPriceTick = {
    index: number;
    tick: SymbolPriceTick;
};

const priceFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
});

const gridTheme = themeQuartz.withParams({
    accentColor: '#23816a',
    backgroundColor: '#ffffff',
    borderColor: '#d9ddd6',
    foregroundColor: '#17201d',
    headerBackgroundColor: '#f0f1ec',
    headerTextColor: '#78817d',
});

function SymbolGridCellRenderer({ value }: ICellRendererParams<SymbolGridRow, SymbolGridCell>) {
    if (!value) return null;

    return (
        <div className="symbol-grid-cell">
            <span className="grid-symbol">{value.symbol}</span>
            <span className={`grid-price price-${value.direction}`}>{priceFormatter.format(value.price)}</span>
        </div>
    );
}

function updateSymbolGridRows(
    currentRows: SymbolGridRow[],
    changedTicks: IndexedSymbolPriceTick[],
): SymbolGridRow[] {
    const changedCellsByRow = new Map<number, SymbolGridCell[]>();

    changedTicks.forEach(({ index, tick }) => {
        if (index < 0 || index >= 100) return;

        const rowIndex = Math.floor(index / 10);
        const columnIndex = index % 10;
        let cells = changedCellsByRow.get(rowIndex);
        if (!cells) {
            cells = currentRows[rowIndex].cells.slice();
            changedCellsByRow.set(rowIndex, cells);
        }

        cells[columnIndex] = tick;
    });

    if (changedCellsByRow.size === 0) return currentRows;

    const nextRows = currentRows.slice();
    changedCellsByRow.forEach((cells, rowIndex) => {
        nextRows[rowIndex] = { ...currentRows[rowIndex], cells };
    });
    return nextRows;
}

type SymbolPriceGridProps = {
    changedTicks: IndexedSymbolPriceTick[];
};

export function SymbolPriceGrid({ changedTicks }: SymbolPriceGridProps) {
    const [rowData, setRowData] = React.useState<SymbolGridRow[]>(() => Array.from({ length: 10 }, (_, rowIndex) => ({
        rowIndex,
        cells: Array<SymbolGridCell>(10).fill(undefined),
    })));

    React.useEffect(() => {
        if (changedTicks.length === 0) return;

        setRowData(currentRows => updateSymbolGridRows(currentRows, changedTicks));
    }, [changedTicks]);

    const columnDefs = React.useMemo<ColDef<SymbolGridRow, SymbolGridCell>[]>(() =>
        Array.from({ length: 10 }, (_, columnIndex) => ({
            headerName: `${columnIndex * 10}-${columnIndex * 10 + 9}`,
            colId: `symbols-${columnIndex}`,
            valueGetter: params => params.data?.cells[columnIndex],
            cellRenderer: SymbolGridCellRenderer,
            sortable: false,
            resizable: false,
            flex: 1,
            minWidth: 96,
        })),
        []);

    return (
        <section className="feed-section" aria-label="Live symbol price grid">
            <div className="feed-toolbar">
                <div>
                    <span className="live-indicator" />
                </div>
            </div>
            <div className="symbol-grid">
                <AgGridReact<SymbolGridRow>
                    rowData={rowData}
                    columnDefs={columnDefs}
                    getRowId={params => String(params.data.rowIndex)}
                    theme={gridTheme}
                    rowHeight={42}
                    headerHeight={0}
                    suppressCellFocus
                />
            </div>
        </section>
    );
}