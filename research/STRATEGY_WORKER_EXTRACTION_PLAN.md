# Walk-forward evidence improvement plan

Learning owns strategy outcome attribution and replay. Insights provides market inputs and fee-aware markouts; it does not infer strategy performance from raw signals. Keep the worker boundaries and change the live gate only after the new evidence pipeline is reproducible and offline validation is complete.

This plan targets Scout strategy/profile evidence used by the Strategy live-entry gate. Learning’s per-parameter `walk_forward_threshold` tuner is a separate path; do not merge it into this evaluator until its candidate history and estimand are covered by the same replay design.

## Milestone 1: Make the historical data complete and auditable

1. Inventory the Scout SQLite collections and Learning outcome store. Verify which fields are present for signal/evaluation time, costs, strategy/profile parameters, regime, outcome group, baseline, and candidate selection.
2. Make analytical replay read all durable outcomes in timestamp order. The runtime replay index hydrates only the latest 4,096 Scout outcomes; SQLite is the durable source and eviction only removes the runtime copy. Report missing or truncated history as coverage, never as a complete sample.
3. Audit `scout_plan_results`: it stores selected groups/audits and is cached separately, but may not record every candidate tried or the selection process. Add a durable append-only search record containing search ID, as-of cutoff, all candidates and parameter sets considered, baseline, selection metric, selected candidate, and evaluation window.
4. Give outcomes a stable identity and eligibility rules: completed before the evaluation origin, fee-aware, exact profile and symbol, explicit regime, and grouped when they share an opportunity/portfolio decision. Keep missing marks/costs and any broader-profile fallback explicit.

**Deliverable:** a versioned, point-in-time dataset contract plus durable outcome/search readers with coverage diagnostics.

## Milestone 2: Generate rolling-origin out-of-sample returns

1. Predeclare replay settings: expanding or rolling training window, minimum history, origin cadence, evaluation-window length, final holdout, and candidate-selection objective. Persist a config/version hash with results.
2. At each origin, select/tune using only records available before that origin. Apply the selected candidate to the next untouched interval; repeat chronologically. Use the final holdout once for confirmation.
3. Emit aligned portfolio/time-period return streams for each candidate and baseline, net of modeled fees and costs. Include flat/cash periods when the replay establishes that no position was held; do not silently convert missing outcome marks into zero returns. Preserve opportunity/group IDs, timestamps, profile, regime, fold boundaries, and missing-outcome coverage. Trade-only averages can favor candidates that trade less often and are not comparable as portfolio expectancy.
4. Keep the strict `evaluated_at < origin_at` rule. Retain the existing non-overlap treatment or purge training observations whose label windows intersect an evaluation window; set any embargo before seeing results. Do not mix symbols or portfolio legs as if independent.

Rolling-origin evaluation is the primary deployment-style estimate ([Tashman, 2000](https://doi.org/10.1016/S0169-2070(00)00065-0)).

**Deliverable:** deterministic replay output that can reproduce each fold and explain each included/excluded observation.

## Milestone 3: Add dependence-aware inference

1. Calculate selected-candidate net expectancy against the baseline from the out-of-sample stream. Keep positive-return rate and non-overlap count as diagnostics.
2. Calculate a one-sided lower confidence bound using a stationary/block bootstrap over synchronized time blocks or complete outcome groups. Predeclare bootstrap count, block-length method, confidence level, and random seed; report block-length sensitivity and effective sample size.
3. Report fold/window coverage, drawdown and downside summaries alongside the interval. Flag regime instability and sparse regimes; do not let one aggregate interval hide them.
4. After the full search ledger is available, run Hansen’s SPA test on the searched candidate family versus baseline using a dependence-aware bootstrap. Preserve the full tried set, including failed candidates. SPA is a family-level test for superior performance, not a guarantee that a selected candidate will remain profitable ([Hansen, 2005](https://citeseerx.ist.psu.edu/document?doi=3fc027710d7a2bab33abe3ee33e93c765190394f&repid=rep1&type=pdf)).
5. Add Sharpe/DSR or PBO/CSCV only if a separate Sharpe-selection or search-fragility question warrants it; neither replaces forward evaluation ([DSR](https://davidhbailey.com/dhbpapers/deflated-sharpe.pdf), [PBO/CSCV](https://escholarship.org/uc/item/4w1110bb)).

The stationary bootstrap supports inference for weakly dependent stationary series; regime changes must be surfaced rather than assumed away ([Politis & Romano, 1994](https://users.ssc.wisc.edu/~bhansen/718/Politis%20Romano.pdf)).

**Deliverable:** a reproducible evidence summary with expectancy interval, SPA result when eligible, diagnostics, and explicit evidence status/coverage.

## Milestone 4: Validate the replacement and cut over

1. Version the evidence contract across Learning’s decision snapshot, AutoTrader profile payload, and Strategy gate. Include evidence status, point-in-time cutoff, profile/regime, sample and effective counts, window coverage, estimate/interval, and search/SPA metadata.
2. Validate replay offline across symbols, regimes, windows, sparse-data cases, and missing-data cases. Confirm determinism and strict time cutoffs; replay saved inputs to verify the new estimator and its decisions.
3. Set acceptance thresholds before evaluating the final holdout. Replace the average-return/positive-rate gate directly with the new expectancy-based rule. If the replacement fails validation, revise the replacement before cutover; do not retain the old rule as a fallback or run both gates against live decisions.
4. Define how insufficient or stale evidence affects a new entry before implementation. Keep this distinct from evidence of negative expectancy, and preserve immutable entry-time evidence for trade attribution and audit.

**Deliverable:** offline validation report and a directly replacing expectancy-based entry gate.

## Milestone 5: Make every entry blocker obvious

1. Replace reason-string classification with structured gate IDs and results. Each result records pass/fail/indeterminate, a stable reason code, observed value, required threshold, and relevant evidence identity/as-of time.
2. Carry a decision/cycle ID through signal eligibility, strategy evaluation, walk-forward gating, candidate selection, position/risk checks, execution checks, and order submission. Record all applicable failures, not only the last error or first rejection.
3. Publish an entry funnel per cycle: signals received, fresh/directional candidates, candidates passing strategy and evidence checks, candidates passing portfolio/risk/execution checks, selected orders, and submitted orders. Distinguish “no eligible signal” from “candidate rejected” and “order failed after selection.”
4. Expose the current cycle’s primary blocker and counts by gate, symbol, and strategy in the AutoTrader UI/API, with drill-down to each candidate’s full reasons and the concrete values that failed. Mark gates that are informational or indeterminate so they are not confused with blockers.
5. Keep a bounded recent history and timestamp so an empty or stale diagnostics panel is itself visible. Avoid a dashboard that reports zero trades without a corresponding reason or an explicit diagnostics-unavailable state.

**Deliverable:** an end-to-end “Why no entry?” view that identifies the dominant blocker and lets the user inspect every candidate and gate decision.

## Acceptance criteria

- Replay reads complete durable history or reports exactly what is missing; the runtime 4,096-row cache is never treated as the whole history.
- Every search records the complete candidate family and selection cutoff; SPA is withheld when this history is incomplete.
- No outcome evaluated at or after an origin contributes to its training or evaluation result.
- Candidate and baseline returns are aligned, fee-aware, and resampled in time/group blocks.
- The final holdout is never used to tune. Offline replay validates the replacement; only the replacement gate runs on live decisions after cutover.
- Every cycle with no submitted trade reports whether there were no eligible candidates, which gates rejected candidates, or why a selected order failed; diagnostics include structured reasons and observed-versus-required values.
- Gate counts reconcile through the entry funnel, and the UI/API expose the current reason with freshness and candidate-level detail.
