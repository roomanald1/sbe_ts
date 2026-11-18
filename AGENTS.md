# Project Guidance

## Architecture
- React: `src/typescript/workspace/apps/react-app`; shared UI: `src/typescript/workspace/packages/ui-platform`.
- Rust services: `src/rust/` (`feed_handlers`, `insights`, `auto_trader`, `strategy`, `learning`, `position_management`, `paper_exchange`).
- Signal flow: `feed_handlers` → `insights` → `auto_trader` via `StrategySignal`. Evaluators consume signals; position management handles execution.
- Do not edit generated SBE codecs unless required by protocol changes.

## Tools
- Follow `@RTK.md`; use RTK for supported commands and ast-grep for structural searches.
- Prefer targeted searches, reads and tests. For blocked reads, use `rtk bulk-read --question` or exact line ranges.
- Use Context7 for unfamiliar or version-specific APIs.
- Use Python normally for file transformations; verify replacements and inspect diffs.
- Preserve complete diagnostics. Correctness takes priority over token savings.

### Output Management
- For repository searches, exclude generated data, large JSON files and logs unless explicitly needed. Use `rg --max-columns 300 --max-columns-preview` to prevent enormous matching lines from flooding output. Narrow searches by directory and file type wherever practical.
- Keep individual command outputs below 20K characters where practical.
- Narrow `rtk rg` searches by directory, file type or symbol; avoid repository-wide output dumps.
- Use `rtk bulk-read --question` for large files and `sed -n '100,160p' file.rs` for exact code.
- Run focused tests first; avoid printing full test logs unnecessarily.
- For potentially large outputs, save complete results to `/tmp/` and inspect relevant sections.
- Avoid printing entire diffs; inspect changed files individually when necessary.
- Never discard errors, diagnostics or source context required for correctness.

## Development
- Rust: `rtk cargo test --manifest-path src/rust/<crate>/Cargo.toml`; use `cargo fmt` after edits.
- React: `./nx <target> react-app`; run targeted tests and format changed files.
- Keep business logic in Rust; frontend models handle UI state and views handle rendering.
- Strategies follow existing patterns, consume `StrategySignal`, and keep entry/open-position decisions in `evaluate.rs`.
- Preserve learned parameter bounds and feedback attribution.
- For VWAP API changes, update both trade ticket and Orders flows.
- Never restart services without explicit permission.
- For performance changes, consult `research/BENCHMARK_BASELINE.md`.

## Beads
- Use `bd` for persistent tasks; inspect existing work, update progress and close only after verification.
- Run `bd prime` when context is missing. No Markdown TODO tracking.

## Completion
- Run relevant checks and report failures or outstanding work.
- Do not commit, push or sync unless explicitly requested.