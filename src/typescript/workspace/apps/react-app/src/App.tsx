import React from 'react';
import './App.css';
import { Connection, type FeedSource } from './connection';
import { useObservable } from './use_observable';
import { Header } from './views/Header';
import { FeedControls } from './views/FeedControls';

function App() {
  const [feedSource, setFeedSource] = React.useState<FeedSource>('okx');
  const connection = React.useMemo(
    () => new Connection(feedSource),
    [feedSource],
  );

  React.useEffect(() => {
    void connection.connect();

    const disposeOnPageHide = () => {
      console.info('[book] UI page is unloading; disposing subscriptions');
      connection.dispose();
    };
    window.addEventListener('pagehide', disposeOnPageHide);

    return () => {
      window.removeEventListener('pagehide', disposeOnPageHide);
      connection.dispose();
    };
  }, [connection]);

  const state$ = useObservable(connection?.connection_state, 'connecting', [
    connection,
  ]);
  const error$ = useObservable(connection?.error_message, undefined, [
    connection,
  ]);
  const changeFeedSource = React.useCallback((source: FeedSource) => {
    setFeedSource(source);
  }, []);

  return (
    <main className="terminal">
      <Header error$={error$} state$={state$} feedUrl={connection?.url ?? ''} />
      <FeedControls
        connection={connection}
        feedSource={feedSource}
        connectionState={state$}
        onFeedSourceChange={changeFeedSource}
      />
    </main>
  );
}
export default App;
