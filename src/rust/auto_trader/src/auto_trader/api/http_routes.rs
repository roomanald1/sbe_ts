pub fn router() -> Router {
    let json_settings_path = settings_path();
    let settings_path = settings_binary_path(&json_settings_path);
    let binary_state = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| rmp_serde::from_slice::<Settings>(&bytes).ok());
    let loaded_binary = binary_state.is_some();
    let stored_state = if binary_state.is_none() {
        std::fs::read(&json_settings_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    } else {
        None
    };
    let mut settings = binary_state
        .or_else(|| {
            stored_state
                .as_ref()
                .and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok())
        })
        .unwrap_or_default();
    migrate_closed_trade_metrics(&mut settings);
    if stored_state.as_ref().is_some_and(|value| {
        value.get("strategies").is_none() && value.get("research_strategies").is_none()
    }) {
        if let Some(strategy) = stored_state
            .as_ref()
            .and_then(|value| value.get("strategy"))
            .and_then(|value| serde_json::from_value::<Strategy>(value.clone()).ok())
        {
            settings.strategies = vec![strategy];
        }
    }
    // The previous release defaulted this new cap to three, which can strand existing books above the cap.
    if settings.max_open_positions == 3 {
        settings.max_open_positions = default_max_open_positions();
    }
    if stored_state
        .as_ref()
        .is_some_and(|value| value.get("realized_pnl").is_none())
    {
        settings.realized_pnl = settings
            .positions
            .values()
            .map(|position| position.realized_pnl)
            .sum();
    }
    for (symbol, position) in settings.positions.iter_mut() {
        if position.trade_id.is_none() {
            position.trade_id = position.entry_decision_id.clone();
        }
        if position.quantity.abs() > 1e-12 && position.strategy.is_none() {
            position.strategy = strategy_from_entry_context(&position.entry_context);
        }
        if position.quantity.abs() > 1e-12
            && position.entries > settings.max_entries_per_auto_position
        {
            let expected_strategy = position.strategy.as_deref();
            let actual_entry_count = settings
                .orders
                .iter()
                .filter(|order| {
                    order.symbol == *symbol
                        && matches!(order.purpose.as_str(), "signal" | "scout")
                        && order.filled_quantity > 1e-12
                })
                .filter(|order| match expected_strategy {
                    Some(strategy) => order.strategy.as_deref() == Some(strategy),
                    None => order.strategy.as_deref().is_none_or(|strategy| {
                        strategy == "generic-insight-signals" || strategy == "live-insights"
                    }),
                })
                .filter_map(|order| order.decision_id.as_ref())
                .collect::<BTreeSet<_>>()
                .len();
            position.entries = actual_entry_count
                .max(1)
                .min(settings.max_entries_per_auto_position as usize)
                as u32;
        } else if position.quantity.abs() > 1e-12 && position.entries == 0 {
            position.entries = 1;
        }
    }
    let legacy_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| (symbol.clone(), position.clone()))
        .collect::<Vec<_>>();
    for (symbol, position) in legacy_positions {
        let strategy = position
            .strategy
            .clone()
            .unwrap_or_else(|| "live-insights".into());
        let lots = settings
            .strategy_positions
            .entry(symbol.clone())
            .or_default();
        if lots.is_empty() {
            lots.insert(strategy, position.clone());
        }
        let mut entries_by_category = BTreeMap::<String, u32>::new();
        for (strategy, lot) in lots.iter() {
            let category = strategy_category_key(strategy_category(Some(strategy)));
            *entries_by_category.entry(category.to_owned()).or_default() += lot.entries;
        }
        if let Some(aggregate) = settings.positions.get_mut(&symbol) {
            aggregate.entries_by_category = entries_by_category;
        }
    }
    for lots in settings.strategy_positions.values_mut() {
        for position in lots.values_mut() {
            if position.trade_id.is_none() {
                position.trade_id = position.entry_decision_id.clone();
            }
        }
    }
    let strategy_by_symbol = settings
        .positions
        .iter()
        .filter_map(|(symbol, position)| {
            position
                .strategy
                .as_ref()
                .map(|strategy| (symbol.clone(), strategy.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for order in &mut settings.orders {
        if order.purpose == "stop-loss"
            && order.strategy.as_deref() == Some("generic-insight-signals")
        {
            if let Some(strategy) = strategy_by_symbol.get(&order.symbol) {
                order.strategy = Some(strategy.clone());
            }
        }
    }
    if let Err(error) = persist_settings(&settings_path, &settings) {
        crate::service_log!(
            "ERROR",
            "Could not persist AutoTrader state migrations: {}",
            error.1
        );
    } else if !loaded_binary && stored_state.is_some() {
        if let Err(error) = std::fs::remove_file(&json_settings_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::service_log!(
                    "WARN",
                    "Could not remove migrated JSON AutoTrader state {}: {error}",
                    json_settings_path.display()
                );
            }
        }
    }
    settings.used_notional_today = allocated_notional(&settings);
    if settings.utc_day != utc_day() {
        settings.utc_day = utc_day();
    }
    let enabled = std::env::var("AUTO_TRADER_ENABLED")
        .map(|value| value == "true")
        .unwrap_or(settings.enabled);
    settings.enabled = enabled;
    let (audit_tx, mut audit_rx) = mpsc::channel(256);
    let (execution_audit_tx, mut execution_audit_rx) = mpsc::unbounded_channel();
    let audit_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                Some(payload) = execution_audit_rx.recv() => store_execution_audit_until_saved(&audit_client, &payload).await,
                Some(payload) = audit_rx.recv() => { store_auto_trader_audit(&audit_client, &payload).await; },
                else => break,
            }
        }
    });
    let carry_path = dated_carry_path(&json_settings_path);
    let carry_book = std::fs::read(&carry_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DatedCarryBook>(&bytes).ok())
        .unwrap_or_default();
    let (state_updates, _) = tokio::sync::broadcast::channel(64);
    let state = AutoTraderState {
        // Every worker shares the cycle lock. Bound upstream calls so an
        // unresponsive position manager cannot stall signal decisions forever.
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        latest_daily_strategy_snapshot: Arc::new(Mutex::new(None)),
        strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        settings_persist_notify: Arc::new(Notify::new()),
        state_updates,
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
        daily_signal_notify: Arc::new(Notify::new()),
        pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
        live_signal_notify: Arc::new(Notify::new()),
        signal_book_prewarms: Arc::new(Mutex::new(BTreeMap::new())),
        audit_tx,
        execution_audit_tx,
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path,
        dated_carry: Arc::new(Mutex::new(carry_book)),
        dated_carry_work_lock: Arc::new(Mutex::new(())),
        dated_carry_path: carry_path,
    };
    start_settings_persistence_worker(&state);
    let shadow_replay_state = state.clone();
    tokio::spawn(async move {
        for attempt in 0..12 {
            if replay_recent_scout_shadows(&shadow_replay_state).await {
                break;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            if attempt == 11 {
                crate::service_log!("WARN", "Recent Scout shadow replay stopped after repeated Insights connection failures");
            }
        }
    });
    let rates_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = refresh_usd_rates(&rates_worker_state).await {
                crate::service_log!(
                    "WARN",
                    "USD allocation rates refresh failed; using cached rates: {}",
                    error.1
                );
            }
        }
    });

    let signal_worker_state = state.clone();
    tokio::spawn(async move {
        let mut prefer_daily = true;
        loop {
            let daily_notification = signal_worker_state.daily_signal_notify.notified();
            let live_notification = signal_worker_state.live_signal_notify.notified();
            tokio::pin!(daily_notification);
            tokio::pin!(live_notification);

            let pending = if prefer_daily {
                let daily = {
                    let mut queue = signal_worker_state.pending_daily_signals.lock().await;
                    take_next_pending_signal(&mut queue)
                };
                if let Some(dashboard) = daily {
                    Some((PendingSignalKind::Daily, dashboard))
                } else {
                    let live = {
                        let mut queue = signal_worker_state.pending_live_signals.lock().await;
                        take_next_pending_signal(&mut queue)
                    };
                    live.map(|dashboard| (PendingSignalKind::Live, dashboard))
                }
            } else {
                let live = {
                    let mut queue = signal_worker_state.pending_live_signals.lock().await;
                    take_next_pending_signal(&mut queue)
                };
                if let Some(dashboard) = live {
                    Some((PendingSignalKind::Live, dashboard))
                } else {
                    let daily = {
                        let mut queue = signal_worker_state.pending_daily_signals.lock().await;
                        take_next_pending_signal(&mut queue)
                    };
                    daily.map(|dashboard| (PendingSignalKind::Daily, dashboard))
                }
            };

            if let Some((kind, dashboard)) = pending {
                prefer_daily = kind == PendingSignalKind::Live;
                let enabled = {
                    let settings = signal_worker_state.settings.lock().await;
                    signal_worker_state.enabled.load(Ordering::Relaxed)
                        && match kind {
                            PendingSignalKind::Daily => {
                                settings.strategies.iter().any(|strategy| {
                                    !strategy.is_live_insight() && !strategy.is_dated_carry()
                                })
                            }
                            PendingSignalKind::Live => {
                                settings.signal_strategies_enabled
                                    || settings
                                        .strategies
                                        .iter()
                                        .any(|strategy| !strategy.is_dated_carry())
                            }
                        }
                };
                if enabled {
                    let worker = match kind {
                        PendingSignalKind::Daily => "Daily strategy",
                        PendingSignalKind::Live => "Live insight",
                    };
                    if let Err(error) = run_cycle(&signal_worker_state, dashboard).await {
                        crate::service_log!("ERROR", "{worker} cycle failed: {}", error.1);
                    }
                }
            } else {
                tokio::select! {
                    _ = &mut daily_notification => {},
                    _ = &mut live_notification => {},
                }
            }
        }
    });

    let position_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if position_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_position_maintenance(&position_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader position maintenance failed: {}",
                        error.1
                    );
                }
            }
        }
    });

    let hedge_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if hedge_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_hedge_evaluation(&hedge_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader hedge evaluation failed: {}",
                        error.1
                    );
                }
            }
        }
    });
    let alternatives = alternative_strategies::router(&state);
    Router::new()
        .route("/positions/auto-trader/stream", get(auto_trader_stream))
        .route("/positions/auto-trader", get(get_status).put(set_enabled))
        .route(
            "/positions/auto-trader/dated-carry",
            get(get_dated_carry).put(update_dated_carry),
        )
        .route(
            "/internal/auto-trader/dated-carry-signal",
            axum::routing::post(receive_dated_carry_signal),
        )
        .route(
            "/internal/auto-trader/daily-strategy-signals",
            axum::routing::post(receive_daily_strategy_signals),
        )
        .route(
            "/internal/auto-trader/shadow-learning-outcome",
            axum::routing::post(receive_shadow_learning_outcome),
        )
        .route(
            "/positions/auto-trader/parameters",
            get(get_decision_parameters),
        )
        .route(
            "/internal/auto-trader/signal",
            axum::routing::post(signal_ready),
        )
        .route(
            "/positions/auto-trader/settings",
            axum::routing::put(update_settings),
        )
        .route("/positions/auto-trader/activity", get(get_activity))
        .with_state(state)
        .merge(alternatives)
}

async fn receive_shadow_learning_outcome(
    State(state): State<AutoTraderState>,
    Json(outcome): Json<ShadowLearningOutcomeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if outcome.decision_id.is_empty()
        || outcome.decision_id.len() > 240
        || !outcome.learning_context.starts_with("scout:")
        || !outcome.simulated_net_pnl_usd.is_finite()
        || outcome.symbol.is_empty()
        || !matches!(outcome.exit_scenario.as_str(), "strategy" | "tight")
        || outcome.scout_parameter_key.is_some() != outcome.scout_parameter_value.is_some()
        || outcome.signal_timestamp_ms > outcome.evaluated_timestamp_ms
        || outcome.evaluated_timestamp_ms > current_time_ms().saturating_add(5_000)
        || current_time_ms().saturating_sub(outcome.signal_timestamp_ms) > 7 * 86_400_000
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid shadow learning outcome".into()));
    }
    let strategy = Strategy::from_name(&outcome.strategy)
        .ok_or((StatusCode::BAD_REQUEST, "Unknown strategy in shadow learning outcome".into()))?;
    let probe_key = match (outcome.scout_parameter_key.as_deref(), outcome.scout_parameter_value) {
        (Some(parameter), Some(value)) => {
            if parameter == StrategyParameterKey::LearningRate.as_str() {
                return Err((StatusCode::BAD_REQUEST, "Learning rate is not a scoutable strategy behavior".into()));
            }
            let Some(spec) = all_strategy_parameter_specs(strategy).into_iter().find(|spec| spec.key.as_str() == parameter) else {
                return Err((StatusCode::BAD_REQUEST, "Unknown strategy parameter in scout outcome".into()));
            };
            let position = (value - spec.min) / spec.step;
            if !value.is_finite() || value < spec.min || value > spec.max
                || (position - position.round()).abs() > 1e-6
            {
                return Err((StatusCode::BAD_REQUEST, "Scout parameter value is outside its registered bounds or step".into()));
            }
            let expected_profile_symbol = if matches!(strategy, Strategy::RiskAdjustedTrendPortfolio | Strategy::RiskAdjustedMonthlyMomentum) {
                "BTC-USDT"
            } else {
                outcome.symbol.as_str()
            };
            let profile_symbol = outcome.scout_parameter_profile_symbol.as_deref().unwrap_or(&outcome.symbol);
            if profile_symbol != expected_profile_symbol || profile_symbol.is_empty() || profile_symbol.len() > 32 {
                return Err((StatusCode::BAD_REQUEST, "Invalid strategy profile symbol".into()));
            }
            Some(strategy_parameter_probe_learning_key(profile_symbol, strategy.as_str(), parameter, value))
        }
        (None, None) => None,
        _ => unreachable!("parameter probe pair was validated"),
    };

    if !outcome.net_return_percent.is_finite()
        || !outcome.taker_fee_rate.is_finite()
        || outcome.taker_fee_rate < 0.0
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid shadow return metrics".into()));
    }
    let outcome_status = if outcome.net_return_percent > 1e-12 {
        "won"
    } else if outcome.net_return_percent < -1e-12 {
        "lost"
    } else {
        "flat"
    };
    let outcome_audit = serde_json::json!({
        "record_schema_version": 1,
        "decision_id": format!("{}|outcome", outcome.decision_id),
        "shadow_decision_id": outcome.decision_id.clone(),
        "stage": "scout-outcome",
        "status": outcome_status,
        "timestamp": outcome.evaluated_timestamp_ms,
        "signal_timestamp": outcome.signal_timestamp_ms,
        "evaluated_timestamp": outcome.evaluated_timestamp_ms,
        "symbol": outcome.symbol.clone(),
        "strategy": outcome.strategy.clone(),
        "scout": true,
        "shadow_exit_scenario": outcome.exit_scenario.clone(),
        "shadow_net_return_percent": outcome.net_return_percent,
        "shadow_taker_fee_rate": outcome.taker_fee_rate,
        "scout_parameter_key": outcome.scout_parameter_key,
        "scout_parameter_value": outcome.scout_parameter_value,
        "scout_parameter_profile_symbol": outcome.scout_parameter_profile_symbol,
    });
    let mut settings = state.settings.lock().await;
    if settings.shadow_learning_outcome_ids.contains(&outcome.decision_id) {
        crate::service_log!("INFO", "Duplicate Scout learning outcome ignored: sample_id={} symbol={}", outcome.decision_id, outcome.symbol);
        drop(settings);
        queue_auto_trader_audit(&state, outcome_audit);
        return Ok(StatusCode::NO_CONTENT);
    }
    if let Some(probe_key) = probe_key {
        // Keep the exact probe cohort for parameter attribution.
        record_learning_outcome(settings.learning.entry(probe_key).or_default(), outcome.simulated_net_pnl_usd);
    } else {
        let stats = settings.learning.entry(outcome.learning_context).or_default();
        record_learning_outcome(stats, outcome.simulated_net_pnl_usd);
    }
    // Every scout exit also contributes to the strategy-level profile, including
    // parameter-probe samples, so shadow results can ease sparse trade learning.
    let strategy_profile_symbol = outcome.scout_parameter_profile_symbol.as_deref().unwrap_or(&outcome.symbol);
    let strategy_key = strategy_shadow_learning_key(
        strategy_profile_symbol,
        strategy.as_str(),
        &outcome.exit_scenario,
    );
    record_learning_outcome(
        settings.learning.entry(strategy_key).or_default(),
        outcome.simulated_net_pnl_usd,
    );
    if !outcome.sample_group_id.is_empty() {
        let group_finished = settings.shadow_sample_active_scenarios
            .get_mut(&outcome.sample_group_id)
            .is_some_and(|scenarios| {
                scenarios.remove(&outcome.exit_scenario);
                scenarios.is_empty()
            });
        if group_finished {
            settings.shadow_sample_active_scenarios.remove(&outcome.sample_group_id);
        }
    }
    settings.shadow_learning_outcome_ids.insert(outcome.decision_id.clone());
    while settings.shadow_learning_outcome_ids.len() > 20_000 {
        let Some(oldest) = settings.shadow_learning_outcome_ids.iter().next().cloned() else {
            break;
        };
        settings.shadow_learning_outcome_ids.remove(&oldest);
    }
    drop(settings);
    request_settings_persist(&state)?;
    crate::service_log!("INFO", "Scout learning outcome recorded: sample_id={} symbol={} strategy={} scenario={} net_return_percent={} simulated_pnl_usd={}", outcome.decision_id, outcome.symbol, outcome.strategy, outcome.exit_scenario, outcome.net_return_percent, outcome.simulated_net_pnl_usd);
    queue_auto_trader_audit(&state, outcome_audit);
    Ok(StatusCode::NO_CONTENT)
}

async fn auto_trader_stream(State(state): State<AutoTraderState>, websocket: axum::extract::ws::WebSocketUpgrade) -> axum::response::Response {
    websocket.on_upgrade(move |socket| async move {
        use axum::extract::ws::Message;
        use futures_util::{SinkExt, StreamExt};
        let mut updates = state.state_updates.subscribe();
        let (mut sender, mut receiver) = socket.split();
        let mut periodic = tokio::time::interval(Duration::from_secs(15));
        periodic.tick().await;
        let mut previous = auto_trader_stream_snapshot(&state).await;
        let initial = serde_json::json!({ "type": "snapshot", "state": previous }).to_string();
        if sender.send(Message::Text(initial.into())).await.is_err() { return; }
        loop {
            let should_refresh = tokio::select! {
                update = updates.recv() => match update {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = periodic.tick() => true,
                incoming = receiver.next() => if incoming.is_none() { break } else { false },
            };
            if should_refresh {
                let snapshot = auto_trader_stream_snapshot(&state).await;
                let mut changes = serde_json::Map::new();
                if let (Some(current), Some(old)) = (snapshot.as_object(), previous.as_object()) {
                    for (key, value) in current {
                        if old.get(key) != Some(value) { changes.insert(key.clone(), value.clone()); }
                    }
                }
                previous = snapshot;
                if !changes.is_empty() {
                    let payload = serde_json::json!({ "type": "delta", "changes": changes }).to_string();
                    if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                }
            }
        }
    })
}

async fn auto_trader_stream_snapshot(state: &AutoTraderState) -> serde_json::Value {
    let status = get_status(State(state.clone())).await.0;
    let activity = get_activity(State(state.clone())).await.0;
    let parameters = get_decision_parameters(State(state.clone())).await.0;
    let dated_carry = get_dated_carry(State(state.clone())).await.0;
    let insights_url = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let audit = match state.client.get(format!("{insights_url}/signals/auto-trader-execution-audit?limit=500")).send().await {
        Ok(response) if response.status().is_success() => response.json::<serde_json::Value>().await.ok(),
        _ => None,
    };
    serde_json::json!({ "status": status, "activity": activity, "parameters": parameters, "dated_carry": dated_carry, "execution_audit": audit.unwrap_or_else(|| serde_json::json!([])) })
}

async fn signal_ready(
    State(state): State<AutoTraderState>,
    Json(dashboard): Json<Dashboard>,
) -> StatusCode {
    static INGRESS_REQUESTS: AtomicU64 = AtomicU64::new(0);
    static INGRESS_SYMBOLS: AtomicU64 = AtomicU64::new(0);
    static LAST_INGRESS_LOG_MS: AtomicU64 = AtomicU64::new(0);

    let incoming_symbols = dashboard.best.len() as u64;
    let settings = state.settings.lock().await;
    let generic_enabled = state.enabled.load(Ordering::Relaxed)
        && (settings.signal_strategies_enabled
            || settings
                .strategies
                .iter()
                .any(|strategy| strategy.is_live_insight())
            || settings
                .scout_strategies
                .iter()
                .any(|strategy| strategy.is_live_insight()));
    if generic_enabled {
        let mut queue = state.pending_live_signals.lock().await;
        let mut prewarm_symbols = Vec::new();
        for signal in dashboard.best.iter().cloned() {
            let mut selected_live = settings
                .strategies
                .iter()
                .filter(|strategy| strategy.is_live_insight())
                .collect::<Vec<_>>();
            selected_live.extend(
                settings
                    .scout_strategies
                    .iter()
                    .filter(|strategy| strategy.is_live_insight()),
            );
            let has_subscription = if selected_live.is_empty() && settings.signal_strategies_enabled
            {
                Strategy::any_live_strategy_receives(&signal)
            } else {
                selected_live
                    .iter()
                    .any(|strategy| strategy.receives_live_insight(&signal))
            };
            if !has_subscription {
                continue;
            }
            let symbol = signal.symbol.clone();
            if !signal.stale {
                prewarm_symbols.push(symbol.clone());
            }
            queue_pending_signal(
                &mut queue,
                PendingSignalKind::Live,
                symbol,
                Dashboard {
                    best: vec![signal],
                    walk_forward: dashboard.walk_forward.clone(),
                    missed_opportunity_bands: dashboard.missed_opportunity_bands.clone(),
                    markets: dashboard.markets.clone(),
                    queued_at: None,
                    queue_wait_ms: None,
                },
            );
        }
        drop(queue);
        let now = Instant::now();
        let mut prewarms = state.signal_book_prewarms.lock().await;
        prewarms
            .retain(|_, requested_at| now.duration_since(*requested_at) < Duration::from_secs(300));
        let position_url = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".to_owned());
        for symbol in prewarm_symbols {
            let should_request = prewarms.get(&symbol).is_none_or(|requested_at| {
                now.duration_since(*requested_at) >= Duration::from_secs(60)
            });
            if !should_request {
                continue;
            }
            prewarms.insert(symbol.clone(), now);
            let client = state.client.clone();
            let url = format!("{position_url}/internal/market-book/{symbol}/subscribe");
            let requested_symbols = Arc::clone(&state.signal_book_prewarms);
            tokio::spawn(async move {
                let succeeded = client
                    .post(url)
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success());
                if !succeeded {
                    requested_symbols.lock().await.remove(&symbol);
                }
            });
        }
        drop(prewarms);
        state.live_signal_notify.notify_one();
    }
    let requests = INGRESS_REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
    let symbols = INGRESS_SYMBOLS.fetch_add(incoming_symbols, Ordering::Relaxed) + incoming_symbols;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last_logged = LAST_INGRESS_LOG_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last_logged) >= 60_000
        && LAST_INGRESS_LOG_MS
            .compare_exchange(last_logged, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        crate::service_log!(
            "INFO",
            "AutoTrader signal ingress acknowledged: generic_enabled={generic_enabled} cumulative_requests={requests} cumulative_symbols={symbols}"
        );
    }
    StatusCode::ACCEPTED
}

fn migrate_closed_trade_metrics(settings: &mut Settings) {
    if settings.closed_trade_metrics_version >= 3 {
        return;
    }
    let count = settings.closed_trade_returns_percent.len();
    if settings.closed_trade_timestamps_ms.len() != count
        || settings.closed_trade_realized_usd.len() != count
        || settings.closed_trade_fees_usd.len() != count
    {
        settings.closed_trade_returns_percent.clear();
        settings.closed_trade_timestamps_ms.clear();
        settings.closed_trade_realized_usd.clear();
        settings.closed_trade_fees_usd.clear();
    } else {
        let mut valid = Vec::with_capacity(count);
        for index in 0..count {
            let return_percent = settings.closed_trade_returns_percent[index];
            let realized = settings.closed_trade_realized_usd[index];
            let fees = settings.closed_trade_fees_usd[index];
            if return_percent.is_finite()
                && return_percent >= -100.0
                && realized.is_finite()
                && fees.is_finite()
            {
                valid.push((
                    return_percent,
                    settings.closed_trade_timestamps_ms[index],
                    realized,
                    fees,
                ));
            }
        }
        settings.closed_trade_returns_percent = valid.iter().map(|item| item.0).collect();
        settings.closed_trade_timestamps_ms = valid.iter().map(|item| item.1).collect();
        settings.closed_trade_realized_usd = valid.iter().map(|item| item.2).collect();
        settings.closed_trade_fees_usd = valid.iter().map(|item| item.3).collect();
    }
    settings.closed_trade_metrics_version = 3;
}

#[cfg(test)]
mod closed_trade_metrics_migration_tests {
    use super::*;

    #[test]
    fn clears_legacy_outcomes_without_changing_ledger_pnl() {
        let mut settings = Settings {
            closed_trade_metrics_version: 0,
            closed_trade_returns_percent: vec![-2405.0],
            closed_trade_timestamps_ms: vec![123],
            closed_trade_realized_usd: vec![-0.89],
            closed_trade_fees_usd: vec![1.99],
            daily_realized_pnl: -11.15,
            ..Settings::default()
        };
        migrate_closed_trade_metrics(&mut settings);
        assert!(settings.closed_trade_returns_percent.is_empty());
        assert!(settings.closed_trade_timestamps_ms.is_empty());
        assert!(settings.closed_trade_realized_usd.is_empty());
        assert!(settings.closed_trade_fees_usd.is_empty());
        assert_eq!(settings.daily_realized_pnl, -11.15);
        migrate_closed_trade_metrics(&mut settings);
        assert_eq!(settings.closed_trade_metrics_version, 3);
    }
}
