import React from 'react'
import './App.css'
import { Connection } from './connection';
import { useObservable } from './use_observable';
import { decodeSymbolPrice, type SymbolPriceTick } from './types/symbol_price';
import { auditTime, filter, map } from 'rxjs';
import { Header } from './Header';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef, type ICellRendererParams } from 'ag-grid-community';

ModuleRegistry.registerModules([ClientSideRowModelModule]);

const priceFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 4,
});
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});

type SymbolGridCell = {
  symbolId: number;
  tick: SymbolPriceTick | undefined;
};

type SymbolGridRow = {
  rowIndex: number;
  cells: SymbolGridCell[];
};

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

  const { symbolId, tick } = value;
  const priceClass = tick ? `grid-price price-${tick.direction}` : 'grid-price grid-price-empty';

  return (
    <div className="symbol-grid-cell">
      <span className="grid-symbol">{tick?.symbol ?? symbolId}</span>
      <span className={priceClass}>{tick ? priceFormatter.format(tick.price) : '--'}</span>
    </div>
  );
}

function App() {
  const [connection, setConnection] = React.useState<Connection | undefined>(undefined);

  React.useEffect(() => {
    const currentConnection = new Connection();
    setConnection(currentConnection);
    void currentConnection.connect();

    return () => {
      currentConnection.dispose();
    };
  }, []);


  const state$ = useObservable(connection?.connection_state, 'connecting', [connection])
  const error$ = useObservable(connection?.error_message, undefined, [connection])

  const ticksObservable = React.useMemo(() => {
    if (!connection) return undefined;

    const symbols = new Map<string, SymbolPriceTick>();
    const ticks: SymbolPriceTick[] = []; // stable
    let version = 0;
    return connection.data.pipe(
      map(data => decodeSymbolPrice(data, symbols)),
      filter((tick): tick is SymbolPriceTick => tick !== undefined),
      auditTime(50),
      map(() => {
        ticks.length = 0;
        symbols.forEach(t => ticks.push(t));
        return ({ ticks, version: version++ });
      })
    );
  }, [connection]);

  const ticks$ = useObservable(ticksObservable, {ticks:[], version: 0}, [ticksObservable]);
  const rowData = React.useMemo(() => {
    const ticksById = new Map<number, SymbolPriceTick>();
    const unindexedTicks: SymbolPriceTick[] = [];
    ticks$.ticks.forEach(tick => {
      const symbolId = Number(tick.symbol);
      if (tick.symbol.trim() !== '' && Number.isInteger(symbolId) && symbolId >= 0 && symbolId < 100) {
        ticksById.set(symbolId, tick);
      } else {
        unindexedTicks.push(tick);
      }
    });

    const ticksBySlot = Array.from({ length: 100 }, (_, symbolId) => ticksById.get(symbolId));
    let nextUnindexedTick = 0;
    for (let symbolId = 0; symbolId < ticksBySlot.length && nextUnindexedTick < unindexedTicks.length; symbolId++) {
      if (!ticksBySlot[symbolId]) ticksBySlot[symbolId] = unindexedTicks[nextUnindexedTick++];
    }

    return Array.from({ length: 10 }, (_, rowIndex) => ({
      rowIndex,
      cells: Array.from({ length: 10 }, (_, columnIndex) => {
        const symbolId = columnIndex * 10 + rowIndex;
        return { symbolId, tick: ticksBySlot[symbolId] };
      }),
    }));
  }, [ticks$.version]);
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
    <main className="terminal">
      <Header error$={error$} ticks$={ticks$.ticks} state$={state$} />

      <section className="feed-section" aria-label="Live symbol price events">
        <div className="feed-toolbar">
          <div>
            <span className="live-indicator" />
          </div>
        </div>
        <div className="symbol-grid">
          <AgGridReact<SymbolGridRow>
            rowData={rowData}
            columnDefs={columnDefs}
            theme={gridTheme}
            rowHeight={42}
            headerHeight={0}
            suppressCellFocus
          />
        </div>
      </section>
       <section className="feed-section" aria-label="Live symbol price events">
        <div className="feed-toolbar">
          <div>
            <span className="live-indicator" />
            <span className="toolbar-title">Incoming events</span>
          </div>
        </div>
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th scope="col">SYMBOL</th>
                <th scope="col">PRICE</th>
                <th scope="col">RECEIVED AT</th>
              </tr>
            </thead>
            <tbody>
              {(ticks$?.ticks.length ?? 0) === 0 ? (
                <tr>
                  <td className="empty-state" colSpan={4}>
                    {state$ === 'connected' ? 'Waiting for the first SBE message…' : 'Connecting to the publisher…'}
                  </td>
                </tr>
              ) : ticks$?.ticks.map((tick) => (
                <tr key={tick.symbol}>
                  <td><span className="symbol-tag">{tick.symbol}</span></td>
                  <td className={`price-cell price-${tick.direction}`}>{priceFormatter.format(tick.price)}</td>
                  <td className="time-cell">{timeFormatter.format(tick.receivedAt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </main>
  )
}



export default App


