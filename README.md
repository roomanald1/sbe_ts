
Deployed:
https://sbe-ts-1.onrender.com/



For local development without containers, you can still run `./nx serve react-app` and `./run-backend-services.sh` separately.

Run Nx from the repository root with `./nx <target> <project>`, for example:

```sh
./nx build react-app
./nx lint react-app
```

## Local backend services

Run backend services together from the repository root in the non-container development setup:

```sh
./run-backend-services.sh
```

The UI remains a separate process (`./nx serve react-app`). FeedHandlers owns RSocket on port 10001 and instrument lookup on port 3001. Position Management owns the position ledger, account, and portfolio APIs on port 3002. Execution listens on port 3003. Insights serves signals on port 3004. AutoTrader owns its trading loop and settings API on port 3005. The new Learning service stores versioned bounded profiles on port 3010; the Scouts service forwards shadow outcomes to Learning on port 3011. AutoTrader persists pending feedback in its state file and publishes it asynchronously, then polls Learning for profile updates.

The launcher points `POSITIONS_FILE` and `SIGNALS_FILE` at the existing persisted files under their owning services. `LEARNING_STATE_FILE` defaults to `src/rust/position_management/learning_state.json`. FeedHandlers owns OKX connectivity, SBE encoding, and synthetic feed streams. Position Management owns the ledger and portfolio APIs. AutoTrader owns its hot-path decisions and order loop. Scouts owns shadow-outcome forwarding. Learning owns bounded profile updates. Execution owns order and VWAP APIs. Insights owns signal persistence and close insights.
## Run and manage the local stack

Copy monitoring/.env.example to monitoring/.env and set a unique Grafana password, then launch the separate app and infrastructure Compose projects:

    ./start-all.sh

To run only one side, use `./start-infra.sh` or `./start-system.sh`; stop them with `./stop-infra.sh` or `./stop-system.sh`. You can pass Compose service names to start or stop only selected services, for example `./start-infra.sh grafana`.

Launch Compose through these scripts so bind mounts resolve to this repository's data directories. If services were previously launched from the control container, refresh the controller and recreate the stateful services with `./start-infra.sh service-control` followed by `./start-system.sh position-management auto-trader insights learning`.

The web container runs Vite with the React app source mounted from the workspace, so frontend edits appear through hot reload without rebuilding its image. Service control mounts its source into the container and restarts its Node server when `server.mjs` changes, so its page and server edits also take effect without rebuilding. Stack startup does not rebuild images; after changing a backend service's source or Dockerfile, rebuild only that service with `docker compose -p sbe-system -f system/compose.yml up -d --build <service>`.

Open the service control page at http://localhost:8080. It shows each container's state and can start, stop, or restart services independently, either project as a group, or everything together. The app system is defined in [system/compose.yml](system/compose.yml); monitoring infrastructure is defined in [monitoring/docker-compose.yml](monitoring/docker-compose.yml). Individual services restart automatically after process failure. Use `./stop-all.sh` to stop both projects while preserving their containers and data volumes. The React app remains available at http://localhost:5173, Grafana at http://localhost:3000, and generated coverage is linked from the control page when present.

## Code coverage

Run ./coverage.sh from the repository root to generate HTML coverage reports. The combined TypeScript and Rust index is at src/typescript/workspace/apps/react-app/public/coverage/index.html. It links to the frontend report under typescript/ and each Rust crate report under rust/<crate>/. The frontend command uses the existing Node test suite, and Rust coverage runs each crate's unit tests through cargo-llvm-cov.

Install the Rust coverage tool once with cargo install cargo-llvm-cov if it is missing.
