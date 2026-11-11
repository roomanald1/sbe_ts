# Scouts

Learning owns Scout selection and scheduling. A strategy worker accepts an
explicit `ScoutWorkAssignment` over RSocket request-response, evaluates only
the assigned strategy and candidate, and returns a typed `ScoutWorkResult`.
The worker does not create assignments or choose follow-up work.

## Strategy workers

Run an individual strategy worker by setting `SCOUT_WORKER_STRATEGY` to a
registered strategy name. The process then serves RSocket WebSocket on
`SCOUT_WORKER_LISTEN_ADDR` (default `0.0.0.0:10200`) and uses its own SQLite
file from `SCOUT_WORKER_STATE_PATH`. The database stores idempotent assignment
results and the strategy's durable Scout state; duplicate assignment IDs with
a different request are rejected.

Full Scout instance history stays in SQLite. Each process keeps at most 4,096
recently touched instance records in its in-memory cache. Reports read the full
seven-day report window from SQLite, decision-ID lookups fetch records on
demand, and the portfolio evidence index is rebuilt by streaming persisted
records. Cache eviction does not remove persisted Scout history.

The worker bounds resource use with configurable settings:

- `SCOUT_WORKER_MAX_IN_FLIGHT` (default `1`): excess assignments are rejected
  immediately rather than queued in worker memory.
- `SCOUT_WORKER_MAX_PAYLOAD_BYTES` (default `1048576`): maximum serialized
  assignment size.
- `SCOUT_WORKER_MAX_PERSISTED_RESULTS` (default `10000`): retained result rows
  per strategy database.

Compose defines one worker service and SQLite state file per registered
strategy. These services use the `scout-workers` Compose profile so they start
only when the Learning dispatch path is enabled. CPU and memory limits can be
set with `SCOUT_WORKER_CPU_LIMIT` and `SCOUT_WORKER_MEMORY_LIMIT`.

The legacy Scouts HTTP process now exposes only health and process metrics. All
Scout application routes run inside strategy workers and are reached through
Learning's RSocket coordinator. Existing Scout state was partitioned into the
17 per-strategy SQLite databases in `/data/workers/`; worker startup does not
load a shared database or legacy JSON file. The Compose workers share the
`scouts-state` volume and use separate files under `/data/workers/`.
