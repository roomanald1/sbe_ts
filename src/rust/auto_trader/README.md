# AutoTrader

AutoTrader owns live trading coordination, portfolio and risk gates, order
submission, position state, and live outcome attribution. Strategy decisions
are evaluated by Strategy X live workers; AutoTrader routes each registered
strategy request to its dedicated worker and remains responsible for applying
the returned decision.

## Strategy X live workers

`AUTO_TRADER_LIVE_WORKER_ENDPOINTS` is a comma-separated mapping of
`strategy=ws://host:port` entries. Compose configures one always-on worker per
registered strategy using `strategy-x-live-<strategy>` at port `10201`. The
workers use `STRATEGY_X_WORKER_ROLE=live` and
`STRATEGY_X_WORKER_STRATEGY=<strategy>`. They are stateless and do not share
Scout databases or queues.

Live evaluation requests have bounded concurrency, payload size, and timeout.
Worker failures reject new strategy decisions; protective order handling and
position management continue through their existing path.

When a cycle cannot select a trade, AutoTrader places at most one strongest
rejected, named live-strategy candidate in its persisted Learning feedback
outbox. The event carries the exact profile/parameters, direction, score,
strength, and market context. Generic signal-only candidates and Scout probes
are excluded. Learning measures later directional markouts from persisted
10-second features and book marks; these observations are diagnostic and are
not represented as net trade returns.

## Listeners and state

- AutoTrader's service RSocket listener is configured by
  `AUTO_TRADER_RSOCKET_PORT` (Compose port `10006`; host port `10008`).
- Its operations HTTP listener is available on port `3005`.
- `AUTO_TRADER_STATE_FILE` points to the persisted AutoTrader state file.

Process-local caches are bounded: the Learning response cache retains at most
128 entries for 30 seconds; latest live strategy signals retain one sample only
for active strategy positions and expire after 10 seconds; market-book prewarm
requests expire after 5 minutes and cap at 4,096 symbols. Audit queues retain
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
