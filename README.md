
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
| Paper Exchange | Simulated paper matching and VWAP scheduling | RSocket 10006; HTTP 3003 operations |
| Insights | Live signal generation, enriched market events, research sampling/outcomes, signal persistence, daily/carry signals, and news | RSocket 10004 / 10007 / 10011; HTTP 3004 operations |
| AutoTrader | Trading loop, settings, and snapshots | RSocket 10006 (host 10009); HTTP 3005 operations |
| News sentiment | Local headline classifier used by Insights | RSocket 10012; HTTP 3006 operations |
| Learning | Scout coordination, bounded strategy profiles, outcome feedback, and profile-attributed evidence | RSocket 10010; HTTP 3010 operations |
| Strategy | One always-on stateless live evaluator and one Learning-side Scout worker per strategy | RSocket; Compose uses live 10201 and Scout 10200 |
| Insights Maintenance | Nightly signal archival, raw-record cleanup, training archive expiry, and audit compaction | Background process |

Browser-to-service and service-to-service application traffic uses RSocket request-response or request-stream interactions. HTTP listeners are reserved for health, process/resource and queue metrics, documentation, and service-control operations. Exchange and vendor APIs remain HTTP.

Insights consumes raw FeedHandlers events, derives enriched market data, generates live signal snapshots, and publishes them to AutoTrader and Learning while independently capturing research samples. AutoTrader sends production strategy entry, position, and rebalance decisions to the dedicated Strategy live workers. Learning selects and coordinates Scout work; optional strategy-specific Scout workers execute it and keep isolated state. The live workers are stateless and do not mount the Scout state volume. Paper Exchange owns simulated books and fills, then returns match results for Position Management to persist. External venue adapters should submit orders to exchanges and report venue fills separately.

In the non-container development setup, `run-backend-services.sh` starts both Strategy roles for all 18 registered strategies. It binds each RSocket listener and metrics listener to its own loopback port and stores Scout worker databases under `src/rust/learning/strategy_state/`.

The launcher points `POSITIONS_FILE`, `AUTO_TRADER_STATE_FILE`, `SIGNALS_FILE`, and `LEARNING_STATE_FILE` at persisted files under their owning services. Position data is stored in `src/rust/position_management/positions.json`; AutoTrader state is stored in `src/rust/auto_trader/auto_trader_state.json`; Insights signal history is stored in `src/rust/insights/signals.jsonl`; Learning state is stored in SQLite. Scout state is partitioned into per-strategy worker databases at `/data/workers/<strategy>.sqlite` in the existing `sbe-scouts-state` volume. Strategy Scout services use the optional `scout-workers` profile; live evaluators run independently and mount no persistent volume. Insights Maintenance runs nightly retention and archival against the signal database.
## Run and manage the local stack

Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password, then launch the separate app and infrastructure Compose projects:

    ./start-all.sh

To run only one side, use `./start-infra.sh` or `./start-system.sh`; stop them with `./stop-infra.sh` or `./stop-system.sh`. You can pass Compose service names to start or stop only selected services, for example `./start-infra.sh grafana`.

To rebuild images and recreate every service in both Compose projects, run `./rebuild-all.sh`. It rebuilds the homepage and benchmarks bundles, builds each distinct system image once, starts the system project to create its network, then rebuilds and starts infrastructure. To rebuild only changed system services, run `./rebuild-services.sh`; it compares service configuration, crate sources, and local Cargo path dependencies with the last recorded deployment, builds each distinct image once, then waits for changed services to become healthy or running before updating that record. The live and Scout strategy containers share one image and receive their role and strategy as runtime settings. After upgrading its fingerprint format, it rebuilds services once to establish the new baseline. Set `SCOUT_WORKERS_ENABLED=false` to omit the optional strategy Scout workers.

For targeted system rebuilds, use `just rebuild <service> [service ...]`, for example `just rebuild insights auto-trader`. Use `just rebuild-scouts` to rebuild all Scout workers. Targeted rebuilds force-recreate only the named services and do not restart dependencies. Run `just --list` to see the available recipes.

Common development commands are also available through Just: `just fmt <crate>`, `just fmt-rust`, `just check <crate>`, `just test <crate> [filter ...]`, `just typecheck`, and `just diff-check`. Crate names use their directory names, such as `auto_trader`.

Launch Compose through these scripts so bind mounts resolve to this repository's data directories. If services were previously launched from the control container, refresh the controller and recreate the stateful services with `./start-infra.sh service-control` followed by `./start-system.sh position-management auto-trader insights learning`.

The web container runs Vite with the React app source mounted from the workspace, so frontend edits appear through hot reload without rebuilding its image. The service-control homepage is the `homepage` Nx/Vite React app; `start-infra.sh` builds it before launching the server on port 8080. Use `./nx serve homepage` on port 5174 for homepage development. Stack startup does not rebuild images; after changing a backend service's source or Dockerfile, rebuild only that service with `docker compose -p sbe-system -f system/compose.yml up -d --build <service>`, or use `./rebuild-all.sh` for the full stack.

Open the service control page at http://localhost:8080. It shows each container's state and can start, stop, or restart services independently, either project as a group, or everything together. The app system is defined in [system/compose.yml](system/compose.yml); monitoring infrastructure is defined in [monitoring/docker-compose.yml](monitoring/docker-compose.yml). Individual services restart automatically after process failure. Use `./stop-all.sh` to stop both projects while preserving their containers and data volumes. The React app remains available at http://localhost:5174, Grafana at http://localhost:3000, and generated coverage is linked from the control page when present.

## Code coverage

Run ./coverage.sh from the repository root to generate HTML coverage reports. The combined TypeScript and Rust index is at src/typescript/workspace/apps/react-app/public/coverage/index.html. It links to the frontend report under typescript/ and each Rust crate report under rust/<crate>/. The frontend command uses the existing Node test suite, and Rust coverage runs each crate's unit tests through cargo-llvm-cov.

Install the Rust coverage tool once with cargo install cargo-llvm-cov if it is missing.

Coverage reports are generated on demand with `./coverage.sh`; coverage is not part of the commit hook.
