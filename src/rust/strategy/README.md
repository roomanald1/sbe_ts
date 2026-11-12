# Strategy X

Strategy X contains the shared evaluator library and two service roles.
AutoTrader calls the stateless live-evaluation worker for trading decisions;
Learning selects and schedules Scout experiments and dispatches them to the
separate stateful Scout worker. Workers evaluate assigned strategies and
return typed results; they do not choose strategies or schedule follow-up work.

## Strategy workers

Each registered strategy has two separate process roles. AutoTrader uses the
always-on stateless live evaluator (`STRATEGY_X_WORKER_ROLE=live`) on
`STRATEGY_X_LIVE_LISTEN_ADDR` (default `0.0.0.0:10201`). Learning dispatches
Scout work to the optional Scout evaluator (`STRATEGY_X_WORKER_ROLE=scout`) on
`SCOUT_WORKER_LISTEN_ADDR` (default `0.0.0.0:10200`). Both roles select the
strategy with `STRATEGY_X_WORKER_STRATEGY`.

Scout workers use their own SQLite file from `SCOUT_WORKER_STATE_PATH`. The
database stores idempotent assignment results and durable Scout state; duplicate
assignment IDs with a different request are rejected. Live workers have no
Scout database, volume, or Scout assignment state.

Full Scout instance and outcome history stays in SQLite. Runtime hydration
keeps at most 4,096 recent outcome samples and 4,096 recent portfolio-instance
evidence rows; the strategy-profile index is derived only from the hydrated
outcome window. Plan-response idempotency caches 32 responses, and active audit
registration caches 128 groups/4,096 audits. Older plan responses are fetched by
request ID from SQLite; Insights registration recovery pages the full durable
active-audit collection. The instance point-lookup cache also holds at most
4,096 records. Reports query durable instance and outcome history by their
requested time window, and decision-ID lookups fetch records on demand. Cache
eviction never deletes persisted history. Scout status and walk-forward evidence
describe the recent in-memory outcome window, while report history remains
durable. Startup does not rewrite or compact persisted instance history. Read
connections use a 4 MiB SQLite page cache, disable mmap, and spill large JSON
sorts to temporary files so full-database scans do not become retained
per-worker RAM.

The worker bounds resource use with configurable settings:

- `SCOUT_WORKER_MAX_IN_FLIGHT` (default `1`): excess assignments are rejected
  immediately rather than queued in worker memory.
- `SCOUT_WORKER_MAX_PAYLOAD_BYTES` (default `1048576`): maximum serialized
  assignment size.
- `SCOUT_WORKER_MAX_PERSISTED_RESULTS` (default `10000`): retained result rows
  per strategy database.

Compose defines two worker services per registered strategy: live workers run
continuously, and Scout workers use the `scout-workers` profile so they start
only when Learning dispatch is enabled. Scout CPU and memory limits can be set
with `SCOUT_WORKER_CPU_LIMIT` and `SCOUT_WORKER_MEMORY_LIMIT`; live limits use
`STRATEGY_X_LIVE_CPU_LIMIT` and `STRATEGY_X_LIVE_MEMORY_LIMIT`.

There is no central health-only Scouts process. Scout application routes run
inside strategy-specific Scout workers and are reached through Learning's
RSocket coordinator. Existing Scout data remains in 17 per-strategy SQLite
files at `/data/workers/<strategy>.sqlite` on the named volume
`sbe-scouts-state`; the Compose volume key is `strategy-x-state`. Worker startup
does not load a shared database or legacy JSON file.
