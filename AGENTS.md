# Project Guidance

## Architecture

- React: `src/typescript/workspace/apps/react-app`; shared UI: `src/typescript/workspace/packages/ui-platform`.
- Rust services: `src/rust/` (`feed_handlers`, `insights`, `auto_trader`, `strategy`, `learning`, `position_management`, `paper_exchange`).
- Signal flow: `feed_handlers` → `insights` → `auto_trader` via `StrategySignal`; evaluators consume signals and position management handles execution.
- Scouts live in `src/rust/strategy/src/scouts/`, not a standalone crate.
- Keep business logic in Rust; frontend models manage UI state and views handle rendering.
- Strategy entry and open-position decisions belong in `evaluate.rs`. Preserve learned parameter bounds and feedback attribution.
- Avoid editing generated SBE codecs unless protocols change. Update both trade-ticket and Orders flows for VWAP API changes.

## Tools and Navigation

- Follow `RTK.md`. Use RTK for supported commands; run `sed`, `cat`, `bd` and `./nx` directly.
- Use the repository `Justfile` for recurring development and service commands; inspect `just --list` and prefer the matching `just` recipe over memorized direct commands.
- Check relevant `bd remember` knowledge before exploring unfamiliar code. Save confirmed, reusable file locations, symbols and architectural relationships using `bd remember`; correct stale entries after refactors.
- Search likely crates first using targeted `rg`, `rg -l` and ast-grep. Discover uncertain paths with `rg --files` before searching contents.
- Reuse confirmed findings; avoid repeated or overlapping searches without new evidence.
- Use Context7 for unfamiliar or version-specific APIs.
- Use Python for suitable transformations; verify changes and inspect diffs.
- Parallelise independent reads and checks when safe; avoid competing Cargo builds.

## Output Management

- Keep command output below 20K characters where practical.
- Narrow searches and inspect relevant file sections rather than dumping entire files, diffs or test logs.
- Capture large outputs in `/tmp/`, then inspect targeted excerpts, counts or summaries.
- Preserve complete diagnostics and evidence needed for correctness; never silently truncate essential data.

## Docker Operations

- Never start, stop, restart, recreate or redeploy services without explicit permission. Read-only inspection is allowed.
- For authorised deployments, prefer `docker compose --progress quiet`; verify with `rtk docker compose ps`.
- Prefer continuous `docker compose logs --follow` over overlapping polling windows.
- Capture long-running logs to `/tmp/`; return bounded summaries while retaining complete evidence.
- Preserve relevant entry, candidate, signal, rejection, skip and gate diagnostics when investigating trading behaviour.

## SQLite Queries

- Inspect unfamiliar schemas before querying.
- Prefer filtered queries, explicit columns and SQL aggregation over large dumps.
- Use `LIMIT` for exploration, never when complete results are required.
- Extract relevant JSON fields instead of printing large values.
- Save large results to `/tmp/` and inspect targeted subsets without losing information needed for calculations or trading decisions.

## Development and Validation

- Start with focused checks and tests; expand when changes cross components or affect dependencies.
- Rust: use `just test <crate> [filter ...]`, `just check <crate>`, and `just fmt <crate>`; use `just fmt-rust` when formatting all frequently edited Rust crates.
- React: use `just typecheck` for the app typecheck; format changed files.
- Use `just diff-check` to verify whitespace before completion.
- Reuse valid test results when code and configuration remain unchanged; avoid redundant expensive checks.
- For performance changes, consult `research/BENCHMARK_BASELINE.md`.

## Beads and Completion

- Use Beads issues for multi-step tasks, progress and outstanding work; use `bd remember` for reusable cross-task knowledge.
- Run relevant quality checks and final `git status` before completion.
- Never commit, push or sync unless explicitly requested.
