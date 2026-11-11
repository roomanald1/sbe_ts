# Strategy worker extraction plan

## Objective

Merge the `scouts` and `strategies` Rust crates into one `strategy_x` crate,
then move every production strategy decision out of Autotrader and into the
out-of-process Strategy X service, while retaining Autotrader as the live
trading coordinator. Keep Scout as the Learning-owned experiment and candidate
concept, not a separate service or crate.

Live trading is the first-class workload. Scout evaluation must never consume
capacity reserved for live decisions.

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
- **Live Signals** is a bounded, near-stateless handoff for computed live
  signals, or is removed if delivery can be folded into the chosen feature
  owner. It must not rebuild and retain a second `SignalEngine` for the same
  event stream.
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
   signals and records strategy-level decisions and outcomes.
2. **Post-trade exit-timing analysis:** Autotrader sends Learning attributed
   trade lifecycle/outcome feedback. Learning schedules longer-horizon,
   non-trading replay/observation jobs for that exact strategy/profile and
   trade context. Once observation windows mature, Learning compares actual
   entry/exit and realized outcome with counterfactual strategy exits to
   estimate whether the strategy exited too early or too late.

These jobs do not launch additional live strategies or orders. They run in an
isolated research workload, cannot compete with the reserved live Strategy X
pool, and produce delayed, attributed learning evidence. Keep actual trade
outcomes distinct from counterfactual replay outcomes; only aggregate repeated
and profile-matched evidence into bounded learning updates.

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
  reserved CPU and memory, separate from Scout and replay workers. Run exactly
  two Strategy X processes per registered strategy: one live process called by
  Autotrader and one research process called by Learning. Keep the live process
  stateless; only the Learning-side process needs Scout/replay persistence.
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
| Autotrader latest strategy signals | Map of symbol/strategy to `Vec<Signal>`; refreshed keys are replaced | Add per-key sample, global entry, and byte caps plus TTL/idle expiry. Expire stale symbols even when updates stop. |
| Autotrader audit queues | 16,384 decision and 4,096 execution records | Replace record-only caps with byte and record budgets; preserve critical execution records and coalesce/drop replaceable diagnostics under pressure. |
| Autotrader metrics/cache | Several timing samples cap at 500; Learning response cache caps at 512 entries | Audit all retained maps/vectors and cache payload sizes; keep fixed-size rings and latest diagnostics only. |
| Learning state | SQLite persistence, but the full `LearningState` is loaded into RAM | Make SQLite the durable source of history; load only active profiles, compact aggregates, recent idempotency state, and bounded working sets. Query old outcomes/replay rows by profile and time range; avoid cloning full history for persistence/analysis. |
| Learning retention | Seven-day outcomes per variant; threshold samples and event IDs cap at 100,000; exit observations cap at 10,000 | Cap/compact total variants and bytes too. Move long-lived deduplication to indexed durable storage and compact old profile outcomes to bounded sufficient statistics without losing profile identity. |
| Learning exit timing | `/exit-timing` currently accepts a precomputed observation and retains at most 10,000 observations; it does not create replay jobs | Extend/replace this ingestion with idempotent attributed trade lifecycle events and durable asynchronous replay scheduling. Do not make Autotrader calculate the counterfactual result before sending it to Learning. |
| Insights signal engine | Per-symbol windows are fixed-size (for example 512 prices and 2,048 trades); research reads cap snapshots at 2,000 and outcomes at 20,000 | Add global active-symbol limits and idle engine eviction. Keep range reads paged and avoid caching decoded history in overlapping result caches. |
| Live Signals | Sixteen shard-local actor maps retain a second `SignalEngine` per observed symbol without visible idle eviction. Each engine can keep 3,601 one-second full-depth book samples; four per-symbol DashMaps also hold latest/market/pending payloads. | Establish one live rolling-feature owner. Remove Live Signals' duplicate `SignalEngine`, actor maps, and 16 shards if it becomes a thin handoff; otherwise remove/merge the duplicate Insights engine. Keep only a bounded, coalescing latest-signal delivery cache. |
| Cross-service signal state | FeedHandlers publishes raw events; Insights enriches them and maintains a research engine; Live Signals recomputes live signals from the enriched event stream. Compose currently sets `INSIGHTS_RESEARCH_ONLY=true`, disabling Insights' own direct live signal-push worker. | Remove the research-only mode and its Compose setting. Insights always computes/publishes live signals at live cadence, while research capture runs independently at its own cadence. Remove Live Signals as a second signal producer (or retain only a non-computing bounded handoff during migration). Avoid duplicate rolling state and duplicate Autotrader/Scout deliveries. |
| Insights queues/caches | Writer queue 4,096, signal queue 8,192 plus shards; calibration cache can hold 22,000 decoded records | Add byte-aware limits and coalesce replaceable snapshots. Remove Insights walk-forward calculations/caches and strategy-level missed-opportunity caches. Retain only dashboard caches with explicit bounds and single-flight refresh. |
| Strategy X deployment | One Scout worker process per strategy, each with a 2 GiB default limit | Run two processes per strategy: a stateless live worker for Autotrader and a separate Scout/replay worker for Learning. Right-size each process; keep Scout SQLite/idempotency state out of the live process. |
| Core service limits | Worker limits exist; Autotrader, Learning, and Insights have no explicit Compose memory limits | Add measured limits/reservations for all core services. Reserve Autotrader and live Strategy X headroom first; cap Learning and research pools independently. |

Do not shrink already bounded per-symbol feature windows by guesswork. First
remove unneeded/duplicated caches and cap global cardinality, then profile peak
RSS and p95/p99 latency at the supported symbol count. Reduce feature windows
only if strategy behavior remains equivalent and hot-path latency stays within
budget.

The short `book_history` (up to 601 full-depth samples over roughly one
minute) is used by live snapshot absorption/replenishment calculations and
research feature extraction. Do not delete it without replacing those temporal
calculations with equivalent compact rolling state. The separate one-hour
`counterfactual_book_history` is not consumed by Live Signals and should be
disabled there; Insights may retain it only for active research evaluation, or
replace it with durable quote-path storage.

The existing Insights research capture samples at a 10-second cadence, while
counterfactual book paths sample at one second. Before promising any replay or
exit-timing horizon, inventory which symbols/features/intervals are actually
persisted and retained, and compare that resolution with each strategy's
decision cadence. Add a separate bounded capture path or report insufficient
history where it cannot support the requested counterfactual; do not imply the
current research store contains every raw feed event.

For the current high-memory incident, Live Signals' duplicate per-symbol engine
state is a direct suspect. Walk-forward is not called by Live Signals: it is
computed in Insights, whose `/internal/live-context` route can schedule the
scan. Live Signals polls `/internal/live-news-context`, which returns only news
context and weight. Its counterfactual quote-path buffer is unnecessary there:
a full sample contains bid, ask, and seven levels on each side, and up to 3,601
samples are retained per symbol (about 0.85 MiB per symbol before other engine
state). Insights has its own research engine while Live Signals recomputes live
signals from the enriched event stream. Remove the duplicate ownership rather
than only tuning the buffer. Runtime actor count is not exposed; add it during
diagnosis. Investigate Insights' walk-forward scans and Learning's full-state
hydration separately for their high RSS readings.

## Work sequence

### 1. Establish feature ownership and remove duplicate state

- Trace each raw feed, enriched event, derived feature, news overlay, signal,
  research record, and consumer. Identify data/decisions that no downstream
  consumer uses.
- Select Insights as the single always-live signal calculator and use its
  existing direct AutoTrader push path. Remove `INSIGHTS_RESEARCH_ONLY` and the
  conditional branch that replaces live snapshot cadence with research
  sampling. Run research sampling independently as bounded background work.
  Remove Live Signals' duplicate engine, 16 shard workers, per-symbol actor
  maps, and separate news overlay; if needed during migration, keep only a
  bounded non-computing handoff. Cut over both producers together to prevent
  duplicate or missing signals and Scout inputs.
- Keep Insights' per-symbol state compact and bounded. Remove one-hour
  full-depth quote arrays from the live calculation path; preserve only the
  short state required for live features. Write research quote paths to durable
  storage or capture them only for active research jobs.
- Compare existing live outputs during the transition and preserve signal,
  news attribution, and delivery semantics before deleting the duplicate path.

### 2. Merge the crates and define contracts

- Move the `strategies` library modules and `scouts` service modules into one
  crate at `src/rust/strategy_x`, retaining a library target and the Strategy X
  service binary target. Move the service's implementation and public APIs
  rather than introducing a crate dependency cycle.
- Update all Rust dependencies and imports, root build configuration, Docker
  build arguments, Compose references, CI/scripts, and docs from the two old
  crate paths/names to `strategy_x`.
- Keep module-level boundaries inside the merged crate: strategy evaluation
  and contracts stay reusable; HTTP/RSocket service handlers and persistence
  stay in service modules. Learning/Autotrader depend on the library API, not
  on service internals.
- Add versioned, typed live evaluation request/response contracts in the
  `strategy_x` library API. Include correlation ID, strategy ID, profile ID and
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
- Remove walk-forward fields from market-signal/Insights contracts and remove
  their production/consumption from Insights. Keep the evidence as a separate
  Learning-owned evaluation context so it cannot be mistaken for a market
  feature or reused across strategy/profile variants.
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

- Expose a live request-response route alongside the Scout route.
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
  completed/failed status. Replay uses a separate Strategy X endpoint/pool and
  resource budget from both live decisions and Scout candidate evaluation.
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
  remove duplicate Live Signals/Insights rolling engines and the Live Signals
  shard pipeline if it becomes a thin handoff, remove Insights walk-forward
  caches, and replace Learning's whole-state RAM hydration with selective and
  compacted durable-state access.
- Add measured Compose memory limits/reservations to Autotrader, Learning,
  Insights, and both Strategy X processes for every strategy. Do not inherit
  the current 2 GiB-per-strategy default for each of the two processes; measure
  their real footprints and budget the doubled process count explicitly.
- Ensure Insights has no walk-forward fields, endpoints, calculations, or
  dependencies on Learning outcome history after the migration.

### 4. Route the complete Autotrader decision path

- Add an Autotrader client for all live entry paths, including live insight,
  daily strategy, dated-carry entry, and open-position decisions.
- Send decision and trade lifecycle/outcome events to Learning with stable
  trade/signal IDs and the exact strategy/profile/parameter snapshot. Learning
  uses those events to schedule asynchronous replay; it does not ask Autotrader
  to run parallel live strategies.
- Send batches of independent evaluations when practical, while preserving
  per-request identity and response validation.
- Keep final risk gates, position state, order decisions, and outcome recording
  in Autotrader. Worker output is a strategy decision, not an executable order.
- Do not leave a permanent mixture of in-process and remote production
  evaluation paths.

### 5. Deploy the merged service

- Rename Compose services, environment variables, worker endpoints, docs,
  health checks, and monitoring labels that identify the Scouts runtime.
- Configure exactly two deployments/endpoints per registered strategy, one
  live endpoint for Autotrader and one Scout/replay endpoint for Learning.
  Keep research workers on an opt-in profile; live workers remain available to
  Autotrader independently of Learning's dispatch.
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

- Remove Autotrader's direct evaluator calls and dependency on the evaluator
  library once every path uses the worker API; retain shared contract types.
- Update every affected module README to describe the resulting architecture
  and ownership boundaries, including Strategy X, Learning, Autotrader,
  Insights, and any affected feed/position-management modules. Remove stale
  Scouts-service and Insights-owned walk-forward descriptions. Document the
  live versus Scout worker pools, request contracts, failure behavior, and
  service dependencies where relevant.
- Add independent live and Scout metrics for request rate, queue/in-flight
  saturation, latency percentiles, timeouts, rejects, errors, and Scout backlog.
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
7. The `scouts` and `strategies` crates are merged into one `strategy_x` crate;
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
12. There is one owner of live rolling feature state. Live Signals is a bounded
    signal handoff or is removed; it does not hold a second per-symbol engine
    map or duplicate book history.
13. Insights has one always-live operating mode. Research sampling runs
    independently and cannot disable or replace live signal generation; the
    Compose `INSIGHTS_RESEARCH_ONLY` switch is removed and there is one producer
    of live Autotrader and Scout inputs.

## Operational constraint

Implementation and Compose changes do not authorize cycling running services.
Restart only the affected services after explicit authorization, then run the
homepage System liveness check as required by the repository instructions.
