
Deployed:
https://sbe-ts-1.onrender.com/



run locally:

UI:         ```./nx serve react-app```
Backend:    ```./run-backend-services.sh```

Run Nx from the repository root with `./nx <target> <project>`, for example:

```sh
./nx build react-app
./nx lint react-app
```

## Local backend services

Run the backend services together from the repository root:

```sh
./run-backend-services.sh
```

The UI remains a separate process (`./nx serve react-app`). FeedHandlers owns RSocket on port 10001 and instrument lookup on port 3001. Position Management owns the position ledger, account, and portfolio APIs on port 3002. Insights serves signal reads and position close insights on port 3004. AutoTrader owns its trading loop and settings API on port 3005. Execution listens on port 3003 and forwards order and VWAP routes to Position Management, plus AutoTrader routes to the AutoTrader service.

The launcher points `POSITIONS_FILE` and `SIGNALS_FILE` at the existing persisted files under their owning services. FeedHandlers owns OKX connectivity, SBE encoding, and synthetic feed streams. Position Management owns the ledger and portfolio APIs. AutoTrader owns its strategy state and trading loop. Execution owns order and VWAP APIs. Insights owns signal persistence and close insights.
## Run the full local stack

Set a unique Grafana password in monitoring/.env (copy monitoring/.env.example first), then run all services from the repository root:

    ./start-all.sh

This launches the backend services, React UI, and Grafana monitoring in the background and returns to the shell. Open the UI at http://localhost:5173 and use its Grafana link for the dashboard. Run ./stop-all.sh to stop everything. Monitoring and app logs are written under logs/; startup and supervisor output is in logs/start-all.log.
