export function Header(props: { state$: any; error$: any; feedUrl: string }) {
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
          <a
            className="grafana-link"
            href="http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading"
            target="_blank"
            rel="noreferrer"
          >
            Grafana ↗
          </a>
          <a className="grafana-link" href="/coverage/index.html" target="_blank" rel="noreferrer">
            Coverage ↗
          </a>
          <div className={`connection-status ${props.state$}`}>
            <span className="status-dot" />
            {props.state$ === 'connected'
              ? 'LIVE'
              : props.state$ === 'connecting'
                ? 'CONNECTING'
                : 'RECONNECTING'}
          </div>
        </div>
      </header>

      <div
        className={`feed-notice${props.error$ ? '' : ' feed-notice-empty'}`}
        role="status"
        aria-hidden={!props.error$}
      >
        {props.error$ ?? ''}
      </div>
    </>
  );
}
