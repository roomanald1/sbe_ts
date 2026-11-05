# Backend service split

The backend services run as separate processes. Insights Maintenance is a dedicated binary in the Insights Cargo project and shares its database:

| Project | Ownership | Listener |
| --- | --- | --- |
| [feed_handlers](./feed_handlers/) | OKX and synthetic feed connectivity, SBE encoding, instrument lookup, and normalized market streams | RSocket 10001, HTTP 3001 |
| [position_management](./position_management/) | Position ledger, account and portfolio APIs, and order lifecycle persistence | HTTP 3002 |
| [execution](./execution/) | Paper matching and VWAP scheduling | HTTP 3003 |
| [live_signals](./live_signals/) | Per-symbol live signal computation, news overlay, and direct signal delivery to AutoTrader; Insights exposes cached research context only | HTTP 3007 |
| [insights](./insights/) | Research sampling, outcome tracking, signal persistence and reads, daily/carry signal production, and close-out calculations | HTTP 3004 |
| [insights-maintenance](./insights/) | Nightly signal archival, raw-record cleanup, training archive expiry, and audit compaction | Background process |

Launch the local backend processes with ./run-backend-services.sh; it starts Live Signals, Insights Research, and Insights Maintenance. Serve the UI separately with ./nx serve react-app on port 5173. Position Management state is persisted in position_management/positions.json; AutoTrader state is persisted in its own auto_trader/auto_trader_state.sqlite database; signal history is persisted in insights/signals.jsonl.

Services communicate through runtime APIs and independent FeedHandlers RSocket subscriptions. Live Signals consumes market events directly and sends current signal dashboards to AutoTrader over HTTP. Insights Research independently consumes the same ordered event stream at the research cadence, owns outcome and signal persistence, and serves cached news and walk-forward context to Live Signals. Position Management consumes other feed streams over RSocket and calls Execution over HTTP for matching and VWAP decisions. Execution proxies the public /positions/orders and /positions/vwap routes to Position Management over HTTP.
