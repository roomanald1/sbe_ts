
Deployed:
https://sbe-ts-1.onrender.com/



## TypeScript formatting

Install the recommended Prettier VS Code extension to format TypeScript and TSX files on save. To enable automatic formatting of staged TypeScript changes at commit time, run this once in each clone:

```sh
./setup-git-hooks.sh
```

The setup command sets this repository's `core.hooksPath` to `.githooks` only when no hook path is already configured; it leaves existing Git hook configuration untouched. From `src/typescript/workspace`, run `npm run format` to format app and package TypeScript source, or `npm run format:check` to verify it.

For local development without containers, you can still run `./nx serve react-app` and `./run-backend-services.sh` separately.

Run Nx from the repository root with `./nx <target> <project>`, for example:

```sh
./nx build react-app
./nx lint react-app
./nx serve homepage
./nx build homepage
```

## Local backend services

Run backend services together from the repository root in the non-container development setup:

```sh
./run-backend-services.sh
```

The UI remains a separate process (`./nx serve react-app`). Backend services run as separate processes:

| Service | Responsibilities | Listener |
| --- | --- | --- |
| FeedHandlers | Raw OKX and synthetic feeds, SBE encoding, and instrument lookup | RSocket 10001; HTTP 3001 operations |
| Binance Feed | Binance daily-candle retrieval | RSocket 10002; HTTP 3012 operations |
| Position Management | Position ledger, account/portfolio APIs, and order lifecycle persistence | RSocket 10005; HTTP 3002 operations |
| Execution | Paper matching and VWAP scheduling | RSocket 10006; HTTP 3003 operations |
| Insights Research | Research sampling, outcomes, signal persistence, daily/carry signals, news, and close-out calculations | RSocket 10004 / 10007 / 10011; HTTP 3004 operations |
| AutoTrader | Trading loop, settings, and snapshots | RSocket 10006 (host 10008); HTTP 3005 operations |
| News sentiment | Local headline classifier used by Insights | RSocket 10012; HTTP 3006 operations |
| Live Signals | Per-symbol signal computation and news overlay | RSocket clients; HTTP 3007 operations |
| Learning | Scout coordination and bounded strategy profiles | RSocket 10010; HTTP 3010 operations |
| Scouts | One RSocket worker per strategy; the central listener only serves operations | Worker RSocket 10200; HTTP 3011 operations |
| Insights Maintenance | Nightly signal archival, raw-record cleanup, training archive expiry, and audit compaction | Background process |

Browser-to-service and service-to-service application traffic uses RSocket request-response or request-stream interactions. HTTP listeners are reserved for health, process/resource and queue metrics, documentation, and service-control operations. Exchange and vendor APIs remain HTTP.

Insights consumes raw FeedHandlers events, derives enriched market data, and publishes it to React and Live Signals over RSocket. Learning selects and coordinates Scout work; strategy-specific Scout workers execute it and keep isolated state. Execution calls Position Management over RSocket for position-owned order operations.

The launcher points `POSITIONS_FILE`, `AUTO_TRADER_STATE_FILE`, `SIGNALS_FILE`, and `LEARNING_STATE_FILE` at persisted files under their owning services. Position data is stored in `src/rust/position_management/positions.json`; AutoTrader state is stored in `src/rust/auto_trader/auto_trader_state.json`; Insights signal history is stored in `src/rust/insights/signals.jsonl`; Learning state is stored in SQLite. Scout state is partitioned into per-strategy worker databases in the `scouts-state` volume; workers do not import the old shared state at startup. Insights Maintenance runs nightly retention and archival against the signal database.
## Run and manage the local stack

Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password, then launch the separate app and infrastructure Compose projects:

    ./start-all.sh

To run only one side, use `./start-infra.sh` or `./start-system.sh`; stop them with `./stop-infra.sh` or `./stop-system.sh`. You can pass Compose service names to start or stop only selected services, for example `./start-infra.sh grafana`.

Launch Compose through these scripts so bind mounts resolve to this repository's data directories. If services were previously launched from the control container, refresh the controller and recreate the stateful services with `./start-infra.sh service-control` followed by `./start-system.sh position-management auto-trader insights learning`.

The web container runs Vite with the React app source mounted from the workspace, so frontend edits appear through hot reload without rebuilding its image. The service-control homepage is the `homepage` Nx/Vite React app; `start-infra.sh` builds it before launching the server on port 8080. Use `./nx serve homepage` on port 5174 for homepage development. Stack startup does not rebuild images; after changing a backend service's source or Dockerfile, rebuild only that service with `docker compose -p sbe-system -f system/compose.yml up -d --build <service>`.

Open the service control page at http://localhost:8080. It shows each container's state and can start, stop, or restart services independently, either project as a group, or everything together. The app system is defined in [system/compose.yml](system/compose.yml); monitoring infrastructure is defined in [monitoring/docker-compose.yml](monitoring/docker-compose.yml). Individual services restart automatically after process failure. Use `./stop-all.sh` to stop both projects while preserving their containers and data volumes. The React app remains available at http://localhost:5173, Grafana at http://localhost:3000, and generated coverage is linked from the control page when present.

## Code coverage

Run ./coverage.sh from the repository root to generate HTML coverage reports. The combined TypeScript and Rust index is at src/typescript/workspace/apps/react-app/public/coverage/index.html. It links to the frontend report under typescript/ and each Rust crate report under rust/<crate>/. The frontend command uses the existing Node test suite, and Rust coverage runs each crate's unit tests through cargo-llvm-cov.

Install the Rust coverage tool once with cargo install cargo-llvm-cov if it is missing.

Coverage reports are generated on demand with `./coverage.sh`; coverage is not part of the commit hook.
