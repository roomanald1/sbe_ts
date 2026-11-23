# AutoTrader learning gap remediation plan — 22 November 2026

## Objective

Determine whether AutoTrader's recent learning is improving, and restore enough accounting, feedback-delivery, and replay coverage to make that conclusion reliable. Treat the 22 November live database review as a starting snapshot; refresh the evidence before changing code or interpreting subsequent performance.

## Audit baseline

The read-only snapshot showed:

- In the latest 200 closed outcomes, the older 100 had 42 wins, 58 losses, and −$0.22 realized P&L. The newer 100 had 37 wins, 63 losses, and −$6.56 realized P&L.
- Eight outcomes in the newer half ranged from −50% to −99.6%. Together they accounted for −$6.35 of that half's realized P&L. Excluding those eight, the other 92 averaged −0.23% with a 40% win rate, compared with −0.20% and 42% in the older half. This is flat to slightly worse, while the dollar deterioration is concentrated in the eight extreme records.
- Learning had 126 exit observations in the prior 24 hours, ending at 11:00 UTC. The pending AutoTrader exit-timing queue included observations through 14:58 UTC. Stop-losses accounted for 84 of the 126 observations; their mean exit return was −0.51%, mean 30-minute hold-to-horizon return −0.33%, and 51/84 had positive post-exit return. This is a reason to study exit timing by context, not enough evidence by itself to widen stops.
- AutoTrader's execution-feedback outbox contained 3,763 events, dating back to 13 November. Learning had zero execution-stat rows and zero execution-event IDs. The general feedback outbox held 10,000 missed-opportunity candidates. The publisher processes general feedback and continues before execution feedback, so a persistently non-empty general queue can starve execution learning ([http_routes.rs](../../src/rust/auto_trader/src/auto_trader/api/http_routes.rs)).
- Strategy replay jobs showed 1,401 failed, 290 unsupported, and 74 completed; 246 failed in the prior 24 hours. Persisted walk-forward evidence commonly reported partial history, unknown cost coverage, zero OOS folds, and an unvalidated holdout.
- The daily metric filters returns below −100%, so values as low as −99.6% remain eligible ([activity_learning.rs](../../src/rust/auto_trader/src/auto_trader/api/activity_learning.rs)). The eight extreme records were not reconciled to the available Learning exit observations in this review.

These figures describe the initial captured snapshot and are superseded for queue and trend triage by the follow-up capture below. The extreme records remain unreconciled; refresh all metrics again after remediation.

## Follow-up read-only capture — 22 November 2026, 18:44–18:48 UTC

Copied the three Compose databases into `/tmp/learning-gap-audit-2026-11-22/` for read-only inspection. AutoTrader and Position Management share `sbe-position-management-data`; Learning uses `sbe-learning-data`. The database files reported WAL journal mode but no `-wal` or `-shm` sidecars were present at capture. `PRAGMA quick_check` returned `ok` for all three copies. The copies were taken sequentially, so use AutoTrader-only results for its metrics and do not claim a transactionally aligned cross-service join from this capture. SHA-256: AutoTrader `048e15bbffebd80211e9ede561bb81161d2580f1a45189f62fe76a0b533e193f`, Position Management `8c048fabea6b524527cbc289259df1f239047ba1ff3b7ab53f9ff4ba9ff537a0`, Learning `d635ebedf3b1cfe97a3e133a52cb48f78a3e773dd6b57f7abeeddef4ff3b1c10`.

Recomputed the latest 200 AutoTrader closed-return rows in timestamp order:

| Cohort | Wins | Mean recorded return | Median recorded return | Sum of recorded realized USD |
| --- | ---: | ---: | ---: | ---: |
| Older 100 | 41/100 (41%) | −0.209% | −0.464% | −$0.2221 |
| Newer 100 | 37/100 (37%) | −6.485% | −0.503% | −$6.5524 |

Eight newer outcomes at or below −50% total −$6.3455. The other 92 newer rows have 37 wins (40.2%), mean return −0.228%, median −0.498%, and −$0.2069 recorded realized USD. This remains a dollar deterioration concentrated in the eight records and a modestly worse per-trade return in the remaining rows. Returns and realized USD are the stored fields; fee inclusion and denominators still need reconciliation. No uncertainty intervals or attribution-controlled comparison have been computed, so this is not evidence to promote a parameter change.

The eight outlier timestamps are 12:27:37, 12:32:38, 12:45:22, 13:07:32, 13:45:08, 13:48:28, 14:43:42, and 14:46:17 UTC. The summary arrays do not contain stable trade IDs. A timestamp match identifies a `strategy-disabled` AutoTrader close order for each, and its order timestamp exactly matches the Position Management exit fill's `parent_order_id`; the association from the summary row to that order remains inferred by a 0–139 ms timestamp difference.

| UTC | Symbol | Stored return | Stored realized USD | AutoTrader close-order realized P&L | Position exit fill ID |
| --- | --- | ---: | ---: | ---: | --- |
| 12:27:37 | BNB-USDT | −90.216% | −$0.9374 | −0.001949 | `1795350457195-0-5446` |
| 12:32:38 | BTC-USDT | −92.614% | −$0.8878 | −0.002240 | `1795350758994-0-5454` |
| 12:45:22 | SOL-USDT | −50.084% | −$0.5304 | −0.002661 | `1795351522106-0-5467` |
| 13:07:32 | BTC-USDT | −84.560% | −$0.8999 | −0.002029 | `1795352852716-0-5473` |
| 13:45:08 | DOGE-USDT | −59.294% | −$0.6368 | −0.002132 | `1795355108220-0-5480` |
| 13:48:28 | BTC-USDT | −74.763% | −$0.9079 | −0.002485 | `1795355308466-0-5492` |
| 14:43:42 | DOGE-USDT | −76.376% | −$0.6402 | −0.001816 | `1795358622977-0-5528` |
| 14:46:17 | BTC-USDT | −99.646% | −$0.9051 | −0.001669 | `1795358777478-0-5539` |

The recorded summary losses are far larger than the corresponding close-order realized deltas. This establishes an accounting inconsistency requiring investigation, but does not yet prove which amount or return is correct: the close path includes accumulated position P&L, and all prior fills, partial fills, fees, and lifecycle IDs still need to be reconciled before changing the accounting formula. The corresponding Learning trade-event join is unavailable from this capture because the AutoTrader feedback outbox contains only missed-opportunity candidates.

Other refreshed queue and replay observations:

- AutoTrader held 3,766 execution events (oldest observed 222.5 hours; newest 3.0 hours), while Learning had zero execution-stat rows and zero execution-event IDs. The generic feedback outbox was full at 10,000 missed-opportunity candidates, ranging from 245.4 to 241.0 hours old. Learning also held 10,000 missed-opportunity jobs (9,247 completed, 753 unsupported). These counts confirm both execution-feedback starvation and stale candidate backlog at capture.
- AutoTrader held 258 exit-timing events (oldest 37.3 hours, newest 3.4 hours). Learning contained 1,549 exit observations, 110 observed in the preceding 24 hours through 11:30 UTC. Of those, 70 were stop-loss exits: mean recorded exit return −0.506%, mean hold-to-horizon net return −0.319%, 12/70 positive hold-to-horizon outcomes, and 42/70 positive post-exit returns. The 30-minute horizon and costs still need confirmation per event; post-exit return is diagnostic only.
- Learning had 1,769 replay jobs: 74 completed, 1,405 failed, and 290 unsupported. In the preceding 24 hours, 219 were created: 217 failed and 2 unsupported. Failures split into 188 history-route failures, 29 worker timeouts, and 2 other errors. All 219 scanned zero signals; 5 reported a cadence gap. This capture does not establish whether a zero-signal scan exhausted history or stopped at a limit.

This is a bounded diagnostic refresh, not the complete report: it narrows the code work to fairness/retention, exit-observation status, and a confirmed position-recovery accounting defect. The rolling summary row still links to its close order by timestamp rather than an embedded stable ID, the Learning-event join is unavailable, exact replay-history coverage is unknown for historical jobs, and post-remediation service behavior remains to be verified after rollout.

## Code remediation completed for review

- **Accounting and eligibility:** the cause was position recovery cloning Position Management's per-symbol aggregate into a fresh unattributed lot, retaining cumulative realized P&L and old entry basis. Closing that lot then counted the aggregate as one trade. Recovery now creates a fresh unattributed lifecycle from the current open quantity, average price, fees, and timestamp without changing aggregate/account P&L. The schema-v4 migration flags historical filled unattributed `strategy-disabled` closes matched to metric timestamps; these records remain stored but are excluded from eligible headline and strategy-learning returns until audited. New recovered closes and finite returns below −100% persist as `needs_audit`. Activity and monitoring responses show the pending count and five latest reasons. Existing history without audit metadata remains unchanged; no live database was edited.
- **Feedback queues:** the worker rotates through profile registration, trade/Scout outcomes, execution outcomes, and missed opportunities, so each class gets bounded service. A full missed-opportunity queue evicts its oldest missed candidate to admit a newer one, never evicts trade outcomes, and counts/logs candidate drops when outcomes fill capacity. The internal queue endpoint reports per-class depths, ages, attempts, acknowledgements, HTTP rejections, transient errors, terminal drops, and the latest reason/time.
- **Replay and exit observations:** replay history RSocket requests now use the configured replay deadline (120 seconds by default) after a 2-second request timeout caused bounded Insights SQLite history scans to fail. Replay jobs persist scanning, complete, exhausted, truncated, or unavailable history status and route response details. Exit observation deferrals persist their reason and next retry; exit-timing feedback records attempts, delivery errors, acknowledgement time, and creation-to-ack age while retaining unacknowledged events.

Focused implementation tests passed, including all 206 AutoTrader unit tests and 9 AutoTrader integration tests, plus focused Learning and Strategy replay-route tests. `just diff-check` passed. Existing unrelated working-tree changes were left in place.

## Deployment verification — 22 November 2026

After review, AutoTrader and Learning were rebuilt and redeployed; both report healthy. The first release build caught a Strategy dependency error (Tokio was optional although the default-feature timeout path now needs it); Tokio is now a normal dependency and the release build succeeds. The first runtime pass then exposed exit-timing payloads over Learning's 16 KiB limit. AutoTrader now downsamples only oversized price paths evenly, preserves endpoints and event IDs, and includes original point counts. A focused test verifies the compacted request stays below a 15 KiB safety limit. AutoTrader was redeployed with this fix.

The post-deploy AutoTrader SQLite snapshot passed `PRAGMA quick_check`. The schema-v4 migration marked all eight historical unattributed closes `needs_audit`. The exit-timing feedback queue fell from 258 pending to 0, with no further 413 responses; AutoTrader removes those events only after Learning returns its durable acceptance acknowledgement. Execution feedback received 357 acknowledgements and had 3,409 events still pending at the snapshot, so that older backlog continues to drain. Missed-opportunity feedback had 331 HTTP rejections and 32 permanent rejections terminally dropped; inspection showed these 4xx responses could otherwise pin the queue head. Permanent 400/413/422 candidates now receive counted terminal drops, while trade/Scout/execution/registration rejections rotate to the back without losing outcomes. The queue remains bounded at 10,000 because new candidates continue to arrive, while oldest-candidate evictions are counted. This confirms progress and visibility, not a steady-state age objective; no service-level target has yet been accepted.

Both services remained healthy after deployment. The live databases were not edited directly. The stale historical returns remain stored but are now excluded from eligible metrics and surfaced as audit work. Performance remains inconclusive until that audit and broader replay coverage are completed.

## Statistical stop-width check — 22 November 2026, 22:47 UTC

Used fresh immutable SQLite snapshots and excluded the one stop observation linked by symbol/time to a `needs_audit` closed row. Among 205 matured exit observations from the preceding 24 hours:

| Exit reason | Count | Share | Mean net return | Median | Positive |
| --- | ---: | ---: | ---: | ---: | ---: |
| Stop-loss | 121 | 59.0% | −0.513% | −0.508% | 0/121 |
| Profit-protection | 60 | 29.3% | +0.170% | +0.144% | 59/60 |
| Take-profit | 24 | 11.7% | +0.238% | +0.173% | 24/24 |

The overall mean recorded exit return was −0.225%, with 83/205 positive (40.5%). This supports the user's concern about the current reward/loss balance, though it is still a short, recent sample. The take-profit trigger is uncommon in these observed exits; profit-protection wins are frequent but smaller than the average stop-loss.

For 121 stop-loss observations with a complete 30-minute hold-to-horizon outcome, simply remaining open for the full horizon changed the paired mean return by +0.058 percentage points (63 improved, 54 worsened, 4 tied). The cluster bootstrap 95% interval was −0.014 to +0.150 percentage points across only 12 symbol-day clusters. The horizon mean was still −0.455%. Positive post-exit marks (68/121) are diagnostic and do not mean a wider stop would have captured those prices.

I also ran a paired quote-path replay on 167 stop-loss events with complete fee, policy, and price-path inputs (1,025 retained stop observations existed; these complete inputs span 23.6 hours, and 35/167 paths were compacted). Each candidate used the same entry, side, quote path, fee assumption, take-profit and profit-protection rules, changing only the stop to 1.25×, 1.5×, or 2× its applied value. Differences below are candidate minus current-stop replay return:

| Stop width | Mean paired return change | 95% symbol-day bootstrap interval | Mean extra hold | Replayed exits (SL / profit-protection / TP / horizon) |
| --- | ---: | ---: | ---: | --- |
| 1.25× | +0.011 pp | −0.027 to +0.050 pp | 11.0 min | 91 / 6 / 2 / 68 |
| 1.5× | +0.007 pp | −0.031 to +0.039 pp | 15.9 min | 56 / 6 / 2 / 103 |
| 2× | −0.001 pp | −0.033 to +0.029 pp | 18.7 min | 30 / 6 / 2 / 129 |

Every interval includes zero. The current-stop replay's mean absolute error against actual recorded stop returns was 0.055 pp, larger than each candidate's mean change. The tested path is sampled and some histories were compacted; the replay also cannot guarantee a counterfactual fill at a quote. This is exploratory evidence and does not support widening the live stop. Do not select the best of many candidate widths on this same sample.

A sound decision procedure is a **paired, chronological out-of-sample replay**: predeclare a small set of wider stops, compare each trade under the current and candidate rules with side-aware executable quotes and identical fee/slippage assumptions, and report paired net-return differences, tail loss/drawdown, additional holding time, and the rate that the wider stop is hit before recovery. Use a symbol-day block bootstrap for uncertainty, then reserve a later untouched period for confirmation. Stop-loss effects depend on the return process (including momentum versus mean reversion), so there is no universally correct wider level; searching many widths also creates data-snooping risk. See [Kaminski and Lo, “When Do Stop-Loss Rules Stop Losses?”](https://www.sciencedirect.com/science/article/abs/pii/S138641811300030X), [Bailey et al., “The Probability of Backtest Overfitting”](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2326253), and [White, “A Reality Check for Data Snooping”](https://doi.org/10.1111/1468-0262.00152).

## Adaptive stop-boundary learning — 23 November 2026

The evidence above is too weak to widen the live stop directly, but it is enough to justify testing the boundary more deliberately. Strategy Scouts now include bounded 1.25×, 1.5×, and 2× stop-loss candidates (rounded to the catalog step and capped at its maximum). Those wider arms receive reserved batch slots only when the exact active symbol/strategy/parameter profile has at least 20 learning samples, win rate at or below 45%, and negative realized P&L. Otherwise scouts continue with adjacent one-step probes. This makes exploration responsive to clear baseline losses without treating a short exploratory replay as proof.

For paired Scout outcomes involving `stop_loss_percent`, promotion now requires the one-sided 80% t lower confidence bound to exceed zero, with the existing minimum of 10 paired samples. Other parameters retain the one-sided 90% bound. This deliberately lowers the promotion bar for stop-loss experiments while still requiring positive paired evidence; it does not change active stops by itself. Wider-stop candidates must earn promotion through their paired outcomes, and the replay/holdout requirements above still apply to any broader performance claim.

Focused Learning and Strategy tests cover the lower promotion threshold, bounded stop candidates, and wider-stop batch reservation. `just check learning`, `just check strategy`, the Strategy service suite, and `just diff-check` passed. The full Learning suite retains one previously tracked unrelated threshold expectation failure (`sbe_ts-a7fk.1`).

The Learning service and all 17 Strategy Scout workers were rebuilt and redeployed on 23 November 2026. Compose health verification reported 18/18 healthy.

## Review findings and evidence requirements

The plan's order is appropriate: establish sample validity and restore feedback/replay coverage before tuning or promoting strategy parameters. The snapshot is useful for locating failures, but it is not a performance conclusion. The eight extreme records still require stable-ID reconciliation; their cause must not be inferred from their magnitude.

Source review found two details to carry into implementation:

- The replay worker already recovers persisted `Running` jobs as `Pending` and preserves its durable cursor. Verify this behavior against current persisted jobs and tests before changing replay scheduling.
- Strategy-specific trade feedback accepted finite returns below −100% even though the closed-trade activity path rejects them. For spot trades, that allows an impossible loss into learning while omitting it from the headline metric. Align eligibility at the confirmed spot return boundary, and do not generalize that bound to leveraged products without a product-specific denominator check.

For every refreshed database snapshot, retain the snapshot time, database/schema identity, and query or script revision used. Record join provenance per trade (`exact`, `inferred`, `unavailable`) and keep an explicit unknown state when the required lifecycle evidence is absent. A report must include coverage denominators (records requested, found, joined, excluded, unknown), not just successful examples.

Before setting service-level objectives, measure current queue depth, oldest age, throughput, retries, acknowledgements, rejections, and terminal drops by event class. Publish the proposed lag objective and its basis in the report; do not call it agreed until an owner accepts it. Every event should be visibly in one state: pending, in flight, acknowledged, retrying, rejected, expired, or terminally dropped, with the transition reason and timestamp retained where applicable.

## Work plan

### 1. Reconcile closed-trade accounting

1. Capture consistent read-only snapshots of AutoTrader, Position Management, and Learning databases, including SQLite WAL state where present.
2. Join each of the eight extreme outcomes to the position lifecycle, fills, entry and exit orders, fees, strategy decision, and Learning event by stable IDs. Record which joins are exact, inferred, or unavailable.
3. Verify the realized-P&L and return denominators against position quantity, cost basis, side, partial fills, fees, and any scale-ins or scale-outs.
4. Determine whether the outcomes are valid losses, accounting/attribution defects, or legacy/unattributed closes. Do not silently discard a large loss solely because it is an outlier.
5. Add regression tests for the confirmed failure mode. Define explicit validity and “needs audit” states for samples that fail reconciliation; ensure these states are visible in metrics and excluded from strategy-specific learning until resolved.
6. Verify the spot-return eligibility boundary against the actual instrument and denominator. Keep unresolved accounting defects in `needs audit`; do not treat an outlier cutoff as reconciliation.

**Deliverable:** an outlier reconciliation report and tested accounting/eligibility behavior.

**Acceptance:** each extreme outcome is explained and either correctly attributed or explicitly marked unknown; no invalid sample silently changes a learned profile or headline metric.

### 2. Prevent execution-feedback starvation

1. Confirm current outbox depths, oldest-event ages, and Learning acknowledgements from fresh snapshots and service diagnostics.
2. Change publisher scheduling so trade outcomes retain priority and execution outcomes receive bounded service even while missed-opportunity feedback remains queued. Preserve idempotent event IDs and retry behavior.
3. Add an explicit retention policy for missed-opportunity candidates: coalesce equivalent context/profile events, expire stale candidates, or otherwise bound their backlog without dropping trade outcomes.
4. Add metrics for queue depth, oldest-event age, attempt count, acknowledgement count, rejection status, and terminal drops by event class.
5. Verify that delivered execution outcomes populate Learning's execution statistics and that a restart or timeout resumes delivery without duplicates.
6. Document the bounded missed-opportunity retention rule (equivalence key, age limit, capacity, and outcome for a full queue) and expose counts for coalesced, expired, and dropped candidates.

**Deliverable:** fair, observable feedback delivery with a documented retention and retry policy.

**Acceptance:** execution feedback is acknowledged and represented in Learning; queue age stays within an agreed service objective; duplicate delivery does not double-count samples.

### 3. Restore replay and exit-observation coverage

1. Group replay failures by cause, worker, route, symbol, time span, scanned-record count, and attempt. Reproduce recent timeouts and history-route failures.
2. Inspect replay pagination, record limits, deadlines, cursor progression, and retry policy. A timeout after scanning 4,096 records with zero signals must report whether history was exhausted, truncated, or unavailable.
3. Drain pending exit-timing observations and account for every item as accepted, retrying, rejected, or expired. Track creation-to-acceptance lag and expose the oldest pending age.
4. Add tests for large histories, empty pages, worker timeout/restart, route failure, partial history, and idempotent replay.
5. Preserve explicit evidence status. Do not label partial history, unknown costs, zero OOS folds, or an unvalidated holdout as validated evidence.
6. For each exit observation, expose creation time, last collection attempt, latest status/reason, next retry time, acceptance time, and terminal state; summarize oldest pending age and creation-to-acceptance lag.

**Deliverable:** a replay/exit-feedback reliability report and tested recovery behavior.

**Acceptance:** eligible replay jobs complete reliably or state the exact missing coverage; exit observations arrive within an agreed lag objective; evidence metadata reflects actual coverage.

### 4. Reassess strategy and exit performance

After the accounting and delivery work is complete:

1. Rebuild the prior-versus-latest comparison using reconciled closed trades and net returns after fees. Report sample size, win rate, mean and median return, dollar P&L, drawdown, and data coverage.
2. Separate unknown-attribution trades. Break results down by strategy, symbol, side, market regime, execution type, and exit reason where sample sizes permit.
3. Compare stop-loss and profit-protection exits with post-exit and hold-to-horizon outcomes using consistent side-aware returns and complete costs. Treat post-exit movement as a diagnostic, not a counterfactual fill.
4. Require point-in-time training data, complete cost coverage, adequate effective sample size, out-of-sample folds, and an untouched holdout before recommending or promoting parameter changes.
5. Publish a concise “improving / flat / worsening / inconclusive” conclusion and state which evidence supports it.
6. Include uncertainty (confidence intervals or another stated uncertainty method) for aggregate returns and win rate, and mark slices too small for a useful estimate instead of ranking them.

**Deliverable:** a reproducible performance report that separates strategy results from accounting and telemetry quality.

**Acceptance:** every recommendation cites its sample, costs, coverage, and validation status; no promotion relies on an unvalidated holdout.

## Sequencing and scope

Do steps 1–3 before using the current headline returns to tune strategy parameters. Step 4 can proceed on already validated subsets while gaps are being repaired, but must label incomplete data and avoid live parameter changes.

The plan originally excluded service restarts and deployment. The user later explicitly requested rebuild and deployment; AutoTrader and Learning were deployed as recorded above. Do not edit live databases directly, commit, or push as part of this work.
