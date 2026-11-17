# Insights

Insights consumes raw ticker, trade, and order-book events from FeedHandlers. It owns rolling live feature calculation and publishes versioned `StrategyInsightUpdate` events on its RSocket stream at port `10004`. The `strategy-signals:*` stream replays the latest bounded per-symbol updates and daily snapshot to new subscribers, then publishes updates. Live and Scout strategy workers subscribe directly; Insights does not push market signals to AutoTrader or Learning.

Each update includes its event ID and timestamp. Intraday updates carry the derived feature snapshot and current book needed by Scout observations. Daily candle snapshots and dated-carry candidates/settlements use the same stream contract. Strategy workers reject stale events and resubscribe after stream lag to receive the latest snapshots. Insights still reads AutoTrader configuration to prepare daily strategy feature inputs and carry size limits; these are control/configuration reads, not signal delivery.

Insights always runs live feature generation. Research sampling and persistence run independently and cannot disable signal generation. There is no `INSIGHTS_RESEARCH_ONLY` mode or separate Live Signals producer.

Process-local market state is bounded: active per-symbol engines and the latest signal replay map are capped at 256 symbols by default and expire together after 60 seconds idle. FeedHandlers currently selects the top 100 spot instruments. If the cap is reached while every engine is active, Insights rejects a new symbol rather than evicting rolling state still needed for live features. News attribution pending state expires after 60 seconds and is capped at 8,192 entries. Research history and dashboard calibration data live in SQLite; the dashboard-only calibration window caches at most 1,000 snapshots and 5,000 outcomes for up to 15 seconds.

## Listeners

- Market-event and strategy-update RSocket stream: port `10004` (`signal-events:*`, `tickers:*`, `strategy-signals:*`).
- Internal RSocket routes: port `10007`.
- Browser RSocket: port `10011`.
- Operations HTTP: port `3004` (health, metrics, and documentation only).

Insights remains the source of market-derived features and research history. Learning owns Scout profiles and replay policy. AutoTrader owns trading profiles and final order/risk decisions. Exchange and vendor API requests remain HTTP.
