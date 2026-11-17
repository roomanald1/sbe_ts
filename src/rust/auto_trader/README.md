# AutoTrader

AutoTrader owns which live strategies and parameter profiles may trade, plus
portfolio risk, position state, order submission, and live outcome attribution.
It does not receive Insights market-signal pushes or evaluate strategy rules.
Strategy workers subscribe to Insights, evaluate their active profiles, and
report profile-versioned decisions to AutoTrader. AutoTrader applies its final
risk and order checks before entering; it consumes worker exit decisions during
five-second position maintenance while protective orders remain active.

## Strategy live workers

Compose runs one live worker process per registered strategy at port `10201`.
Each runner subscribes to Insights on `INSIGHTS_STRATEGY_STREAM_URL`, refreshes
its profile set from AutoTrader, and evaluates all configured profiles against
the same feature update. `active_live_strategy_profiles` in AutoTrader settings
can hold up to 512 bounded profiles across strategies. If no explicit profile
set is configured for a strategy, AutoTrader supplies its learned/default
profile per symbol. Worker callbacks include the event ID, profile ID/version,
parameter hash, and decision. The decision endpoint rejects stale events,
disabled strategies, and profile snapshots that are no longer active.

Worker subscriptions use reconnect snapshots and a bounded deduplication map.
Live feature updates older than five seconds fail closed for entries. Daily
strategy snapshots refresh every 30 seconds; position exits hold safely when
the last worker decision is stale. Worker failures do not disable protective
order reconciliation.

Live strategy workers report decision transitions and periodic status for
active profiles. AutoTrader records profile-attributed live rejections in its
Learning feedback outbox so Learning can measure later directional markouts
from persisted 10-second features and book marks. Those observations are
diagnostic, not net trade returns. Raw generic signals and Scout probes are
not sent to AutoTrader.

## Listeners and state

- AutoTrader's service RSocket listener is configured by
  `AUTO_TRADER_RSOCKET_PORT` (Compose port `10006`; host port `10008`).
- Its operations HTTP listener is available on port `3005`.
- `AUTO_TRADER_STATE_FILE` points to the persisted AutoTrader state file.

Process-local caches are bounded: the Learning response cache retains at most
128 entries for 30 seconds; latest strategy signals retain one sample only for
active strategy positions and expire after 10 seconds. Pending worker decisions
coalesce by symbol and profile ID, retain only that symbol's market book, and
cap at 256 items by default. Each queued signal and its single-symbol book is
limited to an estimated 256 KiB, so the queue has a roughly 64 MiB payload
ceiling. Configure `AUTO_TRADER_MAX_PENDING_LIVE_SIGNAL_ITEMS`; the former
`AUTO_TRADER_MAX_PENDING_LIVE_SYMBOLS` name remains accepted as an alias.
Compose caps glibc
allocator arenas at four by default through `AUTO_TRADER_MALLOC_ARENA_MAX`.
Audit queues retain
at most 16,384 decision records / 4,096 execution records and an estimated
8 MiB of payload each. Replaceable decision rejections may be coalesced or
evicted to stay within the limits. Execution audit records are never evicted to
make room; if their queue fills, the new event is reported as dropped and the
existing audit health latch pauses new entries. The byte estimate includes
per-value and per-container overhead, but is an approximation of retained
allocator usage. These limits do not trim open positions, order state, or
durable feedback outboxes.

## Run and verify

From the repository root:

```sh
cargo run --manifest-path src/rust/auto_trader/Cargo.toml
cargo test --manifest-path src/rust/auto_trader/Cargo.toml
cargo fmt --manifest-path src/rust/auto_trader/Cargo.toml --all
```
