import React from 'react'
import './App.css'
import { Connection } from './connection';
import { useObservable } from './use_observable';
import { decodeSymbolPrice, type SymbolPriceTick } from './types/symbol_price';
import { bufferTime, filter, map } from 'rxjs';
import { Header } from './Header';
import { SymbolPriceGrid } from './SymbolPriceGrid';
import { SymbolPriceTable } from './SymbolPriceTable';

function App() {
  const [connection, setConnection] = React.useState<Connection | undefined>(undefined);
  const [dataView, setDataView] = React.useState<'grid' | 'table'>('grid');

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
    const symbolIndices = new Map<string, number>();
    const ticks: SymbolPriceTick[] = [];
    return connection.data.pipe(
      map(data => decodeSymbolPrice(data, symbols)),
      filter((tick): tick is SymbolPriceTick => tick !== undefined),
      bufferTime(50),
      filter(changedTicks => changedTicks.length > 0),
      map(changedTicks => {
        const latestChangedTicks = new Map<number, SymbolPriceTick>();
        changedTicks.forEach(tick => {
          let index = symbolIndices.get(tick.symbol);
          if (index === undefined) {
            index = ticks.length;
            symbolIndices.set(tick.symbol, index);
          }

          const latestTick = symbols.get(tick.symbol);
          if (!latestTick) return;

          ticks[index] = latestTick;
          latestChangedTicks.set(index, latestTick);
        });
        return {
          ticks,
          changedTicks: Array.from(latestChangedTicks, ([index, tick]) => ({ index, tick })),
        };
      })
    );
  }, [connection]);

  const ticks$ = useObservable(ticksObservable, { ticks: [], changedTicks: [] }, [ticksObservable]);

  return (
    <main className="terminal">
      <Header error$={error$} state$={state$} feedUrl={connection?.url ?? ''} />
      <div className="view-switcher" role="group" aria-label="Choose data view">
        <button
          type="button"
          aria-pressed={dataView === 'grid'}
          onClick={() => setDataView('grid')}
        >
          Grid
        </button>
        <button
          type="button"
          aria-pressed={dataView === 'table'}
          onClick={() => setDataView('table')}
        >
          Table
        </button>
      </div>
      {dataView === 'grid' ? (
        <SymbolPriceGrid ticks={ticks$.ticks} changedTicks={ticks$.changedTicks} />
      ) : (
        <SymbolPriceTable ticks={ticks$.ticks} state={state$} />
      )}
    </main>
  )
}



export default App
