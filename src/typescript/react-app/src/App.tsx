import React from 'react'
import './App.css'
import { Connection } from './connection';
import { useObservable } from './use_observable';
import { decodeSymbolPrice, type SymbolPriceTick } from './types/symbol_price';
import { auditTime, filter, map } from 'rxjs';
import { Header } from './Header';

const priceFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 4,
});
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});


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

  return (
    <main className="terminal">
      <Header error$={error$} ticks$={ticks$.ticks} state$={state$} />

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


