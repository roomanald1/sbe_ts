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
        market_state: None,
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
            false,
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

/// Benchmark the decision-parameters route's learning-state snapshot, profile
/// construction, walk-forward evidence lookup, and JSON encoding.
pub fn benchmark_decision_parameters_path(
    iterations: usize,
    retained_variants: usize,
) -> (f64, f64, f64, usize, usize) {
    use std::{hint::black_box, time::Instant};

    assert!(iterations > 0, "benchmark iterations must be positive");
    assert!(retained_variants <= 20_000, "fixture supports at most 20,000 variants");
    let mut settings = Settings::default();
    // Model a persisted learning state with many historical evidence entries;
    // only a small subset belongs to the profiles returned by this endpoint.
    for index in 0..20_000 {
        let symbol = format!("BENCH-{}-USDT", index % 16);
        settings.learned_walk_forward_evidence.insert(
            format!("historical-{index}|live-momentum"),
            strategy::StrategyWalkForwardEvidence {
                evidence_contract_version: 2,
                estimator: strategy::StrategyWalkForwardEstimator::CompletedOutcomeClusters,
                history_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                cost_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                search_family_coverage: strategy::StrategyWalkForwardCoverage::Unknown,
                point_in_time_cutoff_timestamp_ms: None,
                final_holdout_status: strategy::StrategyWalkForwardValidationStatus::NotValidated,
                oos_fold_count: 0,
                spa_p_value: None,
                sample_count: index,
                positive_net_rate: Some(0.51),
                average_net_return_percent: Some(0.12),
                last_sample_timestamp_ms: index as u64,
                expectancy_ci_low_percent: None,
                expectancy_ci_high_percent: None,
                effective_sample_size: Some(index as f64),
                window_count: 0,
                max_drawdown_percent: None,
                downside_loss_percent: None,
                evidence_scope: strategy::StrategyWalkForwardEvidenceScope::Unknown,
            },
        );
        if index < retained_variants {
            settings.learned_profiles.entry(format!("{symbol}:live-momentum")).or_insert_with(|| {
                strategy::StrategyProfile {
                    profile_id: format!("{symbol}:live-momentum"),
                    version: 1,
                    symbol: symbol.clone(),
                    strategy: "live-momentum".into(),
                    parameters: BTreeMap::new(),
                    learning_samples: 0,
                    positive_samples: 0,
                    realized_pnl: 0.0,
                }
            });
            settings.learned_scout_parameter_variants.push(strategy::ScoutParameterVariant {
                symbol,
                strategy: "live-momentum".into(),
                parameter: "minimum_momentum".into(),
                candidate_value: 0.01,
                sample_count: index,
                minimum_samples: 10,
                average_net_improvement_percent: 0.02,
                lower_90_one_sided_improvement_percent: Some(0.01),
                upper_90_one_sided_improvement_percent: Some(0.03),
                paired_baseline_increase_samples: 5,
                paired_baseline_decrease_samples: 4,
                paired_baseline_same_samples: 1,
                paired_baseline_unknown_samples: 0,
                paired_baseline_delta_mean: Some(0.02),
                paired_baseline_delta_samples: 10,
                promoted: false,
                rejected: false,
            });
        }
    }
    let symbols = strategy_profile_symbols(&settings);
    let strategies = Strategy::all()
        .filter(|strategy| !strategy.is_legacy_composite())
        .collect::<Vec<_>>();
    let mut output_bytes = 0;
    let mut compact_bytes = 0;
    let mut output_variants = 0;
    let snapshot_time = std::time::Duration::ZERO;
    let mut profile_time = std::time::Duration::ZERO;
    let mut json_time = std::time::Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        let mut profiles = strategy_parameter_profiles_for(
            black_box(&settings),
            symbols.clone(),
            strategies.clone(),
        );
        for profile in &mut profiles {
            let profile_strategy = Strategy::from_name(&profile.strategy).unwrap_or_default();
            profile.walk_forward_evidence = walk_forward_evidence_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
            profile.walk_forward_evidence_by_regime = walk_forward_evidence_by_regime_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
        }
        output_variants = profiles
            .iter()
            .flat_map(|profile| &profile.parameters)
            .map(|parameter| parameter.scout_variants.len())
            .sum();
        profile_time += started.elapsed();

        let started = Instant::now();
        let encoded = serde_json::to_vec(black_box(&profiles)).unwrap();
        output_bytes = encoded.len();
        black_box(encoded);
        json_time += started.elapsed();

        let compact_started = Instant::now();
        let catalog = strategies.iter().map(|strategy| (strategy.as_str().to_owned(),
            strategy.parameter_specs().iter().map(|spec| (spec.key.as_str(), spec.label, spec.default, spec.min, spec.max, spec.step, spec.unit)).collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>();
        let values = profiles.iter().map(|profile| (&profile.symbol, &profile.strategy,
            profile.learning_samples, profile.positive_rate, profile.realized_pnl,
            profile.walk_forward_evidence.as_ref(), &profile.walk_forward_evidence_by_regime,
            profile.parameters.iter().map(|parameter| (&parameter.key, parameter.value, parameter.learned_value, parameter.configured_value, parameter.overridden)).collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        let compact = serde_json::to_vec(black_box(&(catalog, values))).unwrap();
        compact_bytes = compact.len();
        black_box(compact);
        black_box(compact_started.elapsed());
    }
    let to_us = |elapsed: std::time::Duration| elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64;
    (
        to_us(snapshot_time),
        to_us(profile_time),
        to_us(json_time),
        output_bytes.saturating_sub(compact_bytes),
        output_variants,
    )
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
            latest_worker_diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
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
