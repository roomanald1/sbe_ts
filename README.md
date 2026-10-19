
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

Run the current backend plus the four service processes together from the repository root:

```sh
./run-backend-services.sh
```

The UI remains a separate process (`./nx serve react-app`). FeedHandlers owns RSocket on port 10001 and instrument lookup on port 3001. Position Management owns the position ledger, account, and portfolio APIs on port 3002. Insights serves signal reads and position close insights on port 3004. Execution listens on port 3003 and currently forwards order and VWAP routes to Position Management while execution is extracted.

The launcher points `POSITIONS_FILE` and `SIGNALS_FILE` at the existing persisted files under their owning services. FeedHandlers owns OKX connectivity, SBE encoding, and synthetic feed streams. Position Management owns the ledger and portfolio APIs. Execution owns order and VWAP APIs. Insights owns signal persistence and close insights.
