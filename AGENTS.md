# Project Guidance

## 1. Architecture and Scope

- React: `src/typescript/workspace/apps/react-app`; shared UI: `src/typescript/workspace/packages/ui-platform`.
- Rust services: `src/rust/` (`feed_handlers`, `insights`, `auto_trader`, `strategy`, `learning`, `position_management`, `paper_exchange`).
- Signal flow: `feed_handlers` → `insights` → `auto_trader` via `StrategySignal`; evaluators consume signals and position management handles execution.
- Scouts: `src/rust/strategy/src/scouts/`, not a standalone crate.
- Keep business logic in Rust; frontend models manage state and views render UI.
- Strategy entry/open-position decisions belong in `evaluate.rs`; preserve learned bounds and feedback attribution.
- Avoid editing generated SBE codecs unless protocols change. Update trade-ticket and Orders flows together for VWAP API changes.
- Reuse existing paper execution, scouting, learning, risk and execution infrastructure. Do not introduce duplicate systems.
- Stay within the requested scope. Make the smallest correct change and stop once verified.
- Preserve existing unrelated working-tree changes.
- Do not change live trading settings, deploy services or modify runtime databases without explicit authorisation.

## 2. Mandatory Tooling-First Workflow

Use existing deterministic tooling before manual exploration, data extraction, calculation or repeated model reasoning.

For substantial implementation tasks, the coordinator runs:

`python3 scripts/local/prepare.py --mode quick`

Review repository drift, affected work packages, diagnostic impact, contract candidates, evidence freshness and blockers.

- Reuse valid preparation reports; do not repeat preparation in every child agent.
- Use `--mode full` when fresh financial or integration evidence is required. Inspect individual results; full mode does not refresh every diagnostic.
- Use `--refresh-bundles` when relevant work-package inputs change.
- Inspect nested results. Successful process exit does not establish acceptance.
- Use `--help` to establish unfamiliar script interfaces instead of reading entire implementations.
- Do not recreate existing diagnostic capabilities in temporary scripts.
- Do not run every available diagnostic by default. Select tools according to the task and its acceptance requirements.
- Do not extend diagnostic tooling during implementation unless a demonstrated blocker requires it.

### Tool Selection

| Task | Preferred tooling |
|---|---|
| Repository preparation | `scripts/local/prepare.py` |
| Repository drift | `scripts/local/freshness_check.py` |
| Known Rust symbol investigation | `scripts/agent/efficiency/symbol_report.py` |
| Targeted definition and caller extraction | `scripts/agent/efficiency/symbol_lookup.py` |
| Repository and dependency discovery | Existing `scripts/local/` repository utilities |
| Discover existing diagnostic tools | `scripts/agent/efficiency/tool_catalog.py` |
| Work-package generation | `scripts/local/generate_work_packages.py` |
| Bundle verification | `scripts/local/bundle_verify.py` |
| Financial snapshot | `scripts/local/financial_snapshot.py` |
| Financial health and reconciliation | `scripts/local/financial_health.py`, `financial_reconciliation.py` |
| Runtime and learning analysis | Existing `scripts/local/` utilities |
| Database discovery and queries | `scripts/db/` |
| RSocket and API investigation | `scripts/api/` |
| Trading and quantitative diagnostics | `scripts/diagnostics/` |
| Regression selection and execution | `scripts/local/regression_runner.py` |
| Regression evidence reuse | `scripts/agent/efficiency/regression_identity.py` |
| Performance threshold evaluation | `scripts/local/performance_gate.py` |
| Bounded command execution | `scripts/agent/efficiency/bounded_run.py` |
| Context and token analysis | `scripts/agent/`, `.codex/session_review.py` |

Read relevant README files or `scripts/diagnostics/CODEX_INSTRUCTIONS.md` when unfamiliar with a tool. Do not repeatedly reload documentation already understood.

Evidence is stored under `~/.cache/sbe-diagnostics/`. Keep raw evidence on disk and consume compact summaries and explicit references.

## 3. Context, Navigation and Token Efficiency

### Context Management

- Treat context as a limited resource. Avoid accumulating unrelated tasks in long-running sessions.
- Prefer fresh specialist sessions and bounded work packages.
- Monitor context at major task boundaries; compact or hand off before excessive accumulation.
- Before compaction, persist verified reusable discoveries with `bd remember` and record task progress, evidence references, blockers and remaining work.
- After compaction, restore only essential context.
- Never reload entire plans, large logs, completed investigations or unrelated work packages without justification.
- Prefer compact structured handoffs over transferring complete conversation histories.
- Avoid repeating discovery performed by the coordinator or another specialist.
- Reuse confirmed evidence and source locations supplied in handoffs.
- Do not accumulate large tool outputs merely because the model supports a large context window.
- Complete bounded tasks and close specialist sessions when their work is finished.

Advisory context budgets:

| Role | Target compaction threshold | Target maximum context |
|---|---:|---:|
| Coordinator | 80K | 131K |
| Architect / Quantitative / Financial | 60K | 98K |
| Rust / Integration | 40K | 65K |
| React | 30K | 49K |
| Explorer / Documentation | 20–30K | 32–49K |

These are operational targets, not verified runtime limits. Respect actual model capabilities and effective agent configuration. Adjust for initial context overhead and task complexity. Do not sacrifice correctness or mandatory invariants to meet a token target.

### Repository Navigation

- Use memories injected by `bd prime` before searching. Query `bd memories <topic>` only when additional detail is needed.
- Consult available architectural memories for AutoTrader APIs, entry decisions, learning, scouting, strategy execution, React outcomes and runtime infrastructure.
- Save verified reusable paths, symbols, contracts and architectural relationships with `bd remember`. Avoid duplicate memories.
- Prefer existing source maps, symbol maps and repository indexes before broad searches.
- Search likely crates or directories first. Never run unrestricted root searches for common identifiers, ports, timestamps or numbers.
- Discover uncertain paths using `rg --files`; use targeted `rg`, `rg -l`, ast-grep and bounded excerpts.
- Prefer function and type inspection over reading entire large files.
- Avoid repeated or overlapping excerpts unless source changes or new evidence justify them.
- Exclude generated/build trees, vendor code and large logs unless relevant.
- Keep architectural navigation knowledge in Beads and repository indexes, not large inventories in AGENTS.md.

### Symbol-Level Investigation

For a known Rust symbol requiring implementation, dependency, test or change-impact investigation, prefer:

`python3 scripts/agent/efficiency/symbol_report.py SYMBOL --crate CRATE --limit 5 --brief`

The report provides:

- Target definition and source location.
- Cross-crate lexical references.
- Caller candidates.
- Related type candidates.
- Ranked direct, related and behavioural test candidates.
- Extracted assertions.
- Working-tree change-impact candidates.
- Suggested regression commands.
- Compact Markdown context brief.

The default brief is stored at:

`~/.cache/sbe-diagnostics/context-brief-CRATE-SYMBOL.md`

Use the following workflow:

1. Reuse an existing valid report or generate one if needed.
2. Read the compact context brief.
3. Inspect the target definition and relevant caller locations.
4. Inspect related types only where necessary to establish contracts.
5. Prioritise DIRECT and RELATED tests over BEHAVIOURAL candidates.
6. Verify relevant assertions and identify missing regression coverage.
7. Implement the smallest correct change.
8. Run appropriate focused and crate-level validation.

Additional rules:

- Use `--limit 5` by default; increase only when necessary.
- Use `--min-score 60` when behavioural results are excessively broad.
- Use `--show-all` only when investigating missing test candidates.
- Do not load the complete JSON report unless detailed evidence is required.
- Do not regenerate reports repeatedly unless relevant source has changed or the previous report is insufficient.
- Avoid repeating searches already answered by the report.
- Treat caller relationships, related types and cross-crate references as lexical candidates, not verified semantic dependencies.
- Behavioural relevance scores are ranking hints, not coverage probabilities.
- Generic strategy types and identifiers may produce irrelevant behavioural candidates.
- Unclassified Git changes are not necessarily unrelated.
- Extracted assertions do not establish complete test coverage.
- Verify actual source relationships before editing or recording architectural memories.
- Do not execute every suggested regression command automatically; select validation according to the actual change and acceptance requirements.

For simple definition or caller extraction, use the lighter-weight utility:

`python3 scripts/agent/efficiency/symbol_lookup.py SYMBOL --crate CRATE --definition --callers`

- Use `--references` when lexical reference enumeration is needed.
- Use `--definition` for targeted definition extraction.
- Use `--callers` to identify enclosing caller candidates.
- Prefer this utility when a full symbol report is unnecessary.
- Use targeted RTK reads, `rg`, ast-grep or rust-analyzer when structural discovery is insufficient.
- Do not run both symbol tools automatically for the same investigation.

### Command Efficiency

- Follow `RTK.md` without repeatedly rereading it.
- Use RTK for supported commands; run `sed`, `cat`, `bd` and `./nx` directly.
- Prefer repository Justfile recipes; inspect `just --list` when needed.
- Keep command output below approximately 20K characters where practical.
- Use `bounded_run.py` for commands expected to produce large outputs.
- Save larger outputs to disk and inspect bounded summaries.
- Preserve complete diagnostics when needed for verification.
- Use Context7 for unfamiliar/version-specific APIs and Python for deterministic transformations.
- Avoid repeated expensive commands, full database dumps and large API responses.
- Prefer server-side filtering, projection and pagination when supported.
- Do not request full payloads when only counts, summaries or selected fields are required.
- Use existing evidence files rather than repeatedly querying unchanged data.
- Do not suppress errors or truncate evidence in ways that prevent reliable validation.
- Avoid unnecessary parallel tool calls that retrieve overlapping information.

### Evidence and Regression Reuse

- Distinguish verified evidence from historical, provisional or candidate evidence.
- Reuse previous successful validation only when all relevant inputs remain unchanged and the prior result is applicable.
- Check source changes, configuration, fixtures, dependencies and execution environment before reusing regression evidence.
- Use `regression_identity.py` to identify potential reuse; do not treat identity candidates as automatic permission to skip tests.
- Prefer focused tests during implementation and broader regression checks at integration boundaries.
- Coordinate expensive Rust builds and avoid competing builds across agents.
- Do not repeatedly run unchanged failing tests without a new hypothesis or relevant source change.
- Record failed commands, failure details and evidence paths in handoffs.
- Do not weaken assertions, financial invariants or acceptance criteria merely to obtain passing tests.
- Do not treat a successful command, synthetic self-test, connectivity check or threshold evaluation as complete implementation acceptance.

### Historical Improvement

The coordinator or developer periodically runs:

`python3 .codex/session_review.py --days 7 --repo <repository-root> --navigation --context --tooling --recommendations`

Review context growth, repeated reads, overlapping searches, oversized outputs, tooling adoption and repeated tests.

- Do not run historical analysis independently in every agent.
- Treat historical recommendations as investigation candidates, not proof of wasted work.
- Verify findings against current source before updating Beads or tooling.
- Improve existing tools and guidance rather than creating unnecessary new utilities.
- Measure subsequent behaviour before declaring an optimisation successful.
- Evaluate total token consumption, context growth, tool-call counts, output sizes and implementation correctness.
- Do not assume that RTK output savings, context reductions and navigation savings are additive.
- Do not interrupt implementation to optimise tooling without a demonstrated material benefit.

## 4. Agent Coordination and Handoffs

### Delegation

- The coordinator owns the overall plan, work-package dependencies, shared invariants, evidence requirements and final integration.
- Delegate bounded tasks with explicit scope, source locations, acceptance criteria and relevant evidence references.
- Provide specialists with the relevant work-package requirements rather than the entire master plan when possible.
- Avoid overlapping implementation ownership across agents.
- Use one build owner for expensive shared Rust builds.
- Do not spawn additional agents merely to repeat investigation or review already completed work.
- Verify that required agent capabilities are available before relying on delegation.
- Respect actual agent configuration and runtime limits.

### Named profile and resource policy

- Use the project-scoped named profiles in `.codex/agents/` for delegated work. Give each child one bounded work package, its Beads ID, exact paths/symbols, relevant contracts, acceptance criteria and required evidence; do not pass the full plan by default.
- Keep no more than two child agents active and default to one high-reasoning child at a time. Concurrent high-reasoning children require documented necessity and resource headroom. Do not allow nested delegation from specialist profiles.
- The coordinator profile owns dependency order, shared invariants, build scheduling and integration. Keep architecture, risk and concurrency review on the architect profile.
- Treat per-profile context and compaction thresholds as advisory on runtimes that do not expose effective settings. Check context at work-package boundaries, checkpoint before the profile target, and resume from compact Beads notes rather than deliberately filling context to test a limit.
- Record profile name, requested/effective model and reasoning, parent linkage, context evidence, Beads task and start/end usage counters when available. Mark missing runtime or usage fields as unavailable; do not infer them from configuration or cached-token ratios.

### Handoff Requirements

Each substantial work-package handoff should identify:

- Task and work-package identifier.
- Completed changes and affected files.
- Verified architectural discoveries.
- Tests executed and actual results.
- Evidence and diagnostic report paths.
- Remaining blockers and uncertainties.
- Outstanding acceptance requirements.
- Recommended next action.

Keep handoffs compact. Reference complete evidence on disk rather than embedding large logs, source files or conversation histories.

Use `bd remember` for reusable verified repository knowledge and task records for task-specific progress.

Do not record unverified lexical relationships as architectural facts.

<!-- AGENT-EFFICIENCY-V1 -->

## 5. Agent Efficiency Utilities

Use `scripts/agent/efficiency/` to supplement existing tooling:

| Utility | Purpose |
|---|---|
| `context_watch.py --role ROLE` | Advisory context checks at major task boundaries |
| `symbol_report.py SYMBOL --crate CRATE --limit 5 --brief` | Ranked symbol, caller, type, test and Git-impact investigation |
| `symbol_lookup.py NAME --crate CRATE` | Lightweight structural source extraction |
| `bounded_run.py -- COMMAND` | Bounded command output with retained full logs |
| `tool_catalog.py QUERY` | Discover existing diagnostic capabilities before creating scripts |
| `navigation_candidates.py` | Coordinator reviews historical navigation candidates and records verified discoveries with `bd remember` |
| `budget_dashboard.py --since DATE` | Coordinator monitors implementation token consumption |
| `regression_identity.py` | Identify possible regression evidence reuse |

Do not repeat these checks unnecessarily in every agent.

Treat warnings, rankings and generated mappings as advisory evidence.

The existing efficiency toolkit is the implementation baseline. Do not create replacement utilities or extend their feature sets unless a concrete implementation blocker demonstrates the need.

<!-- END-AGENT-EFFICIENCY-V1 -->

## 6. Implementation Completion

Before declaring a task complete:

- Verify the requested behaviour and relevant acceptance criteria.
- Run appropriate regression checks and inspect their actual results.
- Check patch integrity with `just diff-check`.
- Confirm that unrelated working-tree changes have been preserved.
- Record evidence paths, remaining uncertainties and blockers.
- Persist verified reusable navigation discoveries using `bd remember`.
- Provide a concise implementation summary.
- Stop once the requested scope is complete.

Do not continue into unrelated refactoring, additional diagnostics, speculative improvements or tooling development without justification.
