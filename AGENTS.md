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

## Execution-strategy map

- Position management is in src/rust/position_management; execution and VWAP scheduling are in src/rust/execution.
- Both React entry points use the same /positions/vwap API: the trade ticket uses TradeTicketModel.ts and TradeTicketPopup.tsx; the Orders page uses OrderManagerModel.ts and Positions.tsx. Update both client flows if the request or status contract changes.
- POST /positions/vwap accepts symbol, side, fractional quantity, duration (5–390 minutes), and max participation (1–100%). The backend generates a provisional U-shaped 78-bin curve and starts immediately. GET lists run status; DELETE cancels. Run state is in memory; paper child fills are persisted in the positions ledger.
- The strategy consumes the existing OKX trade and book feeds; it does not require SBE schema or generated-code changes. For VWAP tasks, start with the files above and trace feed/SBE code only if the data path changes.
- Exclude positions.json from broad searches with -g !positions.json: it is a large runtime fill ledger, not source. Avoid generated SBE output unless the schema or encoding is part of the task.

## Project conventions

- Use AG Grid for tabular data in views instead of native HTML tables, matching the shared data-grid styling. Style inputs and selects with existing app form/control styles rather than browser defaults.
- Keep business logic in model classes; React views render model state and forward user actions to models.
- Follow existing RxJS patterns and clean up subscriptions. For a new feed, trace the existing backend-to-frontend path and check subscription cleanup.
- For dashboard panels, register the widget in apps/react-app/src/views/FeedControls.tsx and follow an existing model/view pair.

## Build caches

- Run frontend targets from the repo root with ./nx <target> react-app. Nx caches build, lint, and typecheck; serve and preview are continuous.
- Cargo reuses build artifacts in the crate's target directory. Avoid cleaning caches unless stale output is suspected.
- Prefer the narrowest relevant validation; avoid broad builds or tests unless requested or needed.

## Shell command hygiene

- Run commands from the WSL repository root and use repository-relative paths that exist. Confirm paths mentioned in errors with rg --files or test -e before using them.
- If Bash startup emits errors from /home/ronnie/.profile, use a non-login shell when supported. Treat startup output as a profile configuration issue; do not copy it into a command or execute it.
- Use rg for source searches. Use fixed-string mode (rg -F) for literal text containing regex metacharacters; quote and escape regex patterns deliberately.
- Put only executable command text in the command field. Do not paste terminal output, error lines, diff markers, or explanatory prose there.
