# Strategy

Strategy contains the shared evaluator library and two process roles per
strategy. Both roles subscribe to Insights' typed `strategy-signals:*` stream.
Live workers load enabled profiles from AutoTrader and report profile-versioned
decisions back to it. Scout workers load profiles from Learning and return
Scout results and feedback to Learning. Workers own strategy rules; coordinators
own profile selection, while AutoTrader retains final risk and order authority.

## Strategy workers

Each registered strategy has two process roles. Live workers use
`STRATEGY_WORKER_ROLE=live`, `STRATEGY_WORKER_STRATEGY`, and
`STRATEGY_LIVE_LISTEN_ADDR` (default `0.0.0.0:10201`). Scout workers use
`STRATEGY_WORKER_ROLE=scout` and `SCOUT_WORKER_LISTEN_ADDR` (default
`0.0.0.0:10200`). Both roles subscribe through
`INSIGHTS_STRATEGY_STREAM_URL`. Scout workers also subscribe to Learning
through `LEARNING_SCOUT_STREAM_URL` (defaulting to `LEARNING_RSOCKET_URL`).
Learning streams profile, feedback, configuration, and historical replay
updates into each worker's latest-value cache, and delivers assignments on the
same stream. Workers return assignment results separately over RSocket
request/response. Profile snapshots refresh the live and dated-carry scout
profile cache; a never-seen symbol is bootstrapped once, then its snapshot is
broadcast to the worker caches.
Profiles in one process share a single Insights subscription.

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

- `SCOUT_WORKER_MAX_IN_FLIGHT` (default `1`): bounds concurrent Learning
  assignments and lifecycle requests; excess work is rejected rather than
  queued in worker memory. Insights updates use a separate serial subscription
  lane and do not consume this assignment capacity.
- `SCOUT_WORKER_MAX_PAYLOAD_BYTES` (default `1048576`): maximum serialized
  assignment size.
- `SCOUT_WORKER_MAX_PERSISTED_RESULTS` (default `10000`): retained result rows
  per strategy database.

Compose defines `strategy-live-<strategy>` and
`strategy-scout-<strategy>` services. Live workers run continuously; Scout
workers use the optional `scout-workers` profile. Scout CPU and memory limits
use `SCOUT_WORKER_CPU_LIMIT` and `SCOUT_WORKER_MEMORY_LIMIT`; live limits use
`STRATEGY_LIVE_CPU_LIMIT` and `STRATEGY_LIVE_MEMORY_LIMIT`.

Each worker exposes internal `/health` and `/metrics` listeners. Metrics include
role/strategy request, error, timeout, saturation, in-flight, latency, RSS, and
CPU series. `app_strategy_worker_insights_connected` and
`app_strategy_worker_last_insight_timestamp_ms` report subscription state and
the newest event consumed; live workers expose the currently installed
AutoTrader profile-set version. Scout workers export
`app_strategy_worker_insight_route_duration_seconds` for the time spent
dispatching an Insights update through the local handler. It excludes stream
transport and upstream event delivery. Scout workers also export cached
open/closed instance counts from their own SQLite database, refreshing the aggregate at most once per minute;
the exporter does not load full Scout reports to populate Grafana.

There is no central health-only Scouts process. Scout application routes run
inside strategy-specific Scout workers and are reached through Learning's
RSocket coordinator. Existing Scout data remains in 17 per-strategy SQLite
files at `/data/workers/<strategy>.sqlite` on the named volume
`sbe-scouts-state`; the Compose volume key remains `strategy-x-state` to retain
the existing Scout databases. Worker startup
does not load a shared database or legacy JSON file.
