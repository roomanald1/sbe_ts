# Trading Strategy Improvement Plan — v35

## Objective
Improve net profitability, evidence quality and capital efficiency through autonomous learning, targeted scouting and portfolio-aware execution. Optimise deployment of all available eligible capital for sustainable risk-adjusted net returns. Reduce unnecessary exploratory costs and manual intervention without weakening risk controls.

## Step 0 — Codex agent profiles and verified delegation (complete)
Step 0 acceptance passed on 24 November 2026. Its setup, profile probes, runtime checks and limitations below are retained as historical evidence; they are not active prerequisites for implementation or a reason to repeat preparation.
Completed preflight and staged-profile setup are recorded below. No schema/profile discovery or setup remains active; preserve installed operational configuration. Use these advisory context monitoring indicators for observation only (not execution limits or mandatory handoff triggers):

| Profile (staged filename stem) | Reasoning | Advisory context monitoring indicator |
|---|---|---:|
| coordinator | High | 200K |
| architect (including risk/concurrency), quantitative, financial | High | 150K |
| rust_worker, integration | Medium | 120K |
| react_worker | Medium | 100K |
| explorer, documentation | Low | 60K |

These are monitoring indicators only, not execution limits or mandatory handoff triggers. Continue working beyond them when productive, prefer handoffs at natural implementation boundaries, and allow native compaction to operate normally.

- Installed the delegation policy in AGENTS.md and verified representative native controls. The coordinator delegates bounded work within the configured two-child maximum; specialists cannot spawn nested agents. Phase 0 owns its separate Cargo build/check queue.
- Completed representative low, medium and high profile probes, model override, concurrency and fresh-session persistence checks. Static validation covered remaining profiles. Unsupported native reasoning/context overrides remain documented limitations under the approved advisory policy; no additional probe or allowance reading is required absent a material runtime change.
- Recorded profile mapping, configuration paths, smoke-test evidence and limitations in Beads `sbe_ts-mmy6`. Token and allowance monitoring is optional; per-work-package accounting and budget reassessment are not required.
- **Step 0 acceptance — complete:** profiles configured, coordinator delegation rules installed, representative child selection and effective settings verified on the intended runtime, context policy demonstrated, and unsupported controls handled under the approved advisory fallback. No new probe is required absent a material runtime/configuration change.

### Verified preflight and staged resource policy (24 November 2026)
- Native VS Code 1.147.0 Agent Host used Codex SDK 0.163.0, distinct from WSL CLI 0.170.0. Native multi_agent was enabled; multi_agent_v2 disabled; reasoning_effort_override unavailable on the native SDK. No SDK upgrade or experimental flag change was made.
- Native strict-config accepted the tested top-level model/context/agent controls and rejected a deliberately invalid key. This established parser recognition, not per-profile hard enforcement. Representative profile, persistence, model override and concurrency probes subsequently passed as recorded below and in Beads `sbe_ts-mmy6`; Step 0 is complete.
- Historical 16 child sessions all recorded GPT-6 Luna medium, unnamed role, depth one; largest two recorded 23.79M and 17.19M cumulative tokens, average input per call 161K and 149K respectively, cache hits about 99%. Measure cache/compaction tradeoffs; do not equate raw input savings with Plus allowance savings.
- Nine named profiles (coordinator, architect, quantitative, financial, rust_worker, integration, react_worker, explorer, documentation) were installed and validated under the approved runtime policy. Historical setup is complete; retain existing config/hooks and do not rerun probes absent a material runtime/configuration change.

### Resume checklist — do not redo completed work
- [x] Identify installed SDK and CLI versions and feature flags; establish strict parser recognition with positive and negative controls. The SDK does not support `--print-startup-config`; do not retry it. Bundled bubblewrap fallback was reported, not a configuration failure.
- [x] Establish historical usage baseline from session database and rollout logs: 16 children, approximately 79M cumulative input, two largest averaging 161K and 149K input per observed call. All recorded medium reasoning. These are historical telemetry, not allowance charges.
- [x] Prepare temporary `.codex/agents/profile_probe.toml` (user executed creation command). Prepare the nine exact named profiles, `config.fragment.toml`, `budgets.json`, `AGENTS.append.md` and `monitor.py` under `~/.codex-agent-staging/`. Verified the staged files and monitor without regenerating or overwriting them. The temporary project probe was removed after runtime verification.
- [x] **Allowance checkpoint:** the user explicitly directed not to inspect allowance. No allowance or reset readings were collected or inferred; this exception is recorded in Beads task `sbe_ts-mmy6`.
- [x] **Minimal first profile probe:** at the user's direction to proceed without allowance inspection, ran exactly one `profile_probe` child and received `PROFILE_PROBE_OK`. It ran at depth one, with parent linkage, provider `openai`, SDK `0.163.0`, model `gpt-6-luna` and low reasoning. The user subsequently approved advisory context checkpoints because the runtime reports a 828,400-token model window and exposes no effective auto-compaction threshold. No reset state is claimed.
- [x] Verify child settings in SQLite and rollout metadata. In a fresh native conversation after installing profiles, verified `explorer` low, `react_worker` and `rust_worker` medium, and `architect` and `coordinator` high, all with effective model `gpt-6-luna` and provider `openai`. Static validation covered the other named profiles. A temporary isolated `model_override_probe` confirmed effective `gpt-6.1-sol` against a `gpt-6-luna` parent; it was removed after the check. Two medium children overlapped and completed, verifying the two-child concurrency limit; all children were depth one. The fresh conversation was reused across profile probes to verify profile persistence.
- [x] Backed up the prior project config, preserved `hooks = true`, installed nine named profiles, set verified project agent limits and the available `gpt-6-luna` model, and added the delegation/resource policy to AGENTS.md. Per-profile context/compaction keys were omitted because the native runtime does not expose them; advisory context targets and checkpoints remain in profile instructions under the user's approval. Coordinator and architect profiles retain high reasoning. No SDK upgrade or experimental flag change was made.
- [x] Step 0 acceptance complete. Effective profile selection, representative reasoning tiers, model selection/override, concurrency, fresh-session persistence and coordinator policy are verified. Context targets use the approved advisory fallback; allowance values remain intentionally uninspected. No pending runtime probe or implementation prerequisite remains.

### Completed diagnostic preparation and verified profile setup (24 November 2026)
- Initial static profile inspection found the coordinator's staged instructions prohibited all delegation. The installed coordinator profile now delegates bounded work-package slices under the two-child maximum; specialists cannot spawn nested agents. The fresh profile probes ran in a new conversation after configuration installation; an earlier probe from the pre-install conversation inherited its stale medium setting and is not used as acceptance evidence.
- User approved advisory context checkpoints and explicitly directed that allowance not be inspected. Runtime metadata reports a model context window of 828,400 tokens and no effective auto-compaction threshold. The nine named profiles therefore keep their context targets and checkpoint guidance in developer instructions while omitting unverified hard context/compaction keys. The prior project config backup is `/tmp/sbe-v35-config-20261124T151856Z.toml.bak`; the original hooks setting is preserved. The temporary `profile_probe.toml` and `model_override_probe.toml` files were removed after verification.
- Runtime evidence for all smoke probes, model override, concurrency and per-agent token counts is recorded in Beads task `sbe_ts-mmy6`; raw session metadata is in `/home/ronnie/.codex/state_5.sqlite` and rollouts under `/home/ronnie/.codex/sessions/2026/11/24/`. `monitor.py --hours 1` reports 13 agent sessions, 241,783 cumulative input tokens, 72,704 cached input tokens, 169,079 uncached input tokens and 163 output tokens for the observed probe set. Allowance remains uninspected as directed.
- Historical preparation after profile installation: `/home/ronnie/.cache/sbe-diagnostics/preparation_report-znb7j18u.json`. Bundle generation and verification completed; preparation acceptance was `NOT_EVALUATED`, Learning analysis and performance gate evidence were missing, and regression status reflected an older failing baseline. Later verified suite results and the current Phase 0/performance blockers are recorded below; this snapshot is not current acceptance evidence.
- Historical user-run preflight reported 51 PASS, 17 WARN, 0 FAIL while native delegation verification was still pending. Later representative probes completed Step 0 as documented above. Beads prime, RTK and deterministic agent/diagnostic scripts remain reusable; detailed telemetry accounting is optional and no allowance savings are inferred.
- Reuse `scripts/db/` read-only tools and catalogue: 148 queries, 145 schema/execution-tested, three blocked by large-cell limits; none automatically promoted to semantically verified. Runtime Insights databases accessible; runtime Learning, AutoTrader, positions and scout database access remains unresolved. Never substitute local data for runtime evidence or sum potentially overlapping audit databases. Verify authority, units, mode, epoch, timestamps and scope.
- Reuse `scripts/api/` RSocket client and five CLI utilities: all four services connected; 14/18 catalogue endpoints returned application success, two quality endpoints timed out, two skipped for size. Learning summary/profiles, AutoTrader feedback queues/open counts, Insights dashboard/best and all eight Position Management endpoints passed. Decision snapshot and monitoring metrics exceed default 4 MB; explicit bounded larger retrieval and compact projections are available but not yet verified for these endpoints. Filtering is client-side, not a server workload reduction. Application success is not financial validation. Keep the default 4 MB response guard; oversized endpoints are opt-in heavyweight diagnostics only, with explicit per-request byte/time budgets and bounded concurrency. The current guard applies after receipt, not before server computation or transport allocation. Prefer server-side projection, pagination or bounded summaries where supported; do not raise the global limit. Keep raw evidence on disk and return only bounded projections to agents; test output caps, oversize rejection and memory usage. Never silently truncate financial evidence.
- Phase 0: validate actual response contracts and build read-only adapters into existing `scripts/diagnostics/` tools; prioritise trading health, learning readiness, feedback health, entry starvation, reconciliation, capital efficiency, scout performance and comparable snapshots. Reuse existing Rust calculations and paper/scout execution, not duplicate engines. Keep routine diagnostics bounded; exclude timed-out quality endpoints and oversized snapshots by default. Validate timeout cleanup, output bounds, exit codes, evidence provenance and adapter fixtures. Do not rerun expensive discovery already completed unless evidence is stale or contradictory.

## Operational ownership and shared trading invariants
AGENTS.md is authoritative for operational plumbing (RTK, reusable repository knowledge, inspection safety, command/output limits, build coordination and delegation). The additions below have been reconciled into AGENTS.md; retain both instructions and resolve future conflicts explicitly:
- Delegate slices: one work package, relevant Phase 0 contracts, exact known paths/symbols, acceptance criteria and Beads ID; never send the entire plan by default. Use fresh targeted contexts and the verified profile policy.
- Scope work to relevant crates (`feed_handlers`, `insights`, `auto_trader`, `strategy`, `learning`, `position_management`, `paper_exchange`) or UI layers (`src/typescript/workspace/apps/react-app`, `src/typescript/workspace/packages/ui-platform`). Verify command availability and use `just test <crate> [filter ...]`, `just check <crate>`, `just typecheck`, `just diff-check` as applicable. Coordinate Cargo builds/checks through one build owner or queue to avoid contention.
- Keep tool output near the existing ~20K-character guideline; redirect large logs/exports to `/tmp/`, inspect bounded summaries, and hand off concise findings with file/line references and pass/fail counts. Follow existing read-only inspection rules.
- Preserve generated SBE codecs unless protocol changes require regeneration; coordinate trade-ticket and Orders flows for VWAP API changes. Apply these guardrails in Phase 0 and Section 6.

### Parallel implementation and dependency-driven scheduling (mandatory)
- Beads is the durable ownership ledger: parent issues may remain `in_progress` for active coordination, while each leaf stays `open` unless a named person or agent is actively assigned and working it. Claim leaves at start; return stale, abandoned, or unassigned leaves to `open` after checking notes, dependencies, working-tree ownership and active sessions. Record ownership and handoff evidence in Beads. An empty `bd ready` result does not establish that no independent work exists; inspect open plan leaves and dependencies.
- Maximize parallel implementation across independent work packages. Treat this plan as a dependency graph, not a sequential checklist: identify implementation-ready Beads tasks and delegate non-conflicting slices immediately.
- Keep all child-agent slots up to the configured two-child maximum occupied whenever useful independent work exists, and reassign slots promptly as specialists finish. The coordinator implements independent work while specialists run unless this would compromise coordination or independent review.
- Assign separate file/component ownership before parallel edits. Serialize Cargo builds/checks through the named build owner when necessary; do not serialize source implementation because of the build queue.
- Block only on genuine technical dependencies, never on an unrelated phase or wave being incomplete. Distinguish implementation dependencies from validation, acceptance and activation dependencies. Permission to implement does not imply acceptance or deployment approval.
- Prioritize substantive application code and regression tests over additional investigation, documentation and preparation. Preserve all financial integrity, risk controls, performance objectives and independent validation requirements.

Canonical invariants (apply to every package and wave):
- **I1 — Reuse:** audit before building; reuse existing scouting, forecasting, rho, replay and schedulers. No new shadow mode or paper execution engine.
- **I2 — Evidence integrity:** distinguish confirmed defects from hypotheses; never fabricate evidence. Preserve historical records, configuration versions and provenance (scout, AutoTrader paper, live exploratory, production). Distinguish missing from zero; reconcile counts/exclusions; use consistent net returns, costs, eligibility and time windows, chronological out-of-sample validation, realistic execution costs, effective independent samples and uncertainty. Prevent leakage, duplication and overfitting.
- **I3 — Risk and abstention:** preserve hard portfolio exposure, symbol concentration, loss, leverage and operational safety limits, manual overrides and atomic reservations. Never bypass or relax hard risk gates, force trades/frequency/full investment, or increase leverage to meet utilisation or evidence targets. Exploration is bounded policy, not a risk override.
- **I4 — Performance:** apply Shared hot-path performance requirements; analytics stay off trading-critical paths. Preserve authoritative execution/evidence and bounded resources.
- **I5 — Validation and activation:** incremental implementation, independent review and cumulative regression gates; no reduction in correctness or tests to save tokens. No live behaviour/configuration changes or deployment without separate approval.

Phase 0 maintains canonical schemas and lifecycle contracts; the integration matrix below owns shared edge cases and regression scenarios. Store detailed field definitions, fixtures and test cases once in versioned contracts or linked Beads tasks; work packages reference them and retain feature-specific acceptance criteria. Both are foundational workstreams, not blanket blockers for independent implementation. Apply each requirement as an implementation or validation dependency only where technically required.

### Implementation tooling and evidence checkpoints

AGENTS.md remains authoritative for command usage, advisory context monitoring indicators and navigation. The coordinator owns the following checkpoints; specialists consume relevant summaries and do not repeat shared preparation. Use existing tools, not replacement scripts. Verify unfamiliar CLI options with `--help`. All generated evidence is advisory until its underlying inputs, scope and results are verified.

| Checkpoint | Existing tools and required action |
|---|---|
| Step 0 probes | Complete. Allowance readings were intentionally not collected per user direction; no new probe is required absent a material runtime/configuration change. |
| Phase 0 start, after Step 0 acceptance | Run `python3 scripts/local/prepare.py --mode quick`; inspect drift, impact, bundle verification and blockers. Use `freshness_check.py` for targeted fingerprint comparisons. |
| Plan or relevant bundle input changes | Run `python3 scripts/local/prepare.py --mode quick --refresh-bundles` or existing `generate_work_packages.py` and `bundle_verify.py` interfaces. Verify plan hash, HEAD, relevant dirty-source changes, evidence freshness and package requirements; do not treat bundle generation as acceptance. |
| Phase 0 financial baseline and reconciliation-dependent gates | Use `financial_snapshot.py`, then `financial_health.py --manifest PATH` and `financial_reconciliation.py --manifest PATH` against the same collected manifest. Prefer the additive Position Management single-ledger snapshot endpoint; its snapshot ID and locked ledger clone establish internal snapshot consistency only, not an independent Paper Exchange execution-ledger reconciliation. Inspect component results, fee completeness and unmatched parent-order rows. Use existing financial adapters for focused analysis. |
| Phase 0 and integration boundaries | Use `prepare.py --mode full` when fresh financial/integration evidence is required. Inspect nested semantic statuses, not just process exit. Explicitly collect any missing learning, runtime or performance evidence; full preparation does not refresh every diagnostic. |
| Each WP start | Read its verified bundle, shared invariants, relevant contracts and prior Beads findings. For known Rust symbols use `python3 scripts/agent/efficiency/symbol_report.py SYMBOL --crate CRATE --limit 5 --brief`; read the compact brief. Use `symbol_lookup.py` for simpler extraction instead, and targeted RTK/rg/ast-grep/rust-analyzer to verify relationships. Reuse valid reports. Lexical rankings are not coverage or semantic proof. |
| During each WP | Use existing `scripts/db/`, `scripts/api/`, `scripts/diagnostics/` and `scripts/local/` tools for bounded evidence collection. Use `tool_catalog.py` to discover unfamiliar capabilities, `bounded_run.py` for large command outputs, and `context_watch.py --role ROLE` at major task boundaries when useful. Keep raw evidence on disk. Record verified reusable discoveries with `bd remember`. |
| Each WP validation | Use `regression_runner.py --package N` for advisory dry-run selection; verify current manifest, selected commands and actual acceptance requirements before execution. Coordinate builds through the single build owner. Use `regression_identity.py` only to identify potential reuse; verify all relevant inputs and prior successful results before reuse. Run missing focused/integration tests and `just diff-check`. |
| Each WP completion | Record actual tests, acceptance status, evidence paths, blockers and incremental usage; hand off compactly. No dependent work advances on failed required tests. |
| Each wave gate | Refresh affected evidence and bundles as needed; execute cumulative required regression and CI matrix. Evaluate measured release-build benchmark evidence with `performance_gate.py --input PATH`; the evaluator is not a benchmark generator and threshold success alone is not acceptance. Preserve all existing gate requirements. |
| Optional usage review | Existing manual monitoring is sufficient. Token telemetry and forecasts may inform prioritization when available; missing values are not blockers, and overruns do not require investigation or changes to correctness gates. |

### Optional token and allowance monitoring

Detailed token accounting, allowance readings and budget reviews are optional. When consulted, distinguish missing data from zero and do not infer allowance charges from token totals or cache ratios. Token forecasts are informational; overruns do not require investigation and never justify skipping correctness, financial or risk gates.

The original planning estimates (37.5M baseline, 30M stretch, 50M contingency) are retained as historical, informational forecasts, not work limits, allowance estimates or acceptance criteria. WP16 remains deferred. No WP allocations, recurring reviews, overrun investigations or reforecasting are required.

### Versioned implementation budget dashboard (explicitly deferred)

The versioned dashboard enhancement is explicitly deferred; existing manual monitoring is sufficient. Do not extend tooling or make this enhancement a prerequisite for Phase 0 or any implementation. Every remaining requirement, interface and acceptance detail in this section is historical planning material only and is not an active task.

| Phase / wave | Immutable input-token baseline | Cumulative baseline |
|---|---:|---:|
| Phase 0 | 1,500,000 | 1,500,000 |
| Wave 1A | 4,000,000 | 5,500,000 |
| Wave 1B | 1,000,000 | 6,500,000 |
| Wave 2 | 8,500,000 | 15,000,000 |
| Wave 3 | 18,000,000 | 33,000,000 |
| Wave 4 | 4,500,000 | 37,500,000 |
| Total | 37,500,000 | 37,500,000 |

Historical specification only: store this baseline in a versioned configuration linked to v35 and the exact plan SHA-256; preserve the original estimates. No budget configuration or WP allocation is required.

- Maintain persistent run identity, repository identity, plan hash, wave/WP slice, Beads task, session ID, parent linkage, start/end checkpoints and attribution provenance. Include coordinator, specialists, resumed sessions, failed attempts, rework and integration overhead. Attribute by explicit session/request boundaries, not overlapping wall-clock windows alone. Deduplicate active/archived copies and cumulative usage snapshots; account for resets, resumed sessions and missing telemetry. If a session crosses tasks, record verified counter boundaries; otherwise report ambiguous consumption as unattributed. Never silently exclude it or assign it twice.
- Report total input (including cached input), cached and uncached input, output and reasoning where exposed, request counts, compactions and context statistics where supported. Do not add reasoning to output if already included. Report coverage, missing sources, attribution conflicts and unattributed totals explicitly. Reconcile attributed plus unattributed implementation usage to observed implementation totals; separate unrelated and preparation usage. Do not claim completeness if telemetry is missing.
- Record actual five-hour and weekly allowance readings, observation times and reset times separately from token telemetry; allow manual entry and unknown values. Never estimate allowance charges from input or cache ratios. Flag resets and unrelated concurrent usage rather than inventing per-WP allowance charges.
- Persist checkpoints atomically with concurrency protection, idempotent checkpoint IDs and auditable corrections; retain compact JSON evidence and terminal summaries. Preserve existing date/repository summary mode and avoid scanning or loading full transcripts into model context. No model calls are required for accounting.
- Show per-slice and per-wave actual, baseline, variance, remaining budget, completion status, cumulative actual and unattributed usage. Forecast final consumption as actual to date plus explicitly estimated remaining work; label assumptions and uncertainty, and do not extrapolate from incomplete waves as if complete. Report variance against completed-scope budgets separately from total budget remaining. Keep original baseline visible even after reforecasting.

Historical proposed CLI (do not implement while deferred):

```bash
python3 scripts/agent/efficiency/budget_dashboard.py --init-plan v35
python3 scripts/agent/efficiency/budget_dashboard.py --start-wave 1A
python3 scripts/agent/efficiency/budget_dashboard.py --checkpoint --wp 2
python3 scripts/agent/efficiency/budget_dashboard.py --plan-summary
python3 scripts/agent/efficiency/budget_dashboard.py --complete-wave 1A
```

Historical interface proposal only; no implementation, checkpoints or budget reviews are required while this enhancement is deferred.

**Dashboard acceptance (historical only):** this enhancement is deferred. Existing manual monitoring is sufficient; no additional preparation or tooling is required.

### Phase 0 contract and ownership baseline — v1 (24 November 2026)

This is the canonical target lifecycle contract; it does not silently change existing wire formats. `schema_version=1` is required on newly persisted canonical lifecycle records. Existing `StrategyMarketSignal`, evaluator `StrategySignal`, diagnostic JSON and `strategy::FeedbackEvent` remain their current contracts until separately reviewed, migrated and tested. In particular, do not treat diagnostic IDs as durable lifecycle IDs or modify generated SBE codecs under this specification alone.

| Entity / fields | Canonical meaning and writer | Existing source / compatibility |
|---|---|---|
| Event | `event_id` stable and idempotent; `schema_version`; `event_type`; `occurred_at_utc_ms`; optional `observed_at_utc_ms`; source/service. One producer per event kind. | FeedHandlers publishes market inputs; Insights derives signal updates. Existing `StrategyMarketSignal.timestamp` is `u64`, but its epoch/unit needs explicit producer verification before migration. |
| Opportunity / episode | `opportunity_id` identifies one accepted AutoTrader entry reservation. `episode_id` identifies the upstream stable strategy thesis and remains distinct from source `event_id`; AutoTrader preserves the versioned Strategy worker's episode ID on accepted reservations. New events within an unchanged thesis reuse the episode; supported thesis-state changes establish a new one. AutoTrader's decision correlation key reuses an existing reservation identity on retry, while opportunity/trade IDs remain reservation-scoped. | Strategy's versioned live worker is the authoritative episode writer; AutoTrader validates and copies its v1 episode ID and fails closed if a v1 report omits it. Legacy signals without a versioned episode retain conservative per-reservation IDs. A fully filled stop that closes the position advances AutoTrader's durable per-symbol/strategy/profile reset generation; Strategy changes the episode hash only after receiving that generation, and AutoTrader requires acknowledgement plus a fresh v1 event before re-entry. Partial stops and non-stop exits do not advance the generation; same-key resets without an authoritative completed stop remain unsupported. Manual/carry legacy records may have no identity. AutoTrader identity-map retention and cross-instance epoch governance remain open. |
| Decision / provenance | `decision_id`, opportunity/episode IDs, UTC decision time, strategy/profile ID plus immutable profile hash/version, execution environment and entry policy, source, eligibility, outcome/reason. AutoTrader owns entry reservation identity; Position Management persists optional lifecycle links on fills; Learning consumes explicit outcome attribution. | `EntryCandidateDiagnostic` IDs remain diagnostic correlation only. New AutoTrader reservation records capture entry profile ID, `execution_mode=paper`, strict/exploratory `entry_policy` and source. When the entry signal supplies every declared strategy parameter, `profile_hash` is SHA-256 v1 over a sorted JSON tuple of symbol, strategy, supplied profile ID and complete parameter map; incomplete/unavailable inputs remain explicitly unknown. `TradeOutcome` carries optional schema version, canonical trade/opportunity/episode IDs, entry profile ID/hash, execution mode, entry policy and source; its existing `profile_id` remains the parameter-derived Learning key. Legacy events decode with absent values and no inferred attribution. |
| Order / fill / trade | Position Management persisted `PositionTrade` is authoritative fill record; Paper Exchange is matcher; AutoTrader state is an operational mirror. Use stable `order_id`, `fill_id`, `trade_id`; signed quantity (+ buy, − sell), positive execution price and absolute notional with explicit quote currency; UTC event times. Partial fills preserve parent order/trade linkage. | New Position Management fills now receive optional versioned `pm-fill:v1:<epoch>:<sequence>` IDs from allocator state persisted with trade rows in the same SQLite transaction; SQLite restart preserves the epoch/sequence, while legacy rows remain unlinked and no fill ID is inferred from the old row `id`. The allocator assumes one active serialized service writer and is not cross-process CAS-protected; some coalesced maker/VWAP/TWAP/carry writes may return before the background snapshot commits, so durable-before-response is not established for every internal fill path. It identifies PM ledger fill rows, not venue fills. Additive optional `lifecycle` metadata passes from AutoTrader reservations through market, resting-maker, VWAP/TWAP, protective-barrier and ordinary exit fills; partial fills retain it. Position Management owns fill persistence and AutoTrader owns outcome-outbox creation. Canonical server `order_id` and its explicit association, quote currency across all producers, non-AutoTrader writers and manual/carry opportunity linkage remain unresolved. |
| Costs and returns | For v1 paper fee records, Position Management `PositionTrade.fee` is the authoritative ledger value; current writers record a fee expense and `signed_fee_cashflow` normalizes legacy sign variants to a negative cashflow. The ledger has no authoritative producer/version marker to distinguish a rebate from a positive legacy fee, so rebates/credits remain unresolved and must not be inferred or represented by `-abs` as if proven. Convert each known ledger cashflow to reporting currency once, then `net_realized_pnl = gross_realized_pnl + sum(converted_signed_cost_cashflows)`. Derived `fees_paid`/`fees_by_quote` are positive display magnitudes, not additional cashflows; do not subtract them again when realized P&L already includes signed fees. If an interface instead carries positive cost magnitudes, subtract those magnitudes once and never also add signed fees. Do not negate an already-negative fee or subtract the same ledger fee again through an aggregate. `net_return_percent = 100 * net_realized_pnl / initial_entry_notional_reporting_currency`; denominator and conversion are immutable for the trade. Never equate absent fees with zero. | Current `TradeOutcome.net_return_percent` is total P&L / initial entry notional × 100, but fee inclusion/reconciliation is not established. `TradeReplayContext` has numeric entry/exit fees without fee currency/type. Mark financial values unresolved until ledger reconciliation establishes semantics and precision. |
| Eligibility / lifecycle | Explicit states `eligible`, `excluded(reason)`, `quarantined(reason)`, `unknown`; never encode missing as zero. Terminal outcomes are closed, cancelled/no-fill, expired, or unresolved. Replay/restart is idempotent by event ID; out-of-order inputs are retained and reconciled before terminalization. | Learning deduplicates durable event IDs. Only newly identity-linked `TradeOutcome` events carry lifecycle schema version 1; other FeedbackEvent variants retain their current wire schema. Preserve legacy decoding; missing identity/provenance routes to quarantine, not inferred attribution. |

**Single-writer/authority map.** FeedHandlers owns emitted feed events; Insights owns derived market/strategy snapshots and UI caches; strategy owns evaluator and `FeedbackEvent` type definitions; AutoTrader owns entry decisions/reservations, operational execution mirror and completed-outcome outbox; Position Management owns validated order acceptance and persisted position/fill ledger; Paper Exchange owns matching results; Learning owns outcome validation/deduplication, eligibility persistence and replay jobs; React models/views own read projections only. No separate Rust Risk service/crate was identified: entry/sizing/protective policy is in AutoTrader and order validation/capacity enforcement in Position Management. These boundaries are source-inventory findings, not authorization to change interfaces.

**Requirements-to-test matrix / current evidence.** Direct existing candidates: strategy evaluator tests; AutoTrader `legacy_learning_compat.rs` attribution/outbox tests; Position Management `market_order_submission.rs` persistence tests; Paper Exchange `orders_api.rs` matcher tests; Learning `feedback.rs` deduplication and `scout_replay.rs`; React `AutoTraderOutcomesModel.test.ts`; scouting tests under `strategy/src/scouts/tests`. Current focused coverage includes AutoTrader canonical identity idempotence/SQLite restart/outbox propagation and a production selected-entry/close/outbox fixture through the real Learning router and Learning SQLite restart, PM lifecycle-link serialization/legacy absence, durable PM client-order replay tests, and the closed-outcomes model's explicit-ID projection. The real-PM lifecycle fixture writes fills through Position Management RSocket to temporary SQLite, restarts PM and checks duplicate idempotency; a lost-ack proxy test commits a PM order, drops the acknowledgement, reloads AutoTrader's durable settings into a fresh state, recovers one fill/position/outcome and verifies a subsequent replay is a no-op. These fixtures do not launch and kill an OS AutoTrader process or verify the rendered UI. Supported completed-stop reset behavior is covered; partial and non-stop reset sources remain intentionally unsupported. `cross_asset_paper_test.rs` is a diagnostic binary reporting `execution: none_paper_only`; paper soak tools are runtime monitors, not deterministic source tests. Reuse those existing harnesses; integration child inventory is in `sbe_ts-gu78.8`.

**Gate evidence and owners.** Ronnie/coordinator owns the serialized Cargo build/check queue and cumulative acceptance. WP01 regression selection was dry-run only. The older AutoTrader log `/home/ronnie/.cache/sbe-diagnostics/autotrader-confirmed-repair-1795528215.log` recorded 212 passed / 2 failed and remains historical; the intermediate 226/102/35 test counts also predate this implementation. Current full suites pass: AutoTrader 232 library + 9 binary tests; Position Management 47 passed/1 ignored; Learning 105 library + 1 operational HTTP test; Insights 46; Strategy 79, plus 208 with the service feature enabled. The latest Insights release fixture measured raw decode→feature evaluation→local signal publication at 128/128 outputs, 6,493.08 signals/s, p50 132,756ns, p95 184,615ns, p99 240,072ns, p99.9 243,819ns; it excludes downstream risk/order-ready, exchange, acknowledgement and fills. AutoTrader release local-service-double evidence is in `/home/ronnie/.cache/sbe-diagnostics/bounded-gdlgibt9.log` (x86_64-linux, HEAD `bfd76b9f1fe2f31fd5c6892ef259103d3652aef9`, dirty tree): 128 sequential precomputed events at 43.26/s (event p50/p95/p99/p99.9 20.802/30.221/58.225/139.941ms); 128 emergency IOC exits at 2,633.96/s (trigger-to-response 0.368/0.483/0.541/0.712ms); and 128 concurrent events over four symbols at 12.40/s, zero audit drops, with burst event p50/p95/p99/p99.9 1,925.844/9,101.496/9,374.881/10,301.423ms and queue wait 20.887/38.492/39.961/40.242ms. Each event in this burst currently uses an isolated AutoTrader state/SQLite path; treat this as an isolated-state stress case, not a shared-service representative burst. All three component records are explicitly unaccepted; throughput is below the 100+ signal/sec objective, and raw market/Insights, exchange acknowledgement and fills are unsupported. Latest financial evidence `/home/ronnie/.cache/sbe-diagnostics/financial_snapshot-cre_d5nf.json` and `/home/ronnie/.cache/sbe-diagnostics/financial_reconciliation-aqo20__u.json` used the backward-compatible legacy five-endpoint fallback because the running service does not expose the new route. Account and all-time realized P&L matched at -59.47994385 USD; 8,267 fee fills equalled 8,267 PositionTrade rows, but separate endpoint skew was 0.501 seconds and the diagnostic remains PROVISIONAL/NOT_EVALUATED with ledger consistency, fee metadata, mode/identity and independent execution-ledger blockers. The new composite snapshot code has focused tests and deterministic script-contract validation but no runtime evidence. Quantitative acceptance remains unverified. Reused preparation `/home/ronnie/.cache/sbe-diagnostics/preparation_report-1xst3ehz.json` has `REVIEW_REQUIRED` / `NOT_EVALUATED`; it is historical and not current acceptance. No preparation or tests are repeated solely to update this documentation.

**Latest shared-state AutoTrader benchmark.** The release sample at `/home/ronnie/.cache/sbe-diagnostics/bounded-725x1y52.log` supersedes the earlier isolated-state stress results above for shared-service interpretation. It used one shared AutoTrader state and SQLite path with a local zero-fill PM double, but the Tokio test executor was current-thread. Precomputed sequential input ran 128 decision cycles at 3,046.96 events/s (128 event samples; one submitted order and 127 skips; event p99 0.493ms/p99.9 27.580ms; its single PM response 54.300ms). Emergency IOC ran 128/128 submissions at 3,171.13 events/s with p99 0.499ms/p99.9 0.547ms. The shared four-symbol burst ran 128 cycles with four submissions, 124 skips and zero audit drops at 1,886.29 events/s; event p99 0.067ms/p99.9 0.067ms, cycle-lock wait p99 43.609ms/p99.9 43.669ms, and only four PM response samples. All outputs are explicitly unaccepted: a current-thread test runtime is not the production multi-thread runtime, and sustained representative soak, target hardware, exchange/fill latency and raw market-to-order timing remain unverified.

**Financial snapshot follow-up.** The latest 22:11 UTC manifest used the backward-compatible five-endpoint fallback because the running service does not expose the new route. Source adds `/positions/financial-snapshot`, which serializes account, positions, append-order realized P&L, fee summary and coverage from one locked ledger clone with a unique snapshot ID, per-currency FX as-of values and explicit paper mode. Market marks are sampled during account projection, so the response claims ledger consistency only and does not claim an atomic FX/mark/exchange snapshot. The existing snapshot/reconciliation scripts consume and validate this composite response, including fee/parent row count identities, currencies, timestamp inversions and refusal of an external-ledger claim. Position Management passes 46 tests/1 ignored; the weighted-average append-vs-timestamp counterexample passes. Three deterministic script-contract tests, legacy fallback smoke and py_compile pass. The running service was not rebuilt or restarted, so no runtime evidence from the new endpoint is claimed. Durable epoch, unknown/mismatched fee currencies, unmatched parent-order rows and independent execution-ledger reconciliation remain separate gates.

**Linked gates.** `sbe_ts-gu78.1` owns the Section 4 data-integrity audit; `sbe_ts-gu78.2` owns the Section 13 evidence-coverage contract; `sbe_ts-gu78.3` owns the Wave 2 ledger/reconciliation gate. The integration inventory and lifecycle ownership map are `sbe_ts-gu78.8` and `sbe_ts-gu78.6`. Their detailed criteria remain authoritative in Beads. Phase 0 is a foundational, cross-wave workstream: each contract, ownership, fixture and evidence requirement gates only work that depends on it. Engineering correctness fixes, diagnostics, instrumentation, lifecycle/recovery improvements, integration fixtures and performance work may proceed when their own interfaces and safety dependencies are understood. Implementation permission is not acceptance or deployment approval. Financial reconciliation, trustworthy attribution, eligible Learning evidence and statistical validation remain mandatory before accepting strategy optimisation or promoting learned parameters. Unrecoverable historical attribution stays explicitly quarantined and does not block forward implementation once new writes and their recovery path are verified.

### Preparation status and unresolved evidence

The user-run symbol report v3.4.1 passed four self-tests, static regex checks and both repository investigations (closed-trade and budget), and generated JSON and compact briefs. Closed-trade behavioural candidates fell from 131 to five; budget still produced 88 behavioural candidates, so prioritise direct tests and verify candidates. This is not measured token savings. Freeze the efficiency toolkit unless implementation exposes a material blocker.

Existing preparation, financial adapters, bundle generation, regression selection and performance evaluation should be reused where required. Their synthetic checks and successful process exits do not prove runtime correctness. The latest validated post-fix suite results are listed in the Phase 0 gate evidence above; retain the older failing baseline for comparison. Revalidate only when changed inputs and the relevant acceptance gate require it. Step 0 native child-profile verification is complete as recorded above. No live activation or deployment is authorised by this update.

## Mandatory cross-system integration and regression testing
- Establish a shared deterministic harness in Phase 0 by extending existing replay, scouting and paper infrastructure; exercise actual production business logic with recorded feeds, synthetic scenarios, controlled clocks/seeds and simulated exchange/transport boundaries. Apply I1. Capture baseline failures and link them to Beads tasks; do not treat known failures as accepted correctness.
- Maintain a requirements-to-test matrix covering every work package and cross-component contract, with named owners, fixtures, expected outcomes and CI results. Each agent adds regression tests for its changes; coordinator owns shared harness and wave gates. Independent review of critical assertions is mandatory.
- Lifecycle and accounting: signal → opportunity/episode → reservation → risk approval → post-only order/revalidation → partial/full fills → protection/exit → ledger → Learning → API/UI. Reconcile quantities, fees, conversions, realised/unrealised P&L and balances at documented asset precision; distinguish estimated costs from actual fees. Test duplicate/out-of-order events, cancellation races and idempotent recovery.
- Capital and risk: concurrent signals across symbols/timeframes, competing reservations, rejects, timeouts, partial fills, closures and mid-trade restarts. Verify conservation accounting for cash, reserved funds, positions, unsettled amounts, fees and liabilities; no double allocation or leaked slots. Test hard limits on every path, correlated clusters, protective exits, best eligible risk-adjusted allocation, cross-horizon redistribution, released capital recycling, tiny-position saturation and justified cash retention when edge is absent.
- Evidence and recovery: disconnect RSocket, exceed 4,096 cache entries, restart workers, inject persistence failures and duplicate events; verify acknowledged durable records survive, bounded pagination/recovery, correct profile/version/mode attribution, explicit incomplete states and no double counting. Exercise scout scheduling through promotion, walk-forward refresh, missed-opportunity evaluation, forecast calibration, gap detection, backfill, accepted evidence and decision reassessment.
- Learning and safety: replay multiple regimes with chronological isolation (only outcomes available at decision time), reproducible versioned parameters, sparse/stale/missing/NaN evidence, invalid forecasts, worker stalls and deteriorating execution. Verify hierarchical fallback or abstention, no silent default accuracy or risk bypass, bounded parameter/allocation changes, demotion, circuit breakers and safe recovery. Test profitable and negative-edge fixtures with known expected decisions; distinguish deterministic correctness from empirical profitability.
- UI and performance: verify API and rendered UI agree with authoritative ledger, reservations, eligibility, evidence coverage and failure states; include representative browser integration tests. Exercise bounded queues, storage, memory, retries and latency under representative load and failures; establish measured budgets and prevent telemetry from blocking execution.
- Fast-path isolated [ENG] fixes in any wave: atomic changes may merge into the working branch after relevant unit tests and existing integration regressions pass, with a brief Beads record. Exclude shared schema/interface, trading behaviour and risk-control changes from this fast path; retain coordinated review for these and full matrix sign-off and independent review at wave gates.
- Run applicable suites in CI at every wave gate, including all prior regression tests. Do not remove failing tests, weaken assertions, mock away the business logic under test or claim completion from compilation/unit tests alone. Document any justified test changes and remaining coverage gaps. Integration success demonstrates tested correctness, not guaranteed future profitability.

## Shared hot-path performance requirements
- Measure monotonic elapsed time from in-memory market/signal event ingress (including queue wait and required feature evaluation) to validated, risk-approved, reserved order payload ready for transmission. Measure raw market-event and precomputed-signal ingress separately; report rejected-decision latency, emergency stop trigger-to-output, exchange transmission/acknowledgement/fill latency separately. Instrument stages, p50/p95/p99/p99.9, throughput, queue age, contention, CPU, memory and dropped telemetry; avoid coordinated omission and success-only sampling.
- Proposed internal latency objectives (stretch / acceptance): p50 0.5/2 ms, p95 2/5 ms, p99 5/10 ms; emergency IOC stop trigger-to-output p99 1/2 ms. Acceptance values are upper bounds under representative sustained and burst workloads, including at least 100 signals/sec and simultaneous cross-symbol bursts. Establish actual baseline, hardware, build mode, workload, sample counts and reproducible benchmark configuration in Phase 0; any revised budget requires documented evidence and coordinator approval, not silent relaxation.
- Current component benchmark evidence (24 November 2026): release build on `x86_64-linux`, Git HEAD `bfd76b9f1fe2f31fd5c6892ef259103d3652aef9`, dirty source tree, 128 events each. Precomputed AutoTrader ingress: 128/128 completed, 43.36 events/s; event latency p50/p95/p99/p99.9 = 21.148/30.114/41.205/135.014 ms. Emergency IOC exit: 128/128 completed, 2,893.58 events/s; trigger-to-response p50/p95/p99/p99.9 = 0.312/0.519/0.971/1.111 ms. Evidence is retained in `/home/ronnie/.cache/sbe-diagnostics/bounded-_tey8_o2.log`; its records are explicitly `unaccepted_release_execution_and_independent_review_required`. The runner uses a local zero-fill Position Management RSocket double; ingress event and asynchronous submit-ready/RSocket timings are separate, and raw market/Insights, exchange acknowledgement and fills are unsupported. This is a production-wrapper component measurement, not the sustained concurrent 100+ signals/sec acceptance benchmark, so it does not pass or revise any latency objective.
- Keep decision state in memory where practical: incremental rolling features, precomputed immutable versioned learning/correlation snapshots, consistent freshness checks, bounded allocations/queues and minimal contention. Audit existing architecture before selecting actors, atomic snapshots or other concurrency mechanisms. Avoid unnecessary synchronous DB, RSocket, filesystem and analytics calls; preserve atomic reservations, final hard risk checks, durable/recoverable order intent and idempotent execution. Measure any necessary durability cost rather than bypassing correctness. Non-critical telemetry uses bounded asynchronous batch persistence; explicit overload policy must never silently discard authoritative execution or accepted evidence.
- Prioritise emergency protective exits over new entries and background work without bypassing mandatory execution safety. Bound and isolate scout, learning, walk-forward, correlation, persistence and UI workloads; apply backpressure, fairness and resource budgets so background activity cannot starve trading. On stale state, saturation or persistence failure, safely suspend affected entries while preserving protective exits and reconciliation.
- Extend the shared integration harness with release-build load, burst, contention, failure and soak tests using actual production components: normal traffic, 100+ signals/sec, simultaneous reservations, maximum configured scout workload (currently reported up to 5,000), concurrent snapshot updates, slow/unavailable DB, RSocket disconnect, UI subscriptions and emergency stops during bursts. Verify latency objectives, no unbounded queue/memory growth, no lost authoritative events, no reservation/risk regressions and safe degradation/recovery. Report stage-level bottlenecks and compare identical workloads against baseline. Run representative performance checks at each dependent gate; full load/soak validation in Wave 2 and cumulative regression in Waves 3–4.

## Phase 0 — Contracts, ownership and triage
- Apply AGENTS.md SBE/VWAP guardrails. Coordinator maps actual runtime dependencies and assigns one writer per shared component in Beads; other agents use agreed interfaces. Assign a named Cargo build/check queue owner in Beads; all agents, including explorers, request builds through that owner under AGENTS.md. Freeze versioned canonical opportunity/episode/trade IDs, profile hash, execution mode, timestamps, signed quantity/notional, cost and net-return formulas, evidence eligibility and lifecycle events before changing those shared contracts. Include migration/backward compatibility; changes require coordinated review. Independent work using stable existing contracts proceeds in parallel.
- Decision contract: signals/market intelligence → Learning edge, confidence and evidence requirements → AutoTrader ranking/sizing/reservations → final Risk approval → Execution/reconciliation → durable outcomes → Learning. Evidence scheduler fulfils Learning requests; exit lifecycle belongs to AutoTrader with Risk and Execution safeguards. Risk vetoes all paths (I3). UI reads authoritative contracts. Assign single owners for intelligence, Learning, scheduler, AutoTrader, Risk, Execution, persistence and UI.
- Tag Beads tasks [ENG] (deterministic plumbing/correctness) or [QUANT] (hypothesis/model evaluation). Dependent QUANT tasks require ENG acceptance and stable contracts; independent audits may proceed concurrently. Preserve versioned paper baseline before changes. Audit the hot path and establish reproducible latency/resource baselines and benchmark fixtures before optimisation. Benchmark release builds on actual deployment hardware under representative concurrent workloads; record runtime/build, hardware, workload, p99 and p99.9, including emergency IOC stops. Investigate missed targets before any coordinator-approved, documented budget revision; preserve risk checks, recoverability and emergency priority. The emergency 2 ms acceptance objective applies to p99, not p99.9.

## 1. Strict entry starvation and exploratory fallback — Critical [ENG diagnosis / QUANT analysis]
Trace actual strict/scout/exploratory paths; quantify pass/fail rates, threshold shortfalls and fallback reasons by strategy, symbol and regime. Determine whether insufficient edge, restrictive gates, configuration or broken evidence/promotion explains rare strict entries. Evaluate net expectancy by strength band (including 0.300), exploratory costs and progress towards eligibility. Expose the decision funnel. Apply I3.

## 2. Profit protection and fee accounting — Critical [ENG]
Reconcile SL/TP, profit protection, rewards and promotion on net returns. Investigate 0.18% estimated round-trip fees versus 0.15% protection activation; verify actual entry/estimated exit costs, maker/taker classification, partial fills, conversions and double counting. Ensure protection genuinely locks net profit; test regressions.

## 3. Winning versus losing trades — Critical [QUANT; after reconciliation]
Analyse each evidence source separately by strategy, symbol, direction, regime, strength, flow/book imbalance, VWAP, momentum, trend, volatility, spread, liquidity, aggressor cost, fill delay, holding duration and exit reason. Measure net expectancy, profit factor, average net win/loss, break-even win rate, costs and gross/net outcomes. Reconcile positive/negative/neutral rates and denominators (ETH/TRX positive expectancy despite low positive rates). Identify pre-entry predictive features versus execution/exit defects. Measure forward returns at 10s/30s/1m/5m/15m/30m and suitable daily horizons; separate signal decay from fill delay and costs.

**Risk-adjusted performance contract (applies to WP3, WP5, WP13–WP15 and UI widgets 5–8).** Add annualised Sharpe and Sortino ratios, Calmar ratio, maximum drawdown, annualised volatility and downside deviation, plus rolling 7-, 30- and 90-day results. Support portfolio, exchange, strategy, symbol, strategy/symbol, profile version, direction, horizon, and strict/exploratory cohorts. AutoTrader and Scout observations are separate populations and are never combined or ranked against one another. Each result carries sample count, elapsed observation duration, interval coverage/completeness, net-cost basis, capital-attribution rule, risk-free-rate assumption, annualisation/interval convention, statistical suitability and confidence interval/method when supported.

Calculate only from authoritative, net-of-cost, consistently sampled valuation-return intervals. Returns use attributable portfolio capital and explicit treatment of deposits, withdrawals, transfers, overlapping positions and capital changes; do not annualise trade-level returns or infer an arbitrary trade frequency. Missing or incomplete valuation intervals withhold the affected series and rolling window; never synthesize zero returns. Use a documented calendar-time convention (including inactive but actually valued intervals), disclose the cadence and risk-free-rate conversion, and account for serial correlation in annualised estimates and uncertainty. Define maximum drawdown from the attributed net wealth series and Calmar from annualised compound return divided by positive maximum drawdown. Zero variance, undefined denominator, short duration, sparse/irregular sampling or insufficient coverage yields an unavailable metric with a reason, not zero or an infinite ratio. Confidence must not imply independence where observations overlap or are serially dependent.

Expose sortable/filterable metrics and comparable historical periods in Strategy Performance tables/heatmaps, Portfolio & Reconciliation, Autonomous Capital Allocation, Learning/profile evaluation, historical What-if, and exit-policy candidate comparisons. Preserve the approved compact card, table and heatmap visual language. Risk-adjusted performance may inform allocation/promotion only alongside net expectancy, uncertainty, execution costs, liquidity, drawdown, correlation and out-of-sample stability; do not optimize Sharpe alone or introduce an arbitrary promotion threshold. Deterministic calculation tests cover formulas, incomplete/missing intervals, irregular sampling, annualisation, zero volatility, capital attribution and withholding; UI model/render tests cover provenance, grouping, sorting/filtering and historical comparisons. Risk metric delivery is additive and must not displace critical-path financial reconciliation or evidence acquisition.

## 4. Scouting, learning and shared data integrity audit — Critical [ENG]
[ENG] One coordinating agent traces actual signal, scout, outcome, learning, promotion, execution and reporting paths. Verify realistic scout fills/costs, comparable AutoTrader outcomes, incremental live evidence, parameter consumption, expiry, persistence and promotion blockers. Investigate common ingestion, attribution, transport and persistence causes before algorithm changes:
- Execute the canonical Section 4 audit checklist below, covering realised learning, replay, daily coverage, walk-forward and missed opportunities.
Acceptance: reconcile eligible counts/returns and demonstrate accepted outcomes influence learning; apply shared evidence/recovery tests, including missing attribution, mode separation, profile changes and empty/populated replay. Treat screenshot anomalies as investigation targets, not confirmed defects.

## 5. Multi-timeframe strategies and dynamic allocation — Critical [ENG + QUANT]
Keep ranking, sizing and atomic reservations within the shared latency budget; test concurrent opportunities and consistent portfolio state.
[QUANT] Verify daily evidence and why positive daily strategies rarely execute. Evaluate 15m/1h/4h variants and higher-timeframe alignment using existing scouting; compare regimes, net expectancy, frequency, holding duration, drawdown and capital/time efficiency. Apply I3; do not transplant parameters blindly.
- [ENG, Wave 1A] Trace sizing defaults, exploratory notional, minimum order/dust rules, rounding, balances, cap precedence and atomic reservation creation/release on fills, cancellation, closure and restart. Explain $11.76/$1,000 with 10/10 slots before optimiser work; do not assume a leak. Apply shared capital/risk tests.
- [QUANT] Audit budget multipliers, risk appetite and cap precedence (including intraday 10/10). Replace arbitrary strategy/horizon caps where justified with bounded learning-led budgets and capacity by strategy/symbol/timeframe. Use edge, independent evidence, confidence, freshness, execution, drawdown, utilisation and correlation; smooth changes and preserve aggregate limits.
- AutoTrader ranks opportunities by net edge, confidence, incremental risk, capital/time efficiency and information value. Prioritise validated strict opportunities; reserve capacity where justified, separate production and exploratory budgets, reduce/suspend negative-edge allocation while retaining scouting. Allow unused capital; switch existing positions only when benefit exceeds costs/risks.
- Optimise the whole available capital pool, not individual strategy budgets in isolation. Reconcile deployable funds, balances, reservations, liquidity and hard risk headroom; dynamically recycle released capital and reallocate idle budgets across eligible strategies, symbols and horizons. Size and rank competing opportunities by marginal expected net portfolio benefit per unit of capital and time, accounting for uncertainty, covariance, costs and drawdown. Investigate persistent underutilisation (baseline $11.76/$1,000 with 10/10 intraday slots); distinguish justified cash reserves from avoidable allocation, eligibility or execution bottlenecks. Apply I3.
- Show base/effective budgets, multipliers, utilisation, reservations, deployable idle funds, reasons for idle capital, marginal opportunity value, rankings, eligibility, evidence gaps and rejection rationale. Compare fixed-budget/cap baselines; apply shared capital/risk tests, including strict starvation and allocation stability. Acceptance: compare net return on total available capital, risk-adjusted return, drawdown, turnover, time-weighted utilisation and avoidable idle capital against fixed-allocation baselines under equivalent conditions; demonstrate that higher utilisation improves portfolio outcomes rather than merely increasing exposure.

## 6. Instrumentation and UI consolidation — Critical [ENG]
The original six dashboard areas remain required capabilities. They are disaggregated into eight independently configurable operator widgets in the UI requirements below; this preserves the original scope while making each operator workflow separately inspectable and testable.
Apply I4; verify instrumentation overhead and freeze stage timing contracts before dependent analytics. Apply the SBE/VWAP guardrails in AGENTS.md.
[ENG] Freeze instrumentation schema before dependent analytics. Capture gross/net MFE/MAE, profit capture relative to positive executable net MFE, giveback/losses, time to extremes/first profit, signal at decision/submission/fill and during holding, fill delay, gross/net returns, expected/actual fees/slippage, trigger versus execution return, configuration and opportunity IDs. Record adverse post-stop movement at 100ms/500ms/1s/5s. Expose in decision/gates/position/learning drill-downs. Use bounded ring buffers, asynchronous batched writes, configurable TTL/downsampling and storage budgets for high-frequency telemetry; monitor dropped samples and backpressure. Apply I4 and the shared overload/restart tests.

Consolidate dashboard into six areas:
- AutoTrader: status, capital, P&L, active/completed-today scouts, evidence gaps and actionable ranked opportunities (not raw signals or unknown edge presented as proven).
- Open positions: exposure, validated Close-out Insights forecasts and exit management.
- Closed positions: realised outcomes and attribution.
- Strategy Performance: profitability, eligibility, allocation and signal diagnostics.
- Learning: Evidence Coverage, effectiveness, parameter sensitivity, Scout Outcomes/replay (paired baseline/variant, markouts, provenance), walk-forward, realised learning, Signal Performance and Missed Opportunity Bands.
- Portfolio Risk: correlation, concentration, hedging and limits.
Audit legacy Best Signals dependencies, 50% accuracy defaults, score, unused 5m context, alphabetical ordering and BTC-BRL heading/USDT mismatch. Reuse validated strength/direction, ranges, trend/volatility, flow, book pressure and VWAP. Retire redundant widgets only after equivalent diagnostics and integrity are verified; avoid duplicate calculations/subscriptions. Show missing/stale/failed data explicitly.

## 7. Post-only entry revalidation — High [ENG + QUANT]
Revalidate resting orders during the existing 5m timeout against strength, aligned flow, VWAP, regime, net edge and original opportunity. Cancel invalidated orders; record reasons. Handle partial fills, cancellation races, transient fluctuations, stale data and rate limits; retain timeout safety limit.

## 8. Repeated exploration and cooldown — High [ENG + QUANT]
Reuse opportunity episode IDs; distinguish emergence, continuation, invalidation and reset. Prevent repeated exploration of unchanged signals; review 2m post-stop cooldown and meaningful reset requirements. Quantify avoided costs and duplicated evidence; no arbitrary cooldown increase.

## 9. Correlation-aware portfolio decisions — High [ENG + QUANT]
Per I4, publish consistent versioned snapshots with freshness and conservative fallback.
Audit existing rho/hedge calculations, returns, alignment, lookbacks and data quality. Measure rolling instrument, signal and strategy P&L correlations separately from evidence dependence. Investigate same-symbol/direction overlap and whether compatible strategies can share execution evidence. Record BTC momentum, breadth, relative performance and strategy agreement; use matched-horizon benchmarks/feature ablation.
- [ENG] Establish aligned return and exposure contracts. [QUANT] Start with conservative signed directional exposure limits per validated correlated cluster; evaluate existing rho before introducing advanced covariance optimisation.
- Compute covariance risk from signed exposure and volatility; monitor gross/net/long/short exposure, incremental risk and concentration. Use robust estimates, conservative stale/sparse fallbacks and stress tests for correlation/hedge breakdown.
- Integrate validated cluster risk into ranking, sizing and allocation first; defer continuous covariance optimisation to Wave 4 unless existing implementation is validated and sufficient evidence supports it; no blanket correlation rejection. Evaluate hedge ratios and net risk reduction after costs, borrowing/funding and account constraints.
- Estimate independent evidence from overlapping windows, common events and duplicated outcomes (not instrument rho alone); target missing independent evidence through Section 13. Show matrices, freshness, risk, concentration, effective samples and decision rationale. Compare profitability, drawdown, turnover and evidence quality against existing baselines.

## 10. Expected net edge and predictive profitability — High [ENG + QUANT]
Estimate gross edge minus fees, spread, slippage and execution uncertainty without double counting. Do not equate strength/VWAP with expected return. Require sufficient net edge for production; permit only bounded evidence-led exploratory exceptions. Show edge, costs, confidence and gates.
Audit existing “Profitable in 5m” calculation: horizon endpoint versus crossing profitability versus current P&L; distinguish existing positions from prospective entries and fill uncertainty. Validate calibration, discrimination, expected return and downside against actual subsequent outcomes and simple P&L/momentum baselines. Evaluate 30s/1m/5m/15m/30m/1h and longer where justified. Integrate only validated incremental forecasts into entry, revalidation, ranking, protection and early exits; show horizon, uncertainty, freshness, calibration and outcomes. Unreliable forecasts cannot silently drive production; request missing evidence via Section 13.

## 11. Entry filtering and automated parameter optimisation — High [QUANT; Wave 3]
Reuse Trade-flow what-if and Missed Opportunity Bands across applicable strategies/symbols/timeframes. Distinguish fixed-direction gate replay from full signal recalculation; pass counts are not trades. Evaluate skipped-signal markouts (1m/5m/15m/60m), recorded SL/TP, fee/spread assumptions, barrier ordering (ambiguous stop-first) and executable returns versus merely crossing breakeven.
[QUANT] Calibrate post-only fill probability, queue uncertainty and conditional post-fill returns/adverse selection using observed fill/miss evidence; distinguish markouts from executable counterfactuals, include unfilled orders and sensitivity bounds rather than arbitrary haircuts.
Quantify profitable opportunities missed, losses avoided, fees saved, frequency, downside, total net profitability and learning coverage by gate/threshold/direction/regime. Compare paired candidates where appropriate against versioned baselines with existing scouting/walk-forward; tune only supported filters (I3). Learning requests missing candidate evidence; expose results, uncertainty, progress and promotion rationale. Do not optimise solely for win rate.

## 12. Early exits and holding duration — High [ENG + QUANT]
Test flow reversal, weakening strength, invalidated VWAP, regime change and lost remaining edge. Analyse time to profit, MFE, signal decay and recovery by holding duration. Use validated forecasts where beneficial. Evaluate bounded post-only non-emergency exits with IOC fallback; retain hard IOC stops and prevent concurrent over-closing.

## 13. Learning, promotion and active evidence acquisition — Critical [ENG Wave 2 / QUANT Wave 3]
Review 45% positive-rate gate (including profitable low-hit-rate subsets), 0.05 learning rate, rewards, bounds, decay and update frequency. Promote on net expectancy, payoff ratio, profit factor, independent samples, confidence, freshness, out-of-sample performance and execution quality. Evaluate symbol/regime-specific learning and provenance weighting. Use hierarchical fallback/shrinkage from shared strategy priors to timeframe/regime/symbol only with sufficient effective independent evidence; empirical Bayes is optional. Bound degrees of freedom, update frequency and allocation/threshold changes; use smoothing, hysteresis and stability tests. Track convergence, parameter stability and rolling/out-of-sample performance versus fixed baselines; demote deteriorating production profiles. Show eligibility, reasons and parameter history.

Learning owns evidence requirements; existing schedulers fulfil them. Publish versioned in-memory decision snapshots and enforce background resource budgets and trading priority under maximum configured workload:
- Identify decision-specific gaps by strategy/symbol/timeframe/direction/regime/strength, including zero-sample strategies, missing history/warm-up, stale walk-forward, scouts, forecast calibration, signal performance and rejected-signal outcomes. Define freshness and effective evidence requirements, not universal sample targets.
- Prioritise by information value, blocked decision, uncertainty, freshness and capacity; reserve under-sampled coverage, avoid redundant correlated samples/parameter combinations. Request supported backfills, recover worker/reporting paths and schedule targeted scouts/walk-forward evaluations; deduplicate jobs, bound retries, respect market availability and concurrency limits.
- Validate and ingest results with provenance; reassess decisions and gaps, stop/reduce collection when sufficient (including negative evidence). Reconcile signal total/directional/neutral/costed counts, legacy missing fields and execution assumptions; evaluate accuracy separately from net returns by horizon and context. Avoid double counting markouts, overlapping windows or scout outcomes.
- Evidence Coverage UI: implement the canonical coverage contract below; expose actionable gaps, collection state and accepted evidence progress.
- Persist and reconcile gap/job state on restart; detect stalled collection, failed ingestion and expiry. Acceptance: every enabled strategy has sufficient evidence or an explicit gap and acquisition plan/blocker; deliberately missing/stale evidence triggers targeted work, accepted outcomes close gaps and decisions are reassessed. Apply shared evidence/recovery and learning tests, including negative evidence and unavailable history/signals. Apply I2 and I3.

## 14. Exploration efficiency — Medium [QUANT]
Use existing scouting for inexpensive evidence; justify live exploration by incremental execution information. Apply I3; calibrate scout execution from comparable live outcomes. Record rejected context and subsequent movement; cap exploratory fees, losses and exposure; allocate by information value.

## 15. Autonomous profile-owned exit parameters — Critical [ENG + QUANT]
Global AutoTrader controls own hard capital, exposure, loss, position-count and risk limits. They do not define universal per-trade protective distances. The current global status/UI reports 0.50% stop / 0.75% target, while protective strategy parameter catalogues commonly bootstrap at 0.50% / 0.80%; selected profile parameters and active validated exit policies can override these values. Strategies may also disable fixed stops or targets. The global pair is therefore not universally executed, but broad strategy defaults remain fixed bootstrap values rather than evidence learned. Source inspection also confirms that `adapt_exit_policy` changes the global fallback target with `risk_appetite`; many strategy profiles shadow it, while unprofiled/counterfactual/legacy fallback paths remain sensitive to it. A further trace confirmed a concrete target override: exit and activity logic used `max(horizon_take_profit_percent, global take_profit_percent)`, which could raise a selected horizon target to the global fallback. The open-position activity response also substitutes global stop/target values when position values are absent. These leaks are being removed from selected policy execution and replaced with separately identified fallback/current fields; compatibility values must not be presented as the entry selection. Keep defaults during the replacement, expose exact persisted source, and test that risk appetite/global fallback never silently overrides a selected profile value. See Beads `sbe_ts-c7ag.5` and `sbe_ts-q8or.15`.

### 15.1 Parameter ownership, learning and propagation
- Separate operator safety constraints from strategy policy. Validated strategy profiles own stop-loss and take-profit distances, maximum holding duration, profit-protection activation/giveback, trailing activation/distance, breakeven protection and supported volatility-adjusted thresholds. Attribute every selected value to symbol, strategy, profile ID/version and its source. Add direction, regime, volatility, exchange and horizon only where effective independent evidence supports that granularity.
- Evaluate candidate exits with executable MFE/MAE and post-exit paths, fill/fee/FX costs and the historical What-if Simulator. Compare net expectancy and expectancy in R, Profit Factor, payoff ratio, average winner/loser, winner preservation, loser recovery, drawdown, risk-adjusted return, capital efficiency, holding duration, execution costs and robustness across conditions. Do not optimize win rate or gross return alone. Preserve effective-sample, confidence, holdout, walk-forward and promotion safeguards; prefer stable parameter regions to isolated optima.
- Implement an explicit fallback hierarchy: validated symbol-specific profile; validated broader strategy profile; bounded exploratory policy; documented bootstrap/fallback default; insufficient evidence; awaiting validation. Record the exact source/provenance selected for each order and position. Never describe an unvalidated default as learned. Preserve existing fail-closed and controlled exploration behavior where no valid protection exists.
- Keep learned thresholds within operator-defined hard limits. Wider stops must not increase monetary risk: size from the selected stop and authoritative initial risk while accounting for available capital, fees/costs, liquidity, slippage/gap risk, existing exposure, remaining daily loss and correlation constraints. A stop is not a guaranteed maximum loss. Adjust size to the applicable policy rather than enforcing identical distances.
- Trace Learning → validated profile → Strategy → signal → AutoTrader → allocation → Position Management → barrier. Persist profile ID/version, parameter source, selected initial stop/target, immutable initial-risk denominator, actual barrier thresholds, subsequent adjustments/reasons, trigger and execution prices. Preserve original risk for realized-R calculations. A profile promotion must not silently alter an open position beyond its authorized protection policy.
- Test distinct symbols under one strategy, distinct strategies for one symbol, supported direction/regime/volatility variants, broader-profile fallback, insufficient evidence, promotion/version change, restart persistence, exact barrier propagation, unsafe-policy rejection, stop-aware sizing and monetary risk, open-position protection under profile changes, unchanged initial risk, Learning safeguards, and UI provenance. Vary global risk appetite with a validated profile and prove that its exit parameters remain unchanged; separately verify and label fallback-only adaptation. Include deterministic end-to-end fixtures proving two positions receive different profile-selected barriers; isolated calculation tests alone are insufficient.
- UI: remove global fixed SL/TP from the primary Controls summary. Retain genuine hard safety limits and identify bootstrap settings as fallbacks where needed. Show selected policies in Strategy Performance and Learning, and actual per-position thresholds/provenance/adjustments in Orders and Positions. Add a configurable read-only Learned Exit Policies view with exchange, symbol, strategy, profile version, supported direction/regime, stop, target, max hold, trailing/profit protection, planned reward/risk, parameter source, effective samples, validation status, last evaluation and promotion. Drill down into prior/candidate values, path evidence, What-if results, sensitivity heatmaps, winner/loser transitions, out-of-sample validation, promotion history and accept/reject reasons. The What-if Simulator must not write trading configuration.
- Acceptance: exact profile-selected parameters reach protective execution and survive durable recovery; monetary risk/capital invariants and existing protection remain correct; missing/unsafe evidence fails closed; all offline deterministic contracts pass. Empirical strategy selection, promotion and paper activation additionally require complete reconciled evidence and their existing approval gates.

## 15.2 Volatility-adjusted protection [ENG + QUANT; dependency of 15.1]
Evaluate existing 0.50%/0.80% strategy bootstrap defaults separately from global 0.50%/0.75% UI settings. Compare bounded volatility-adjusted exits against the validated profile baseline using executable MFE/MAE, duration, costs and symbol/regime where evidence supports it. Preserve economic minimums, maximum monetary risk and stop-aware sizing; no activation on incomplete historical evidence.

## 16. Passive stop-loss execution — Deferred [ENG + QUANT; deferred]
Out of implementation scope until Sections 2, 7 and 12 are validated in production and separately approved. Compare maker savings with fill probability, queue uncertainty, latency, adverse movement and tail losses using Section 6 evidence. Never replace hard IOC stops with unprotected post-only orders.

## Execution waves and dependencies
Pre-0. **Agent configuration:** complete; use the verified profiles and runtime policy already recorded. No setup or probe is pending.
0. **Foundations across waves:** Phase 0 maintains the baseline, dependency graph, versioned schemas, single-writer contracts, shared harness, fixtures, invariant matrix and CI gates. Enforce each acceptance gate at the dependent work package or promotion boundary; Phase 0 is not a blanket prerequisite for independent implementation.
1A. **Engineering triage and P0 fixes:** Sections 2/4 and sizing/reservations (5). Trace and fix confirmed accounting, attribution, persistence, transport and sizing defects first; regression-test and reconcile affected data before dependent diagnostics. Include the Insights entry_fill lifecycle correctness investigation and bounded audit SQL remediation in the dedicated Insights reference subsection below; independent existing-feature audits may run concurrently.
1B. **Behavioural diagnostics:** Sections 1/3 and existing functionality audits (5–13). Use verified evidence only; preliminary gross markouts must be labelled non-executable and cannot justify tuning. Record unresolved dependencies.
2. **Foundations:** complete ENG remediation, freeze Section 6 instrumentation, establish shared evidence/risk contracts, active evidence scheduling/recovery and coverage UI (4/6/9/13); revalidate orders and deduplicate exploration (7/8) after relevant contracts pass. Consolidate verified diagnostics and pass the Insights diagnostic workload isolation, cancellation and sample-population contract in the dedicated Insights reference subsection below.
**Mandatory reconciliation gate:** pass the canonical Wave 2 reconciliation specification below before statistical evaluation, optimization acceptance or learned-parameter promotion that depends on reconciled evidence (I2, I5). Independent engineering implementation, instrumentation and deterministic fixtures may proceed while it is open.
3. **Evidence-led improvements:** independently implement and validate forecasts, filters, exits, learning/promotion, multi-timeframe strategies, dynamic allocation and initial correlation cluster risk (5/9–14). Offline implementation and isolated deterministic tests may proceed when their technical inputs are ready. Strategy acceptance requires the reconciled Wave 2 baseline, strict financial/attribution gates, eligible Learning evidence and statistical validation. Validate features individually against the frozen Wave 2 baseline using paired existing scout/replay evaluations where applicable. Activate one feature at a time in the shared paper portfolio with coordinator-controlled configuration and a soak window based on sufficient effective samples and relevant market conditions, not arbitrary elapsed time or trade counts. While a feature soaks, agents may prepare and validate subsequent features in parallel using existing offline replay/scouting and isolated configuration fixtures; do not activate them in the shared paper portfolio or contaminate the frozen comparison. Record incremental net benefit, uncertainty and rollback criteria; test combined-feature interactions before final acceptance. Retire redundant widgets only after verification.
4. **Advanced optimisation:** volatility-adjusted exits and justified advanced covariance sizing (9/15); passive stops (16) deferred outside current scope.

Wave labels organize dependencies, implementation priorities and cumulative acceptance; they do not prescribe a global start order. Start any work package when its own technical inputs are ready, even while unrelated work or gates in another phase/wave remain open. A failed validation blocks only the affected dependent acceptance, promotion or activation path.

### Confirmed Insights audit findings and remediation (ENG; Sections 2/3/4/6)
- Evidence from user-run runtime validation and source inspection on 24 November 2026: `src/rust/insights/src/signal_engine/store/signal_store_audit.rs`, `load_auto_trader_execution_audit_sync` and `count_auto_trader_decisions`; consumers `signal_store_decision_quality.rs` and `signal_store_execution_quality.rs`; RSocket routes in `browser_rsocket.rs`. Both quality endpoints exceeded the Python client 15-second deadline; execution quality also timed out with a one-hour `since` request. Internal client timeout is reportedly eight seconds; cleanup defect is suspected, not proven. Both consumers use a 300-second cache keyed by exact `since`, hold their respective cache mutex through computation, and load up to 10,000 execution audit records. SQLite reads run via `spawn_blocking`; compressed payloads are gzip-decoded and JSON-parsed after collection. Decision quality additionally counts decisions.
- **Wave 1A correctness priority:** loader predicate `stage = 'entry' OR stage LIKE 'exit%'` excludes `entry_fill`, although decision quality explicitly handles cumulative `entry_fill` updates. Source mismatch confirmed; frequency and actual financial impact not yet measured. Trace audit writers, actual stage populations and consumers. Reproduce resting entry, partial/full cumulative fills, cancellation, external exits, duplicates, fees and restart/replay; verify quantity, weighted entry price, fee attribution and net P&L against ledger. Correct the loader/consumer contract without double-counting cumulative updates or unintentionally changing execution-quality order sample semantics. Include lifecycle events spanning time boundaries and deterministic ordering for equal timestamps. Reconcile affected history or quarantine under the canonical gate; do not tune on suspect quality metrics.
- **Wave 1A/2 performance:** both queries use `(?1 IS NULL OR timestamp >= ?1)`. Runtime EXPLAIN on `insights/auto_trader_audit.sqlite` with literal one-hour cutoff showed execution `SCAN ... USING INDEX auto_trader_audit_timestamp` versus direct predicate `SEARCH ... (timestamp>?)`; decision count `SEARCH ... USING COVERING INDEX auto_trader_audit_stage_timestamp (stage=?)` versus `(stage=? AND timestamp>?)`. These establish improved access plans, not measured query speed or proven timeout root cause. Split bounded and unbounded parameterised SQL paths, preserving semantics; verify actual bound-parameter plans and equivalence on fixed fixtures. Benchmark bounded stage counts and representative payload reads separately, cold/warm cache, SQL vs decompression/JSON vs aggregation vs serialization. No new index by default; consider selective indexes only after profiling and measuring write/storage overhead. Evaluate existing daily summaries for exact historical counts, including partial days, freshness and deduplication; never silently approximate.
- **Wave 2 diagnostic contract:** support explicit bounded windows (1h/6h/24h/7d/30d as appropriate), report exact cutoff/as-of, mode/scope, cache age, population counts, returned record counts, truncation and incomplete lifecycle exclusions. The 10,000 execution record cap is not a scan bound; all-decision counts and capped execution samples must not be presented as identical populations. Five-minute rounded starts can improve cache reuse only when the actual cutoff is disclosed. Use short cache lookup/publication critical sections only; never hold the cache mutex across database awaits, gzip decompression, JSON parsing, aggregation or serialization. Coalesce identical in-flight requests (single-flight), with bounded waiters and cancellation-safe cleanup; prevent stale computations overwriting newer cache entries. Bound distinct-window concurrency, queue length, blocking-worker occupancy and aggregate memory/IO budgets; reject or defer excess diagnostic requests explicitly rather than queue indefinitely. Isolate diagnostic capacity from trading-critical work and durable audit writes. Do not release the mutex merely to permit an unbounded cache-miss stampede. Investigate cancellation/deadlines end-to-end including Node socket/process cleanup and blocking SQLite work; client timeout alone does not cancel a running database query. Keep diagnostic work isolated from signal/entry/exit paths; preserve emergency exit priority and durable audit writes.
- **Acceptance:** record baseline and post-fix query plans, elapsed execution time, scanned/returned rows where measurable, CPU/memory/IO, decompression cost, cache hit/miss and concurrent request behaviour. Prove bounded queries complete within an agreed measured endpoint budget (initial target below existing eight-second client deadline for routine windows), or explicitly classify as heavyweight with separate resource/deadline policy. Test timeout/disconnect cleanup, cancellation, repeated requests, truncation and UI/API consistency. Add concurrent same-key and distinct-key cache-miss tests, slow/decompression-heavy fixtures, cache-hit latency during computation, failed/cancelled leader recovery, bounded waiter/worker saturation and oversized snapshot tests. Demonstrate no cache lock spans expensive computation, no duplicate same-key computation under contention, no unbounded queue or thread-pool growth, and no trading latency regression under diagnostic saturation; report explicit overload outcomes and resource measurements. Compare representative release-build trading latency under concurrent diagnostics to frozen baseline and enforce existing hot-path acceptance objectives. No promotion of affected metrics until ledger/lifecycle tests and population semantics pass. Record evidence in linked Beads tasks; no Rust or database changes have yet been made by this planning update.

### Canonical Wave 2 reconciliation gate specification
Create one linked Beads gate task from this specification during Phase 0; maintain detailed fixtures, quarantine cases and epoch checks there rather than repeating them in work packages. The following requirements remain mandatory:

Ledger P&L, fees, balances and reservations reconcile within documented currency precision/rounding; eligible counts reconcile exactly with explicit exclusions/unattributed records. Demonstrate outcome ingestion, profile provenance, restart recovery and instrumentation completeness; isolate unrecoverable history from quantitative evaluation. Establish a versioned clean-slate epoch after fixes: require exact reconciliation within documented asset precision for all post-fix replay/paper runs, and account for historical records through verified recovery or explicit quarantined exclusions. Preserve legacy records and report unresolved historical discrepancies; do not block progression on open-ended forensic reconstruction when current correctness is proven. Learning acquires fresh evidence where needed. Coordinator signs off reproducible tests; no blanket percentage tolerance or tuning on corrupted data.

### Canonical Section 4 audit checklist
During Phase 0 copy this checklist into the linked Section 4 Beads task. Delegate only relevant audit slices; retain all requirements.

- Realised learning: 1,374+ reported trades versus zero profile-attributed results; budget multipliers show no closed trades; P&L totals disagree. Trace ledger → eligibility → learning → persistence → API → UI; verify entry profile ID/version through closure, partial exits and restarts, execution-mode filters and exclusions. Recover attribution only when supported and within a bounded investigation; preserve unrecoverable records with explicit `legacy_unattributed` provenance and exclude them from quantitative training/evaluation.
- Scout replay: RSocket closed, 0/0 durable records, cache limit 4,096. Verify generation, retention, eviction, routing and retrieval; reconcile legitimate count differences. Add bounded pagination, cancellation/reconnection and explicit completeness/errors; durable learning must not depend on runtime cache or UI connectivity. Version durable replay schema and retain exact entry parameter hashes and historical readers across migrations.
- Daily coverage: dated futures carry missing worker reports/zero returns; Donchian missing evidence/BTC SMA200 rejection. Check supported instruments, history, warm-up, worker health, reporting, natural signal frequency and ingestion. Verify zero is not a missing-value default; allow separately labelled gate counterfactuals in existing scouting.
- BTC SMA regime scope is strategy-local, never a global AutoTrader filter. Current required strategies are Donchian 55/20/10 BTC200 (including the 20-day hold variant), BTC-regime trend sleeves, risk-adjusted trend portfolio and risk-adjusted monthly momentum. Own-asset SMA, RSI2, relative-strength leader, live insight strategies and dated carry do not inherit this gate. A required but missing BTC regime fails closed; the gate is direction-independent within each strategy contract. Preserve rejected candidates for separately attributed Scout observation where that contract allows, while never bypassing a required gate for a live or exploratory entry. Deterministic acceptance covers gated and ungated strategies across symbols/directions, strict and supported exploratory paths, and missing required regime. Donchian-20-BTC parameter probes remain explicitly unsupported/unclassified until separately specified; do not imply they are tested variants.
- Walk-forward: missing Donchian samples, stale intraday evidence (18 Nov), ADA momentum 213 raw/5.1 effective and trend 48 raw/1 effective. Audit scheduling and data [ENG]; validate independent windows [QUANT], effective samples, confidence intervals (including unexpectedly narrow intervals), drawdown/downside, freshness and ingestion.
- Missed opportunities: zero samples despite rejections; trace eligible skipped signals, horizon completion, evaluation, persistence and filters.

### Canonical Evidence Coverage UI contract (Section 13)
During Phase 0 create a linked Beads contract/task from this checklist. Supply it only to agents implementing or validating the coverage UI and its authoritative backend. All fields and state distinctions remain mandatory.

- Evidence Coverage UI: gap reason/root cause, priority, blocked decision, missing data ranges, required/current effective samples/windows, confidence, freshness, last worker heartbeat/decision, last/next evaluation, queued/active/completed/excluded/accepted work, progress over time, last progress, recovery action and blocker. Distinguish queued, collecting, waiting for market, capacity-blocked, stale/stalled and resolved. Show actual accepted evidence gains separately from activity.

### Mandatory integration gates (cumulative)
| Gate | Required evidence before dependent work advances |
|---|---|
| Step 0 | Agent profiles, effective child settings, delegation and context policy verified; limitations resolved or approved. |
| Phase 0 | For every dependent work package, its required contract/owner, deterministic fixture, applicable baseline and known failures are documented and verified. Independent implementation may proceed around unmet unrelated Phase 0 evidence; this does not imply feature acceptance or deployment approval. |
| Wave 1A | Accounting, attribution, persistence, reservation and concurrency regressions pass for remediated paths; affected Insights audit lifecycle metrics reconcile or remain quarantined. |
| Wave 1B | Diagnostics reproducible against reconciled evidence; chronological isolation verified. |
| Wave 2 | Full lifecycle, recovery, evidence acquisition, instrumentation, API/UI and reconciliation suites pass; shared latency acceptance objectives, burst/failure and soak tests pass, including concurrent bounded diagnostic workloads and timeout cleanup. |
| Wave 3 | Learning, promotion, forecasts, filtering, allocation, risk and circuit-breaker scenarios pass; strict financial reconciliation, trustworthy attribution, eligible evidence and statistical validation pass before optimization acceptance or learned-parameter promotion; paired/sequential paper validation and combined-feature interaction tests are documented; cumulative latency and resource regression checks pass. |
| Wave 4 | Advanced optimisation, cumulative regression, load and safety suites pass. |

Coordinator records CI run, fixture/configuration versions, coverage and independent review in Beads. Failed required tests block dependent progression; independent work may continue without bypassing gates. Apply I5.

## Delivery and acceptance
For each package report confirmed behaviour, data/code references, changes, dependencies, tests/results, measured benefit, remaining risks and activation/rollback requirements. Coordinator resolves overlaps and dependencies; implementation, validation and live activation are separate milestones. Measure net expectancy/profitability, fees, payoff ratio, drawdown, capital efficiency, net return on total available capital, avoidable idle capital, independent evidence, gap closure, strict eligibility and execution reliability, internal latency percentiles and background-load isolation—not feature count or trading volume.

- Runtime circuit breakers: monitor stale/missing evidence, worker stalls, unexpected costs/slippage, reconciliation failures, allocation oscillation and risk breaches. Define calibrated thresholds, minimum evidence, recovery conditions and tests per feature; fall back to last validated safe configuration, reduce exposure or suspend affected entries. Preserve protective exits. Apply I5.
- Measure incremental contribution of each feature using existing scouting, walk-forward and AutoTrader paper mode; bound scheduler compute/storage budgets, monitor queue fairness and accepted evidence per resource unit.


## Operational Trading UI and Independent Functional Verification — Critical [ENG]

This section incorporates the full requirements from `research/V35_UI_VIEW_PLAN.md` into the authoritative v35 master plan. The source file is marked superseded only after this incorporation; this master plan and the original v35 sections remain authoritative. The UI specifications supplement and preserve Sections 1–16, canonical contracts, financial gates, risk controls, performance targets, promotion criteria and WP16 deferral.

Governance reconciliation: one existing v35 coordinator owns both functional lifecycle and UI implementation, plan/Beads state, shared contracts and integration. Up to two existing specialists work concurrently with disjoint file ownership; no nested delegation; the coordinator owns and serializes Cargo builds. Canonical service contracts remain authoritative and versioned; React must not duplicate business logic or fabricate missing values. Existing financial reconciliation, runtime acceptance, statistical promotion, performance and deployment gates remain distinct. Correct empty, unavailable and insufficient-evidence UI states can be accepted deterministically without a completed trade, but they do not satisfy the separate end-to-end trading or empirical v35 gates. No deployment or commit occurs without approval.

Immediate parallel delivery priority: (1) Decision Funnel Diagnostics; (2) Execution Quality and the first settled trade lifecycle investigation; (3) autonomous shared capital allocation; (4) Learning and targeted evidence acquisition; (5) AutoTrader Controls and Safety. Capital allocation offline engineering is complete under `sbe_ts-gu78.28`/`.29`; deployed behavior remains unverified. WP5 and WP6 are Critical. Do not change existing financial, risk, promotion or performance thresholds.

The complete operator mission, governance-compatible dashboard architecture, eight widget specifications, data owners, interactions, metrics, visualisations, historical contracts, Grafana boundary, performance/resource rules, implementation phases, testing and acceptance criteria follow. All widgets must proceed incrementally; UI work must not wait for the backend lifecycle to produce a fill, and backend defects must not be hidden by the UI.


### UI 1. Mission

Deliver a comprehensive operational trading UI that makes the behaviour, performance, intelligence and financial integrity of the automated trading system visible and understandable.

The UI must enable the operator to understand:

- What the system is doing.
- Why individual symbols and strategies are or are not trading.
- Why trading opportunities are accepted, rejected or ranked out.
- How capital is autonomously allocated.
- Whether orders are being executed effectively.
- Whether positions are correctly managed and protected.
- Whether trading is profitable after all costs.
- Whether Learning is improving trading intelligence.
- Whether financial outcomes are independently reconciled.
- Whether intervention is required.

The UI must support autonomous trading rather than require manual management of individual strategy or horizon allocations.

This implementation must also serve as an independent functional review of v35, identifying missing functionality, incorrect implementations, observability gaps and omissions in the master plan.

### UI 2. Agent Responsibilities and Working Arrangements

#### UI 2.1 Independent existing v35 coordinator

The existing v35 coordinator is the single overall coordinator for this work and retains the current task context, master plan, Beads ownership, serialized build queue, trading behavior and overall acceptance responsibility. UI work is an independent concurrent implementation track, not a second overall coordinator.

Do not assume that existing v35 functionality is correct or complete.

The coordinator must actively implement and integrate work, rather than operate solely as a project manager.

#### UI 2.2 Existing v35 coordinator

The existing v35 coordinator retains responsibility for:

- Trading functionality.
- Runtime validation.
- Trading-system defects.
- Financial reconciliation.
- Quantitative evaluation.
- Performance acceptance.
- Overall v35 acceptance.

Coordinate shared backend changes through Beads and explicit file ownership.

#### UI 2.3 Specialist agents

Use existing specialist configurations where available:

- React specialist: widgets, charts, tables, interactions, layouts and frontend testing.
- Backend contract specialist: authoritative APIs, diagnostic contracts, functional verification and missing data.
- Grafana or integration specialist when required: technical monitoring and cross-service verification.

Run independent work in parallel where safe.

Avoid conflicting edits, competing builds and duplicated investigations.

#### UI 2.4 Existing tooling

Reuse:

- Existing AGENTS.md instructions.
- Existing Codex specialist configurations.
- Beads for task management and dependencies.
- Persistent repository navigation knowledge through `bd remember`.
- RTK.
- Existing bounded execution and diagnostic scripts.
- Existing API and database inspection tools.
- Existing regression and preparation tooling.

Do not undertake another general tooling optimisation or repository discovery exercise.

### UI 3. Relationship to the v35 Master Plan

#### UI 3.1 Authoritative sources

Before implementation, read:

- `src/rust/auto_trader/TRADING_IMPROVEMENTS.MD`.
- All applicable v35 work packages and acceptance criteria.
- The latest `plan.md`.
- Relevant open, completed, blocked and in-progress Beads tasks.
- Existing source implementation and backend contracts.
- Relevant deterministic tests and runtime evidence.
- Existing React widget catalog and rendering implementation.
- Existing Grafana dashboards and exporters.

WP16 remains deferred.

The v35 master plan remains authoritative for trading behaviour, risk management, financial integrity, Learning, performance and acceptance requirements.

This UI plan supplements the master plan. It does not replace it.

#### UI 3.2 Requirements traceability

Maintain a traceability matrix linking each widget and its capabilities to:

- Relevant v35 section and work package.
- Exact functional requirement.
- Authoritative backend service.
- Existing implementation and API.
- Relevant source files.
- Associated Beads tasks.
- Available deterministic tests.
- Available runtime evidence.
- Missing or incomplete functionality.
- Required frontend changes.
- Required backend changes.
- Acceptance status.

The work-package references in this UI plan are initial mappings. Verify them against the authoritative master plan rather than treating them as exhaustive.

Review every applicable v35 work package for operationally relevant information that should be visible in the UI.

Do not assume a requirement is satisfied simply because a field exists, a widget displays a value or a Beads task is closed.

#### UI 3.3 UI implementation as functional verification

Use implementation of each widget to verify the underlying system behaviour.

For example:

- Capital Allocation must establish that capital is genuinely shared and dynamically allocated.
- Decision Funnel Diagnostics must establish that relevant decision stages are observable and correctly attributed.
- Orders and Positions must establish that protective exits, holding-duration rules and recovery states operate correctly.
- Execution Quality must establish that matching, cancellation, revalidation and execution costs are captured correctly.
- Portfolio and Reconciliation must establish that independent financial evidence is complete.
- Strategy Performance must establish that outcomes are attributable, fee-adjusted and evidence-qualified.
- Learning must establish that acquisition activity produces measurable accepted evidence gains.

A visually correct widget does not constitute acceptance of the underlying trading functionality.

#### UI 3.4 Gap classification

Classify every discovered gap as:

1. Frontend presentation defect.
2. Backend observability gap.
3. Backend functional defect.
4. Missing implementation of an existing v35 requirement.
5. Master plan omission, ambiguity or contradiction.
6. Runtime or empirical evidence gap.
7. New enhancement outside v35.

For each gap, record:

- Affected widget and operator workflow.
- Relevant master plan requirement.
- Expected versus actual behaviour.
- Supporting source, test or runtime evidence.
- Responsible backend service.
- Trading, financial, risk, Learning and performance implications.
- Dependencies.
- Proposed resolution.
- Acceptance criteria.
- Associated Beads task.

Reopen previously completed tasks where evidence demonstrates that their acceptance criteria were not satisfied.

#### UI 3.5 Gap resolution

Resolve confirmed frontend defects and existing v35 observability requirements.

Coordinate backend functional defects and missing v35 implementation with the existing v35 coordinator.

Do not conceal backend defects through:

- Frontend workarounds.
- Inferred financial values.
- Fabricated diagnostics.
- Duplicate business logic.
- Misleading default values.
- Artificially successful status indicators.

Where backend functionality is missing, implement or coordinate the authoritative contract rather than duplicating the functionality in React.

#### UI 3.6 Master plan amendments

Where implementation reveals a genuine omission, contradiction or ambiguity in v35:

- Identify the exact affected section.
- Explain why the existing requirement is insufficient.
- Explain the consequences.
- Propose the smallest necessary amendment.
- Identify affected work packages, contracts, tests and dependencies.
- Record the proposed amendment in `plan.md` and Beads.
- Obtain approval before materially changing authoritative requirements.

Do not silently change trading policies, financial contracts, risk limits, promotion criteria or performance targets.

Record genuinely new enhancements in a separate next-wave candidate backlog.

Do not create a replacement v36 master plan during this work.

### UI 4. Dashboard Architecture

Retain eight independently configurable operator widgets:

| Widget | Purpose |
|---|---|
| AutoTrader Controls and Safety | Configure overall trading and risk |
| Decision Funnel Diagnostics | Explain why opportunities progress or fail |
| Orders and Positions | Inspect the complete trading lifecycle |
| Execution Quality | Evaluate execution effectiveness and costs |
| Portfolio and Reconciliation | Understand financial performance and integrity |
| Capital Allocation | Understand autonomous capital distribution |
| Strategy Performance | Evaluate cost-adjusted strategy performance |
| Learning and Evidence Acquisition | Understand autonomous improvement |

Each widget must be independently:

- Addable.
- Removable.
- Movable.
- Resizable.
- Configurable.
- Filterable where applicable.

These are individual widgets, not eight fixed pages.

#### UI 4.0 Visual direction and permanent mockup references (2026-11-27)

The concrete visual references and handling rules are maintained under [research/v35-ui-mocks/](../../../research/v35-ui-mocks/); begin with [START_HERE.md](../../../research/v35-ui-mocks/START_HERE.md). The PNGs are the visual targets and matching HTML files are layout references only. Their numbers, trades, statuses, and market values are illustrative fixtures, never production data. Preserve the UI plan's superseded status: this master plan remains authoritative for behavior, data provenance, architecture, dependencies, and acceptance.

| Widget target | PNG visual target | HTML layout reference |
|---|---|---|
| AutoTrader Controls and Safety | [01-controls.png](../../../research/v35-ui-mocks/01-controls.png) | [01-controls.html](../../../research/v35-ui-mocks/01-controls.html) |
| Decision Funnel | [02-funnel.png](../../../research/v35-ui-mocks/02-funnel.png) | [02-funnel.html](../../../research/v35-ui-mocks/02-funnel.html) |
| Orders and Positions — open | [03-positions.png](../../../research/v35-ui-mocks/03-positions.png) | [03-positions.html](../../../research/v35-ui-mocks/03-positions.html) |
| Orders and Positions — closed | [03b-closed.png](../../../research/v35-ui-mocks/03b-closed.png) | [03b-closed.html](../../../research/v35-ui-mocks/03b-closed.html) |
| Execution Quality | [04-execution.png](../../../research/v35-ui-mocks/04-execution.png) | [04-execution.html](../../../research/v35-ui-mocks/04-execution.html) |
| Portfolio and Reconciliation | [05-portfolio.png](../../../research/v35-ui-mocks/05-portfolio.png) | [05-portfolio.html](../../../research/v35-ui-mocks/05-portfolio.html) |
| Autonomous Capital Allocation | [06-capital.png](../../../research/v35-ui-mocks/06-capital.png) | [06-capital.html](../../../research/v35-ui-mocks/06-capital.html) |
| Strategy Performance | [07-performance.png](../../../research/v35-ui-mocks/07-performance.png) | [07-performance.html](../../../research/v35-ui-mocks/07-performance.html) |
| Learning and Evidence Acquisition | [08-learning.png](../../../research/v35-ui-mocks/08-learning.png) | [08-learning.html](../../../research/v35-ui-mocks/08-learning.html) |
| Excursions and Exit Analysis | [09-excursions.png](../../../research/v35-ui-mocks/09-excursions.png) | [09-excursions.html](../../../research/v35-ui-mocks/09-excursions.html) |

Match the mockups' compact trading-terminal hierarchy: dark navy/slate surfaces, concise sans-serif typography, aligned tabular numbers, compact summary metrics and controls, dense banded tables, restrained semantic green/red/amber, clear selected tabs, and supporting visualisations. Avoid oversized cards, repeated headings, verbose diagnostics, excessive padding and unused space. Use responsive proportions and wrapping rather than fixed mockup pixel dimensions; expanded details must not overlap or require horizontal scrolling. Preserve the actual Paper mode label and clearly distinguish AutoTrader execution from Scout simulation.

For Orders and Positions, implement the open and closed states in [03-positions.png](../../../research/v35-ui-mocks/03-positions.png) and [03b-closed.png](../../../research/v35-ui-mocks/03b-closed.png): a compact summary strip, population/venue/column controls, dense sortable tables, an inline Net vs Breakeven bar with only sourced executable-close, breakeven, stop, target and dynamic protection markers, and compact expandable execution/identity details. The closed table should support the reference's net P&L/return/R/MFE/MAE/exit/evidence columns, but each field must remain unavailable unless its authoritative evidence is complete. Signal-managed positions must not show inactive fixed thresholds. Preserve stable lifecycle identity, copyable technical diagnostics inside expanded detail, and tested per-instance `Model.ts` state ownership.

Render only source-backed values. When a mark, barrier, cost, identity, reconciliation state, or excursion is absent or stale, use the existing explicit unavailable/stale semantics and omit unsupported chart markers; never substitute the static HTML's values. These supplied targets update the visual acceptance criteria, not trading behavior, eligibility, financial accounting or risk settings.

#### UI 4.1 Default dashboard presets

#### Trading Overview

- AutoTrader Controls and Safety.
- Portfolio and Reconciliation.
- Capital Allocation.
- Decision Funnel Diagnostics.
- Orders and Positions.

#### Strategy and Learning

- Strategy Performance.
- Learning and Evidence Acquisition.
- Decision Funnel Diagnostics.
- Capital Allocation.

#### Execution Analysis

- Decision Funnel Diagnostics.
- Execution Quality.
- Orders and Positions.

These are customisable presets.

Preserve user-created dashboard layouts and existing widget configuration where possible.

#### UI 4.2 Research and market dashboards

Preserve optional access to existing research and general market functionality.
Keep the **Crypto News & Signals** widget available in the optional market
dashboard/catalog; it is not part of the default automated-trading presets.

Do not clutter the default automated trading workspace with unrelated market widgets.

Grafana remains responsible for technical infrastructure monitoring.

#### UI 4.3 Mandatory tested Model.ts architecture

Every independently stateful operator widget and drill-down view must have a corresponding tested `*Model.ts`; this applies to all eight widgets, including existing views being consolidated. Follow the established `MarketLiquidityModel` and `useBehaviorSnapshot` pattern and reuse current models rather than adding a second state framework.

- TSX is presentation-only: render model snapshots, bind controls to explicit model commands, compose presentational components, and forward React lifecycle/prop changes. Do not put fetches, subscriptions, timers, business logic, state machines or asynchronous workflows in TSX.
- Each widget instance owns its model and persisted view configuration. Models expose typed observable `state$` and explicit commands, and own applicable initialization, activation/deactivation, visibility, connection/scope, filter/group/time-range changes, drill-down/back, pagination, refresh, loading/error/stale state, cancellation, retry, configuration persistence and disposal. Authoritative trading/financial calculations remain in backend services.
- Constructors must not launch unmanaged side effects during render. Forward stable or changed inputs explicitly through the repository's supported lifecycle pattern. Repeated activation/cleanup and React Strict Mode must not duplicate subscriptions/timers. Hidden widgets suspend work; disposal releases subscriptions/timers; superseded requests are invalidated and late results cannot overwrite current state. Share transport/cache infrastructure where appropriate, not mutable instance view state.
- Every model has independent model-level tests using the existing test framework. Cover initial state; activation/deactivation; changed inputs; subscriptions; cancellation and out-of-order results; error/recovery and stale/missing data; filters/navigation; persistence; disposal; repeated mount/unmount and visibility; and independent instances as applicable. Use deterministic fake transport/clock fixtures. Keep rendered component/browser tests for presentation, event binding, responsive sizing and integration; these complement model tests.
- Acceptance of each widget requires its tested model, thin TSX view, and passing relevant model and integration tests. Record implementation, deterministic test, deployment, runtime verification and empirical/trade-dependent status separately. Deliver widget increments as dependencies permit; this gate must not make UI delivery wait for completed-trade evidence. Apply the rule to affected v35 views only; do not expand into an unrelated application-wide refactor.

#### UI 4.4 Mandatory completion, scrolling, and legacy consolidation

The eight required operational widgets and the additional Excursions & Exit Analysis widget are not accepted solely because they have models, pass component tests, or render explicit unavailable states. Track each widget independently through implementation, authoritative backend data availability, runtime verification, visual fidelity against its supplied PNG/HTML reference, scrolling/responsive behavior, interaction testing, legacy replacement and operator approval.

- Trace every required operational value from authoritative producer through route/subscription and DTO, into the tested `Model.ts`, and finally to the rendered control or visualization. Classify missing values as source absence, incorrect subscription, contract mismatch, deployment mismatch, stale/incomplete evidence or frontend mapping defect. Repair defects at their owning layer. An unavailable state is valid only while the specific upstream blocker is recorded; it is not permanent acceptance for a value the system should provide.
- Restore the Orders and Positions inline Net vs Breakeven bar with only source-valid executable-close, breakeven, selected stop/target and dynamic protection markers. Show entry value, executable mark, net unrealized P&L, exposure and protection exception status when authoritative data exists. Signal-managed positions must not inherit fixed fallback protection markers.
- Excursion acquisition is event-driven, samples no more slowly than one second, keeps a ten-minute general rolling market buffer and complete trade-specific paths, captures post-exit observations and chronological threshold crossings, and supports deterministic historical What-if replay only from timestamped, fee/FX-attributed evidence. Diagnose every incomplete path individually, distinguishing unrecoverable historical gaps from live acquisition defects. Never fabricate backfill, relabel synthetic surfaced records as natural outcomes, or reduce Learning/promotion requirements. A deterministic fixture proves software behavior; a subsequently occurring natural Paper trade is separate runtime/empirical evidence.
- Audit scrolling and overflow across the full dashboard, prioritizing dense tables. Prefer generic `ui-platform` primitives: vertically scroll table bodies with sticky/preserved column headers; use horizontal table scrolling only where responsive columns cannot fit; keep expanded row details accessible; prevent clipped rows/cells/controls and adjacent-widget overflow; style scrollbars consistently; verify resize behavior; and avoid redundant nested vertical scrollers. Other oversized widget content must also remain accessible. Test actual widgets at small, medium and large sizes.
- Use the dashboard container as the sole primary title/header. Remove duplicate internal titles, redundant status/explanation copy, excessive whitespace, unnecessary metric cards and repeated controls; move durable diagnostics into expandable details or tooltips. Use shared responsive spacing and typography tokens rather than per-widget fixed mockup dimensions.
- Inventory every dashboard catalogue/render entry as retain unchanged, retain and improve, consolidate into an approved v35 widget, superseded and remove, or explicitly deferred. Name the replacement for each consolidation. Before removal, trace non-UI consumers through subscriptions, handlers, DTOs, aggregators, pollers, workers, caches, configuration and tests, including Grafana, Learning, reconciliation and operational monitoring. Quantify subscription, processing, memory and complexity reductions. Do not migrate, rewrite, reorder, replace or auto-upgrade existing saved views/layouts or widget configurations. Preserve their serialized type, position, filters and settings; retired types referenced by saved views require a non-destructive unavailable-widget state or minimal compatibility renderer until the operator manually replaces/removes them. Do not remove shared data, research, Scout/signal workflows or a dependency whose consumers are uncertain.
- For each approved view, verify populated deterministic fixtures for every supported interaction, filters, sorting, drill-down, details, multiple independently configured instances, persistence, loading/stale/unavailable/error states, lifecycle cancellation/disposal, scrolling, resizing and visual reference fidelity. Verify actual runtime separately. Capture compact browser screenshots at comparable small, medium and large sizes for operator review.
- Maintain the completion matrix in `plan.md`; Beads owns durable task status. Keep implementation, runtime/financial evidence, visual fidelity and operator approval as separate columns. Never close a widget task when its data, runtime, visual, scrolling, legacy-replacement or operator-approval gates remain unresolved. Keep Paper-only mode, existing risk settings, historical records, performance limits, Learning safeguards and WP16 deferral unchanged.

### UI 5. Widget 1 — AutoTrader Controls and Safety

#### UI Description

The central control panel for autonomous trading.

The operator configures overall capital, trading scope and risk restrictions. The trading system determines how eligible capital is allocated across opportunities.

#### UI Operator questions

- Is automated trading enabled?
- Is the intended execution mode active?
- What global risk restrictions apply?
- Is trading blocked by a safety latch or audit failure?
- What capital and trading limits are effective?

#### UI Summary information

Display:

- AutoTrader enabled/disabled status.
- Effective execution mode.
- Paper Exchange confirmation.
- Audit health.
- Safety latch state.
- Emergency stop state.
- Total trading capital.
- Available trading budget.
- Active risk restrictions.
- Effective configuration version.
- Configuration freshness.
- Latest configuration change where available.

#### UI Configurable controls

Retain authorised controls for:

- Global capital limits.
- Maximum order notional.
- Daily notional limit.
- Daily loss limit.
- Maximum open positions.
- Exploration budget.
- Bootstrap stop/target fallback settings only where required; identify their source and do not present them as effective trade parameters. Remove global fixed SL/TP from the primary Controls summary.
- Permitted symbol and strategy scope.
- Existing justified safety overrides.

Separate frequently used controls from advanced configuration.

Material trading-control changes must require appropriate confirmation and backend acknowledgement.

#### UI Remove legacy controls

Remove:

- Editable per-strategy allocation maps.
- Editable per-horizon allocation maps.
- Manual allocation repair warnings.
- Obsolete allocation configuration dependencies.
- Transmission of stale legacy budget maps.

Do not simply hide these controls while leaving their obsolete restrictions active.

Do not introduce a live-venue switch.

#### UI Navigation

Link to:

- Capital Allocation.
- Decision Funnel Diagnostics.
- Portfolio and Reconciliation.

#### UI Authoritative data owner

AutoTrader.

#### UI Initial v35 mapping

WP5, WP6, WP9 and global risk and execution requirements.

#### UI Functional verification

Confirm:

- Capital allocation is genuinely autonomous.
- Obsolete manual budgets do not constrain eligible strategies.
- Zero legacy allocations do not disable eligible strategies.
- Effective settings match displayed settings.
- Global safety restrictions remain enforced.
- Configuration updates do not overwrite unrelated settings.

### UI 6. Widget 2 — Decision Funnel Diagnostics

#### UI Description

The primary diagnostic widget for understanding the complete decision pipeline.

It must identify precisely which symbols and strategies are trading, which are not, and why.

An overall aggregate funnel is insufficient.

#### UI Operator questions

- Are strategies generating signals?
- Which symbols generate opportunities?
- Which symbol/strategy combinations are inactive?
- Which gates reject candidates?
- Why are eligible candidates not selected?
- Why are selected candidates not submitted?
- Why are submitted orders not filling?
- Are strict or exploratory opportunities being starved?
- Are decisions ultimately producing reconciled outcomes and useful Learning evidence?

#### UI 6.1 Aggregation dimensions

Support complete funnels grouped by:

- Entire portfolio.
- Individual symbol.
- Individual strategy.
- Symbol and strategy combination.
- Strategy profile.
- Direction.
- Trading horizon.
- Market regime where available.
- Strict eligibility.
- Exploratory eligibility.
- Actual selection mode.

Allow applicable dimensions to be combined.

Include all configured strategies and applicable monitored symbols, including combinations with zero activity.

Distinguish:

- Configured but inactive.
- No qualifying signals.
- Signals rejected.
- Evidence blocked.
- Execution blocked.
- Missing diagnostics.
- Stale data.
- Unsupported symbol/strategy combination.

#### UI 6.2 Funnel stages

Represent the actual production pipeline, including applicable stages for:

1. Market events received.
2. Worker evaluations.
3. Signals generated.
4. Fresh and valid signals.
5. Directional candidates.
6. Strategy gates.
7. Evidence eligibility.
8. Portfolio and risk eligibility.
9. Candidate selection.
10. Execution eligibility.
11. Order attempts.
12. Order submissions.
13. Order acknowledgements.
14. Partial or complete fills.
15. Positions opened.
16. Positions closed.
17. Financial outcomes reconciled.
18. Outcomes accepted by Learning.

The displayed stage ordering must reflect actual production behaviour.

Strict and exploratory eligibility may branch rather than form a single sequential funnel.

Market events, evaluations, decisions, orders, fills and trades are different populations. Do not force them into an artificially monotonic funnel.

Use linked lifecycle sub-funnels where necessary.

#### UI 6.3 Funnel matrix

Provide a sortable, filterable matrix.

Example compact presentation:

| Symbol / Strategy | Signals | Fresh | Directional | Eligible | Selected | Attempted | Submitted | Filled | Closed |
|---|---|---|---|---|---|---|---|---|---|

Allow grouping by symbol, strategy and symbol/strategy combination.

Support:

- Absolute counts.
- Stage conversion percentages.
- Rejection counts.
- Dominant rejection reasons.
- Latest activity timestamp.
- Data freshness.
- Population coverage.
- Pending outcomes.
- Truncation indicators.

Provide expandable detailed columns for additional stages.

Use heatmap-style formatting to highlight:

- Zero activity.
- Persistent starvation.
- Low conversion.
- Dominant rejection stages.
- Execution failures.
- Missing evidence.

Allow sorting by any applicable metric.

#### UI 6.4 Symbol/strategy heatmap

Provide a matrix with:

- Symbols on one axis.
- Strategies on the other axis.
- Selectable metric displayed in each cell.

Selectable metrics should include:

- Signal count.
- Eligible candidate count.
- Strict eligibility rate.
- Exploratory eligibility rate.
- Selection rate.
- Submission rate.
- Fill rate.
- Dominant rejection stage.
- Time since last activity.

Selecting a cell opens the complete funnel for that symbol/strategy combination.

Distinguish genuine zero values from missing or unsupported combinations.

#### UI 6.5 Gate analysis

Expose applicable rejection categories, including:

- Market-data freshness.
- Missing local books.
- Strategy conditions.
- Trend filters.
- Directional confidence.
- Expected net edge.
- Evidence requirements.
- Profile eligibility.
- Exploration eligibility.
- Capital availability.
- Portfolio exposure.
- Correlation restrictions.
- Risk limits.
- Duplicate event or episode reservations.
- Cooldown restrictions.
- Candidate ranking.
- Execution restrictions.
- Post-only revalidation.
- Invalid or incomplete inputs.

Only display gates that exist in the authoritative implementation.

Distinguish primary rejection reasons from additional failed gates.

Do not count multiple failed gates as multiple rejected candidates unless explicitly reporting gate occurrences.

#### UI 6.6 Candidate drill-down

Selecting any funnel cell, stage or rejection category must expose the underlying decisions.

Display:

- Decision ID.
- Event ID.
- Episode ID.
- Cycle ID.
- Symbol.
- Strategy.
- Profile and profile version.
- Direction.
- Horizon.
- Strict eligibility.
- Exploratory eligibility.
- Actual selection mode.
- Decision-time market price.
- Executable bid and ask.
- Market-data source.
- Market-data freshness.
- Gate name.
- Observed value.
- Required value.
- Comparator.
- Pass/fail result.
- Primary rejection reason.
- Additional failed gates.
- Selection and ranking information.
- Associated order, fill and trade identities.

Link to the originating worker evaluation and downstream lifecycle.

Never describe a cycle as having no signals when candidates were generated and rejected.

#### UI 6.7 Historical visualisations

Provide:

- Signal generation over time.
- Directional candidate counts.
- Stage conversion trends.
- Rejection reasons over time.
- Strict versus exploratory eligibility.
- Selection and submission rates.
- Fill rates.
- Persistent symbol starvation.
- Persistent strategy starvation.
- Symbol/strategy conversion comparisons.

Support:

- Latest cycle.
- Last hour.
- Last 24 hours.
- Last seven days.
- Last 30 days.
- Custom ranges.

Limit available ranges to actual retained evidence.

#### UI 6.8 Data integrity

Define stage-specific:

- Populations.
- Denominators.
- Identity.
- Time-window membership.
- Coverage.
- Deduplication rules.

Distinguish unique events, repeated evaluations and unique episodes.

Handle asynchronous order transitions, retries, repeated acknowledgements and partial fills.

Distinguish pending outcomes from terminal failures.

Do not infer strict/exploratory eligibility from eventual submitted order mode.

Extend authoritative diagnostic contracts where necessary.

Use bounded server-side aggregation rather than downloading unlimited history into React.

#### UI Authoritative data owners

- Strategy: worker evaluations.
- AutoTrader: candidate selection, gates and submissions.
- Position Management: order and position transitions.
- Paper Exchange: matching and settlement.
- Insights and FeedHandlers: decision-time market context.
- Learning: accepted outcome and evidence status.

#### UI Initial v35 mapping

WP1, WP4–WP15 where applicable, plus execution and lifecycle requirements.

#### UI Functional verification

Use the widget to identify missing gates, incorrectly applied restrictions, persistent starvation, stale inputs and incomplete decision attribution.

This widget must be capable of explaining the current runtime situation where opportunities are generated but orders remain unfilled.

### UI 7. Widget 3 — Orders and Positions

#### UI Description

A consolidated lifecycle widget replacing overlapping order, fill, open-position and completed-position views.

#### UI Operator questions

- What orders are active?
- What positions are open?
- Are all positions protected?
- Why did an order terminate?
- Why was a position closed?
- Can every trade be traced to its original decision?

#### UI Tabs

- Open Positions.
- Active Orders.
- Protective Exits.
- Order History.
- Closed Positions.

#### UI 7.1 Open positions

Display:

- Symbol, strategy and profile.
- Direction and selection mode.
- Entry price.
- Current executable price.
- Quantity and notional.
- Gross unrealised P&L.
- Net unrealised P&L.
- Paid fees.
- Estimated exit fees.
- Holding duration.
- Maximum permitted holding duration.
- Stop-loss level.
- Take-profit level.
- Selected exit-policy profile/version and parameter source.
- Original selected stop/target and initial monetary risk denominator.
- Subsequent protection adjustments, authorization/reason, and trigger versus execution prices.
- Profit-protection state.
- Protective order and barrier status.
- Forecast-driven exit or deferral state.
- Market-data freshness.
- Position and decision lineage.

#### UI 7.2 Orders

Display:

- Stable order identity.
- Parent order identity.
- Original decision.
- Strategy and profile.
- Selection mode.
- Order type.
- Requested quantity.
- Filled quantity.
- Remaining quantity.
- Limit price.
- Average execution price.
- Maker/taker classification.
- Creation and update timestamps.
- Order age.
- Current status.
- Cancellation, expiry or rejection reason.
- Acknowledgement and recovery state.

Preserve partial-fill and asynchronous lifecycle information.

#### UI 7.3 Protective exits

Display:

- Associated position.
- Protection type.
- Trigger price.
- Protective quantity.
- Barrier status.
- Pending, active, triggered or interrupted state.
- Recovery requirements.
- Last update.
- Relevant audit warnings.

Make missing or interrupted protection prominent.

#### UI 7.4 Closed positions

Display:

- Entry and exit prices.
- Gross realised P&L.
- Net realised P&L.
- Entry and exit fees.
- Holding duration.
- Exit reason.
- Strategy and profile.
- Selection mode.
- Maximum favourable excursion.
- Maximum adverse excursion.
- Independent reconciliation status.
- Learning acceptance or exclusion reason.

Only display excursion metrics where executable price-path evidence is sufficiently complete. Only report realized R when authoritative initial risk and complete reconciled executable-path and fee evidence are available.

#### UI 7.5 Visualisations

- Position exposure by symbol.
- Position exposure by strategy.
- Open position holding duration.
- Position-level net P&L.
- Exit-reason distribution.

#### UI Navigation

Link to:

- Original decision.
- Execution Quality.
- Portfolio reconciliation.
- Strategy Performance.
- Learning outcome.

#### UI Authoritative data owners

Position Management, AutoTrader and Paper Exchange.

#### UI Initial v35 mapping

WP2, WP4, WP6, WP7, WP8, WP12 and WP15.

#### UI Functional verification

Verify protective barriers, fee attribution, holding-duration rules, recovery behaviour, exit reasons and stable lifecycle identity.

Do not infer missing attribution.

### UI 8. Widget 4 — Execution Quality

#### UI Description

Evaluate how effectively trading decisions become actual executions, including the effectiveness and cost of passive orders.

#### UI Operator questions

- Why are orders not filling?
- Are passive orders priced competitively?
- Are orders cancelled prematurely?
- Is revalidation preventing valid executions?
- Are fees and slippage excessive?
- Which symbols and strategies execute most effectively?

#### UI 8.1 Summary metrics

- Eligible opportunities.
- Attempted orders.
- Submitted orders.
- Acknowledged orders.
- Full-fill rate.
- Partial-fill rate.
- Unfilled rate.
- Rejection rate.
- Cancellation rate.
- Expiry rate.
- Time to first fill.
- Time to completion.
- Maker/taker distribution.
- Execution fees.
- Slippage.
- Cost evidence coverage.

Define denominators and distinguish overlapping order outcomes from mutually exclusive terminal states.

#### UI 8.2 Breakdown dimensions

- Symbol.
- Strategy.
- Symbol/strategy combination.
- Direction.
- Horizon.
- Order type.
- Strict/exploratory mode.

#### UI 8.3 Passive-order analysis

For each passive order, show:

- Submitted price.
- Decision-time executable bid and ask.
- Distance from executable market price.
- Order lifetime.
- Market movement during the order lifetime.
- Whether the market reached the order price.
- Available matching evidence.
- Partial-fill behaviour.
- Terminal reason.
- Revalidation reason.
- Cancellation reason.
- Market-data completeness.

Distinguish legitimate passive-order non-execution from:

- Pricing defects.
- Missing or stale market data.
- Matcher defects.
- Premature cancellation.
- Incorrect revalidation.
- Lifecycle inconsistencies.

A market price touching an order price does not automatically establish that the order should have filled.

#### UI 8.4 Opportunity cost

Show observed post-cancellation market movement separately from simulated counterfactual returns.

Define:

- Reference price.
- Executable price source.
- Observation window.
- Evaluation horizon.
- Eligible population.
- Sample count.
- Cost assumptions.
- Evidence completeness.

Do not present hypothetical missed profits or savings as realised financial outcomes.

#### UI 8.5 Historical visualisations

- Fill rates by symbol and strategy.
- Cancellation and expiry trends.
- Time-to-fill distributions.
- Maker/taker proportions.
- Fees over time.
- Slippage over time.
- Execution conversion trends.
- Passive-order effectiveness.

#### UI Authoritative data owners

AutoTrader, Position Management, Paper Exchange and Insights.

#### UI Initial v35 mapping

WP6, WP7, WP8, WP10 and WP14.

#### UI Functional verification

Investigate current unfilled-order behaviour.

Verify matching, passive pricing, revalidation and cancellation against the intended v35 behaviour.

Do not relax trading requirements or force trades to obtain execution evidence.

### UI 9. Widget 5 — Portfolio and Reconciliation

#### UI Description

The authoritative financial overview, showing portfolio performance alongside the completeness and trustworthiness of financial evidence.

#### UI Operator questions

- How much capital do I have?
- How much is available, reserved or deployed?
- Am I making money after costs?
- What is my drawdown?
- Are financial outcomes independently reconciled?
- Is financial evidence missing?

#### UI 9.1 Summary metrics

- Opening capital.
- Current cash by currency.
- Current equity.
- Available trading capital.
- Reserved capital.
- Deployed capital.
- Capital utilisation.
- Gross realised P&L.
- Net realised P&L.
- Unrealised P&L.
- Paid fees.
- Estimated exit fees.
- Contributions and withdrawals.
- Current drawdown.
- Maximum drawdown.

#### UI 9.2 Historical visualisations

- Equity curve.
- Cumulative net P&L.
- Daily and weekly returns.
- Drawdown.
- Capital utilisation.
- Fees over time.
- Cash and position exposure.

Returns must account for contributions and withdrawals using the authoritative financial contract.

Do not subtract fees twice.

#### UI 9.3 Independent reconciliation

Display:

- Financial evidence epoch.
- Source and ledger cursors.
- Snapshot timestamps.
- Independent execution coverage.
- Fee coverage.
- Cash-movement coverage.
- Contribution coverage.
- FX coverage and freshness.
- Complete and incomplete intervals.
- Matched and unmatched records.
- Reconciliation mismatches.
- Missing evidence reasons.
- Source attribution and provenance limitations.

Support drill-down into affected source records and trades.

#### UI 9.4 Evidence states

Clearly distinguish:

- Complete.
- Provisional.
- Not evaluated.
- Incomplete.
- Stale.
- Unavailable.

Do not represent missing values as zero.

Do not represent matching flat-cash intervals as proof of trade profitability.

Do not represent match probes as settled executions.

Do not imply independent reconciliation merely because two projections of the same ledger agree.

#### UI Authoritative data owners

- Position Management: portfolio ledger and financial snapshots.
- Paper Exchange: independent capital, settlement, fee, cash and FX evidence.
- Existing reconciliation tooling: independent comparison.

#### UI Initial v35 mapping

WP2, WP3, WP5, WP6, WP9, WP10 and WP14.

#### UI Functional verification

Verify displayed financial values against authoritative sources.

Verify independent financial reconstruction and completeness semantics.

Do not implement duplicate financial accounting in React.

### UI 10. Widget 6 — Capital Allocation

#### UI Description

A read-only explanation of how the autonomous trading system allocates its shared capital pool across competing opportunities.

This replaces the legacy manual strategy and horizon allocation interface.

#### UI Operator questions

- How much capital is available?
- How much is reserved or deployed?
- Which strategies and symbols receive capital?
- Why is capital allocated to them?
- Why is capital unused?
- Which constraints limit allocation?

#### UI 10.1 Global summary

- Total capital.
- Eligible shared trading pool.
- Available capital.
- Reserved capital.
- Deployed capital.
- Remaining daily notional allowance.
- Exploration allowance.
- Exploration usage.
- Capital constrained by risk limits.

Distinguish stock measures such as available capital from flow limits such as daily traded notional.

#### UI 10.2 Allocation breakdown

Support grouping by:

- Strategy.
- Symbol.
- Symbol/strategy combination.
- Horizon.
- Direction.
- Strict/exploratory mode.

Display:

- Effective allocation.
- Used capital.
- Reserved capital.
- Available capital where attributable.
- Current eligible opportunities.
- Evidence eligibility.
- Validated expected net edge.
- Directional confidence.
- Allocation reason.
- Binding constraints.
- Calculation timestamp.

Do not imply that the shared available pool is separately reserved for every strategy.

Where allocations are opportunity-specific rather than persistent budgets, display them accordingly.

#### UI 10.3 Visualisations

- Shared capital pool utilisation.
- Allocation distribution by strategy.
- Allocation distribution by symbol.
- Allocation distribution by horizon.
- Allocation history.
- Unused capital over time.
- Capital constrained by different restrictions.

#### UI 10.4 Required behaviour

- No editable strategy budgets.
- No editable horizon allocation maps.
- No obsolete allocation repair warnings.
- No stale legacy allocation maps transmitted by the UI.
- No implicit disabling through obsolete zero allocations.
- No double counting across overlapping allocation dimensions.
- Global risk limits remain configurable in AutoTrader Controls.

#### UI Authoritative data owner

AutoTrader shared allocator.

#### UI Initial v35 mapping

WP5, WP6, WP9 and WP10.

Reuse existing Beads tasks:

- `sbe_ts-gu78.28`: backend shared allocator contract.
- `sbe_ts-gu78.29`: read-only allocation UI.

Verify their current status before implementation.

#### UI Functional verification

Confirm:

- Capital is allocated across the whole eligible shared pool.
- Allocation is not constrained by obsolete manual budgets.
- Capital conservation is maintained.
- Concurrent reservations cannot double-allocate capital.
- Cold-start exploration remains appropriately bounded.
- Displayed allocation reasons reflect actual decisions.

The offline shared allocator and read-only allocation view are implemented and deterministically tested under the linked Beads tasks. Keep deployment/runtime activation and empirical allocation evaluation as separate gates; this does not block the other widgets.

### UI 11. Widget 7 — Strategy Performance

#### UI Description

Evaluate which strategies are producing sustainable, cost-adjusted returns, and under which market conditions.

This is the primary comparative performance widget.

#### UI Operator questions

- Which strategies are profitable after fees?
- Which symbols work best for each strategy?
- Which strategies consume capital without producing returns?
- Are strict trades outperforming exploratory trades?
- Which profiles, directions and horizons perform best?
- Is performance improving?
- Is the evidence sufficient to support conclusions?

#### UI 11.1 Summary metrics

- Reconciled trade count.
- Raw sample count.
- Effective sample count.
- Gross P&L.
- Net P&L.
- Net return.
- Win rate.
- Average net win.
- Average net loss.
- Win/loss payoff ratio.
- Profit factor.
- Net expectancy.
- Return uncertainty.
- Drawdown.
- Average holding duration.
- Capital efficiency.
- Fees.
- Execution costs.
- Evidence freshness.

Define all financial denominators and cost bases.

Do not present unsupported ratios where denominators are zero or evidence is incomplete.

#### UI 11.2 Comparison dimensions

- Strategy.
- Symbol.
- Symbol/strategy combination.
- Profile.
- Profile version.
- Direction.
- Horizon.
- Market regime.
- Strict versus exploratory selection mode.

Allow side-by-side comparison of equivalent cohorts.

#### UI 11.3 Strategy performance matrix

Provide sortable strategy-level and symbol/strategy-level tables.

Include:

- Trade count.
- Effective sample count.
- Net P&L.
- Net return.
- Expectancy.
- Win rate.
- Average win/loss.
- Profit factor.
- Drawdown.
- Fees.
- Capital utilisation.
- Evidence status.

Include inactive strategies rather than hiding them.

#### UI 11.4 Visualisations

- Cumulative net performance by strategy.
- Net expectancy comparisons.
- Win/loss distributions.
- Fee-adjusted performance.
- Drawdown comparisons.
- Holding duration versus net return.
- Capital deployed versus net return.
- Performance by market regime.
- Strict versus exploratory performance.
- Symbol/strategy performance heatmap.

#### UI 11.5 Strategy drill-down

Selecting a strategy should expose:

- Individual symbols.
- Directions.
- Active profiles.
- Profile versions.
- Relevant parameters.
- Historical performance.
- Closed trades.
- Execution quality.
- Capital allocation.
- Decision funnel.
- Learning evidence.
- Promotion blockers.

#### UI 11.6 Evidence requirements

- Compare equivalent time windows and cost bases.
- Separate Scout outcomes from actual paper trades.
- Exclude unreconciled or ineligible results from validated performance.
- Display excluded counts and reasons.
- Distinguish descriptive statistics from statistically supported conclusions.
- Show uncertainty and effective sample limitations.
- Do not rank strategies using insufficient evidence.
- Do not infer promotion readiness from profitability alone.

Display observed execution counts even when profitability evidence is insufficient.

#### UI Authoritative data owners

AutoTrader, Position Management, Paper Exchange and Learning.

#### UI Initial v35 mapping

WP1, WP2, WP3, WP5, WP6, WP10–WP15.

#### UI Functional verification

Verify outcome identity, cohort attribution, fee-adjusted returns, effective sample calculations and evidence eligibility.

Identify missing or incorrectly implemented strategy evaluation requirements.

### UI 12. Widget 8 — Learning and Evidence Acquisition

#### UI Description

Explain how Learning is acquiring evidence, resolving uncertainty and improving trading decisions.

The widget must distinguish productive Learning from activity that does not improve eligibility or decision quality.

Preserve the authoritative Section 13 Evidence Coverage requirements.

#### UI Operator questions

- What is Learning currently doing?
- What evidence is missing?
- Why is a strategy or profile not eligible?
- Is Learning actively addressing the gap?
- Is acquisition producing useful evidence?
- Are profiles approaching promotion?
- Is Learning stalled?
- Has Learning improved actual trading behaviour?

#### UI 12.1 Learning progression summary

Display:

- Strategies and profiles monitored.
- Evidence gaps identified.
- Evidence requests queued.
- Evidence acquisition underway.
- Requests waiting for market conditions.
- Capacity-blocked requests.
- Stalled requests.
- Failed requests.
- Completed acquisition work.
- Evidence accepted by the evaluator.
- Profiles meeting evidence requirements.
- Profiles eligible for promotion.
- Actual promotions.
- Rejected or demoted profiles where supported.

Do not assume these states form mutually exclusive sequential populations unless the authoritative contract establishes that.

#### UI 12.2 Evidence coverage matrix

Group by:

- Strategy.
- Profile.
- Symbol.
- Direction.
- Market regime.
- Horizon.

Display:

- Current raw samples.
- Required raw samples.
- Current effective samples.
- Required effective samples.
- Current and required observation windows.
- Confidence.
- Freshness.
- Missing evidence ranges.
- Evidence gap reason.
- Blocked trading or promotion decision.
- Acquisition priority.
- Current request state.
- Last accepted evidence gain.
- Last worker heartbeat.
- Last evaluation.
- Next evaluation.
- Recovery and retry status.

#### UI 12.3 Evidence acquisition drill-down

For each acquisition request, show:

- Reason for the request.
- Strategy and profile.
- Target evidence gap.
- Requested data range.
- Acquisition method.
- Assigned worker.
- Current state.
- Start time.
- Last progress.
- Expected evaluation condition.
- Accepted evidence gained.
- Remaining gap.
- Blocker.
- Recovery action.

Distinguish:

- Waiting for market conditions.
- Waiting for capacity.
- Active acquisition.
- Stale or stalled activity.
- Failed acquisition.
- Completed work.
- Excluded evidence.
- Accepted evidence.

#### UI 12.4 Learning improvement

Show actual accepted improvements where supported:

- Evidence gaps resolved.
- Effective sample gains.
- Increased confidence.
- Validated profile improvements.
- Promotion eligibility changes.
- Actual promotions.
- Strict trading activation following validated promotion.
- Subsequent reconciled trading performance.

Do not equate Scout count, worker activity or completed jobs with successful Learning.

#### UI 12.5 Historical visualisations

- Evidence coverage growth.
- Effective sample growth.
- Accepted evidence gains.
- Evidence acquisition cost.
- Acquisition completion rates.
- Evidence gaps resolved.
- Promotion eligibility progression.
- Actual promotion history.
- Evidence gain relative to exploration cost.

#### UI 12.6 Scout research access

Preserve optional access to existing:

- Scout positions.
- Scout outcomes.
- Scout activity.
- Scout replay history.
- Walk-forward results.

Consolidate decision-relevant information into the Learning widget.

Remove redundant Scout widgets from the default operator workspace, but preserve convenient research and diagnostic access.

Do not expand unrelated research functionality during v35.

#### UI Authoritative data owner

Learning, supported by Strategy, Insights, AutoTrader and financial outcome sources.

#### UI Initial v35 mapping

WP1, WP3, WP4, WP11, WP13, WP14 and WP15.

Preserve the canonical Section 13 contract and existing implementation associated with `sbe_ts-15ok`.

#### UI Functional verification

Verify:

- Evidence gaps correspond to actual blocked decisions.
- Acquisition requests are actionable.
- Worker progress is genuine.
- Accepted evidence gains are correctly attributed.
- Raw samples are not confused with effective samples.
- Promotion gates remain enforced.
- Scout-only evidence is not presented as actual trading profitability.
- Restart recovery does not duplicate or incorrectly accept evidence.

### UI 13. Shared Filters and Cross-Widget Navigation

#### UI 13.1 Shared filters

Provide consistent filters across applicable widgets:

- Time period.
- Symbol.
- Strategy.
- Profile.
- Direction.
- Horizon.
- Market regime.
- Strict/exploratory mode.
- Evidence epoch.

Support dashboard-wide filters and explicit local widget overrides.

Clearly indicate when a widget does not support a particular dimension.

#### UI 13.2 Navigation

Use stable identities to navigate between:

- Worker evaluation.
- Trading decision.
- Entry gate.
- Candidate.
- Order.
- Fill.
- Position.
- Closed trade.
- Financial reconciliation.
- Learning outcome.
- Evidence acquisition request.

Preserve applicable filter context.

Examples:

- Select a symbol/strategy funnel cell to inspect rejected decisions.
- Select a submitted order to inspect its execution quality.
- Select a closed position to inspect its original decision and financial reconciliation.
- Select a strategy to inspect its capital allocation and evidence gaps.
- Select an evidence gap to inspect the affected trading decisions.

Do not require users to manually copy IDs between widgets.

### UI 14. Grafana Responsibilities

Keep technical infrastructure monitoring in Grafana rather than duplicating it in React.

#### UI 14.1 Service monitoring

- Service and container health.
- CPU and memory.
- Queue depth.
- Dropped events.
- Database performance.
- RSocket reliability.
- Request latency.
- Trading pipeline latency.
- Emergency execution latency.
- Resource saturation.

#### UI 14.2 Per-symbol feed and worker reliability

Display:

- Active book subscriptions.
- Latest book timestamp.
- Latest trade timestamp.
- Data age.
- Invalid events.
- Data gaps.
- Reconnects.
- Worker heartbeat.
- Latest worker decision.
- Worker errors.
- Evidence freshness.

Do not infer per-symbol freshness from global feed activity.

#### UI 14.3 Deployment traceability

Display:

- Service and container identity.
- Deployed image digest.
- Source revision.
- Build time.
- Dirty or unknown build status.
- Configuration version.
- Schema version.
- Runtime identity.
- Restart time.
- Attestation status.

Display unknown or unattested provenance explicitly.

Do not infer deployed source identity from image tags.

#### UI 14.4 React boundary

Trading-specific data freshness, decision blockers, safety state and evidence completeness must remain visible in React.

Grafana is not a substitute for operational trading diagnostics.

### UI 15. Existing Widget Disposition

Review the actual widget catalog and rendering implementation before migration.

Initial intended disposition:

| Existing widgets | Disposition |
|---|---|
| Auto Trader | Keep and narrow to global controls |
| Auto Trader Diagnostics | Extend into Decision Funnel Diagnostics |
| Orders, Fills, Live Positions, Completed Positions | Consolidate into Orders and Positions |
| Account, Risk, P&L, Execution Fees | Consolidate into Portfolio and Reconciliation |
| Legacy strategy allocation controls | Remove after backend dependency resolution |
| AutoTrader Outcomes, Strategy Performance, Learning Realised | Consolidate into Strategy Performance |
| Learning Decisions, Learning Scouts, Scout Activity | Consolidate decision-relevant content into Learning |
| Execution Check and passive-order evidence | Consolidate into Execution Quality |
| Scout Positions and Outcomes | Preserve optional research access |
| Scout Replay History and Walk-forward Data | Preserve optional research access |
| Signal replay and research diagnostics | Remove from default operator workspace |
| General market widgets | Preserve outside this plan |
| Portfolio hedging and dated futures carry | Preserve existing controls and expose committed exposure in shared Capital Allocation wherever it consumes the global pool; no dedicated widget unless required |

Retirement means removal from the default operator workspace or replacement of redundant widgets.

Do not unnecessarily delete shared components, research capabilities or historical data.

#### UI Saved-view preservation requirements — no migration

- Do not migrate, rewrite, reorder or auto-replace existing saved views/layouts.
- Preserve saved widget types, positions, ordering, filters and configuration.
- When an obsolete widget type is removed from the add-widget catalogue, preserve a generic non-destructive unavailable-widget placeholder (or minimal compatibility renderer) at its original location until the operator manually replaces/removes it.
- Verify backend and non-UI consumer retirement before removing obsolete handlers, subscriptions, workers, caches or storage.
- Test loading and saving old saved views to prove their content remains intact without automatic migration.
- Avoid destructive changes to user layouts.

### UI 16. UI Design and Interaction Principles

Use:

- Compact summary metrics.
- Historical charts.
- Sortable tables.
- Symbol/strategy heatmaps.
- Expandable rows.
- Related-content tabs.
- Clear drill-down interactions.
- Consistent financial units and precision.
- Consistent timestamps.
- Consistent colour semantics.

Avoid:

- Overloaded diagnostic tables.
- Displaying every backend field simultaneously.
- Multiple widgets answering the same question.
- Unnecessary modal dialogs.
- Excessive visual clutter.
- Duplicate financial calculations.
- Misleading success indicators.

Every widget must support explicit:

- Loading state.
- Empty state.
- Zero activity.
- Stale data.
- Unavailable data.
- Incomplete evidence.
- Truncated population.
- Error state.
- Insufficient statistical evidence.

Do not substitute zero for unknown.

Use clear visual distinctions between confirmed success, pending validation, warnings and failures.

### UI 17. Historical Data and Metric Contracts

Before implementing charts or historical comparisons, establish:

- Authoritative source.
- Available retention period.
- Sampling frequency.
- Aggregation rules.
- Timestamp semantics.
- Identity and deduplication.
- Population coverage.
- Missing interval handling.
- Financial cost basis.
- Evidence epoch.
- Metric version.

Do not combine incompatible historical epochs or profiles without explicit cohort handling.

Do not silently reconstruct missing historical data.

Reuse existing authoritative history wherever possible.

Where new history capture is required, assess whether it is necessary for existing v35 acceptance or an optional next-wave enhancement.

### UI 18. Performance and Resource Requirements

The UI and its diagnostics must not interfere with trading or emergency execution.

Implement:

- Bounded queries.
- Server-side pagination.
- Server-side historical aggregation.
- Bounded chart sample counts.
- Shared subscriptions.
- Reuse of authoritative snapshots.
- Request deduplication.
- Cancellation of obsolete requests.
- Suspension of unnecessary hidden-widget requests.
- Virtualisation of large tables where appropriate.
- Explicit coverage and truncation.
- Bounded diagnostic concurrency.

Avoid:

- Unlimited historical downloads.
- Repeated full-ledger scans.
- Independent polling of the same data by multiple widgets.
- Expensive frontend aggregation of raw event history.
- Unbounded diagnostic memory growth.
- Diagnostic work blocking emergency exits.

Do not introduce synchronous diagnostic persistence into the trading hot path without validating its impact.

Preserve all existing v35 performance acceptance thresholds.

Test representative dashboard workloads alongside trading activity where possible.

### UI 19. Implementation Phases

#### UI Phase 1 — Independent review and contract inventory

- Read authoritative v35 requirements.
- Review all eight widget specifications.
- Inspect existing UI components.
- Identify authoritative data owners.
- Identify available and missing contracts.
- Review existing Beads tasks.
- Create the requirements traceability matrix.
- Classify discovered gaps.
- Establish file ownership.
- Coordinate with the existing v35 coordinator.
- Separate existing requirements from optional enhancements.

Keep this review bounded. Begin implementation once dependencies are understood.

#### UI Phase 2 — Autonomous capital allocation

Priority: Critical.

- Verify the shared allocator contract.
- Resolve obsolete manual allocation dependencies.
- Update AutoTrader Controls and Safety.
- Remove manual strategy and horizon controls.
- Implement read-only Capital Allocation.
- Verify capital conservation and concurrent reservations.
- Verify effective runtime settings.

Reuse existing allocator Beads tasks.

#### UI Phase 3 — Decision Funnel Diagnostics

Priority: Critical.

- Extend existing diagnostics rather than create a duplicate widget.
- Implement per-symbol funnels.
- Implement per-strategy funnels.
- Implement combined symbol/strategy funnels.
- Implement the symbol/strategy heatmap.
- Add strict/exploratory eligibility diagnostics.
- Add gate-level drill-down.
- Add historical aggregation.
- Verify stage populations and denominators.
- Validate against actual runtime decisions.

Identify and coordinate underlying v35 defects.

#### UI Phase 4 — Execution Quality

Priority: Critical.

- Implement order lifecycle and terminal-reason analysis.
- Add passive-order effectiveness.
- Add price competitiveness.
- Add market-context coverage.
- Add time-to-fill metrics.
- Investigate current unfilled-order behaviour.
- Add historical execution metrics.
- Verify matching and revalidation contracts.

Do not force executions or relax risk controls.

#### UI Phase 5 — Orders, Positions and Portfolio

Priority: High.

- Consolidate Orders and Positions.
- Add protective-exit visibility.
- Add complete lifecycle attribution.
- Consolidate Portfolio and Reconciliation.
- Add financial completeness and mismatch presentation.
- Add equity, P&L, drawdown and utilisation charts where supported.
- Verify authoritative financial values.

#### UI Phase 6 — Strategy Performance and Learning

Priority: High.

- Implement strategy and symbol/strategy comparisons.
- Add cost-adjusted performance metrics.
- Add profile and cohort drill-down.
- Extend Evidence Coverage.
- Add Learning progression.
- Add accepted evidence gain analysis.
- Preserve Scout research access.
- Verify promotion and evidence gates.

#### UI Phase 7 — Dashboard integration and Grafana

Priority: High.

- Implement shared filters.
- Implement cross-widget navigation.
- Deliver default dashboard presets.
- Verify independent widget configuration.
- Complete required Grafana reliability views.
- Complete deployment traceability presentation.
- Verify diagnostic workload isolation.

#### UI Phase 8 — Consolidation and retirement

Priority: Medium.

- Verify replacement widgets in the rendered browser.
- Migrate persisted layouts.
- Retire redundant operator widgets.
- Preserve optional research and shared market views.
- Verify all required operational workflows remain possible.

#### UI Phase 9 — Independent v35 acceptance review

- Review the traceability matrix.
- Reassess every affected v35 work package.
- Identify unresolved functional defects.
- Identify unresolved master plan omissions.
- Identify outstanding runtime and empirical evidence.
- Verify Beads status against actual evidence.
- Update `plan.md`.
- Produce the final UI implementation and functional review report.

### UI 20. Parallel Implementation Policy

The existing v35 coordinator should delegate independent work while actively implementing and integrating.

Suggested allocation:

- React specialist: widget presentation, layouts, charts, interactions and frontend tests.
- Backend contract specialist: diagnostics, API completeness, authoritative source verification and missing contracts.
- existing v35 coordinator: traceability, integration, shared navigation, cross-system verification and implementation.

Coordinate material trading-system changes with the existing v35 coordinator.

Use clear ownership of shared files and contracts.

Do not allow both coordinators to independently implement conflicting changes to AutoTrader, Position Management, Strategy, Learning or Paper Exchange.

Use Beads to record dependencies and handoffs.

Avoid unnecessary waiting where independent frontend work can proceed against validated contracts and deterministic fixtures.

### UI 21. Testing and Acceptance Criteria

#### UI 21.1 Widget functionality

- All eight widgets operate independently.
- Default dashboard presets work.
- Shared and local filters behave consistently.
- Cross-widget navigation preserves identity and context.
- Existing saved dashboard views/layouts are not migrated or rewritten; type, ordering, position, filters and configuration remain intact. Retired widget types render a non-destructive unavailable state until the operator manually changes the saved view.
- Redundant manual allocation controls are removed.
- Optional research functionality remains accessible.

#### UI 21.2 Decision diagnostics

- Complete funnels are available per symbol.
- Complete funnels are available per strategy.
- Complete funnels are available per symbol/strategy combination.
- Symbol/strategy heatmaps use authoritative counts.
- Stage counts reconcile within defined populations.
- Retries do not cause double counting.
- Partial fills are handled correctly.
- Asynchronous transitions are correctly attributed.
- Strict/exploratory eligibility is explicit.
- Zero-activity strategies remain visible.
- Gate drill-down identifies actual observed and required values.

#### UI 21.3 Mandatory per-widget completion matrix

The coordinator maintains a row for each of the eight operational widgets plus Excursions & Exit Analysis, with independent evidence/status for: implementation; backend data availability; runtime verification; PNG/HTML visual fidelity; scrolling and responsive behavior; interaction testing; legacy replacement; and operator approval. Link compact screenshots, runtime/source traces, test evidence and Beads issues from each row. `Pending`, `blocked` and `unverified` are valid recorded states; they are not completion. A test pass or unavailable placeholder cannot promote a cell to accepted. Preserve the distinction between synthetic fixtures, surfaced non-empirical records and naturally occurring Paper lifecycle evidence.
- Missing evidence is not misrepresented as zero activity.

#### UI 21.3 Capital allocation

- Legacy manual allocations do not constrain eligible opportunities.
- Shared capital is correctly represented.
- Global risk controls remain effective.
- Concurrent reservations cannot double-allocate capital.
- Allocation reasons originate from actual decisions.
- Stock and flow limits are distinguished.

#### UI 21.4 Execution and lifecycle

- Order status matches authoritative lifecycle state.
- Partial fills and terminal outcomes are correctly represented.
- Protective exits and interrupted barriers are visible.
- Cancellation and rejection reasons are attributable.
- Execution metrics use defined denominators.
- Counterfactual estimates are clearly separated from realised outcomes.

#### UI 21.5 Financial integrity

- Displayed financial values match authoritative sources.
- Fees are not double counted.
- Independent reconciliation status is accurate.
- Incomplete evidence remains visible.
- Unreconciled trades do not contaminate validated performance.
- Match probes are not represented as settled executions.
- Contributions and withdrawals are correctly reflected in return calculations.

#### UI 21.6 Learning

- Scout outcomes are separated from actual paper trades.
- Evidence acquisition activity is distinguished from accepted evidence gains.
- Evidence gap and blocker information is accurate.
- Effective samples are distinguished from raw samples.
- Promotion eligibility is not inferred from activity or raw counts.
- Existing promotion gates remain enforced.

#### UI 21.7 Deterministic fixtures

Test:

- Populated data.
- Empty data.
- Zero activity.
- Stale data.
- Unavailable data.
- Truncated populations.
- Mixed strict/exploratory eligibility.
- Repeated events and retries.
- Partial fills.
- Asynchronous lifecycle transitions.
- Pending orders.
- Interrupted protective barriers.
- Reconciliation mismatches.
- Missing fee or FX evidence.
- Multiple evidence epochs.
- Zero-activity strategies.
- Saved-view round-trip preservation without migration.

#### UI 21.8 Frontend verification

Run:

- Focused React component tests.
- Integration tests.
- Typechecking.
- Rendered browser verification.
- Layout and resizing verification.
- Filter and navigation verification.
- Empty and error-state verification.
- Bounded query and subscription verification.

Do not declare the UI complete based solely on compilation or unit tests.

#### UI 21.9 Backend verification

Run relevant focused regression tests for any modified authoritative contracts.

Verify that observability changes do not alter trading behaviour.

Where a functional defect is fixed, add regression coverage demonstrating the original defect and corrected behaviour.

#### UI 21.10 Runtime verification

Where genuine runtime evidence is available, compare rendered values against authoritative service responses and financial evidence.

Do not invent trades or force execution to populate widgets.

UI acceptance can validate correct empty, incomplete and insufficient-evidence states.

However, UI acceptance does not establish that the underlying v35 runtime or empirical acceptance requirements have been met.

### UI 22. Current Runtime Evidence and Immediate Priorities

Review the latest runtime status rather than assuming earlier snapshots remain current. As of 2026-11-26 16:31 UTC, read-only APIs report enabled paper AutoTrader with healthy audit, $1,000 daily pool available, 137 Insights decision samples, 38 exploratory submitted orders, 37 terminal unfilled expiries and one post-only crossing rejection; there are zero fills, positions, trades, fees or barriers. In the retained 69 AutoTrader cycle reports (11:16–15:57 UTC), 231 fresh directional rows led to 143 selected attempts, 38 submissions and 105 execution-blocked outcomes; 29 cycles reported duplicate exploratory source-event reservations and two reported audit persistence unavailable. These counts are not strict-eligibility denominators. See plan.md and Beads sbe_ts-7eeb for source/evidence references. No trade has been forced and no thresholds or risk controls changed.

Known issues from the preceding v35 review include:

- Exploratory orders were submitted but remained unfilled.
- No completed end-to-end paper trade had been demonstrated.
- Strict eligibility was not explicitly exposed in candidate diagnostics.
- Missing fresh local books blocked some opportunities.
- Duplicate reserved events were among observed blockers.
- Some worker streams experienced recurring idle/reconnect messages.
- Independent financial infrastructure existed but genuine settled-execution reconciliation remained unverified.
- Scout outcomes existed without actual trade Learning samples.
- Legacy manual strategy allocation controls remained visible.
- Some deployed artifacts lacked exact source revision attestation.

These are investigation priorities, not assumptions that the issues still exist.

Verify current status before implementing fixes.

Current Critical delivery order, with the lifecycle and UI workstreams progressing in parallel:

1. Decision Funnel Diagnostics: complete per-symbol, per-strategy and symbol/strategy explanations, including gates and observed/required values.
2. Execution Quality: reservation lifecycle, unfilled orders, cancellations, execution prices, revalidation and matching evidence.
3. Autonomous Capital Allocation: shared-pool use, allocation decisions, binding constraints and removal of legacy dependencies; offline WP5/WP6 code is complete, runtime activation remains pending.
4. Learning and Evidence Acquisition: strict eligibility blockers, evidence gaps, acquisition progress and accepted gains.
5. AutoTrader Controls and Safety: effective configuration, global risk restrictions and safety states.

The Critical backend objective remains a natural end-to-end paper trade through reservation, matching, protection, close, independent financial reconciliation and Learning. UI implementation and honest empty/unknown states do not wait for a trade; v35 runtime and empirical acceptance still require genuine evidence.

### UI 23. Completion Reporting

Maintain `plan.md` and Beads throughout implementation.

At completion, produce a report containing:

#### UI Widget delivery

For each of the eight widgets:

- Implementation status.
- Existing components reused.
- New components.
- Authoritative contracts.
- Supported functionality.
- Missing functionality.
- Tests completed.
- Browser verification.
- Runtime verification.
- Outstanding limitations.

#### UI v35 functional review

For every affected work package:

- Original requirement.
- Existing implementation status.
- Gaps discovered.
- Defects corrected.
- Evidence collected.
- Remaining acceptance blockers.
- Associated Beads tasks.

#### UI Master plan review

List:

- Genuine omissions discovered.
- Ambiguities or contradictions.
- Proposed amendments.
- Approved amendments.
- Unresolved decisions.

#### UI Runtime and empirical evidence

Separately identify:

- Functional implementation complete.
- Deterministically verified.
- Deployed.
- Runtime verified.
- Financially reconciled.
- Empirically validated.
- Performance accepted.

Do not collapse these into a single completion status.

#### UI Next-wave candidates

Record genuinely new capabilities and optional analytical enhancements that are not necessary for v35 completion.

Do not automatically implement them.

### UI 24. Non-Negotiable Delivery Constraints

- Keep v35 authoritative.
- Preserve paper-only execution.
- Preserve existing risk controls.
- Preserve strict entry and evidence gates.
- Preserve independent financial reconciliation requirements.
- Preserve Learning promotion restrictions.
- Preserve performance acceptance thresholds.
- Keep WP16 deferred.
- Do not introduce new trading strategies as part of UI work.
- Do not force trades.
- Do not reset the current financial evidence epoch.
- Do not fabricate missing data.
- Do not duplicate authoritative business logic in React.
- Do not reopen general tooling optimisation.
- Reuse existing Beads tasks where possible.
- Coordinate shared backend changes.
- Maintain clear file ownership.
- Do not commit or deploy without approval.

Complete existing v35 operational visibility and functional requirements first.

Optional enhancements requiring substantial new infrastructure should be recorded for the next improvement wave rather than indefinitely extending v35.

### UI 25. Definition of Done

The UI plan is complete when:

- All eight required operator widgets satisfy their applicable requirements.
- The dashboard presets and custom layouts operate correctly.
- Legacy manual allocation controls and obsolete dependencies are resolved.
- Complete decision funnels are available per symbol, strategy and symbol/strategy combination.
- The operator can trace decisions through execution, financial reconciliation and Learning.
- Financial and evidence limitations are presented accurately.
- Redundant widgets have been safely consolidated.
- Relevant research access remains available.
- Required Grafana monitoring is implemented.
- Source contracts and rendered behaviour have been verified.
- All discovered v35 gaps are resolved or explicitly tracked with dependencies and acceptance criteria.
- Proposed master plan amendments are documented and approved where necessary.
- `plan.md`, Beads and the authoritative v35 master plan remain consistent.

UI completion must not automatically imply v35 completion.

The overall v35 coordinator retains responsibility for declaring v35 accepted only when its required functional, financial, runtime, quantitative and performance gates have been satisfied.

Begin with the bounded independent review, establish traceability and proceed directly into implementation using the existing specialist agent configuration.
