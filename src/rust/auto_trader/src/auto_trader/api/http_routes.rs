pub fn router() -> Router {
    let settings_path = settings_path();
    let stored_state = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let mut settings = stored_state
        .as_ref()
        .and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok())
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
    let carry_path = dated_carry_path(&settings_path);
    let carry_book = std::fs::read(&carry_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DatedCarryBook>(&bytes).ok())
        .unwrap_or_default();
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
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_signals: Arc::new(Mutex::new(BTreeMap::new())),
        signal_notify: Arc::new(Notify::new()),
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

    let generic_worker_state = state.clone();
    tokio::spawn(async move {
        loop {
            let notification = generic_worker_state.signal_notify.notified();
            tokio::pin!(notification);
            let pending = {
                let mut queue = generic_worker_state.pending_signals.lock().await;
                take_next_pending_signal(&mut queue)
            };
            if let Some(dashboard) = pending {
                let enabled = {
                    let settings = generic_worker_state.settings.lock().await;
                    generic_worker_state.enabled.load(Ordering::Relaxed)
                        && (settings.signal_strategies_enabled
                            || settings
                                .strategies
                                .iter()
                                .any(|strategy| !strategy.is_dated_carry()))
                };
                if enabled {
                    if let Err(error) = run_cycle(&generic_worker_state, dashboard).await {
                        crate::service_log!("ERROR", "Generic insight cycle failed: {}", error.1);
                    }
                }
            } else {
                notification.await;
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
        .with_state(state)
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
                .any(|strategy| strategy.is_live_insight()));
    if generic_enabled {
        let mut queue = state.pending_signals.lock().await;
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
        state.signal_notify.notify_one();
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
    // Older windows contain cumulative symbol fees and residual-position returns.
    // Clear them once so exit adaptation never learns from those invalid samples.
    if settings.closed_trade_metrics_version >= 2 {
        return;
    }
    settings.closed_trade_returns_percent.clear();
    settings.closed_trade_timestamps_ms.clear();
    settings.closed_trade_realized_usd.clear();
    settings.closed_trade_fees_usd.clear();
    settings.closed_trade_metrics_version = 2;
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
        assert_eq!(settings.closed_trade_metrics_version, 2);
    }
}
