
Deployed:
https://sbe-ts-1.onrender.com/



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
| FeedHandlers | OKX and synthetic feed connectivity, SBE encoding, instrument lookup, and normalized market streams | RSocket 10001, HTTP 3001 |
| Position Management | Position ledger, account and portfolio APIs, and order lifecycle persistence | HTTP 3002 |
| Execution | Paper matching and VWAP scheduling | HTTP 3003 |
| Insights Research | Research sampling, outcomes, signal persistence and reads, daily/carry signals, and close-out calculations | HTTP 3004 |
| AutoTrader | Trading loop and settings API | HTTP 3005 |
| News sentiment | Local headline classifier used by Insights | HTTP 3006 |
| Live Signals | Per-symbol signal computation, news overlay, and direct signal delivery to AutoTrader | HTTP 3007 |
| Learning | Versioned bounded strategy parameter profiles | HTTP 3010 |
| Scouts | Scout evaluation and outcome forwarding to Learning | HTTP 3011 |
| Insights Maintenance | Nightly signal archival, raw-record cleanup, training archive expiry, and audit compaction | Background process |

Live Signals and Insights Research independently consume FeedHandlers market events. Live Signals sends current signals to AutoTrader; Insights owns signal and outcome persistence and serves cached research context to Live Signals. Position Management consumes market feeds and calls Execution for matching and VWAP decisions. Execution proxies the public `/positions/orders` and `/positions/vwap` routes to Position Management. AutoTrader publishes feedback asynchronously and polls Learning for profile updates.

The launcher points `POSITIONS_FILE`, `AUTO_TRADER_STATE_FILE`, `SIGNALS_FILE`, and `LEARNING_STATE_FILE` at persisted files under their owning services. Position data is stored in `src/rust/position_management/positions.json`; AutoTrader state is stored in `src/rust/auto_trader/auto_trader_state.json`; Insights signal history is stored in `src/rust/insights/signals.jsonl`. `LEARNING_STATE_FILE` defaults to `src/rust/learning/learning_state.sqlite`; the Learning service imports prior JSON state on first startup when needed. FeedHandlers owns OKX connectivity, SBE encoding, and normalized market streams. Live Signals owns low-latency signal computation and delivery. Insights Research owns signal persistence, outcome tracking, signal reads, and close insights; Insights Maintenance runs nightly retention and archival against the same signal database.
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

Commits run the combined coverage suite through the repository's pre-commit hook. To enable it in a checkout, run `git config core.hooksPath .githooks` from the repository root. Coverage failures prevent the commit.
