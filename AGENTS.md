# Agent notes

## Command environment

- Use the Linux path /home/ronnie/dev/sbe_ts as the canonical workspace path for file reads, writes, and terminal working directories. Avoid passing the Windows UNC workspace spelling to edit/patch tools: they may reject a WSL reparse point even though the path works as a shell working directory.
- Run all project commands in Linux under WSL, from /home/ronnie/dev/sbe_ts (or the relevant project directory). This includes nx, cargo, rg, Git, package managers, and build or test commands.
- Do not invoke project commands through Windows PowerShell, Command Prompt, or Windows executables. In particular, run Nx as ./nx <target> react-app from the WSL repository root; do not launch nx or ./nx from Windows, which can trigger an app chooser.
- If the available terminal starts in Windows, open/use a WSL/Linux shell first and run the command there.

## Git on the WSL workspace

- Run Git commands from the repository root. If using Windows Git (C:\Program Files\Git\cmd\git.exe) against this WSL workspace via its UNC path and Git reports dubious ownership, add this repository to Windows Git's global safe-directory list: git config --global --add safe.directory '%(prefix)///wsl.localhost/Ubuntu/home/ronnie/dev/sbe_ts'. This is a one-time per-user setup. Prefer using WSL Git from /home/ronnie/dev/sbe_ts when available.

## Project map

- Frontend: src/typescript/workspace/apps/react-app.
- Backend services: src/rust/feed_handlers, src/rust/position_management, src/rust/execution, and src/rust/insights.
- Generated SBE Rust code: src/rust/feed_handlers/src/sbe/sbe_out/example; schema: src/rust/feed_handlers/src/sbe/sbe_schema.xml. Change generated files only when the schema or protocol requires it.
- Use rg to find relevant symbols and read only the files needed for the task. Avoid lockfiles, build output, dependencies, and generated code unless relevant.
- Root README has run commands; the Rust crate README describes service behavior and profiling.

## Running the full stack

- Do not start, stop, restart, or otherwise cycle running services unless the user explicitly instructs you to do so. This includes `./start-all.sh`, `./stop-all.sh`, Compose commands, and service-specific process commands. If a task appears to need a restart, explain that and leave the services running until explicitly asked.
- Start the complete local stack from the repository root with `./start-all.sh`. It builds and starts the monitoring Compose services, backend services, and React app. Keep the terminal running to supervise the stack; use Ctrl+C or `./stop-all.sh` to stop it.
- Stop the stack from the repository root with `./stop-all.sh`. This stops this repository backend and UI processes and tears down its monitoring Compose services.
- Before starting, ensure `monitoring/.env` exists. Create it from `monitoring/.env.example` and set a unique Grafana password when needed.
- Use these scripts for normal full-stack startup and shutdown instead of starting or stopping the backend, UI, and monitoring services separately. For focused frontend targets, the Nx guidance below still applies.

## Execution-strategy map

- Position management is in src/rust/position_management; execution and VWAP scheduling are in src/rust/execution.
- Both React entry points use the same /positions/vwap API: the trade ticket uses TradeTicketModel.ts and TradeTicketPopup.tsx; the Orders page uses OrderManagerModel.ts and Positions.tsx. Update both client flows if the request or status contract changes.
- POST /positions/vwap accepts symbol, side, fractional quantity, duration (5–390 minutes), and max participation (1–100%). The backend generates a provisional U-shaped 78-bin curve and starts immediately. GET lists run status; DELETE cancels. Run state is in memory; paper child fills are persisted in the positions ledger.
- The strategy consumes the existing OKX trade and book feeds; it does not require SBE schema or generated-code changes. For VWAP tasks, start with the files above and trace feed/SBE code only if the data path changes.
- Exclude positions.json from broad searches with -g !positions.json: it is a large runtime fill ledger, not source. Avoid generated SBE output unless the schema or encoding is part of the task.

## Strategy conventions

- Build every new strategy by following the complete pattern of an existing active strategy, including its strategy folder, evaluator, contract, parameter definitions, registry entry, signal path, and learning integration. Do not add a strategy that bypasses established layers.
- Subscribe to each strategy's required market data in feed_handlers, pass the resulting inputs through insights, and publish them to auto_trader as StrategySignal data. Keep calculations in the appropriate upstream market-data/insights layer; evaluators must make decisions from StrategySignal rather than subscribing to feeds or reading market data directly.
- Parameterize every variable strategy behavior, including entry and exit thresholds, timing, sizing, and any other tunable decision. Define defaults and bounds in the strategy parameter specs, carry the parameters in the signal/profile contract, and ensure bounded learning can update them from feedback. Do not hard-code tunable behavior or introduce untracked per-strategy defaults.
- Preserve the learning feedback loop for new strategies: outcomes/feedback must be attributed to the strategy and its parameter profile so learning can evaluate and adjust those parameters. Scouts may be sent with different adjusted parameter profiles; each scout's evaluations and feedback must remain associated with the parameters it actually used, so feedback updates the corresponding learning profile rather than being mixed across variants.
- Every active strategy under src/rust/auto_trader/src/auto_trader/strategies/ has one evaluate.rs that owns its signal-based entry rule and open-position decision (Enter, Reject, Hold, Exit, or Rebalance). Do not split strategy decisions across entry.rs and exit.rs.
- Strategy evaluators receive StrategySignal only; convert serialized text to enums before evaluation. Keep market-data calculations upstream and price-triggered protective order execution in position management.
- Tunable evaluator thresholds must come from StrategySignal parameters produced by the bounded learned profile. Keep defaults and bounds in the strategy parameter specs; do not duplicate fallback numbers in evaluate.rs. Reject entry if a required learned parameter is missing.
- Put per-strategy entry, exit, and parameter-bound tests in that strategy's evaluate.rs. Add a folder, evaluate.rs, contract.rs, and parameter definitions for every new strategy, and register it in strategies/mod.rs.
- LiveInsights is a legacy compatibility alias for old saved settings and remains inline in the registry; do not recreate its removed strategy folder unless that compatibility design changes.

## Project conventions

- Use AG Grid for tabular data in views instead of native HTML tables, matching the shared data-grid styling. Style inputs and selects with existing app form/control styles rather than browser defaults.
- Keep business logic in model classes; React views render model state and forward user actions to models.
- Follow existing RxJS patterns and clean up subscriptions. For a new feed, trace the existing backend-to-frontend path and check subscription cleanup.
- For dashboard panels, register the widget in apps/react-app/src/views/FeedControls.tsx and follow an existing model/view pair.

## Build caches

- Run frontend targets from the repo root with ./nx <target> react-app. Nx caches build, lint, and typecheck; serve and preview are continuous.
- Cargo reuses build artifacts in the crate's target directory. Avoid cleaning caches unless stale output is suspected.
- After any TypeScript source or test changes, run the React app TypeScript test suite with npm test from src/typescript/workspace/apps/react-app, even when the change seems unrelated to the tests. Run any additional focused typecheck, lint, or build target when the change warrants it.
- After any Rust source or test changes, run the relevant Rust tests and benchmarks for the affected crate(s). Before benchmarking, read research/BENCHMARK_BASELINE.md. Compare the same workloads with the latest recorded run, report any performance degradation along with meaningful changes and environment/workload differences, and append a dated run section with results. Preserve earlier entries. Mark skipped or failed targets explicitly; do not compare unlike workloads as regressions.
- After modifying Rust code, run `cargo fmt` for the affected crate(s) as the final Rust code maintenance step.

## Shell command hygiene

- Run commands from the WSL repository root and use repository-relative paths that exist. Confirm paths mentioned in errors with rg --files or test -e before using them.
- If Bash startup emits errors from /home/ronnie/.profile, use a non-login shell when supported. Treat startup output as a profile configuration issue; do not copy it into a command or execute it.
- Use rg for source searches. Use fixed-string mode (rg -F) for literal text containing regex metacharacters; quote and escape regex patterns deliberately.
- Put only executable command text in the command field. Do not paste terminal output, error lines, diff markers, or explanatory prose there.
