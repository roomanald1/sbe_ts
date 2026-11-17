# Strategy worker architecture and cutover

## Target architecture

- `strategy` is the single Rust crate for strategy evaluation, Scout assignments, and worker services. The old `strategies` crate and `strategy-x` service naming are removed.
- Each registered strategy has two independent processes: a live worker coordinated by AutoTrader and a Scout worker coordinated by Learning. Scout CPU/memory limits must not starve live decisions; Scout state remains durable.
- Insights publishes derived strategy inputs through its strategy stream. Workers subscribe to Insights and evaluate the inputs themselves; Insights does not push decisions to AutoTrader or Learning.
- AutoTrader owns live profile configuration, open-position state, risk gates, and orders. It installs live profile sets in live workers, accepts profile-attributed decisions, and remains responsible for execution and protective orders.
- Learning owns Scout profile configuration, feedback attribution, missed-opportunity tracking, and strategy-specific walk-forward/post-trade replay. Walk-forward evidence is based on a specific strategy/profile and exact historical inputs; missing coverage is indeterminate.
- Insights daily snapshots must distinguish symbols explicitly evaluated and active. Unknown symbols remain unknown so workers cannot infer an exit from a partial scan.
- AutoTrader fetches market-book state on demand for position management. Worker profile and decision caches, Insights per-symbol state, and Learning/Scout RAM working sets remain explicitly bounded; durable history stays in SQLite.
- Grafana worker availability, event freshness, and active profile version panels belong near the top of Runtime Performance. Service-control and launcher names use `strategy-live-*` and `strategy-scout-*`.

## Implemented in the current change

- Strategy workers consume the Insights stream and call back to AutoTrader for live decisions. Live profile sets are AutoTrader-owned; Scout profile snapshots and assignments are Learning-owned.
- Direct Insights-to-AutoTrader signal and daily-signal routes were removed. Live and Scout workers are distinct processes per strategy, with role-specific endpoints and metrics.
- Daily and dated-carry inputs flow to the workers. AutoTrader performs final execution and position risk handling; Learning receives profile-attributed outcomes and missed-candidate feedback.
- The old `strategies` crate was merged into `strategy`; old `strategy-x` service/environment names were renamed. The `strategy-x-state` Docker volume name stays to preserve existing persisted Scout data.
- Compose, local service control, Grafana, and the AutoTrader, Insights, Learning, Strategy, monitoring, and root READMEs were updated for the worker architecture.
- Daily snapshot publication now derives `active_strategy_symbols` from evaluated Insights diagnostics. Evaluated inactive symbols are reported as inactive; unevaluated symbols remain unknown.

## Final verification and cutover

1. Run Rust formatting and tests for AutoTrader, Insights, Learning, and Strategy, plus the React app test suite and exporter tests.
2. Run the affected Rust benchmarks, compare with the latest comparable workloads in `BENCHMARK_BASELINE.md`, and append dated results. Validate Compose configuration, Grafana JSON, and `git diff --check`.
3. Rebuild and restart only the changed system services and their dependent strategy workers. Remove orphaned old service containers while preserving the existing Scout state volume.
4. Run the homepage System liveness check and confirm Prometheus sees all live and Scout worker metrics. Check that Grafana panels appear under Runtime Performance.

## Known limits to verify during cutover

- Initial worker start-up may delay decisions until Insights subscriptions and profile sets are available; AutoTrader must fail closed for new entries while protective-order reconciliation continues.
- Daily decisions depend on the Insights evaluated/active symbol status and profile-attributed feature set. Missing or mismatched profile inputs must produce Hold/unknown behavior, never a synthetic inactive signal.
- Memory measurements during the earlier Scout/Insights/AutoTrader runs were startup and feed-load snapshots, not peak-load guarantees. Recheck after the new deployment and sustained traffic.
