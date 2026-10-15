# Agent notes

## Project map

- Frontend: `src/typescript/workspace/apps/react-app`.
- Backend: `src/rust/rust_sbe_publisher`.
- Generated SBE Rust code: `src/rust/rust_sbe_publisher/src/sbe/sbe_out/example`; schema: `src/rust/rust_sbe_publisher/src/sbe/sbe_schema.xml`. Change generated files only when the schema or protocol requires it.
- Use `rg` to find relevant symbols and read only the files needed for the task. Avoid lockfiles, build output, dependencies, and generated code unless relevant.
- Root README has run commands; the Rust crate README describes service behavior and profiling.

## Execution-strategy map

- VWAP REST lifecycle and paper fills are in `src/rust/rust_sbe_publisher/src/positions.rs`; scheduling and benchmark logic are in `src/rust/rust_sbe_publisher/src/vwap.rs`.
- Both React entry points use the same `/positions/vwap` API: the trade ticket uses `TradeTicketModel.ts` and `TradeTicketPopup.tsx`; the Orders page uses `OrderManagerModel.ts` and `Positions.tsx`. Update both client flows if the request or status contract changes.
- POST `/positions/vwap` accepts symbol, side, fractional quantity, duration (5–390 minutes), and max participation (1–100%). The backend generates a provisional U-shaped 78-bin curve and starts immediately. GET lists run status; DELETE cancels. Run state is in memory; paper child fills are persisted in the positions ledger.
- The strategy consumes the existing OKX trade and book feeds; it does not require SBE schema or generated-code changes. For VWAP tasks, start with the files above and trace feed/SBE code only if the data path changes.
- Exclude `positions.json` from broad searches with -g `!positions.json`: it is a large runtime fill ledger, not source. Avoid generated SBE output unless the schema or encoding is part of the task.

## Project conventions

- Keep business logic in model classes; React views render model state and forward user actions to models.
- Follow existing RxJS patterns and clean up subscriptions. For a new feed, trace the existing backend-to-frontend path and check subscription cleanup.
- For dashboard panels, register the widget in `apps/react-app/src/views/FeedControls.tsx` and follow an existing model/view pair.

## Build caches

- Run frontend targets from the repo root with `./nx <target> react-app`. Nx caches `build`, `lint`, and `typecheck`; `serve` and `preview` are continuous.
- Cargo reuses build artifacts in the crate's `target` directory. Avoid cleaning caches unless stale output is suspected.
- Prefer the narrowest relevant validation; avoid broad builds or tests unless requested or needed.
