# Learning

Learning owns strategy-profile learning, Scout selection and assignment scheduling, outcome aggregation, feedback, context thresholds, and durable historical replay. It coordinates work and sends typed assignments and lifecycle route commands to the strategy-specific Strategy X Scout workers over RSocket. The workers perform Scout evaluation and lifecycle work. Live trading evaluations use the separate always-on Strategy X live worker set and are routed by AutoTrader. Trade closes enqueue profile-attributed, non-trading replay jobs; skipped live Strategy X candidates enqueue bounded missed-opportunity jobs. Learning waits for the configured horizon to mature, then pages Insights' persisted 10-second strategy snapshots and book marks. Only complete snapshot/book coverage produces diagnostic directional mid-markout bands. These are gross markouts, not executable or net P&L, so their net-return and stop-target fields remain empty and they cannot relax entry gates. Generic Insights signals are never used to infer missed opportunities.

## Listeners

- RSocket request-response listener: `LEARNING_RSOCKET_LISTEN_ADDR`, default `127.0.0.1:10010`.
- HTTP listener: `LEARNING_LISTEN_ADDR`, default `127.0.0.1:3010`. It exposes health, process CPU/RSS, and operational documentation only. Domain requests use RSocket.

The Learning RSocket endpoint accepts candidate batches, worker-route requests, Scout feedback, and profile-snapshot requests. Worker-route calls are dispatched to the existing local handlers for allowlisted profile, outcome, configuration, status, feedback, threshold, and execution-learning operations. `/configuration`, `/status`, and `/outcomes` are aggregated across configured strategy workers, with global pagination for outcomes.

## Configuration

- `LEARNING_STATE_FILE`: normalized SQLite state file; defaults to `learning_state.sqlite`.
- `SCOUT_WORKERS_ENABLED`: enables worker dispatch; defaults to `true`.
- `SCOUT_WORKER_ENDPOINTS`: semicolon-separated `strategy=ws://host:port` entries for Scout-role Strategy X workers (Compose service names `strategy-x-scout-<strategy>`, port `10200`).
- `SCOUT_COORDINATOR_MAX_BATCH_SIGNALS`: maximum signals in a batch; defaults to `64`.
- `SCOUT_COORDINATOR_MAX_ASSIGNMENTS`: maximum assignments in a batch; defaults to `512`.
- `LEARNING_REPLAY_HORIZONS_<STRATEGY>`: four increasing comma-separated millisecond horizons for live-strategy trade replay and missed-candidate markouts; defaults to `300000,1800000,7200000,28800000` (5m, 30m, 2h, 8h). The maximum horizon is 24 hours.

Missed-candidate collection accepts at most 10,000 durable events and scans at most 20 pages of 512 history records per candidate. Incomplete history, cadence gaps, corrupt records, missing book marks, or unavailable archived data keep the affected horizon out of the diagnostic bands. The recorded markout is directional mid-price movement from the candidate-time executable side quote; it is not net or realized P&L.
- `SCOUT_COORDINATOR_MAX_BATCH_BYTES`: maximum serialized batch size; defaults to `8388608`.
- `SCOUT_COORDINATOR_WORKER_TIMEOUT_MS`: request timeout per worker; defaults to `3000`.

Learning opens or creates its SQLite database and does not import legacy JSON state at runtime.

Learning's in-memory working set has explicit cardinality limits:

- 4,096 active symbol/strategy profiles. New profile identities receive `503 Service Unavailable` when this limit is full; active profiles are never evicted.
- 20,000 registered profile/parameter variants. The exact variant for each active profile is retained. At capacity, Learning evicts the least recently observed inactive variant from RAM; its SQLite row remains archived. Startup hydrates active variants plus the most recently observed variants. The `learning_variant_recency` table is created automatically and backfilled for legacy rows from their newest outcome timestamp.
- 512 outcomes per variant and 250,000 outcomes / an estimated 64 MiB of outcome detail across the whole in-memory variant set, within the existing seven-day window. Feedback and profile-registration payloads are capped at 16 KiB to bound per-record data. When the global limit is reached, the oldest detailed outcomes are removed across variants while profile and variant identities remain. A durable retention marker prevents startup from rebuilding persisted active-profile counters from incomplete detail.
- 10,000 context-stat keys and 10,000 execution-stat keys. New-key imports/events receive `503 Service Unavailable` when a map is full; existing keys continue to update.
- Existing limits remain: 100,000 threshold samples, 100,000 feedback IDs in RAM, 100,000 execution-event IDs in RAM, 10,000 exit-timing observations, 10,000 strategy replay jobs, 10,000 missed-opportunity jobs, and 2,000 missed-opportunity bands.

Archived variant, context-stat, and execution-stat rows stay in SQLite and are not loaded into RAM after they fall outside the bounded working set. Feedback and execution-event IDs also remain in SQLite after leaving the RAM set; when either ID set has overflowed, a cache miss performs an indexed SQLite existence check before accepting the event. The database can therefore continue growing even though Learning's active state is bounded. Replay/missed-opportunity job rows are durable workflow state and are never removed by the variant compactor; existing queue policy only reclaims terminal jobs to admit new work. Outcome detail beyond the count/window limits is intentionally discarded from the active variant payload, but active profile identities and the profile counters persisted before compaction remain available. SQLite archive size has no automatic retention cap yet.

Compose keeps the Scout worker set under the optional `scout-workers` profile. Each strategy's Scout process uses `/data/workers/<strategy>.sqlite` in the existing `sbe-scouts-state` named volume. Live Strategy X workers are independent of Learning's Scout dispatch and persistence.

## Run and verify

From the repository root:

```sh
cargo run --manifest-path src/rust/learning/Cargo.toml
cargo test --manifest-path src/rust/learning/Cargo.toml
cargo fmt --manifest-path src/rust/learning/Cargo.toml --all
```
