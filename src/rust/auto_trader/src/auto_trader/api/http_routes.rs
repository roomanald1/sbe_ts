// A negative-evidence pause remains in persisted trader settings until the
// operator resets learning. A rolling Learning snapshot alone cannot clear it.
fn preserve_paused_strategy_multipliers(
    previous: &BTreeMap<String, f64>,
    mut incoming: BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    for (strategy, multiplier) in previous {
        if multiplier.is_finite() && *multiplier <= 0.0 {
            incoming.insert(strategy.clone(), 0.0);
        }
    }
    incoming
}

fn next_profile_registration_for_feedback(
    registrations: &[strategies::StrategyProfileRegistration],
    feedback: Option<&FeedbackEvent>,
) -> Option<strategies::StrategyProfileRegistration> {
    match feedback {
        None => registrations.first().cloned(),
        Some(FeedbackEvent::TradeOutcome { profile_id, .. } | FeedbackEvent::ScoutOutcome { profile_id, .. }) =>
            registrations.iter().find(|registration| &registration.profile.profile_id == profile_id).cloned(),
    }
}

#[cfg(test)]
mod strategy_pause_snapshot_tests {
    use super::*;

    #[test]
    fn pause_survives_rolling_evidence_and_disappearing_profiles_until_reset() {
        let prior = BTreeMap::from([
            ("live-momentum".to_owned(), 0.0),
            ("live-trade-flow".to_owned(), 0.8),
        ]);
        let incoming = BTreeMap::from([
            ("live-momentum".to_owned(), 1.0),
            ("live-trade-flow".to_owned(), 1.2),
        ]);
        let merged = preserve_paused_strategy_multipliers(&prior, incoming.clone());
        assert_eq!(merged["live-momentum"], 0.0);
        assert_eq!(merged["live-trade-flow"], 1.2);
        assert_eq!(preserve_paused_strategy_multipliers(&merged, BTreeMap::new())["live-momentum"], 0.0);
        assert_eq!(preserve_paused_strategy_multipliers(&BTreeMap::new(), incoming)["live-momentum"], 1.0);
    }

    #[test]
    fn feedback_is_not_blocked_by_unrelated_profile_registrations() {
        let registration = |id: &str| strategies::StrategyProfileRegistration {
            profile: strategies::StrategyProfile { profile_id: id.into(), version: 0, symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(), parameters: BTreeMap::new(), learning_samples: 0,
                positive_samples: 0, realized_pnl: 0.0 }, specs: vec![],
        };
        let registrations = vec![registration("unrelated"), registration("needed")];
        let event = FeedbackEvent::TradeOutcome { event_id: "trade".into(), observed_at_ms: 1,
            learning_context: None, symbol: "BTC-USDT".into(), strategy: "live-momentum".into(),
            profile_id: "needed".into(), parameters: BTreeMap::new(), specs: vec![], scout: false,
            realized_pnl_usd: -1.0, net_return_percent: -0.1, threshold_observations: vec![] };
        assert_eq!(next_profile_registration_for_feedback(&registrations, Some(&event)).unwrap().profile.profile_id, "needed");
        assert!(next_profile_registration_for_feedback(&registrations[..1], Some(&event)).is_none());
        assert_eq!(next_profile_registration_for_feedback(&registrations, None).unwrap().profile.profile_id, "unrelated");
    }
}

pub fn router() -> Router {
    let state_file_path = settings_path();
    let settings_path = settings_database_path(&state_file_path);
    let database_state = read_settings_database(&settings_path).unwrap_or_else(|error| {
        panic!(
            "Could not read AutoTrader SQLite state {}: {error}",
            settings_path.display()
        )
    });
    let mut settings = database_state.unwrap_or_default();
    migrate_closed_trade_metrics(&mut settings);
    // The previous release defaulted this new cap to three, which can strand existing books above the cap.
    if settings.max_open_positions == 3 {
        settings.max_open_positions = default_max_open_positions();
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
                        strategy == "generic-insight-signals" || strategy == "unattributed"
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
            .unwrap_or_else(|| "unattributed".into());
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
            "Could not persist AutoTrader SQLite state: {}",
            error.1
        );
    }
    settings.used_notional_today = allocated_notional(&settings);
    if settings.utc_day != utc_day() {
        settings.utc_day = utc_day();
    }
    let enabled = std::env::var("AUTO_TRADER_ENABLED")
        .map(|value| value == "true")
        .unwrap_or(settings.enabled);
    settings.enabled = enabled;
    let audit_tx = AuditQueue::new(DECISION_AUDIT_QUEUE_CAPACITY);
    let execution_audit_tx = AuditQueue::new(EXECUTION_AUDIT_QUEUE_CAPACITY);
    let audit_health = Arc::new(AtomicU64::new(3));
    let audit_failure_latched = Arc::new(AtomicBool::new(false));
    let execution_queue = execution_audit_tx.clone();
    let execution_health = Arc::clone(&audit_health);
    tokio::spawn(async move {
        loop {
            let payload = execution_queue.recv().await;
            store_execution_audit_until_saved(&payload, &execution_health, 2).await;
            if execution_queue.depth() == 0 {
                execution_health.fetch_or(2, Ordering::Relaxed);
            }
        }
    });
    let decision_queue = audit_tx.clone();
    let decision_health = Arc::clone(&audit_health);
    tokio::spawn(async move {
        loop {
            let payload = decision_queue.recv().await;
            store_execution_audit_until_saved(&payload, &decision_health, 1).await;
            if decision_queue.depth() == 0 {
                decision_health.fetch_or(1, Ordering::Relaxed);
            }
        }
    });
    let carry_legacy_path = std::env::var_os("POSITIONS_FILE")
        .map(PathBuf::from)
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.join("auto_trader_state.dated_carry.json"))
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| dated_carry_legacy_path(&state_file_path));
    let carry_database_book = read_dated_carry_database(&settings_path)
        .unwrap_or_else(|error| panic!("Could not read AutoTrader dated-carry state: {error}"));
    let carry_book = match carry_database_book {
        Some(book) => book,
        None => {
            let legacy_bytes = std::fs::read(&carry_legacy_path).ok();
            let legacy = legacy_bytes
                .as_deref()
                .and_then(|bytes| serde_json::from_slice::<DatedCarryBook>(&bytes).ok());
            let was_migrated = legacy.is_some();
            let book = legacy.unwrap_or_default();
            if legacy_bytes.is_some() || !carry_legacy_path.exists() {
                persist_dated_carry_book(&settings_path, &book)
                    .unwrap_or_else(|error| panic!("Could not migrate dated-carry state to SQLite: {error}"));
                if was_migrated {
                    let _ = std::fs::remove_file(&carry_legacy_path);
                }
            }
            book
        }
    };
    let (state_updates, _) = tokio::sync::broadcast::channel(64);
    let state = AutoTraderState {
        latest_daily_strategy_snapshot: Arc::new(Mutex::new(None)),
        latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
        strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
        latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        settings_persist_notify: Arc::new(Notify::new()),
        price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
        state_updates,
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
        latest_live_insights_context: Arc::new(Mutex::new(SharedLiveInsightsContext::default())),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        learning_response_cache: Arc::new(Mutex::new(LearningResponseCache::default())),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
        daily_signal_notify: Arc::new(Notify::new()),
        pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
        live_signal_notify: Arc::new(Notify::new()),
        signal_book_prewarms: Arc::new(Mutex::new(BTreeMap::new())),
        audit_tx,
        execution_audit_tx,
        audit_health,
        audit_failure_latched,
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        stale_signal_candidates_total: Arc::new(AtomicU64::new(0)),
        stale_signal_cycles_total: Arc::new(AtomicU64::new(0)),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path: settings_path.clone(),
        dated_carry: Arc::new(Mutex::new(carry_book)),
        dated_carry_work_lock: Arc::new(Mutex::new(())),
        dated_carry_path: settings_path.clone(),
    };
    start_settings_persistence_worker(&state);
    start_learning_feedback_worker(&state);
    start_post_exit_observation_worker(&state);
    let live_context_state = Arc::clone(&state.latest_live_insights_context);
    tokio::spawn(async move {
        let refresh_seconds = std::env::var("AUTO_TRADER_LIVE_CONTEXT_REFRESH_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);
        let mut ticker = interval(Duration::from_secs(refresh_seconds));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let response = tokio::time::timeout(
                Duration::from_secs(3),
                crate::service_rpc::insights_request("GET", "/internal/live-context", None),
            )
            .await;
            match response {
                Ok(Ok(response)) if (200..300).contains(&response.status) => {
                    match serde_json::from_value::<SharedLiveInsightsContext>(response.body) {
                        Ok(context) => *live_context_state.lock().await = context,
                        Err(error) => crate::service_log!(
                            "WARN",
                            "Could not decode shared Insights live context: {error}"
                        ),
                    }
                }
                Ok(Ok(response)) => crate::service_log!(
                    "WARN",
                    "Insights live context returned status {}",
                    response.status
                ),
                Ok(Err(error)) => crate::service_log!(
                    "WARN",
                    "Could not refresh shared Insights live context: {error}"
                ),
                Err(_) => crate::service_log!("WARN", "Insights live context refresh timed out"),
            }
        }
    });
    let carry_sync_book = state.dated_carry.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let book = carry_sync_book.lock().await.clone();
            if let Err(error) = sync_dated_carry_ledger(&book).await {
                crate::service_log!("WARN", "Dated-carry position-manager sync failed: {error}");
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
                        take_next_pending_signal_batch(
                            &mut queue,
                            MAX_LIVE_SIGNALS_PER_CYCLE,
                        )
                    };
                    live.map(|dashboard| (PendingSignalKind::Live, dashboard))
                }
            } else {
                let live = {
                    let mut queue = signal_worker_state.pending_live_signals.lock().await;
                    take_next_pending_signal_batch(
                        &mut queue,
                        MAX_LIVE_SIGNALS_PER_CYCLE,
                    )
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
    Router::new()
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
        .route("/positions/auto-trader/learning-summary", get(get_learning_summary))
        .with_state(state)
}


async fn get_learning_summary() -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let response = crate::service_rpc::learning_request(
        strategies::ScoutWorkerHttpMethod::Get,
        "/summary",
        None,
    ).await.map_err(|error| (axum::http::StatusCode::BAD_GATEWAY, error.to_string()))?;
    Ok(axum::Json(response.body))
}

fn feedback_event_id(event: &FeedbackEvent) -> &str {
    match event {
        FeedbackEvent::ScoutOutcome { event_id, .. }
        | FeedbackEvent::TradeOutcome { event_id, .. } => event_id,
    }
}

#[cfg(test)]
fn scout_feedback_parameter_keys(
    strategy: Strategy,
    parameter_key: Option<&str>,
) -> BTreeSet<String> {
    match parameter_key {
        Some(key) if key.starts_with("combo:") => parse_scout_combination(key, strategy)
            .map(|parameters| parameters.into_keys().collect())
            .unwrap_or_default(),
        Some(key)
            if strategy.parameter_specs()
                .iter()
                .any(|spec| spec.key.as_str() == key) =>
        {
            BTreeSet::from([key.to_owned()])
        }
        _ => BTreeSet::new(),
    }
}

fn start_learning_feedback_worker(state: &AutoTraderState) {
    let learning_url = std::env::var("LEARNING_RSOCKET_URL").ok();
    if learning_url.is_none() { return; }
    let profile_learning_url = learning_url.clone();

    let publisher_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let registration = {
                let settings = publisher_state.settings.lock().await;
                next_profile_registration_for_feedback(&settings.profile_registration_outbox, settings.feedback_outbox.first())
            };
            if let Some(registration) = registration {
                let response = crate::service_rpc::learning_request(
                    strategies::ScoutWorkerHttpMethod::Post,
                    "/profiles",
                    Some(serde_json::to_value(&registration).unwrap_or(serde_json::Value::Null)),
                ).await;
                if response.is_ok_and(|response| (200..300).contains(&response.status)) {
                    let profile_id = registration.profile.profile_id;
                    let mut settings = publisher_state.settings.lock().await;
                    settings.profile_registration_outbox.retain(|pending| pending.profile.profile_id != profile_id);
                    settings.registered_profile_ids.insert(profile_id);
                    drop(settings);
                    if let Err(error) = request_settings_persist(&publisher_state) {
                        crate::service_log!("WARN", "Could not persist profile-registration acknowledgement: {}", error.1);
                    }
                }
                continue;
            }
            let event = publisher_state.settings.lock().await.feedback_outbox.first().cloned();
            if let Some(event) = event {
                let response = crate::service_rpc::learning_request(
                    strategies::ScoutWorkerHttpMethod::Post,
                    "/feedback",
                    Some(serde_json::to_value(&event).unwrap_or(serde_json::Value::Null)),
                ).await;
                if response.is_ok_and(|response| (200..300).contains(&response.status)) {
                    let event_id = feedback_event_id(&event).to_owned();
                    let mut settings = publisher_state.settings.lock().await;
                    settings.feedback_outbox.retain(|pending| feedback_event_id(pending) != event_id);
                    drop(settings);
                    if let Err(error) = request_settings_persist(&publisher_state) {
                        crate::service_log!("WARN", "Could not persist Scout/trade feedback acknowledgement: {}", error.1);
                    }
                }
                continue;
            }
            let execution_event = publisher_state.settings.lock().await.execution_feedback_outbox.first().cloned();
            if let Some(event) = execution_event {
                let response = crate::service_rpc::learning_request(
                    strategies::ScoutWorkerHttpMethod::Post,
                    "/execution-outcomes",
                    Some(serde_json::to_value(&event).unwrap_or(serde_json::Value::Null)),
                ).await;
                if response.is_ok_and(|response| (200..300).contains(&response.status)) {
                    let event_id = event.event_id.clone();
                    let mut settings = publisher_state.settings.lock().await;
                    settings.execution_feedback_outbox.retain(|pending| pending.event_id != event_id);
                    drop(settings);
                    if let Err(error) = request_settings_persist(&publisher_state) {
                        crate::service_log!("WARN", "Could not persist execution-learning acknowledgement: {}", error.1);
                    }
                }
            }
        }
    });

    if profile_learning_url.is_some() {
        let profile_state = state.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(3));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Ok(response) = crate::service_rpc::learning_request(
                    strategies::ScoutWorkerHttpMethod::Get,
                    "/decision-snapshot",
                    None,
                ).await else { continue };
                if !(200..300).contains(&response.status) { continue; }
                let Ok(snapshot) = serde_json::from_value::<strategies::LearnedProfileSnapshot>(response.body) else { continue };
                let mut settings = profile_state.settings.lock().await;
                let mut changed = false;
                if settings.learned_scout_parameter_variants != snapshot.scout_parameter_variants {
                    settings.learned_scout_parameter_variants = snapshot.scout_parameter_variants;
                    changed = true;
                }
                for profile in snapshot.profiles {
                    let key = format!("{}:{}", profile.symbol, profile.strategy);
                    settings.registered_profile_ids.insert(profile.profile_id.clone());
                    if settings.learned_profiles.get(&key).is_none_or(|previous| profile.version > previous.version) {
                        settings.learned_profiles.insert(key, profile);
                        changed = true;
                    }
                }
                let multipliers = preserve_paused_strategy_multipliers(
                    &settings.learned_strategy_size_multipliers,
                    snapshot.strategy_size_multipliers,
                );
                if settings.learned_strategy_size_multipliers != multipliers {
                    settings.learned_strategy_size_multipliers = multipliers;
                    changed = true;
                }
                if settings.recent_profile_context_returns != snapshot.recent_profile_context_returns {
                    settings.recent_profile_context_returns = snapshot.recent_profile_context_returns;
                    changed = true;
                }
                if settings.contextual_stop_loss_percents != snapshot.contextual_stop_loss_percents {
                    settings.contextual_stop_loss_percents = snapshot.contextual_stop_loss_percents;
                    changed = true;
                }
                if settings.learned_context_stats != snapshot.context_learning {
                    settings.learned_context_stats = snapshot.context_learning;
                    changed = true;
                }
                if settings.learned_execution_stats != snapshot.execution_learning {
                    settings.learned_execution_stats = snapshot.execution_learning;
                    changed = true;
                }
                drop(settings);
                if changed {
                    if let Err(error) = request_settings_persist(&profile_state) {
                        crate::service_log!("WARN", "Could not persist learned profile snapshot: {}", error.1);
                    }
                }
            }
        });
    }
}

async fn signal_ready(
    State(state): State<AutoTraderState>,
    Json(mut dashboard): Json<Dashboard>,
) -> StatusCode {
    static INGRESS_REQUESTS: AtomicU64 = AtomicU64::new(0);
    static INGRESS_SYMBOLS: AtomicU64 = AtomicU64::new(0);
    static LAST_INGRESS_LOG_MS: AtomicU64 = AtomicU64::new(0);

    {
        let context = state.latest_live_insights_context.lock().await;
        dashboard.walk_forward = Arc::clone(&context.walk_forward);
        dashboard.missed_opportunity_bands = Arc::clone(&context.missed_opportunity_bands);
    }
    let incoming_symbols = dashboard.best.len() as u64;
    let settings = state.settings.lock().await;
    let generic_enabled = state.enabled.load(Ordering::Relaxed)
        && (settings.signal_strategies_enabled
            || settings
                .strategies
                .iter()
                .any(|strategy| strategy.is_live_insight()));
    if generic_enabled {
        let mut queue = state.pending_live_signals.lock().await;
        let mut prewarm_symbols = Vec::new();
        for signal in dashboard.best.iter().cloned() {
            let selected_live = settings
                .strategies
                .iter()
                .filter(|strategy| strategy.is_live_insight())
                .collect::<Vec<_>>();
            let has_subscription = if selected_live.is_empty() && settings.signal_strategies_enabled
            {
                Strategy::any_live_strategy_receives(&signal)
            } else {
                selected_live.iter().any(|strategy| strategy.receives_live_insight(&signal))
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
                    walk_forward: Arc::clone(&dashboard.walk_forward),
                    missed_opportunity_bands: Arc::clone(&dashboard.missed_opportunity_bands),
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
        for symbol in prewarm_symbols {
            let should_request = prewarms.get(&symbol).is_none_or(|requested_at| {
                now.duration_since(*requested_at) >= Duration::from_secs(60)
            });
            if !should_request {
                continue;
            }
            prewarms.insert(symbol.clone(), now);
            let requested_symbols = Arc::clone(&state.signal_book_prewarms);
            tokio::spawn(async move {
                let succeeded = crate::service_rpc::position_request(
                    "POST",
                    &format!("/internal/market-book/{symbol}/subscribe"),
                    None,
                ).await.is_ok();
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

    #[test]
    fn scout_parameter_feedback_targets_only_the_tested_parameter_values() {
        let strategy = Strategy::from_name("live-momentum").unwrap();
        let specs = strategy.parameter_specs();
        let tested = specs[0].key.as_str();
        assert_eq!(
            scout_feedback_parameter_keys(strategy, Some(tested)),
            BTreeSet::from([tested.to_owned()])
        );
        assert!(scout_feedback_parameter_keys(strategy, None).is_empty());

        let full_profile = specs
            .iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let combination = BTreeMap::from([
            (specs[0].key.as_str().to_owned(), specs[0].default),
            (specs[1].key.as_str().to_owned(), specs[1].default),
        ]);
        let key = scouts::combination_key(&combination, &full_profile).unwrap();
        assert_eq!(
            scout_feedback_parameter_keys(strategy, Some(&key)),
            combination.keys().cloned().collect()
        );
    }
}
