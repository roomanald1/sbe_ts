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
        <div className={`connection-status ${props.state$}`}>
          <span className="status-dot" />
          {props.state$ === 'connected'
            ? 'LIVE'
            : props.state$ === 'connecting'
              ? 'CONNECTING'
              : 'RECONNECTING'}
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
