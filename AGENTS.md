# Agent notes

## Project map

- Frontend: `src/typescript/workspace/apps/react-app`.
- Backend: `src/rust/rust_sbe_publisher`.
- Generated SBE Rust code: `src/rust/rust_sbe_publisher/src/sbe/sbe_out/example`; schema: `src/rust/rust_sbe_publisher/src/sbe/sbe_schema.xml`. Change generated files only when the schema or protocol requires it.
- Use `rg` to find relevant symbols and read only the files needed for the task. Avoid lockfiles, build output, dependencies, and generated code unless relevant.
- Root README has run commands; the Rust crate README describes service behavior and profiling.

## Project conventions

- Keep business logic in model classes; React views render model state and forward user actions to models.
- Follow existing RxJS patterns and clean up subscriptions. For a new feed, trace the existing backend-to-frontend path and check subscription cleanup.
- For dashboard panels, register the widget in `apps/react-app/src/views/FeedControls.tsx` and follow an existing model/view pair.

## Build caches

- Run frontend targets from the repo root with `./nx <target> react-app`. Nx caches `build`, `lint`, and `typecheck`; `serve` and `preview` are continuous.
- Cargo reuses build artifacts in the crate's `target` directory. Avoid cleaning caches unless stale output is suspected.
- Prefer the narrowest relevant validation; avoid broad builds or tests unless requested or needed.
