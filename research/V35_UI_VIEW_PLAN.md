# v35 Trading UI View Plan

> **Superseded:** the eight-widget requirements and profile-owned exit-policy extension below are incorporated into the authoritative [v35 Trading Improvements master plan](../src/rust/auto_trader/TRADING_IMPROVEMENTS.MD), Section 15. Use that master plan for current behavior, implementation order and acceptance. The permanent visual targets are the PNGs and matching HTML references in [research/v35-ui-mocks/](./v35-ui-mocks/), indexed in [START_HERE.md](./v35-ui-mocks/START_HERE.md) and mapped in master-plan UI 4.0. In particular, Orders & Positions must be visually reviewed against [03-positions.png](./v35-ui-mocks/03-positions.png) and [03b-closed.png](./v35-ui-mocks/03b-closed.png); automated tests and the previous redesign do not establish operator visual acceptance. This file remains aligned source context and must not be maintained as a competing specification.

## Goal

Keep the views needed to operate and evaluate the v35 paper trader safely. Add

the missing operator information by extending authoritative views and contracts

where possible. Grafana owns service health, per-symbol feed/worker reliability

and deployment artifact traceability. Consolidate duplicate trading widgets while retaining Scout and signal research capabilities; shared market dashboard

features remain outside this scope.

This is a view plan, not evidence that backend data is already available or that

the current paper cohort supports performance conclusions. Section 13 of

[`TRADING_IMPROVEMENTS.MD`](../src/rust/auto_trader/TRADING_IMPROVEMENTS.MD)

remains authoritative for Learning Evidence Coverage.

## Required operator widgets

Each widget should answer one operator question and remain independently

addable/removable in the dashboard. These are individual widgets rather than

three umbrella views.

| Widget | Operator decision | Existing coverage | Work required | Authoritative data owner |

|---|---|---|---|---|

| **AutoTrader controls and safety** | Is automated paper execution enabled with the intended limits and safeguards? | Existing Auto Trader widget contains settings and controls. | Keep editable global paper/risk controls and clear mode/audit state. Remove legacy per-strategy/per-horizon budget maps and repair warnings; preserve hard global caps and safety overrides. | AutoTrader owns effective settings and entry state. |

| **Decision funnel diagnostics** | Why did a signal become or fail to become an order, and where did it stop? | `AutoTraderDiagnostics` already shows a per-cycle funnel, gate counts and symbol/direction breakdowns. | Separate AutoTrader and Scout funnels and heatmaps. Preserve cycle/event identity and as-of freshness; show decision-time market snapshot/source/freshness; reconcile counts through signals, candidates, strategy/evidence gates, portfolio/risk/execution gates, selection, attempt, submission, acknowledgement, fill and terminal outcome. Drill down to reasons and observed/required values. Distinguish zero, unknown, stale and truncated populations. | AutoTrader owns candidate selection/submission diagnostics; Strategy owns worker evaluations; Position Management and Paper Exchange own order/fill transitions; FeedHandlers/Insights supply decision-time context. |

| **Orders and positions** | What orders, fills, positions and protective exits are active or complete? | Existing Orders/Positions, Fills, AutoTrader live/completed positions and parts of Outcomes overlap. | Separate AutoTrader and Scout modes; reuse Net vs Breakeven bars. Consolidate into one lifecycle widget with active/history filters, protective exits, stable trade/order identity and explicit incomplete attribution. | Position Management owns the order/position ledger; AutoTrader owns request and decision lineage. |

| **Execution quality** | Are passive orders filling effectively, and what opportunity cost is supported by evidence? | Post-only concepts exist in order entry/outcomes; no effectiveness view was found. | Separate Scout simulation diagnostics; compact order pricing bars and inline drill-down. Add eligible/submitted counts, fill/partial-fill/reject/cancel/expiry rates, time-to-fill, fees/slippage, and a defined counterfactual window with sample coverage. Do not claim causal savings or missed profit when evidence is incomplete. | AutoTrader owns request/decision lineage; Position Management owns order lifecycle; Paper Exchange owns simulated match and fee evidence. |

| **Portfolio and reconciliation** | What capital, fees and P&L are recorded, and is the ledger independently complete? | Account, Risk, P&L, Fills, Execution Fees and Outcomes cover parts of finance. | Consolidate opening/current cash, utilization, fees, realized/unrealized values, evidence epoch/cursors and reconciliation completeness. Preserve unknown/incomplete status instead of treating it as zero or reconciled. | Position Management owns portfolio ledger/snapshots; Paper Exchange owns source capital, fee and match records; reconciliation tooling compares them. |

| **Capital allocation** | How is the eligible shared pool allocated by strategy, symbol and horizon? | Current controls expose legacy manual maps; read-only replacement is already planned in Beads. | Provide a dedicated read-only widget for effective allocation, used/reserved/available capital, evidence eligibility and reasons. Keep global caps configurable; never transmit stale/zero legacy maps. Implement after allocator contract. | AutoTrader owns authoritative allocation decisions. Tracked by `sbe_ts-gu78.29`, dependent on backend `sbe_ts-gu78.28`; do not duplicate this work. |

| **Strategy performance** | What do strict and exploratory paper cohorts show? | Learning filters distinguish selection mode; Strategy Performance and Outcomes overlap. | Separate AutoTrader and Scout modes, heatmaps, history and net win/loss bars. Consolidate comparisons by explicit selection mode and comparable cohort, net fees/costs, raw/effective samples, freshness, uncertainty, exclusions and reconciliation status. Show “insufficient evidence” until eligible reconciled trades exist. | AutoTrader owns selection mode and lifecycle; Position Management/Paper Exchange provide financial outcomes. |

| **Evidence coverage and acquisition** | Why is a strategy not eligible, and what evidence work is queued or progressing? | Learning widget implements Section 13 Evidence Coverage; separate Learning decision/Scout panels overlap. | Retain Scout effectiveness and signal research views. Keep gap reason, blocked decision, freshness, sample/effective windows, request states, recovery/blocker and accepted evidence gains separately from activity. Consolidate decision drill-down here without duplicating raw Scout status widgets. | Learning owns evidence requests, coverage and learned profiles. Existing implementation is `sbe_ts-15ok`; preserve the canonical contract in Section 13. |

### Shared widget design and interactions

- Reuse the existing dashboard catalogue, drag/drop, resize and layout persistence. No new dashboard, layout manager or mandatory presets.
- Compact dark trading-terminal styling: minimal padding, small summary metrics, dense sortable tables, configurable columns, compact bars and heatmaps. Expand advanced details rather than permanently occupying space.
- Each widget is independently addable, removable and usable. Support multiple instances with independently persisted settings, filters, grouping, time range and selected view. Adapt content to allocated size; summary first, detail when enlarged.
- Use exchange-aware contracts throughout: distinguish market-data exchange, execution venue/account, execution mode and reporting currency. Support future simultaneous exchanges without hardcoding Binance or Paper; no new exchange integration or live activation in this scope.
- Use consistent drill-down: click a count, metric, row, heatmap cell or blocker to show the exact underlying filtered population inside the widget; expand individual records inline; back preserves filters and grouping. Reuse detail components and stable event/opportunity/episode/decision/reservation/order/fill/trade/profile/evidence links. No separate pages or permanent detail panels.
- AutoTrader and Scouts are first-class but distinct populations. Never combine simulated Scout returns with financial balances or actual execution statistics. Portfolio and Capital Allocation are financial only.
- Use authoritative server-side aggregation, bounded pagination and subscriptions, deduplicate requests, cancel/suspend hidden-widget work, and preserve existing performance constraints. Distinguish zero, missing, stale, incomplete, provisional and truncated data. Do not invent backend values or duplicate business logic in React.


### Mandatory view/model separation and lifecycle testing

Every operator view, including each of the eight widgets and independently stateful detail views, must have a corresponding tested `*Model.ts`. Follow the existing `MarketLiquidityModel` and `useBehaviorSnapshot` pattern; inspect the repository conventions before implementing. Reuse existing models where appropriate rather than introducing a parallel state framework.

- `*.tsx` files are presentation only: render model snapshots, bind controls to model commands, compose presentational components and forward React lifecycle/prop changes to the model. No fetching, subscription orchestration, timers, business logic, state machines or asynchronous workflow logic in TSX.
- `*Model.ts` owns all UI lifecycle events and state transitions: initialization, activation, visibility, connection and scope changes, filter/group/time-range changes, drill-down/back navigation, pagination, refresh, loading/error/stale states, cancellation, retry, configuration persistence and disposal. Expose typed observable state (`state$`) and explicit commands. Keep authoritative trading and financial calculations in backend services.
- Each widget instance owns its model and configuration. Share transport/cache infrastructure where appropriate, not mutable instance-specific view state. Suspend hidden work, release subscriptions and timers on disposal, cancel superseded requests and ignore late responses. Lifecycle methods must be safe under repeated activation/cleanup and React Strict Mode.

Use the established binding style, for example (illustrative existing pattern):

```tsx
const model = useMemo(
  () => new MarketLiquidityModel({ connection, symbol, streamKey: key }),
  [],
);
const state = useBehaviorSnapshot(model.state$);
```

The empty dependency array is appropriate only when constructor inputs are stable for the instance or subsequent changes are explicitly forwarded to the model. Otherwise use the repository's supported dependency/lifecycle pattern. Constructors must not start unmanaged side effects during render; React integration should only forward lifecycle events to the model using the established hook/adapter, with cleanup. Do not copy the example blindly if it would capture stale inputs.

### Model acceptance requirements

- Every corresponding `*Model.ts` must have automated model-level tests using the existing test framework, independent of React rendering. Test initial state, activation/deactivation, changed inputs, subscriptions, cancellation, out-of-order responses, errors/recovery, stale/missing data, filter and navigation state, persistence, disposal and independent instances as applicable.
- Test repeated mount/unmount and hidden/visible transitions, including absence of duplicate subscriptions, leaked timers and updates after disposal. Use deterministic fake transport/clock fixtures where appropriate.
- Retain rendered component/browser tests for presentation, event binding, responsive layout and integration. Model tests complement rather than replace these tests.
- Acceptance for every widget requires its tested model, thin TSX view, and passing relevant model and integration tests. Review existing affected views against this rule; do not introduce unrelated application-wide refactors.


### Contents and interactions

#### 1. AutoTrader controls and safety

- Compact summary: enabled state, paper execution mode, market-data exchange and execution venue/account, audit status, safety latch, emergency stop, effective configuration version/freshness and latest change. Expose supported Scout status and controls separately.
- Compact two-column settings: global trading capital, maximum order size, daily notional and loss limits, maximum positions, exploration allowance, stop loss and take profit. Include allowed symbol/strategy scope and supported scoped overrides; advanced settings expandable.
- Global and individual venue enable/disable where supported. Preserve authorised apply/confirmation, disable and emergency-stop actions. No live switch or unapproved change to risk/exit policy.
- Remove obsolete editable strategy/horizon budget maps and repair warnings, including obsolete backend constraints and stale/zero map transmissions. Keep global and venue-specific hard caps; link to computed allocation without requiring that widget to be installed.
- Show unknown/stale settings explicitly; do not duplicate Grafana infrastructure monitoring.

#### 2. Decision funnel diagnostics

- Separate AutoTrader and Scouts modes; compact summary, funnel/breakdown table and selectable heatmap.
- Header: latest cycle ID, event/time, age/freshness, outcome and primary blocker. Time filters: latest cycle, 1h, 24h, 7d, 30d and custom within retention.
- AutoTrader stages: market events, worker evaluations, received signals, allowed/fresh/directional opportunities, strategy and evidence checks, strict/exploratory eligibility, portfolio/risk gates, selection, execution eligibility, attempts, submission, acknowledgement, partial/full fills, opened/closed positions, reconciled trades and Learning acceptance. Use actual producer stages; distinguish unique events, repeated evaluations, opportunities and episodes. Show denominators, adjacent-stage reconciliation and unavailable/truncated populations; do not force unlike populations into a monotonic funnel.
- Scout stages: signals, eligibility, assignment, opening, completion, evaluation, acceptance/exclusion and effective evidence gain. Do not mix with AutoTrader counts.
- Group/filter by exchange, symbol, strategy, combined symbol/strategy, profile/version, direction, horizon and entry policy. Heatmap metrics include signals, eligibility, strict/exploratory eligibility, selection, submission, fills, dominant blockers and latest activity.
- Candidate detail: event/episode/opportunity/cycle identity, strategy/profile, direction, selection mode, decision-time price/bid/ask/source/book freshness, stage result, gate, observed value, comparator, required value, primary/additional reasons and linked worker/order/fill records.
- Expose original exploratory reservation, source event and episode lineage, terminal release or deduplication status where supported. Never label rejected or post-selection failures as no signal.

#### 3. Orders and positions

- Summary: open positions, pending orders, exposure, net unrealised P&L and protection status. AutoTrader tabs: Open, Orders, Closed, Protection. Scouts tabs: Open, Completed; explicitly simulated. Shared instrument/order filters, exchange/account scope and as-of time.
- Default open-position row includes existing Net vs Breakeven bar, reused rather than rebuilt: loss left/profit right for both directions, current executable close marker, net breakeven, SL/TP markers and dynamic profit protection, net close-now P&L in currency and percent, distances to SL/TP. Include paid entry and supported estimated exit fees, use appropriate executable bid/ask, distinguish trigger prices from executable returns, use consistent net basis and label missing/stale values. Update bars without refreshing the entire table.
- Compact configurable columns: symbol, strategy/profile, direction, entry policy, venue, quantity/notional, entry/current price, holding duration/max hold, gross/net P&L, fees, forecast/exit decisions and freshness. Expand for lineage and protection details.
- Orders: stable IDs, parent/entry link, type, requested/filled/remaining quantity, limit/average execution price, maker/taker, status, created/updated time, acknowledgement, cancellation/rejection/expiry and recovery. Preserve partial fills.
- Protection: type, trigger, quantity, active/pending/triggered/interrupted state, recovery and audit. Highlight missing or invalid protection.
- Closed: entry/exit, gross/net realised P&L, fees, holding duration, exit reason (SL/TP/profit protection/max hold/strategy exit where supported), reconciliation and Learning acceptance/exclusion. MFE/MAE and profit capture only with adequate path coverage. Link to original decision and financial records; unknown attribution remains explicit.

#### 4. Execution quality

- Separate AutoTrader execution and Scout simulation quality. Overview, Orders and History views; compact summary and outcome distribution bar, sortable breakdown by exchange, symbol, strategy, combined, direction, horizon, order type and strict/exploratory policy.
- Report eligible/attempted/submitted/acknowledged counts, full/partial/no-fill, reject/cancel/expiry rates, time to first fill/completion, maker/taker, fees, slippage relative to decision/reference price and known/missing cost coverage. Distinguish overlapping lifecycle counts from mutually exclusive terminal outcomes.
- Click aggregate counts to inspect filtered orders; expand inline using shared order detail. Compact pricing-distance bar shows submitted limit, relevant bid/ask and distance at the labelled observation time; include age/lifetime limit, filled quantity, whether market reached limit, cancellation/revalidation reason and evidence completeness. Touch alone is not proof of a fill.
- Retain diagnostic evidence to distinguish pricing, stale data, matcher, premature cancellation, revalidation and lifecycle problems. No dedicated per-order lifetime chart; retain aggregate historical trends and avoid duplicate market charts.
- Opportunity cost: specify counterfactual reference/source, observation window, horizon, eligible population, sample count and cost coverage. Separate observed post-cancel movement from simulated returns; no causal saved/lost profit claim without validated attribution.
- Scout mode reports simulation coverage, executable price-path availability, cost assumptions, completed/excluded outcomes and quality; never present simulated outcomes as actual fills.

#### 5. Portfolio and reconciliation

- Financial accounts only. Compact summary: equity, opening/current cash by currency, realised/unrealised net P&L, paid fees, trading capital, available/reserved/deployed capital, utilisation, return and drawdown. Show marked position value and valuation/FX timestamps; configurable reporting currency where supported.
- Overview, Balances and Reconciliation views. Consolidated and per-venue/account balances, historical equity/net P&L/fees/drawdown/returns, deposits, withdrawals and capital adjustments. Compact gross winning/losing trade and entry/exit fee attribution to net realised P&L; no double deduction of fees.
- Reconciliation normally a compact status line, expandable to source records and discrepancies. Show independent source/ledger evidence epoch/cursors, captured intervals/history coverage, source-to-ledger counts, matched/unmatched records, mismatch counts and missing fee/mark/identity/FX reasons.
- Distinguish complete, provisional, not evaluated, incomplete, stale and unavailable. Flat cash without settlement is not proof of profitability or complete reconciliation; probes are not settled fills; two projections of one ledger are not independent sources.
- Click totals to inspect underlying transactions. Exclude Scout balances and simulated profit; detailed strategy profitability belongs in Widget 7.

#### 6. Capital allocation

- Read-only autonomous allocation; no manual strategy/horizon budgets. Reuse `sbe_ts-gu78.29` after authoritative allocator contract `sbe_ts-gu78.28`.
- Summary and stacked bar: eligible shared capital/global cap, deployed, reserved, available, utilisation. Show daily notional remaining, exploration allowance/usage, position slots and binding constraints separately; distinguish stock from flow.
- Allocation, Candidates and History views. Group by exchange/account, strategy, symbol, combined, horizon, direction and entry policy. Show deployed/reserved/effective allocation and share; available only where attributable. Never double count shared pools or imply permanent per-strategy budgets. Respect venue-specific collateral and risk constraints.
- Candidates: ranking, eligibility, expected net edge, confidence/effective evidence, proposed amount, strict/exploratory mode, reason and binding constraint, timestamp/source. Proposed amounts are opportunity-specific, not committed capital. Unknown remains unknown.
- Compact blocker counts and clickable filtered candidates: evidence, net edge, reservations, freshness, risk and capital constraints where authoritative. Historical utilisation and constrained/unused capital. Idle capital is valid when opportunities are unsuitable. Scout capacity belongs in Widget 8.

#### 7. Strategy performance

- Separate AutoTrader and Scout modes; Performance, Heatmap and History views. Compact summary of eligible trade/outcome count, net P&L or simulated net return, win rate, profit factor, expectancy and effective evidence.
- Configurable sortable columns and grouping by exchange, strategy, symbol, combined, profile/version, horizon, direction, regime and strict/exploratory policy. Include inactive cohorts and explicit exclusions.
- Metrics: gross/net P&L and return, win rate, profit factor, expectancy/uncertainty, average net win/loss, payoff ratio, breakeven win rate, fees/slippage/cost per trade, drawdown/downside risk, capital efficiency/return per capital-hour, holding duration, time to profit/loss, MFE/MAE, profit capture/giveback, raw/effective samples, confidence, freshness and eligibility, where supported.
- Compact diverging average net win/loss bars with payoff ratio; selectable symbol/strategy heatmap and historical cumulative net performance, expectancy, drawdown and other supported comparisons. Strict versus exploratory side-by-side with comparable windows, regimes, cost basis and cohort definitions.
- Click metrics to inspect trades or Scout outcomes; expand profile parameters, decision/execution/allocation/evidence context and promotion blockers. Exclude unreconciled/ineligible actual outcomes from qualified metrics and report reasons/counts. Show descriptive observations separately from validated conclusions; insufficient evidence must not imply zero or support rankings/promotion. Never rank actual strategies using Scout-only results.

#### 8. Learning and evidence acquisition

- Preserve canonical Section 13 contract and existing `sbe_ts-15ok` implementation. Read-only by default; autonomous prioritisation/acquisition rather than manual Scout commissioning.
- Compact summary: monitored profiles, strict-ready profiles, learning/blocked states and accepted effective gain over selected period. Evidence, Activity, History and Promotion views.
- Evidence matrix grouped by exchange, strategy/profile/version, symbol, direction, regime and horizon: raw/effective current and required samples/windows, coverage bars, confidence, freshness, missing ranges, gap/root cause, priority, blocked decision, last accepted gain, heartbeat/decision and last/next evaluation. Coverage bars are not promotion probabilities; do not invent aggregate requirements.
- Activity: queued, collecting/active, market-waiting, capacity-blocked, stale/stalled, completed, failed, excluded and accepted states. Show acquisition method, job identity, start/progress, expected evaluation, retry/recovery and remaining blocker. States need not be sequential. Click blocker/profile to inspect affected decisions and acquisition tasks.
- Scout effectiveness: active/completed/accepted/excluded counts, acceptance rate, effective gain per completed outcome, gaps resolved and profiles promoted. Separate activity from independent accepted evidence; include actual AutoTrader evidence with provenance. Show Scout capacity and resource constraints here, not as financial allocation.
- History: effective evidence growth, coverage, accepted gains, acquisition cost and completion/gap resolution. Promotion: validated profile changes, eligibility, promotion/rejection/demotion reasons, strict activation and subsequent reconciled performance. Evidence sufficiency alone does not imply positive edge or promotion.
- Preserve configurable signal research views: historical signal replay/performance, missed-opportunity bands, skipped-signal audit, forecast accuracy, strength versus subsequent net returns, paired markouts by horizon, walk-forward validation, entry/exit parameter sensitivity and profile-version comparisons. Retain Scout replay/history and outcomes with provenance and coverage. Do not infer missing ranges or worker health.
- Success is meaningful accepted effective evidence, resolved gaps, validated improvements and ultimately reconciled profitability, not Scout activity alone.

### Grafana dashboards (not React widgets)

\- **Per-symbol feed/worker reliability:** active book subscription; latest

  book/trade timestamps and age; invalid events, gaps and reconnects; worker

  heartbeat, latest decision/error, and evidence freshness. Use explicit

  missing/stale states and avoid deriving a per-symbol guarantee from global

  stream activity.

\- **Deployment traceability:** service/container, deployed image digest,

  source revision and dirty/unknown attestation, build time, schema/config

  version, runtime identity, health and restart time. If the running artifact

  cannot report provenance, display unattested/unknown rather than matching by

  tag or assumed source commit.

The five requested capabilities map to separate work: Decision Funnel

Diagnostics; Execution Quality; Strategy Performance; and Grafana dashboards for

per-symbol reliability and deployment traceability. Capital Allocation and

Evidence Coverage are also required operator widgets, with implementation work

already tracked separately as noted above.

### Market data screens

Standalone market-data screens are **not required for v35 automated paper

operation**. The operator can assess decisions and execution from the funnel,

orders and portfolio views. The funnel must still show or link to the relevant

decision-time market snapshot and its source/freshness so a rejected or selected

signal can be explained. Keep general-purpose chart, order-book, trades,

liquidity and market-overview widgets available for broader dashboard use, but

do not add them to the required v35 operator view set or expand them as part of

this plan.

## Existing widget disposition

The current catalog is in

[`widget_catalog.ts`](../src/typescript/workspace/apps/react-app/src/dashboard/widget_catalog.ts)

and widget rendering is in

[`DashboardWidgetContent.tsx`](../src/typescript/workspace/apps/react-app/src/views/DashboardWidgetContent.tsx).

| Existing widget(s) | v35 disposition | Reason / destination |

|---|---|---|

| Auto Trader | **Keep and narrow** | Retain global paper/risk controls; remove manual per-strategy/horizon budget controls per `sbe_ts-gu78.28/.29`. |

| Auto Trader diagnostics | **Keep and extend** | Dedicated Decision Funnel Diagnostics widget; decision-time market context stays attached to candidate details. |

| Orders, Fills, AutoTrader live/completed positions | **Consolidate** | One Orders and Positions widget with live/history filters. |

| Account, Risk, P&L, Execution Fees | **Consolidate** | Dedicated Portfolio and Reconciliation widget. |

| AutoTrader outcomes, Strategy Performance, Learning realized | **Consolidate** | Dedicated Strategy Performance widget with evidence-qualified comparisons. |

| Learning decisions, Learning Scouts, Learning Scout activity | **Consolidate** | Learning and Evidence Acquisition; preserve Section 13, Scout effectiveness, acquisition tasks and decision drill-down. |

| Execution check and passive-order evidence | **Consolidate/extend** | Dedicated Execution Quality widget, subject to authoritative order/match and counterfactual contracts. |

| Scout positions/outcomes | **Consolidate** | First-class Scout mode in Orders and Positions, Strategy Performance and Learning; exclude from financial portfolio. |

| Scout replay history, Walk-forward data | **Consolidate and retain** | Configurable research views in Learning and Evidence Acquisition. |

| Trade-flow what-if/signal replay, Signal history, Signal performance, Missed-opportunity bands, Skipped-signal audit | **Consolidate and retain** | Configurable signal research views in Learning and Evidence Acquisition; preserve authoritative provenance and coverage. |

| Generic market widgets (market overview, volatility table, price chart, order book, trades, liquidity, news, best signals) | **Outside this v35 change** | Shared market dashboard capability; not required for automated paper operations. The decision funnel retains decision-time market context. This plan does not remove general market views. |

| Portfolio hedging and dated futures carry | **Outside v35 operator minimum; defer** | Retain in the broader app unless a confirmed v35 operator workflow depends on them. |

Retirement means removing the widget from the v35 operator catalog/layout, not

deleting shared backend research data. If persisted layouts reference retired

types, provide a migration or clear unavailable-widget handling before removal.

Usage evidence can guide migration timing; it does not decide whether a view is

needed for v35.

## Delivery order and acceptance

1. **Freeze contracts:** identify exact producer fields, identity, units,

   timestamps, freshness, coverage and unknown-state semantics for each view.

   Define count partitions for the funnel and cohort rules for performance.

2. **Retain AutoTrader controls and deliver Capital Allocation:** preserve

   global safety controls; remove legacy manual budget maps and implement the

   read-only allocation widget after the shared allocator contract (`sbe_ts-gu78.28/.29`).

3. **Extend Decision Funnel Diagnostics:** reconcile the full lifecycle and

   include decision-time market context; do not create duplicate diagnostics.

4. **Deliver Orders and Positions, Portfolio and Reconciliation, and Execution

   Quality** as separate widgets with explicit lifecycle and evidence contracts.

5. **Consolidate Learning and performance** into separate Evidence Coverage and

   Strategy Performance widgets; retain decision drill-down and explicit

   insufficient-evidence states.

6. **Consolidate redundant widgets:** migrate persisted layouts, remove redundant

   v35 catalog entries and verify safety, diagnostics and evidence tasks remain

   possible with the required widgets alone.

Acceptance also requires rendered-browser checks of independent widget instances, resizing, persisted configuration, exact drill-down populations, Scout/AutoTrader separation, shared detail components and bounded hidden-widget requests.

Acceptance requires deterministic fixtures for populated, empty, stale,

truncated and unavailable data; funnel counts reconcile across lifecycle

stages; performance cohorts exclude unreconciled/ineligible results; passive

metrics report their comparison window and coverage; Grafana artifact identity

shows unknown/unattested rather than inventing provenance; and all retained

operator tasks remain possible after widget consolidation. No profitability

claim, live activation, or risk-gate change is part of UI acceptance.

## Current limits

The current v35 status reports no settled paper fills or completed trades, so

strict/exploratory comparative performance and passive execution effectiveness

must initially render as insufficient evidence. Grafana covers operational

health and artifact identity; React retains only decision-specific market

context. The React catalog currently contains many optional widgets while the

default layout is generic market content; this plan scopes the v35 operator

workspace and does not claim that every shared dashboard widget should be

removed.

## Profile-owned exit policy UI extension (2026-11-26)

This addendum is incorporated into master-plan Section 15 and does not create a separate acceptance gate. The AutoTrader Controls summary must omit global fixed SL/TP as if they described every trade. Keep genuine hard safety constraints; if bootstrap/fallback stop and target settings remain necessary, show them only as such and expose source/provenance. The current risk-appetite setting also adapts the global fallback target; it must be labeled as fallback behavior and cannot override a selected profile. Do not alter live paper settings while replacing the display.

Add a configurable, read-only Learned Exit Policies view in Strategy Performance/Learning. Show exchange, symbol, strategy, profile version, direction/regime only when supported by evidence, stop, target, maximum holding duration, trailing stop, profit protection, planned reward-to-risk, parameter source, effective sample size, validation status, last evaluation and last promotion. Drill down to previous/candidate values, executable excursion and post-exit paths, What-if assumptions/results, sensitivity heatmaps, winner/loser transitions, out-of-sample validation, promotion history and accept/reject reasons. Do not infer missing provenance or expose a simulator action that writes trading configuration.

Orders and Positions must show each position's actual selected profile/version/source, original stop/target and initial monetary-risk denominator, current authorized protection thresholds and adjustments/reasons, plus trigger and execution prices. Current API source review found that missing persisted stop/target fields can be filled from global defaults in the activity response, and selected horizon targets could be raised to the global take-profit fallback; those values are not evidence of the entry selection. Keep the displayed selection/source unavailable unless the API explicitly identifies it, and display fallback/current values separately. Profile changes do not silently rewrite existing open-position protection. MFE/MAE, realized R and profitability remain unavailable when executable path, initial risk, fees or reconciliation evidence is incomplete.

Use the existing tested `Model.ts` architecture and shared ui-platform components. Test the model independently from rendering and include browser/rendered verification. Global operator constraints and fallback policy remain separately identified from validated learned parameters. See master Section 15 for candidate metrics, fallback hierarchy, monetary-risk requirements, end-to-end propagation and mandatory deterministic tests.
