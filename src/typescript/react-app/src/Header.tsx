export function Header(props: {state$: any, error$: any, feedUrl: string}) {
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
                <p className="eyebrow">RSocket feed · {props.feedUrl}</p>
                <h1>RSocket SBE</h1>
                <p className="subheading">SBE binary stream, decoded in the browser</p>
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