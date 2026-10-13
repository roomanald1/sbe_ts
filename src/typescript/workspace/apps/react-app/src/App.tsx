import React from 'react'
import './App.css'
import { Connection, type FeedSource } from './connection';
import { useObservable } from './use_observable';
import { Header } from './Header';
import { OrderBookPopup } from './OrderBookPopup';
import { FeedControls } from './FeedControls';

function App() {
  const [feedSource, setFeedSource] = React.useState<FeedSource>('okx');
  const [selectedBook, setSelectedBook] = React.useState<string | undefined>();
  const connection = React.useMemo(() => new Connection(feedSource), [feedSource]);

  React.useEffect(() => {
    void connection.connect();

    return () => {
      connection.dispose();
    };
  }, [connection]);


  const state$ = useObservable(connection?.connection_state, 'connecting', [connection])
  const error$ = useObservable(connection?.error_message, undefined, [connection])
  const changeFeedSource = React.useCallback((source: FeedSource) => {
    setSelectedBook(undefined);
    setFeedSource(source);
  }, []);
  const closeBook = React.useCallback(() => setSelectedBook(undefined), []);

  return (
    <main className="terminal">
      <Header error$={error$} state$={state$} feedUrl={connection?.url ?? ''} />
      <FeedControls
        connection={connection}
        feedSource={feedSource}
        connectionState={state$}
        onFeedSourceChange={changeFeedSource}
        onSelectSymbol={setSelectedBook}
      />
      {feedSource === 'okx' && selectedBook && (
        <OrderBookPopup
          key={selectedBook}
          symbol={selectedBook}
          connection={connection}
          onClose={closeBook}
        />
      )}
    </main>
  )
}



export default App
