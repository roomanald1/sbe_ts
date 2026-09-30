export function Header(props: {ticks$: any, state$: any, error$: any}) {
    return <>
        <header className="topbar">
            <div className="brand-lockup">
                <span className="brand-mark" aria-hidden="true">S</span>
                <span>STREAM / SBE</span>
            </div>
            <div className={`connection-status ${props.state$}`}>
                <span className="status-dot" />
                {props.state$ === 'connected' ? 'LIVE' : props.state$ === 'connecting' ? 'CONNECTING' : 'RECONNECTING'}
            </div>
        </header>

        <section className="page-heading">
            <div>
                <p className="eyebrow">RSocket feed · 127.0.0.1:9001</p>
                <h1>Symbol prices</h1>
                <p className="subheading">SBE binary stream, decoded in the browser</p>
            </div>
            <div className="feed-summary" aria-live="polite">
                <span className="summary-label">SYMBOLS</span>
                <strong className="symbol-count">{props.ticks$?.length.toString().padStart(2, '0')}</strong>
                <span className="summary-unit">latest prices</span>
            </div>
        </section>

        <div
            className={`feed-notice${props.error$ ? '' : ' feed-notice-empty'}`}
            role="status"
            aria-hidden={!props.error$}
        >
            {props.error$ ?? ''}
        </div>
    </>
}