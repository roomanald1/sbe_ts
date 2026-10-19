# Backend service split

Each service is an independent Cargo project with its own process and listener:

| Project | Ownership | Listener |
| --- | --- | --- |
| [feed_handlers](./feed_handlers/) | OKX and synthetic feed connectivity, SBE encoding, instrument lookup, and normalized market streams | RSocket 10001, HTTP 3001 |
| [position_management](./position_management/) | Position ledger, account and portfolio APIs, and order lifecycle persistence | HTTP 3002 |
| [execution](./execution/) | Paper matching and VWAP scheduling | HTTP 3003 |
| [insights](./insights/) | Signal engine and persistence, signal reads, and close-out calculations | HTTP 3004 |

Run all four with ./run-backend-services.sh; serve the UI separately with ./nx serve react-app on port 5173. Position state is persisted in position_management/positions.json; signal history is persisted in insights/signals.jsonl.

Services communicate only through runtime APIs. Position Management consumes feed streams from FeedHandlers over RSocket and calls Execution over HTTP for matching and VWAP schedule decisions. Execution proxies the public /positions/orders and /positions/vwap routes to Position Management over HTTP. FeedHandlers forwards normalized signal inputs to Insights over HTTP. Insights requests position inputs from Position Management over HTTP. There are no Rust path dependencies between service projects.
