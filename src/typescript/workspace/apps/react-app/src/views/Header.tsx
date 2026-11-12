export function Header(props: {
  state$: any;
  error$: any;
  feedUrl: string;
  insightsState$: any;
  insightsError$: any;
  showInsights: boolean;
}) {
  return (
    <>
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-mark" aria-hidden="true">
            S
          </span>
          <span>{props.feedUrl}</span>
        </div>
        <div className="topbar-actions">
          <a className="grafana-link" href="http://localhost:8080">
            Home ↗
          </a>
          <div className={`connection-status ${props.state$}`}>
            <span className="status-dot" />
            {props.state$ === 'connected'
              ? 'LIVE'
              : props.state$ === 'connecting'
                ? 'CONNECTING'
                : 'RECONNECTING'}
          </div>
          {props.showInsights && props.insightsState$ !== 'connected' && (
            <div
              className={`connection-status ${props.insightsState$}`}
              title={props.insightsError$ ?? 'Connecting to Insights'}
            >
              <span className="status-dot" />
              INSIGHTS{' '}
              {props.insightsState$ === 'connecting'
                ? 'CONNECTING'
                : 'RECONNECTING'}
            </div>
          )}
        </div>
      </header>

      <div
        className={`feed-notice${props.error$ ? '' : ' feed-notice-empty'}`}
        role="status"
        aria-hidden={!props.error$}
      >
        {props.error$ ?? ''}
      </div>
      {props.showInsights && props.insightsError$ && (
        <div className="feed-notice" role="status">
          Insights RSocket: {props.insightsError$}
        </div>
      )}
    </>
  );
}
