# Trading Improvements Implementation Status

The requirements and acceptance criteria remain authoritative in [TRADING_IMPROVEMENTS.MD](src/rust/auto_trader/TRADING_IMPROVEMENTS.MD). Durable ownership and dependencies are recorded in Beads; this file is a status snapshot, not a replacement task ledger or a change to the requirements.

## User-confirmed contracts

- Worker decision reports carry a versioned event ID and episode ID. Retries and restarts preserve identity for the same source event; a distinct canonical source event (changed timestamp or content) creates a new atomic episode. A bitwise-identical event cannot encode a logical reset without a producer reset token, so it remains the same ID.
- Re-entry cooldown begins only after a stop fills completely and the position is fully closed. Partial stop fills do not start the timer. After expiry, re-entry requires a fresh versioned event and episode.
- Feed freshness is per symbol and tied to active book subscription, current book mark, and sampled history. Held symbols are explicitly requested. Proxy hedges fail closed without fresh source-provided status; the global stream idle timeout is not a symbol guarantee.

## Current implementation state (2026-11-25)

- Strategy worker emits serde-compatible versioned report identity; Insights now derives a deterministic content-addressed source event ID and passes the corresponding identity through the signal update.
- AutoTrader has bounded durable exploratory-event deduplication and persisted post-stop cooldown state, with gates on entry and cooldown recording on both immediate and protective stop paths.
- Paper Exchange market history contract v3 samples per-symbol book midpoints and reports `fresh` only with an active subscription, a current mark, and history within the published 15-second bound. Held-symbol requests are bounded to 64 symbols and 2048 query bytes.
- AutoTrader requests held-symbol subscriptions in bounded chunks and consumes only v3 per-symbol `fresh` status for proxy correlation eligibility. Missing and stale freshness data keep proxy hedges fail-closed.
- Resting post-only entry snapshots preserve v1 worker event identity. Maintenance only treats a distinct fresh event/episode as new invalidation evidence; exact retries and legacy identity remain non-evidence.
- Scout fixed-horizon late-outcome drops are counted by requested horizon, cumulative lateness, and maximum lateness in the existing latency snapshot (`sbe_ts-x8if`).
- AutoTrader Grafana metrics exporter requests now have bounded pooled-RSocket retry/recovery and visible failures (`sbe_ts-85c3`); source is validated, but the service was not restarted or redeployed.
- Closed legacy outcome rows now remain visible when parent-order P&L attribution is incomplete, with unknown values left unset rather than inferred from fill notional (`sbe_ts-y8r`).

## Validation and remaining work

- Current crate validation: AutoTrader 260 library + 9 binary tests, Strategy library 80/80, Insights library 47/47 plus binary 56/56, and Paper Exchange library 17/17 passed. The two focused lifecycle integration tests also pass individually after the latest identity assertions. One initial build caught an `Arc<BookSnapshot>` coercion error in Paper Exchange; it was fixed before its full suite passed. Rust formatting checks pass for the changed crates.
- Sections 4 (`sbe_ts-x8if`, `sbe_ts-y8r`), 6 (`sbe_ts-85c3`), 7 (`sbe_ts-xnzp`), 8 (`sbe_ts-ink1`), 9 (`sbe_ts-gu78.16`), and the production lifecycle fixture (`sbe_ts-gu78.13`) are complete in Beads. The broader contract/financial acceptance and strict-evidence gap work remain open under `sbe_ts-gu78` and `sbe_ts-khs0`.
- Live settings, running services, deployment, and runtime databases have not been changed.
