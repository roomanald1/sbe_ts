# Strategy worker extraction plan

## Objective

Merge the `scouts` and `strategies` Rust crates into one `strategy` crate,
then move every production strategy decision out of Autotrader and into the
out-of-process Strategy X service, while retaining Autotrader as the live
trading coordinator. Keep Scout as the Learning-owned experiment and candidate
concept, not a separate service or crate.

Live trading is the first-class workload. Scout evaluation must never consume
capacity reserved for live decisions.

## Implementation and verification status (2026-11-12)

- The `strategies` and `scouts` crates are merged as `strategy`. The versioned
  live request contract and stateless worker require an explicit `live` or
  `scout` role and one strategy. No central Scouts or Live Signals service
  remains in source or Compose.
- AutoTrader sends all production live entry, daily, dated-carry,
  open-position, settlement, and rebalance decisions to its dedicated live
  workers. Entry failures fail closed; protective-order reconciliation runs
  before remote evaluation and does not depend on Learning.
- Compose/control/monitoring define 17 live and 17 Scout workers. The Scout
  pool has lower CPU shares and separate state. The local native launcher now
  starts the same two roles per registered strategy with unique loopback ports.
- Insights is the only live signal producer. The research-only switch, Live
  Signals crate, generic raw-signal walk-forward, missed-opportunity producer,
  and one-hour counterfactual book history are removed. The compact one-minute
  book window remains for live features; historical snapshots are paged from
  SQLite at their stored 10-second cadence.
- Learning owns bounded, profile-attributed walk-forward, post-trade replay,
  and missed-opportunity analysis. Replay pages exact strategy snapshots and
  records; incomplete coverage is unsupported/indeterminate. Missed-candidate
  bands use complete directional mid-markouts only and cannot be interpreted
  as net executable returns or relax entry gates.
- Learning's RAM working set now caps outcomes at 512 per variant and 250,000
  globally, variants at 20,000, active profiles at 4,096, and context/execution
  maps at 10,000 each. Older variant, context, and execution rows stay in
  SQLite; durable job and idempotency state is preserved.
- AutoTrader keeps one latest signal per active position strategy key, expires
  it after 10 seconds, and caps audit queues at 8 MiB each. Its position
  prewarm cache is limited to 4,096 entries/5 minutes and Learning response
  cache to 128 entries/30 seconds.
- Insights defaults to at most 256 active symbols (2.5× FeedHandlers' top-100
  universe), evicts all symbol-keyed state together after 60 seconds, rejects
  new symbols if no safe eviction is available, and caps news-attribution
  pending state at 8,192 entries/60 seconds. Its dashboard calibration window
  is limited to 1,000 snapshots and 5,000 outcomes cached for 15 seconds.
- Scout SQLite remains durable. Runtime hydration bounds recent outcomes to
  4,096, plan-response cache to 32, active audits to 128 groups/4,096 rows,
  portfolio evidence to 4,096 records, and instance point cache to 4,096.
  Older reports/retries/registrations query SQLite. Startup no longer rewrites
  the full persisted instance history; SQLite read connections use a 4 MiB page
  cache, disable mmap, and spill sorts to disk. Scout outcome report cache has
  a 4 MiB per-response and 8 MiB aggregate byte cap with a 30-second TTL;
  oversized responses are returned without caching.
- Unit tests passed for AutoTrader (160 library, 7 binary), Learning (58
  library, 1 binary), Insights (40 library, 40 binary), FeedHandlers (63), and Strategy X (159
  with service). React passed 110 tests; homepage typecheck and production build
  passed; exporter passed 7 tests. The Rust microbenchmarks and test/format
  commands are recorded in [BENCHMARK_BASELINE.md](./BENCHMARK_BASELINE.md).
- The full Compose cutover removed the legacy containers and ran 17 live plus
  17 Scout workers. The homepage liveness check passed 17/17; Prometheus saw
  all 34 worker endpoints. Before the final source-side memory reductions,
  measured RSS was about 1.85 GiB AutoTrader, 2.63 GiB Insights, 2.52 GiB
  Learning, and 0.65–0.96 GiB for the five live-strategy Scout workers. No
  container had restarted or OOMed. Compose now reserves 3 GiB for AutoTrader,
  1.5 GiB for Scout workers, and 1 GiB for the React dev server.
- **Runtime deployment completed:** Docker Desktop has the existing
  `sbe-insights-data`, `sbe-learning-data`, and `sbe-scouts-state` volumes. The
  full source and Compose rebuild is deployed, including the renamed `strategy`
  crate/binary and Scout 1.5 GiB cap. The React dev server initially failed
  with Node heap OOM; its limit is now 1 GiB with a 768 MiB Node heap. Homepage
  liveness subsequently passed 17/17, including all 44 system and 8 infra
  containers. Prometheus reports 34/34 worker metrics endpoints.
- After the Scout report-cache cap and rebuild, an early 90-second snapshot
  showed AutoTrader 1.4 GiB, Insights 1.16 GiB, Learning 226 MiB, and maximum
  Scout RSS 1.06 GiB. Several other Scout workers were below 150 MiB. These
  readings were still changing during startup and active feed traffic.
- Early post-rebuild RSS snapshots: AutoTrader 594 MiB, Insights 2.64 GiB,
  Learning 635 MiB. The highest Scout worker reached about 1.08 GiB of its
  1.5 GiB cap (72%). These are short post-start snapshots; measure after a
  longer steady interval and while representative report/replay work runs.
- Remaining memory risks are durable database/WAL growth, total disk retention,
  sustained Scout-worker RSS under reports/replay, and post-rebuild measurements.
  Full peak-load p95/p99 parity against the former in-process evaluator was not
  measured; the tests and benchmarks are unit/microbenchmarks, not an end-to-end
  shadow run.
- **Grafana worker monitoring implemented:** availability, RSS, RSS as a
  percentage of the role's configured limit, throughput/saturation, p95 latency,
  and errors/timeouts panels sit in the Runtime Performance section. Worker
  availability is shown in a role/strategy table; filters cover all 34 series.
  JSON, Prometheus metrics, and the provisioned Grafana API dashboard were
  verified after reload. A histogram bucket double-counting bug was fixed and
  all 34 workers rebuilt; wait for a fresh five-minute live request window
  before trusting the new p95 panel values.

## Target ownership

- **Learning** owns learned profiles, Scout selection and scheduling, outcome
  aggregation, feedback, walk-forward job scheduling/evidence, and post-trade
  exit-timing analysis. It dispatches Scout work and asynchronous replay jobs
  to Strategy X.
- **Autotrader** owns live strategy selection, profile lookup, portfolio and
  risk checks, position and order lifecycle, and trade outcome attribution. It
  sends live entry and position-decision requests directly to Strategy X.
- **Strategy X** is one Rust crate/package containing both the reusable
  strategy library (registry, evaluators, parameter specs, and wire contracts)
  and the service binary (Scout worker routes and live evaluation endpoint).
  It evaluates only the supplied strategy, signal or ordered replay feature
  sequence, learned parameters, and position context. It does not schedule
  analysis jobs, select strategies, consume market feeds, make portfolio
  decisions, or place orders.
- **FeedHandlers** remains the canonical raw market-data transport and book
  normalizer. There must be one owner for derived rolling market features and
  live signal snapshots; it keeps only compact bounded hot state. Autotrader
  combines a snapshot with the selected profile/evidence to form the typed
  Strategy X evaluation request.
- **Insights/research storage** owns durable market-feature history and
  asynchronous research/outcome analysis. Insights always runs its live
  signal path; research sampling is an independent background cadence and must
  not switch off or replace live signal generation. Large historical windows
  and replay data are read from durable storage, not retained as duplicate
  per-symbol arrays in live services.
- **Live Signals** is removed. Insights owns live signal calculation and
  delivery; no second process subscribes to the same stream to rebuild signals.
- Protective price-triggered order execution remains in position management.

Scout work and live evaluation are distinct protocols and workload classes.
Scout assignment/lifecycle routes must not be reused as the live decision API.
Walk-forward evidence is owned by Learning and is keyed to a specific strategy,
symbol, and parameter profile. Learning derives it from prior attributed
strategy outcomes before the candidate's evaluation timestamp. Insights
publishes market features only: remove walk-forward fields and calculations
from insight signals, insight APIs, and insight-side consumers. Learning joins
evidence to the selected strategy/profile. It attaches evidence to Scout work
and publishes profile/evidence snapshots for Autotrader to attach to live
Strategy X requests. Learning does not dispatch live evaluations. Raw market
signals are never walk-forward performance evidence.

There are two related but distinct Learning loops:

1. **Pre-decision walk-forward validation:** when Learning or Autotrader
   evaluates a strategy/profile candidate, Learning can schedule asynchronous
   replay over configured periods. Replay applies that strategy's evaluator
   and exact parameter profile to an ordered sequence of market-feature
   signals and records strategy-level decisions and outcomes. Keep candidate
   validation separate from trade exit analysis so the result cannot be
   mistaken for actual P&L evidence.
2. **Post-trade exit-timing analysis:** Autotrader sends Learning attributed
   close facts through its existing durable feedback path. Learning inserts an
   idempotent replay job keyed by the close event/trade ID and schedules
   longer-horizon, non-trading replay for that exact strategy/profile and
   trade context. Do not run unrelated strategies or duplicate live trades.
   Use strategy-specific configured replay horizons and market-data cadence;
   do not claim a generic one-second execution replay from the current
   10-second research features. Once each window matures, compare actual
   entry/exit and realized outcome with the same strategy/profile's
   counterfactual decision exits. Report decision timing as earlier, later, or
   within tolerance of the actual exit. Report an economic early/late benefit
   only when complete executable-cost evidence shows a meaningful improvement;
   otherwise the economic label is `indeterminate`. Keep 10-second markouts
   explicitly approximate.

Initial replay horizons should be explicit and configurable: live signal
strategies use 5 minutes, 30 minutes, 2 hours, and 8 hours after entry; daily
bar strategies use 1, 5, and 20 completed sessions once daily strategy inputs
are persisted; dated carry uses the trade's scheduled expiry/settlement window.
The live horizons fit the persisted 10-second signal snapshots only through
bounded paging. Daily and carry jobs remain `unsupported` until their exact
input histories are queryable. At 10-second resolution, labels describe
strategy decision timing, not execution-accurate fills.

These jobs do not launch additional live strategies or orders. They use the
existing Learning-side Strategy X Scout process with a separately bounded,
low-priority replay work class; they cannot consume live worker slots. Keep
actual trade outcomes distinct from counterfactual replay outcomes; only
aggregate repeated and profile-matched evidence into bounded learning updates.

Keep Learning's strategy-attributed threshold walk-forward separate from the
generic raw-signal walk-forward formerly calculated in Insights (now removed).
**Implemented:** Learning validates/promotes threshold parameters and publishes
the active profile snapshot; Autotrader no longer stores threshold samples or
recomputes train/holdout scores. User overrides retain precedence, and Scout
candidate parameters are applied for their own evaluation with matching
profile identity. Remove generic signal walk-forward from Insights live
contexts and signal payloads. Replays use fixed training and evaluation
windows with no look-ahead leakage.

## Capacity and priority model

Deploy separate **live** and **scout** worker pools from the same Strategy X
binary and evaluator code. Autotrader can call only the live pool; Learning can
call only the Scout pool. This hard isolation ensures a Scout batch cannot
occupy the slots needed by live trading. Each pool has independent concurrency,
timeouts, resource limits, and metrics. The live pool receives reserved CPU and
memory; Scout workers may be scaled or shed independently.

Do not rely only on a priority queue inside shared worker processes: if Scout
work occupies all in-flight slots, a priority label cannot protect a live
request already waiting for capacity.

## State and market-feature access

Learning and Autotrader do not need to hold the full Insights history in
memory. Keep state local to each service's responsibility and retrieve
historical feature ranges on demand:

- **Autotrader hot state:** current positions/orders, bounded queues of pending
  decisions, latest per-symbol/per-strategy feature snapshots needed for live
  entry and exit evaluation, and short-lived caches. Feed/Insights pushes or
  streams current updates so a time-sensitive decision does not depend on a
  round trip to fetch every feature. Fetch additional context only when needed.
- **Learning durable state:** learned profiles and parameter versions,
  attributed actual and simulated outcomes, walk-forward summaries, and
  durable replay-job/checkpoint state. It does not retain the full market
  feature history in process memory. It serves bounded profile/evidence
  snapshots to Autotrader; Autotrader includes the selected profile and
  strategy-specific evidence in its direct Strategy X request.
- **Historical features:** keep time-series feature history in its durable
  market-data/Insights store. Learning requests only the symbol, feature set,
  and time range required by a replay job, in bounded pages/chunks. Cache only
  bounded working sets and persist job progress so a restart does not require
  reloading the entire history.
- **Replay inputs:** Learning associates retrieved feature sequences with the
  exact strategy/profile and submits bounded replay work to Strategy X. The
  sequence is replayed in timestamp order; strategy-specific decisions and
  outcomes are produced by the evaluator. Insights serves market features,
  not walk-forward scores or strategy conclusions.

Use explicit limits on time range, page size, bytes, and replay concurrency.
If required history is unavailable or incomplete, mark the replay evidence
insufficient; do not silently substitute current snapshots or unrelated
signals. Live evaluation should use its pushed current snapshot and bounded
context, not wait for long-range history retrieval.

## Memory and hot-path isolation

Minimize total memory by eliminating duplicated histories and bounding every
in-memory structure, while reserving a measured memory floor for Autotrader's
hot path:

- Autotrader retains only active positions/orders, settings/profile snapshots,
  latest required per-symbol live features, and bounded pending work. Expire
  stale signal entries and cap diagnostics/audit buffers. Do not keep full
  Insights history, replay windows, or Scout candidate batches in Autotrader.
- Live Strategy X workers are stateless between requests apart from small
  bounded health/metrics state. Use bounded payloads and in-flight requests;
  avoid per-trade history caches. Keep live workers in a dedicated pool with
  reserved CPU and memory, separate from Learning's Scout/replay work class.
  Run exactly two Strategy X processes per registered strategy: one live
  process called by Autotrader and one Learning-side Scout/research process.
  Bound and prioritize Scout assignments and replay inside that Learning-side
  process; do not add a third replay process. Keep the live process stateless;
  only the Learning-side process needs persistence.
- Learning persists profiles, outcomes, and replay job checkpoints in its
  durable store. Page through history and stream/replay bounded chunks rather
  than materializing complete periods or large candidate batches in memory.
  Store compact summaries and provenance, not duplicate feature series.
- Insights keeps only the working window needed to calculate current features
  and bounded query caches. Historical features remain in the durable store
  and are read by range; avoid copying broad history into both Learning and
  Autotrader caches.
- Bound request bytes, queue length, result retention, caches, batch size,
  concurrency, and replay duration. Apply admission control to Learning/replay
  before memory pressure reaches Autotrader or the live Strategy X pool.
- Set process/container memory limits from measured steady-state and peak
  workloads. Alert on RSS, allocation/queue growth, and OOM/restart events.
  Memory limits must leave headroom for the Autotrader process and its live
  worker calls under peak signal load.

Measure resident memory and hot-path p95/p99 latency under the same peak live
signal workload with Scout/replay load enabled and disabled. The acceptance
target is the smallest stable bounded footprint that preserves the existing
Autotrader latency/error budget; do not trade away hot-path headroom to reduce
idle RSS.

## Current-state review and required changes

The current code already has useful bounds, but some important memory is still
duplicated or scales with traffic/history:

| Area | Current protection | Required change |
| --- | --- | --- |
| Autotrader pending live signals | Configurable symbol cap, default 4,096 | Set from the supported active universe; coalesce by symbol and cap bytes as well as entries. |
| Autotrader latest strategy signals | **Implemented:** one newest sample only for active strategy-position keys; expire after 10 seconds. Position count is bounded by the configured 20-position limit. | Monitor strategy exit decisions after expiration and verify active-position count during recovery. |
| Autotrader audit queues | **Implemented:** 16,384 decision / 4,096 execution records and 8 MiB estimated payload each. Decision rejections coalesce/evict; execution audit records are never evicted for admission. Queue saturation drops the incoming record, latches audit health, and pauses new entries. | Observe byte estimate and saturation telemetry under peak market/order load; consider durable disk spill if execution-audit saturation occurs. |
| Autotrader metrics/cache | Several timing samples cap at 500. **Implemented:** Learning response cache caps at 128 entries/30 seconds; market-book prewarm cache caps at 4,096 symbols/5 minutes. | Measure serialized response bytes and remaining diagnostics under peak load; keep fixed-size rings and latest diagnostics only. |
| Learning state | **Implemented:** SQLite is the durable source for archived history; active/recent profiles and bounded aggregates are hydrated into capped working sets. | Measure retained bytes and query latency with representative production history. Keep older outcomes and replay records paged by profile/time and avoid full-history clones. |
| Learning retention | **Implemented:** 4,096 active profiles (admission fails when full), 20,000 in-memory variants (active exact variants preserved; inactive variants evicted by recency), 512 outcomes/variant and 250,000 outcomes / estimated 64 MiB globally, 10,000 context keys, and 10,000 execution keys. Legacy SQLite variant recency is backfilled and only active/recent variants are hydrated; archived variant/context/execution rows and full event-ID sets remain on disk. Overflowed event-ID cache misses query SQLite before accepting duplicates. Existing limits remain: 100k threshold samples and IDs, 10k exit observations/replay jobs/missed-opportunity jobs, 2k bands. | Measure retained bytes and archive DB growth after cutover. Durable archived variant/context/execution rows and replay/missed-opportunity job history can grow over time; define a separate disk-retention policy without weakening attribution or pending-job durability. |
| Threshold walk-forward | Learning stores bounded strategy-attributed threshold samples and publishes active profiles | **Implemented:** Learning is the sole validator; Autotrader consumes the active profile snapshot and no longer persists duplicate samples or scores. Keep the sample history bounded/compacted in Learning. |
| Learning exit timing | **Implemented:** attributed trade-close feedback creates a durable replay job for the exact strategy/profile; Learning pages stored feature snapshots and records horizon results with coverage and indeterminate status. | Measure queue age, replay completion, and storage growth under real trade volume. Daily/carry replay stays unsupported until exact input history is available. |
| Insights signal engine/history | Active symbol state caps at 256 with a 60-second idle TTL; research snapshots are captured every 10 seconds and paged by symbol/time/cursor with coverage metadata. The one-second quote history is removed. | Replay only at captured 10-second resolution; mark execution/slippage or unsupported inputs indeterminate. Verify completeness metadata under production retention. |
| Insights symbol-hash event workers | Sixteen queue workers hash by symbol, preserving same-symbol event order while processing different symbols concurrently; each shard has a bounded 1,024-item channel. They do not retain a duplicate engine per symbol. | Keep ordering/concurrency. Add byte-aware ingress/research queue budgets and profile whether fewer workers/capacity preserve the live latency/error budget. |
| Live Signals | Removed from Compose, service control, monitoring, and source. Before removal, sixteen shard-local actor maps retained a second `SignalEngine` per observed symbol, including 3,601 one-second full-depth book samples. | Keep Insights as the sole live signal producer; do not reintroduce a parallel Live Signals process. |
| Cross-service signal state | The current working tree has Insights as the sole live signal producer: it starts live delivery unconditionally, samples research independently, and Compose/control/monitoring no longer deploy Live Signals. The old `INSIGHTS_RESEARCH_ONLY` switch and Live Signals crate have been removed. | After deployment, verify there is exactly one active Autotrader/Scout producer and compare signal cadence, news attribution, Scout inputs, and error/backpressure behavior with the previous path. |
| Insights queues/caches | Writer queue 4,096, signal queue 8,192 plus 16 bounded symbol-hash shards; active symbols cap at 256; calibration cache holds at most 1,000 snapshots / 5,000 outcomes for 15 seconds; news pending state caps at 8,192 entries / 60 seconds. | Measure queue bytes and peak RSS with the full feed universe. Generic walk-forward, missed-opportunity calculation, and one-hour counterfactual quote history are removed. |
| Strategy workers | One live worker and one Learning-side Scout worker per strategy. Live memory cap is 256 MiB; Scout cap is 1.5 GiB; CPU shares favor live workers 4:1. Scout outcome report cache is capped at 4 MiB per response and 8 MiB total. | Measure sustained RSS under report/replay requests. Before the latest rebuild, three Scout workers reached 0.7–0.96 GiB under the old 1 GiB cap; continue verifying the working set under real report/replay load. |
| Core service limits | Compose caps FeedHandlers 512 MiB, Binance/Execution 256 MiB, Insights 4 GiB, AutoTrader 3 GiB, Learning 10 GiB, and Position Management 8 GiB. | Measure steady and peak RSS under normal market and Scout/replay load. Reduce limits only after preserving trading headroom and confirming no OOMs. |

Do not shrink already bounded per-symbol feature windows by guesswork. First
remove unneeded/duplicated caches and cap global cardinality, then profile peak
RSS and p95/p99 latency at the supported symbol count. Reduce feature windows
only if strategy behavior remains equivalent and hot-path latency stays within
budget.

The short `book_history` (up to 601 full-depth samples over roughly one
minute) is used by live snapshot absorption/replenishment calculations and
research feature extraction. Do not delete it without replacing those temporal
calculations with equivalent compact rolling state. The one-second, one-hour
`counterfactual_book_history` only fed raw-signal stop/target quote-path
analysis and its missed-opportunity consumer. Both production consumers are
now removed, and code search found no remaining non-test reader. The full-depth
counterfactual history, quote-path generation, and obsolete record writer have
been removed; legacy decoding remains only for stored-row compatibility.

The existing Insights research capture samples at a 10-second cadence. The
one-second counterfactual book path has been removed; do not imply that the
current research store contains execution-resolution quote history. Before
promising any replay or exit-timing horizon, inventory which symbols, features,
and intervals are actually persisted and compare that resolution with each
strategy's decision cadence. Add a separate bounded capture path or report
insufficient history where it cannot support the requested counterfactual.

The removed Live Signals implementation retained a second per-symbol engine
and counterfactual quote-path buffer (up to 3,601 full-depth samples, about
0.85 MiB per symbol). That duplicate engine is no longer in the stack. The
separate Insights hour-long counterfactual history and its production quote
path writer are now removed; only legacy database decoding remains. The compact
one-minute history remains required by live features. Insights signal-event queues use 16 symbol-hash shards to
serialize events for the same symbol while processing different symbols in
parallel. Keep this ordering/concurrency mechanism, and measure or reduce its
queue byte caps rather than treating the workers as duplicate per-symbol
state. Learning now uses bounded/selective hydration while retaining durable
records in SQLite. Continue measuring the working set and define disk-retention
rules for archived rows and completed jobs.

## Work sequence

### 1. Establish feature ownership and remove duplicate state

- Trace each raw feed, enriched event, derived feature, news overlay, signal,
  research record, and consumer. Identify data/decisions that no downstream
  consumer uses.
- Insights is the single always-live signal calculator using its direct
  AutoTrader push path. **Implemented in the working tree:** removed
  `INSIGHTS_RESEARCH_ONLY`, made research sampling independent of live snapshot
  cadence, and removed the Live Signals crate/service from Compose, service
  control, monitoring, and documentation. Runtime deployment is complete;
  homepage liveness passed 17/17 after the React heap fix. Full shadow parity
  against the removed in-process path was not run.
- **Implemented:** remove one-hour full-depth quote arrays and their quote-path
  writer from the live calculation path; preserve the bounded one-minute
  window required for live features. Add a dedicated capture path only if a
  replay job can justify its resolution and retention cost.
- Compare existing live outputs during the transition and preserve signal,
  news attribution, and delivery semantics before deleting the duplicate path.

### 2. Merge the crates and define contracts

The crate merge, shared library/service package, typed live contract, and
per-role worker wiring are implemented in the current working tree. Clients
depend on the library without enabling the service feature. Existing Scout
SQLite paths remain unchanged.

- Move the `strategies` library modules and `scouts` service modules into one
  crate at `src/rust/strategy`, retaining a library target and the Strategy X
  service binary target. Move the service's implementation and public APIs
  rather than introducing a crate dependency cycle.
- Update all Rust dependencies and imports, root build configuration, Docker
  build arguments, Compose references, CI/scripts, and docs from the two old
  crate paths/names to `strategy`.
- Keep module-level boundaries inside the merged crate: strategy evaluation
  and contracts stay reusable; HTTP/RSocket service handlers and persistence
  stay in service modules. Learning/Autotrader depend on the library API, not
  on service internals.
- Add versioned, typed live evaluation request/response contracts in the
  `strategy` library API. Include correlation ID, strategy ID, profile ID and
  parameter snapshot, signal features, decision stage, and the relevant open
  position context.
- Return a typed `Enter`, `Reject`, `Hold`, `Exit`, or `Rebalance` decision,
  reason, and auditable parameter/profile identity.
- Keep Scout work assignment/result and Scout lifecycle contracts distinct.
- Add a separate bounded replay job contract for Learning-scheduled analysis.
  Identify strategy, symbol, immutable profile/parameter version, anchor event
  or trade ID, ordered feature window, observation period, and replay purpose.
  Return decisions and strategy-level outcome summaries with provenance;
  clearly distinguish simulated results from actual trade outcomes.
- Keep walk-forward evidence and its strategy/profile identity in Learning's
  contracts; Strategy X receives the already-computed evidence as an input to
  evaluation and does not calculate or own its historical aggregation.
- **Implemented:** remove generic walk-forward results from Insights
  calculation, caches, routes, dashboard payloads, and UI. Keep
  strategy-attributed evidence as a separate Learning-owned evaluation context
  so it cannot be mistaken for a market feature or reused across
  strategy/profile variants.
- Define replay periods, required feature history, maturity rules, and
  exit-timing labels in Learning. Replay consumes ordered feature inputs; the
  Strategy X worker does not subscribe to feeds or fetch market data.
- Define an on-demand historical feature query for Learning replay jobs, with
  bounded windows/pages and completeness metadata. Keep current live feature
  delivery push/stream based for low latency; do not require Autotrader or
  Learning to preload all Insights history into RAM.
- Rename service/runtime identifiers to Strategy X. Retain Scout naming for
  experiments, candidate profiles, assignment concepts, Scout protocol routes,
  and Learning APIs.

### 3. Implement Strategy X live evaluation

- **Implemented:** expose a live request-response route alongside the Scout
  worker route. The response is correlated and preserves strategy/profile
  identity.
- Reuse existing registered evaluators; do not copy strategy logic into the
  service layer.
- Validate contract version, strategy and profile identity, parameter bounds,
  required learned parameters, and request size before evaluation.
- Make live evaluation stateless. Keep Scout idempotency/lifecycle state
  isolated from live requests and avoid coupling live availability to Learning
  state or its database.
- Remove any Strategy X/worker-side walk-forward query or calculation path.
  Learning is the source of truth for historical outcomes and walk-forward
  evidence for both Scout evaluation and any learned profile gating that uses
  it.
- Implement a durable, deduplicated Learning replay-job queue with bounded
  concurrency, retention, retries, expiry, and explicit pending/running/
  completed/failed status. Replay is a separately bounded, low-priority work
  class on Learning's existing Strategy X Scout worker process per strategy;
  it does not create a third process or use the live endpoint. Scout assignment
  concurrency must be reserved/limited so replay cannot starve its work.
- Persist replay cursors/checkpoints and request historical features by exact
  range as each job needs them. Bound in-memory queues and caches in both
  Learning and Autotrader; expose cache age, feature-history gaps, and replay
  input completeness in diagnostics. Verify stored cadence, retention, symbol
  coverage, and fields meet each replay window before scheduling it.
- Inventory and cap all process-local collections, caches, request batches,
  pending queues, and retained results. Set measured per-process/container
  memory budgets and reserve live Autotrader/Strategy X headroom independently
  from Learning, Scout, and replay budgets.
- Specifically cap/expire Autotrader's latest strategy-signal map, make audit
  queues byte-aware, add global Insights symbol cardinality/idle eviction,
  remove the confirmed-unused one-hour `counterfactual_book_history` and its
  quote-path record path, retain only required compact live feature windows,
  and replace Learning's whole-state RAM hydration with selective and
  compacted durable-state access. Keep the 16 Insights symbol-hash event
  workers unless profiling shows an equivalent ordering/concurrency design
  with lower bounded memory.
- Add measured Compose memory limits/reservations to Autotrader, Learning,
  and Insights. Strategy X live workers are capped at 256 MiB and Scout workers
  at 1 GiB based on observed new-worker RSS (up to 544 MiB); verify those caps
  after orphan removal and representative replay load. Preserve more CPU
  shares for live workers than Scout/replay workers.
- **Implemented:** Insights no longer calculates or serves generic signal
  walk-forward results. Remaining `strictWalkForwardPassed` UI fields belong
  to AutoTrader outcome diagnostics, not Insights market-signal payloads.

### 4. Route the complete Autotrader decision path

- **Implemented:** add an Autotrader client for all live entry paths, including live insight,
  daily strategy, dated-carry entry and settlement, open-position decisions,
  and strategy-triggered rebalance recommendations.
- Cover current direct evaluator callsites in `entry/cycle_entry.rs`,
  `daily_strategy_route.rs`, `dated_carry.rs`, `exits/position_exit_rules.rs`,
  and daily position-rebalance logic. The daily route currently evaluates
  signals before and after learned-parameter injection; have one remote result
  drive both status and entry rather than evaluating twice.
- Send decision and trade lifecycle/outcome events to Learning with stable
  trade/signal IDs and the exact strategy/profile/parameter snapshot. Learning
  uses those events to schedule asynchronous replay; it does not ask Autotrader
  to run parallel live strategies.
- Send batches of independent evaluations when practical, while preserving
  per-request identity and response validation.
- Keep final risk gates, position state, order decisions, and outcome recording
  in Autotrader. Worker output is a strategy decision, not an executable order.
- Keep broker/order safety in Autotrader/Position Management. A worker timeout
  must not block protective-order reconciliation; an unavailable strategy
  result must not create a new entry or unsupported strategy exit.
- Do not leave a permanent mixture of in-process and remote production
  evaluation paths.

### 5. Deploy the merged service

- **Implemented in Compose/control/monitoring:** rename service identifiers,
  worker endpoints, health checks, and runtime references to Strategy X.
  Compose defines 17 live workers by default and 17 Scout workers in the
  `scout-workers` profile; five live service hostnames were normalized to
  avoid a duplicated `live-` prefix. The full stack rebuild and cutover completed
  against the existing state volumes; liveness passed 17/17.
- **Implemented in Compose:** configure exactly two deployments/endpoints per
  registered strategy, one live endpoint for Autotrader and one Scout endpoint
  for Learning. Live workers remain available independently of Learning's
  dispatch; Scout workers use the opt-in `scout-workers` profile. Replay will
  share Learning's Scout-role process with its own bounded low-priority work
  class; it does not add a third process per strategy.
- Preserve existing Scout databases and volume contents. If paths or schemas
  move, provide and verify an explicit offline migration; do not import or
  migrate data on service startup.
- Update service dependency declarations so Autotrader's live evaluation does
  not require the Learning coordinator to be available.

### 6. Validate parity, then cut over all strategies

- Shadow every production strategy and decision stage against the worker API
  while the existing in-process evaluator remains authoritative.
- Record and resolve mismatches by strategy, decision stage, profile, and
  contract version. Verify learned parameter attribution and position context.
- Cut over the full strategy set to worker responses after parity is acceptable.
  The parity period is validation, not a strategy-by-strategy rollout.
- On live worker timeout/unavailability, fail closed for new entries and report
  the failure. Existing protective-order and position-management paths must
  continue operating independently. Do not fall back to in-process evaluator
  logic after cutover.

### 7. Remove legacy execution and operate the new boundary

- **Implemented:** remove all production Autotrader direct evaluator calls;
  retain shared Strategy X contracts. The two remaining local evaluator calls
  are test-only.
- **Implemented:** update the affected root, Strategy X, Learning, AutoTrader,
  and Insights README files to describe the architecture and ownership
  boundaries. FeedHandlers already documented its raw-market-data-only role.
  Continue to keep these docs aligned as the remaining replay, history-removal,
  and resource-budget work lands.
- **Implemented:** per-worker `/health` and `/metrics`, exporter scraping, and
  Grafana availability, RSS/limit, throughput/saturation, latency, and
  errors/timeouts panels for all 34 live and Scout services. Panels are in
  Runtime Performance and filterable by role and strategy. Confirm the provisioned
  dashboard reloads in the UI and keep the monitoring README aligned.
- Track replay queue age, maturity horizon, completion rate, counterfactual
  exit labels, and profile-attributed feedback separately. Replay backlog must
  never consume live capacity.
- Alert on live deadline misses and live pool saturation. Apply backpressure or
  reject Scout assignments before they can affect live resources.
- Document service health checks, capacity settings, rollout, rollback, and
  state migration procedures.

## Acceptance criteria

1. Every production entry and open-position strategy decision is evaluated by
   Strategy X out of process; Autotrader owns trading and risk orchestration.
2. Learning coordinates Scout work, and Scout work has no access to the live
   worker pool or its reserved compute capacity.
3. Contract and profile identity are preserved through decisions, fills, and
   learning feedback.
4. Walk-forward evidence is calculated in Learning from prior outcomes
   attributed to the exact strategy/profile and is never substituted with
   un-attributed raw signal history; Insights publishes no walk-forward data.
5. Autotrader trade feedback schedules asynchronous, strategy/profile-matched
   replay to assess exit timing; replay places no live orders and actual
   outcomes remain distinct from counterfactual results.
6. Live worker failure blocks new entries without disabling protective order
   handling or position management.
7. The `scouts` and `strategies` crates are merged into one `strategy` crate;
   service identifiers use Strategy X while Scout-domain terminology and
   persisted state are preserved.
8. Parity, load isolation, failure behavior, service health, and operational
   procedures are verified before retiring the in-process path; all affected
   module READMEs match the implemented ownership and deployment model.
9. Peak-load measurements show bounded memory across all services and no
   material Autotrader hot-path latency or error-rate regression when Scout and
   replay workloads are active.
10. No service loads complete Insights or Learning history into RAM; every
    queue, cache, symbol map, retained result set, and worker pool has explicit
    cardinality and byte bounds.
11. Exactly two Strategy X processes run per registered strategy: Autotrader's
    live worker and Learning's Scout/replay worker, with separate state and
    resource budgets.
12. There is one owner of live rolling feature state: Insights. The Live Signals
    process and crate are removed, the orphaned one-hour counterfactual book
    history is removed, and only the bounded one-minute history required by
    live absorption/research features remains.
13. Insights has one always-live operating mode. Research sampling runs
    independently and cannot disable or replace live signal generation; the
    Compose `INSIGHTS_RESEARCH_ONLY` switch is removed and there is one producer
    of live Autotrader and Scout inputs.

## Operational constraint

Implementation and Compose changes do not authorize cycling running services.
Restart only the affected services after explicit authorization, then run the
homepage System liveness check as required by the repository instructions.
