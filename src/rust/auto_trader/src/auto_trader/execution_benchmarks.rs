fn benchmark_signal(strategy: &str, candidate: bool) -> Signal {
    Signal {
        symbol: "BTC-USDT".into(),
        direction: "bullish".into(),
        strength: 1.0,
        timestamp: 1,
        stale: false,
        score: 1.0,
        ranging_probability: None,
        components: SignalComponents::default(),
        strategy_candidate: candidate,
        strategy: Some(strategy.into()),
        strategy_decision: Some(serde_json::json!({
            "observed_close": 120.0, "primary_level": 100.0, "secondary_level": 100.0,
            "btc_gate_required": true, "btc_close": 120.0, "btc_sma": 100.0,
            "primary_metric_value": 10.0, "strategy_order_size_multiplier": 0.5,
            "monthly_anchor_close_time_ms": 1, "cross_sectional_metric_value": 10.0,
            "cross_sectional_threshold": 5.0
        })),
        market_regime: None,
        news_attribution: None,
        entry_gate: None,
    }
}

/// Benchmark AutoTrader-only paths that are outside the reusable strategy crate.
pub fn benchmark_autotrader_paths(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    let signal = benchmark_signal("donchian-55-btc200", true);
    let parameters = ExecutionParameters::default();
    let signal_payload = serde_json::json!({ "symbol": signal.symbol, "direction": signal.direction,
        "strength": signal.strength, "timestamp": signal.timestamp, "strategy": signal.strategy,
        "strategy_decision": signal.strategy_decision });
    let market = serde_json::json!({
        "bids": (0..20).map(|i| serde_json::json!({"price": 100.0 - i as f64 * 0.01, "size": 1.0 + i as f64})).collect::<Vec<_>>(),
        "asks": (0..20).map(|i| serde_json::json!({"price": 100.02 + i as f64 * 0.01, "size": 1.5 + i as f64})).collect::<Vec<_>>()
    });
    let mut results = Vec::new();
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(serde_json::to_vec(black_box(&signal_payload)).unwrap());
    }
    results.push((
        "signal serialization".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(assess_deep_book(
            black_box(&market),
            "BTC-USDT",
            true,
            2_000.0,
            10.0,
        ));
    }
    results.push((
        "market depth assessment".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let started = Instant::now();
    for index in 0..iterations {
        black_box(build_order_request(
            "BTC-USDT",
            "buy",
            0.25,
            100.0,
            100.02,
            AutoOrderType::Limit,
            parameters,
            "bench-order",
            index as u64,
        ));
    }
    results.push((
        "limit order request construction".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    let overrides = BTreeMap::new();
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(validate_strategy_parameter_overrides(black_box(&overrides)).unwrap());
    }
    results.push((
        "strategy parameter validation".into(),
        started.elapsed().as_nanos() as f64 / iterations as f64,
    ));
    results
}

/// Measure the production signal settings lock/gate/profile/snapshot path with
/// deterministic retained-order fixtures, including an empty and full ledger.
pub fn benchmark_signal_settings_snapshot_path(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};

    assert!(iterations > 0, "benchmark iterations must be positive");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build benchmark runtime");
    let signal = benchmark_signal("live-momentum", false);
    let mut results = Vec::new();

    for retained_orders in [0, 256, MAX_RETAINED_AUTO_TRADER_ORDERS] {
        let mut settings = Settings::default();
        settings.orders = (0..retained_orders)
            .map(|index| {
                serde_json::from_value(serde_json::json!({
                    "timestamp": index,
                    "symbol": format!("BENCH-{index}-USDT"),
                    "side": "buy",
                    "order_type": "Limit",
                    "purpose": "signal",
                    "strategy": "live-momentum",
                    "quantity": 1.0,
                    "filled_quantity": 1.0,
                    "average_price": 100.0,
                    "status": "filled",
                    "decision_id": format!("benchmark-decision-{index}"),
                    "processed_fill_trade_ids": [format!("benchmark-fill-{index}")],
                    "realized_pnl": 0.0
                }))
                .expect("deserialize benchmark order")
            })
            .collect();
        let state = benchmark_snapshot_state(settings);
        let us_per_operation = runtime.block_on(async {
            snapshot_signal_settings(&state, &signal, u64::MAX - 1)
                .await
                .expect("benchmark warmup signal must pass position gates");
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(
                    snapshot_signal_settings(&state, &signal, u64::MAX - 1)
                        .await
                        .expect("benchmark signal must pass position gates"),
                );
            }
            started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64
        });
        results.push((
            format!("signal settings snapshot ({retained_orders} retained orders)"),
            us_per_operation,
        ));
    }
    results
}

fn benchmark_snapshot_state(settings: Settings) -> AutoTraderState {
    let (state_updates, _) = tokio::sync::broadcast::channel(1);
    let path = std::env::temp_dir().join(format!(
        "auto-trader-lock-bench-{}.json",
        std::process::id()
    ));
    AutoTraderState {
        latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
        strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
        latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_worker_decisions: Arc::new(Mutex::new(BTreeMap::new())),
        seen_worker_decision_events: Arc::new(Mutex::new(BTreeMap::new())),
        enabled: Arc::new(AtomicBool::new(true)),
        settings: Arc::new(Mutex::new(settings)),
        settings_persist_notify: Arc::new(Notify::new()),
        price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
        state_updates,
        live_profile_updates: tokio::sync::broadcast::channel(1).0,
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
        daily_signal_notify: Arc::new(Notify::new()),
        pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
        live_signal_notify: Arc::new(Notify::new()),
        audit_tx: AuditQueue::new(1, 1),
        execution_audit_tx: AuditQueue::new(1, 1),
        audit_health: Arc::new(AtomicU64::new(3)),
        audit_failure_latched: Arc::new(AtomicBool::new(false)),
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        stale_signal_candidates_total: Arc::new(AtomicU64::new(0)),
        stale_signal_cycles_total: Arc::new(AtomicU64::new(0)),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path: path.clone(),
        dated_carry: Arc::new(Mutex::new(DatedCarryBook::default())),
        dated_carry_work_lock: Arc::new(Mutex::new(())),
        dated_carry_path: path.with_extension("dated_carry.json"),
    }
}
