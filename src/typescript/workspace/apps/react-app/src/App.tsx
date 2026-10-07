import React from 'react'
import './App.css'
import { Connection, type FeedSource } from './connection';
import { useObservable } from './use_observable';
import { decodeSymbolPrice, type SymbolPriceTick } from './types/symbol_price';
import { bufferTime, filter, map, of, switchMap, type Observable } from 'rxjs';
import { Header } from './Header';
import { SymbolPriceGrid } from './SymbolPriceGrid';
import { SymbolPriceTable } from './SymbolPriceTable';
import { Users } from './Users';

type TicksUpdate = {
  ticks: SymbolPriceTick[];
};

function createTicksObservable(connection: Connection | undefined): Observable<TicksUpdate> | undefined {
  if (!connection) return undefined;

  let symbols = new Map<string, SymbolPriceTick>();
  let symbolIndices = new Map<string, number>();
  let ticks: SymbolPriceTick[] = [];

  return connection.connection_state.pipe(
    switchMap(state => state === 'connected'
      ? connection.data.pipe(
        map(data => decodeSymbolPrice(data, symbols)),
        filter((tick): tick is SymbolPriceTick => tick !== undefined),
        bufferTime(50),
        filter(changedTicks => changedTicks.length > 0),
        map(changedTicks => ({ reset: false as const, changedTicks })),
      )
      : of({ reset: true as const, changedTicks: [] as SymbolPriceTick[] })),
    map(update => {
      if (update.reset) {
        symbols = new Map<string, SymbolPriceTick>();
        symbolIndices = new Map<string, number>();
        ticks = [];
        return { ticks };
      }

      const { changedTicks } = update;
      changedTicks.forEach(tick => {
        let index = symbolIndices.get(tick.symbol);
        if (index === undefined) {
          index = ticks.length;
          symbolIndices.set(tick.symbol, index);
        }

        const latestTick = symbols.get(tick.symbol);
        if (!latestTick) return;

        if (ticks[index]?.price === latestTick.price) return;

        ticks[index] = latestTick;
      });
      return { ticks };
    })
  );
}

function App() {
  const [connection, setConnection] = React.useState<Connection | undefined>(undefined);
  const [feedSource, setFeedSource] = React.useState<FeedSource>('okx');
  const [dataView, setDataView] = React.useState<'grid' | 'table' | 'user'>('grid');

  React.useEffect(() => {
    const currentConnection = new Connection(feedSource);
    setConnection(currentConnection);
    void currentConnection.connect();

    return () => {
      currentConnection.dispose();
    };
  }, [feedSource]);


  const state$ = useObservable(connection?.connection_state, 'connecting', [connection])
  const error$ = useObservable(connection?.error_message, undefined, [connection])

  const ticksObservable = React.useMemo(() => createTicksObservable(connection), [connection]);

  const ticks$ = useObservable(
    ticksObservable,
    { ticks: [] },
    [ticksObservable],
  );

  return (
    <main className="terminal">
      <Header error$={error$} state$={state$} feedUrl={connection?.url ?? ''} />
      <div className="feed-controls">
        <div className="view-switcher" role="group" aria-label="Choose price feed">
          <button
            type="button"
            aria-pressed={feedSource === 'okx'}
            onClick={() => setFeedSource('okx')}
          >
            OKX
          </button>
          <button
            type="button"
            aria-pressed={feedSource === 'synthetic'}
            onClick={() => setFeedSource('synthetic')}
          >
            Synthetic · Fast
          </button>
        </div>
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
          <button
            type="button"
            aria-pressed={dataView === 'user'}
            onClick={() => setDataView('user')}
          >
            User
          </button>
        </div>
      </div>

      {dataView === 'grid' && (
        <SymbolPriceGrid
          ticks={ticks$.ticks}
          feedSource={feedSource}
        />
      )}
      {dataView === 'table' &&(
        <SymbolPriceTable ticks={ticks$.ticks} state={state$} />
      )}
      {dataView === 'user' &&(
        <Users />
      )}
    </main>
  )
}



export default App
