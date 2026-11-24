# Project Guidance

## Architecture
- React: `src/typescript/workspace/apps/react-app`; shared UI: `src/typescript/workspace/packages/ui-platform`.
- Rust services: `src/rust/` (`feed_handlers`, `insights`, `auto_trader`, `strategy`, `learning`, `position_management`, `paper_exchange`).
- Signal flow: `feed_handlers` → `insights` → `auto_trader` via `StrategySignal`; evaluators consume signals and position management handles execution.
- Scouts: `src/rust/strategy/src/scouts/`, not a standalone crate.
- Keep business logic in Rust; frontend models manage state and views render UI.
- Strategy entry/open-position decisions belong in `evaluate.rs`; preserve learned bounds and feedback attribution.
- Avoid editing generated SBE codecs unless protocols change. Update trade-ticket and Orders flows together for VWAP API changes.

## Navigation and Efficiency
- Use memories injected by `bd prime` before searching. Query `bd memories <topic>` only when additional detail is needed; save verified, reusable discoveries with `bd remember`. Avoid duplicates and restore context after compaction.
- Follow `RTK.md` without repeatedly rereading it. Use RTK for supported commands; run `sed`, `cat`, `bd` and `./nx` directly.
- Prefer repository `Justfile` recipes; inspect `just --list` when needed.
- Search likely crates/directories first. Never run unrestricted root searches for ports, timestamps, numbers or common identifiers.
- Discover uncertain paths with `rg --files`; use `rg -l`, targeted `rg`, ast-grep and bounded excerpts. Exclude generated/build trees and large logs unless relevant.
- Reuse confirmed findings; avoid speculative paths, repeated reads and overlapping searches without new evidence.
- Keep command output under ~20K characters where practical. Capture large output in `/tmp/`, inspect summaries and preserve complete diagnostics.
- Use Context7 for unfamiliar/version-specific APIs; Python for suitable transformations.
- Delegate only independent, bounded work. Avoid overlapping investigations and competing Cargo builds.
- Stay within the requested scope; make the smallest correct change and stop once verified.

## Diagnostic tooling maintenance

Diagnostic tooling is a maintained part of the repository, not disposable implementation scaffolding.

For every implementation task:

- Reuse existing tools under `scripts/local/`, `scripts/diagnostics/`, `scripts/api/` and the existing database utilities.
- Update affected diagnostic scripts whenever database schemas, API contracts, financial calculations, telemetry, configuration or trading behaviour change.
- Extend diagnostic coverage when introducing new functionality or metrics.
- Add regression tests for changed diagnostic contracts.
- Preserve backwards compatibility where practical.
- Explicitly reject unsupported schemas rather than silently returning misleading results.
- Regenerate affected work-package context and repository indexes when their source inputs change.
- Record reusable architectural discoveries using `bd remember`.
- Include diagnostic tooling validation in the implementation handoff.

A work package is not complete if its implementation leaves affected diagnostic tooling outdated.

## Docker and Live Data
- Never start, stop, restart, recreate or redeploy services without explicit permission. Read-only inspection is allowed.
- Call `containerToolsConfig` before Docker/Compose commands; use its reported base command.
- Live AutoTrader and Position Management SQLite databases are in Docker volume `sbe-position-management-data`, mounted at `/data`: `auto_trader_state.sqlite` and `positions.sqlite`. Repository-local copies may be stale.
- Resolve the current container/volume before inspection. Query read-only or copy a consistent snapshot to `/tmp/`; preserve WAL sidecars or use SQLite backup. Never edit live databases.
- For authorised deployments, prefer `docker compose --progress quiet`; verify with `rtk docker compose ps`.
- Prefer continuous logs over overlapping polling; capture long-running logs to `/tmp/` and inspect bounded summaries. Preserve entry, candidate, signal, rejection, skip and gate evidence.

## Queries and Validation
- Inspect unfamiliar SQLite schemas. Prefer explicit columns, filters, aggregation and selected JSON fields over large dumps; use `LIMIT` only for exploration.
- Validate narrowly first, expanding for shared contracts or cross-component changes. Don't repeat successful checks without relevant changes.
- Rust: `just test <crate> [filter ...]`, `just check <crate>`, `just fmt <crate>`; `just fmt-rust` for broader formatting.
- React: `just typecheck`; format changed files.
- Run `just diff-check` before completion. Consult `research/BENCHMARK_BASELINE.md` for performance changes.

## Beads and Completion
- Use Beads issues for multi-step tasks; close verified work. Use `bd remember` for reusable cross-task knowledge, not task-specific notes.
- Inspect final `git status` and report changes, checks and outstanding issues.
- Never commit, push or sync without explicit permission.
## Deterministic diagnostic tooling

Reuse scripts/agent and scripts/diagnostics for recurring
investigations. Read scripts/diagnostics/CODEX_INSTRUCTIONS.md
when performing diagnostics. Prefer deterministic extraction
and calculation over repeated model calls. Keep detailed
outputs in /tmp. Validate data adapters against authoritative
contracts; never invent missing data or financial calculations.

## SQLite diagnostic tooling

Use scripts/db for database discovery, schema searches,
bounded read-only queries and execution plans.
Read scripts/db/README.md before database investigations.
Reuse verified SQL from scripts/db/queries.
Never substitute local databases for runtime databases.
Record verified database mappings with bd remember.
Avoid unbounded queries and large inline results.
