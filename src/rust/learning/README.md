# Learning

Learning owns strategy-profile learning, Scout selection, scheduling, outcome aggregation, and feedback. It coordinates work and sends typed assignments and lifecycle route commands to the strategy-specific Scout workers over RSocket. The workers perform Scout evaluation and lifecycle work.

## Listeners

- RSocket request-response listener: `LEARNING_RSOCKET_LISTEN_ADDR`, default `127.0.0.1:10010`.
- HTTP listener: `LEARNING_LISTEN_ADDR`, default `127.0.0.1:3010`. It exposes health, process CPU/RSS, and operational documentation only. Domain requests use RSocket.

The Learning RSocket endpoint accepts candidate batches, worker-route requests, Scout feedback, and profile-snapshot requests. Worker-route calls are dispatched to the existing local handlers for allowlisted profile, outcome, configuration, status, feedback, threshold, and execution-learning operations. `/configuration`, `/status`, and `/outcomes` are aggregated across configured strategy workers, with global pagination for outcomes.

## Configuration

- `LEARNING_STATE_FILE`: normalized SQLite state file; defaults to `learning_state.sqlite`.
- `SCOUT_WORKERS_ENABLED`: enables worker dispatch; defaults to `true`.
- `SCOUT_WORKER_ENDPOINTS`: semicolon-separated `strategy=ws://host:port` entries.
- `SCOUT_COORDINATOR_MAX_BATCH_SIGNALS`: maximum signals in a batch; defaults to `64`.
- `SCOUT_COORDINATOR_MAX_ASSIGNMENTS`: maximum assignments in a batch; defaults to `512`.
- `SCOUT_COORDINATOR_MAX_BATCH_BYTES`: maximum serialized batch size; defaults to `8388608`.
- `SCOUT_COORDINATOR_WORKER_TIMEOUT_MS`: request timeout per worker; defaults to `3000`.

Learning opens or creates its SQLite database and does not import legacy JSON state at runtime.

## Run and verify

From the repository root:

```sh
cargo run --manifest-path src/rust/learning/Cargo.toml
cargo test --manifest-path src/rust/learning/Cargo.toml
cargo fmt --manifest-path src/rust/learning/Cargo.toml --all
```
