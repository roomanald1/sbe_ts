pub fn router() -> Router {
    let settings_path = settings_path();
    let stored_state = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let mut settings = stored_state
        .as_ref()
        .and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok())
        .unwrap_or_default();
    settings.take_profit_percent =
        risk_adjusted_take_profit_percent(settings.stop_loss_percent, settings.risk_appetite);
    if let Err(error) = persist_settings(&settings_path, &settings) {
        crate::service_log!(
            "ERROR",
            "Could not persist risk-adjusted take-profit: {}",
            error.1
        );
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
    for position in settings.positions.values_mut() {
        if position.quantity.abs() > 1e-12 && position.entries == 0 {
            position.entries = 1;
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
    let state = AutoTraderState {
        // Every worker shares the cycle lock. Bound upstream calls so an
        // unresponsive position manager cannot stall signal decisions forever.
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_signals: Arc::new(Mutex::new(BTreeMap::new())),
        signal_notify: Arc::new(Notify::new()),
        audit_tx,
        execution_audit_tx,
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path,
    };
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
        loop {
            let signal_ready = signal_worker_state.signal_notify.notified();
            tokio::pin!(signal_ready);
            let pending = {
                let mut pending_signals = signal_worker_state.pending_signals.lock().await;
                let symbol = pending_signals.keys().next().cloned();
                symbol.and_then(|symbol| pending_signals.remove(&symbol))
            };
            let dashboard = match pending {
                Some(dashboard) => dashboard,
                None => {
                    signal_ready.await;
                    continue;
                }
            };
            if signal_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_cycle(&signal_worker_state, dashboard).await {
                    crate::service_log!("ERROR", "Auto trader signal cycle failed: {}", error.1);
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

    let enabled = state.enabled.load(Ordering::Relaxed);
    let incoming_symbols = dashboard.best.len() as u64;
    let requests = INGRESS_REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
    let symbols = INGRESS_SYMBOLS.fetch_add(incoming_symbols, Ordering::Relaxed) + incoming_symbols;
    let pending_symbols = if enabled {
        let mut pending_signals = state.pending_signals.lock().await;
        for signal in dashboard.best {
            let symbol = signal.symbol.clone();
            pending_signals.insert(
                symbol,
                Dashboard {
                    best: vec![signal],
                    walk_forward: dashboard.walk_forward.clone(),
                    missed_opportunity_bands: dashboard.missed_opportunity_bands.clone(),
                    markets: dashboard.markets.clone(),
                },
            );
        }
        let pending_symbols = pending_signals.len();
        drop(pending_signals);
        state.signal_notify.notify_one();
        pending_symbols
    } else {
        0
    };

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
            if enabled { "INFO" } else { "WARN" },
            "AutoTrader signal ingress: enabled={enabled} cumulative_requests={requests} cumulative_symbols={symbols} pending_symbols={pending_symbols}"
        );
    }
    StatusCode::ACCEPTED
}

async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DecisionParametersResponse> {
    let (learning, active_exit_policy, max_entries_per_auto_position, risk_appetite) = {
        let settings = state.settings.lock().await;
        (
            {
                let mut learning = settings.learning.clone();
                decay_learning_map(&mut learning, current_time_ms());
                learning
            },
            ActiveExitPolicy {
                stop_loss_percent: settings.stop_loss_percent,
                take_profit_percent: settings.take_profit_percent,
            },
            settings.max_entries_per_auto_position,
            settings.risk_appetite,
        )
    };
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
    let contexts = build_context_decision_parameters(&learning, &bands, risk_appetite);
    let thresholds = build_decision_thresholds(risk_appetite, max_entries_per_auto_position);
    let latest_execution = state.latest_execution_parameters.lock().await.clone();
    let refreshed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Json(DecisionParametersResponse {
        refreshed_at,
        thresholds,
        contexts,
        active_exit_policy,
        latest_execution,
    })
}

fn build_context_decision_parameters(
    learning: &BTreeMap<String, LearningStats>,
    bands: &[MissedOpportunityGateBand],
    risk_appetite: f64,
) -> Vec<ContextDecisionParameters> {
    let floor_scale = risk_floor_scale(risk_appetite);
    ["long", "short"]
        .into_iter()
        .flat_map(|side| {
            ["range", "trend", "volatile", "mixed"].map(move |regime| format!("{side}:{regime}"))
        })
        .map(|context| {
            let stats = context_learning_stats(learning, &context);
            let positive_rate =
                (stats.samples > 0).then_some(stats.positive as f64 / stats.samples as f64);
            let strength_floor = (adaptive_missed_opportunity_floor(
                adaptive_strength_floor(learning, &context),
                "strength",
                bands,
            ) * floor_scale)
                .clamp(0.0, 1.0);
            let exploration_score_floor =
                (adaptive_exploration_score_floor(learning, &context, bands) * floor_scale)
                    .max(0.0);
            ContextDecisionParameters {
                exploration_allowed: !context.ends_with(":mixed"),
                context,
                learning_samples: stats.samples,
                positive_rate,
                strength_floor,
                exploration_score_floor,
            }
        })
        .collect()
}

fn build_decision_thresholds(
    risk_appetite: f64,
    max_entries_per_auto_position: u32,
) -> Vec<DecisionParameter> {
    let mut thresholds = Vec::new();
    thresholds.extend(signal_qualification_thresholds(risk_appetite));
    thresholds.extend(scout_qualification_thresholds(risk_appetite));
    thresholds.extend(historical_evidence_thresholds());
    thresholds.extend(counterfactual_relaxation_thresholds());
    thresholds.extend(order_selection_thresholds(risk_appetite));
    thresholds.extend(market_context_thresholds());
    thresholds.extend(learning_thresholds());
    thresholds.extend(risk_and_hedging_thresholds());
    thresholds.extend(risk_and_re_entry_thresholds(max_entries_per_auto_position));
    thresholds.extend(exit_policy_thresholds());
    thresholds
}

fn signal_qualification_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Signal qualification",
            "Maximum signal age",
            MAX_SIGNAL_AGE_SECONDS as f64,
            "seconds",
        ),
        parameter(
            "Signal qualification",
            "Neutral score direction floor",
            NEUTRAL_SCORE_DIRECTION_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Neutral inference maximum range probability",
            NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY,
            "probability",
        ),
        parameter(
            "Signal qualification",
            "Base strict strength floor",
            MIN_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Strict strength floor after poor learning",
            LEARNED_STRENGTH_FLOOR * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Samples before strong negative-evidence adjustment",
            STRONG_NEGATIVE_LEARNING_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Positive rate boundary for strong negative evidence",
            STRONG_NEGATIVE_LEARNING_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Strict strength floor after strong negative evidence",
            STRONG_NEGATIVE_STRENGTH_FLOOR * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Samples before strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Positive rate triggering strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Minimum exploration strength",
            MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Base exploration score floor",
            MIN_EXPLORATION_SCORE * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 10 weak samples",
            EXPLORATION_SCORE_MID_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 20 weak samples",
            EXPLORATION_SCORE_HIGH_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after strong negative evidence",
            STRONG_NEGATIVE_SCORE_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 10-sample adjustment",
            EXPLORATION_SCORE_MID_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 20-sample adjustment",
            EXPLORATION_SCORE_HIGH_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration adjustment positive-rate boundaries",
            EXPLORATION_SCORE_MID_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Exploration stronger adjustment boundary",
            EXPLORATION_SCORE_HIGH_WIN_RATE,
            "rate",
        ),
    ]
}

fn scout_qualification_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![parameter(
        "Scout qualification",
        "Neutral scout score floor",
        SCOUT_NEUTRAL_SCORE_FLOOR * risk_floor_scale(risk_appetite),
        "score",
    )]
}

fn historical_evidence_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Historical evidence",
            "Minimum strict walk-forward samples",
            MIN_SIDE_WALK_FORWARD_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum direction match rate",
            MIN_SIDE_DIRECTION_MATCH_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum positive net rate",
            MIN_SIDE_POSITIVE_NET_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum 95% positive-rate lower bound",
            MIN_SIDE_POSITIVE_NET_LOWER_95,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration evidence samples",
            MIN_EXPLORATION_EVIDENCE_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration positive rate",
            NEGATIVE_EVIDENCE_FLOOR,
            "rate",
        ),
    ]
}

fn counterfactual_relaxation_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Counterfactual relaxation",
            "Minimum resolved stop/target samples",
            COUNTERFACTUAL_RELAXATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum positive stop/target rate",
            COUNTERFACTUAL_RELAXATION_POSITIVE_RATE,
            "rate",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed strength threshold",
            COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD,
            "strength",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed score threshold",
            COUNTERFACTUAL_SCORE_MIN_THRESHOLD,
            "score",
        ),
    ]
}

fn order_selection_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Order selection",
            "Risk appetite for aggressive order priority",
            AGGRESSIVE_ORDER_PRIORITY_APPETITE,
            "%",
        ),
        parameter(
            "Order selection",
            "Range probability for post-only limit",
            (ORDER_POST_ONLY_RANGE + 0.20 * risk_appetite / 100.0).clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "Aggressor cost for post-only limit",
            ORDER_POST_ONLY_AGGRESSOR_COST_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for GTD",
            (ORDER_GTD_RANGE + 0.30 * risk_appetite / 100.0).clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for GTD lower bound",
            ORDER_GTD_MIN_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for limit",
            (ORDER_LIMIT_RANGE + 0.30 * risk_appetite / 100.0).clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for limit",
            ORDER_GTD_MAX_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Volatility for stop-limit",
            ORDER_STOP_LIMIT_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Trend for stop-limit",
            ORDER_STOP_LIMIT_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Trend for stop-market",
            ORDER_STOP_MARKET_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Strength for stop-market",
            ORDER_STOP_MARKET_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Pressure for market order",
            ORDER_MARKET_PRESSURE,
            "pressure",
        ),
        parameter(
            "Order selection",
            "Strength for market order",
            ORDER_MARKET_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Momentum for IOC",
            ORDER_IOC_MOMENTUM_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for IOC",
            ORDER_IOC_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Strength for VWAP",
            ORDER_VWAP_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Range probability for TWAP",
            ORDER_TWAP_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "Volatility for trailing stop",
            ORDER_TRAILING_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for FOK",
            ORDER_FOK_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
    ]
}

fn market_context_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Market context",
            "Range regime probability",
            CONTEXT_RANGE_MIN_PROBABILITY,
            "probability",
        ),
        parameter(
            "Market context",
            "Trend regime volatility units",
            CONTEXT_TREND_MIN_VOLATILITY_UNITS,
            "volatility units",
        ),
        parameter(
            "Market context",
            "Volatile regime threshold",
            CONTEXT_VOLATILE_MIN_PERCENT,
            "%",
        ),
    ]
}

fn learning_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Learning",
            "Minimum execution learning samples",
            MIN_EXECUTION_LEARNING_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Learning",
            "Minimum profitable order-type hit rate",
            0.55,
            "rate",
        ),
        parameter(
            "Learning",
            "Number of parameter profiles",
            PARAMETER_PROFILE_COUNT as f64,
            "profiles",
        ),
    ]
}

fn risk_and_hedging_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Risk and hedging",
            "Portfolio shock threshold",
            PORTFOLIO_SHOCK_PERCENT * 100.0,
            "%",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation samples",
            MIN_HEDGE_CORRELATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation",
            MIN_HEDGE_CORRELATION,
            "correlation",
        ),
    ]
}

fn risk_and_re_entry_thresholds(max_entries_per_auto_position: u32) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Risk and re-entry",
            "Signal cooldown",
            SIGNAL_COOLDOWN_MS as f64 / 1000.0,
            "seconds",
        ),
        parameter(
            "Risk and re-entry",
            "Cooldown after losing stop",
            POST_STOP_REENTRY_COOLDOWN_MS as f64 / 1000.0,
            "seconds",
        ),
        parameter(
            "Risk and re-entry",
            "Required re-entry strength improvement",
            MATERIAL_STRENGTH_CHANGE,
            "strength",
        ),
        parameter(
            "Risk and re-entry",
            "Entries per position",
            max_entries_per_auto_position as f64,
            "entries",
        ),
        parameter(
            "Risk and re-entry",
            "Deep-book impact ceiling",
            DEEP_BOOK_BAND_BPS,
            "bps",
        ),
        parameter(
            "Risk and re-entry",
            "Maximum depth participation",
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            "%",
        ),
        parameter(
            "Risk and re-entry",
            "Minimum deep-book order",
            MIN_DEEP_BOOK_ORDER_NOTIONAL,
            "USD",
        ),
    ]
}

fn exit_policy_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Exit policy",
            "Safe target minimum reward/risk ratio",
            MIN_EXIT_REWARD_RISK_RATIO,
            "ratio",
        ),
        parameter(
            "Exit policy",
            "Minimum net take-profit at maximum risk appetite",
            SMALL_NET_TAKE_PROFIT_PERCENT,
            "% after fees",
        ),
        parameter(
            "Exit policy",
            "Stop-loss minimum",
            MIN_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Stop-loss maximum",
            MAX_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Take-profit minimum",
            MIN_TAKE_PROFIT_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Take-profit maximum",
            MAX_TAKE_PROFIT_PERCENT,
            "%",
        ),
    ]
}

fn parameter(section: &str, name: &str, value: f64, unit: &str) -> DecisionParameter {
    DecisionParameter {
        section: section.to_owned(),
        name: name.to_owned(),
        value,
        unit: unit.to_owned(),
    }
}
async fn set_enabled(
    State(state): State<AutoTraderState>,
    Json(request): Json<EnabledRequest>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if request.enabled {
        let position_url = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        sync_auto_positions(&state.client, &position_url, &state).await?;
    }
    state.enabled.store(request.enabled, Ordering::Relaxed);
    let mut settings = state.settings.lock().await;
    settings.enabled = request.enabled;
    if let Err(error) = persist_settings(&state.settings_path, &settings) {
        crate::service_log!(
            "ERROR",
            "Could not persist Auto Trader enabled state: {}",
            error.1
        );
    }
    Ok(Json(status(&state, &settings)))
}
async fn update_settings(
    State(state): State<AutoTraderState>,
    Json(request): Json<SettingsRequest>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if !request.max_order_notional.is_finite()
        || request.max_order_notional <= 0.0
        || !request.daily_notional_limit.is_finite()
        || request.daily_notional_limit < request.max_order_notional
        || !request.daily_realized_loss_limit.is_finite()
        || request.daily_realized_loss_limit <= 0.0
        || !request.exploration_percent.is_finite()
        || !(0.0..=100.0).contains(&request.exploration_percent)
        || request
            .risk_appetite
            .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        || request.stop_loss_percent.is_some_and(|value| {
            !value.is_finite() || !(MIN_STOP_LOSS_PERCENT..=MAX_STOP_LOSS_PERCENT).contains(&value)
        })
        || request.take_profit_percent.is_some_and(|value| {
            !value.is_finite()
                || !(MIN_TAKE_PROFIT_PERCENT..=MAX_TAKE_PROFIT_PERCENT).contains(&value)
        })
        || request
            .vwap_duration_minutes
            .is_some_and(|value| !(5..=390).contains(&value))
        || request
            .vwap_participation_percent
            .is_some_and(|value| !value.is_finite() || !(1.0..=50.0).contains(&value))
        || request
            .limit_offset_bps
            .is_some_and(|value| !value.is_finite() || !(-10.0..=10.0).contains(&value))
        || request
            .trigger_offset_bps
            .is_some_and(|value| !value.is_finite() || !(5.0..=100.0).contains(&value))
        || request
            .trailing_distance_percent
            .is_some_and(|value| !value.is_finite() || !(0.05..=1.0).contains(&value))
        || request
            .max_entries_per_auto_position
            .is_some_and(|value| !(1..=10).contains(&value))
        || request
            .closed_trade_window
            .is_some_and(|value| !(1..=5000).contains(&value))
        || !(1..=20).contains(&request.max_open_positions)
    {
        return Err((StatusCode::BAD_REQUEST, "Order and total notional limits must be positive with total notional at least the per-order limit; daily loss cap must be positive, exploration must be between 0% and 100%, and max open positions must be between 1 and 20, and entries per position must be between 1 and 10".into()));
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if request.daily_notional_limit < settings.used_notional_today {
        return Err((
            StatusCode::CONFLICT,
            "Total notional limit cannot be less than the Auto Trader's current allocation".into(),
        ));
    }
    settings.max_order_notional = request.max_order_notional;
    settings.daily_notional_limit = request.daily_notional_limit;
    settings.daily_realized_loss_limit = request.daily_realized_loss_limit;
    settings.exploration_percent = request.exploration_percent;
    let recalculate_take_profit =
        request.risk_appetite.is_some() || request.stop_loss_percent.is_some();
    if let Some(value) = request.risk_appetite {
        settings.risk_appetite = value;
    }
    settings.max_open_positions = request.max_open_positions;
    if let Some(value) = request.max_entries_per_auto_position {
        settings.max_entries_per_auto_position = value;
    }
    if let Some(value) = request.stop_loss_percent {
        settings.stop_loss_percent = value;
    }
    if let Some(value) = request.take_profit_percent {
        settings.take_profit_percent = value;
    }
    if recalculate_take_profit {
        settings.take_profit_percent =
            risk_adjusted_take_profit_percent(settings.stop_loss_percent, settings.risk_appetite);
    }
    if let Some(value) = request.vwap_duration_minutes {
        settings.vwap_duration_minutes = value;
    }
    if let Some(value) = request.vwap_participation_percent {
        settings.vwap_participation_percent = value;
    }
    if let Some(value) = request.limit_offset_bps {
        settings.limit_offset_bps = value;
    }
    if let Some(value) = request.trigger_offset_bps {
        settings.trigger_offset_bps = value;
    }
    if let Some(value) = request.trailing_distance_percent {
        settings.trailing_distance_percent = value;
    }
    if let Some(value) = request.closed_trade_window {
        settings.closed_trade_window = value;
        let excess = settings
            .closed_trade_returns_percent
            .len()
            .saturating_sub(value);
        if excess > 0 {
            settings.closed_trade_returns_percent.drain(0..excess);
        }
        let timestamp_excess = settings
            .closed_trade_timestamps_ms
            .len()
            .saturating_sub(value);
        if timestamp_excess > 0 {
            settings
                .closed_trade_timestamps_ms
                .drain(0..timestamp_excess);
        }
    }
    persist_settings(&state.settings_path, &settings)?;
    Ok(Json(status(&state, &settings)))
}
fn status(state: &AutoTraderState, settings: &Settings) -> StatusResponse {
    if let Ok(mut summary) = state.gap_summary.lock() {
        let day = utc_day();
        if summary.utc_day != day {
            *summary = GapSummary {
                utc_day: day,
                ..GapSummary::default()
            };
        }
    }
    StatusResponse {
        enabled: state.enabled.load(Ordering::Relaxed),
        max_order_notional: settings.max_order_notional,
        daily_notional_limit: settings.daily_notional_limit,
        daily_realized_loss_limit: settings.daily_realized_loss_limit,
        exploration_percent: settings.exploration_percent,
        risk_appetite: settings.risk_appetite,
        max_open_positions: settings.max_open_positions,
        max_entries_per_auto_position: settings.max_entries_per_auto_position,
        stop_loss_percent: settings.stop_loss_percent,
        take_profit_percent: settings.take_profit_percent,
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_percent: settings.vwap_participation_percent,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
        closed_trade_window: settings.closed_trade_window,
        used_notional_today: settings.used_notional_today,
        available_notional: (settings.daily_notional_limit - settings.used_notional_today).max(0.0),
        last_decision: settings.last_decision.clone(),
        last_decision_at: settings.last_decision_at,
        decisions_since_trade: state
            .gap_summary
            .lock()
            .map(|summary| summary.decisions)
            .unwrap_or_default(),
        gap_reasons: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .reasons
                    .iter()
                    .map(|(reason, count)| GapReason {
                        reason: reason.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .gate_hits
                    .iter()
                    .map(|(gate, count)| GateHit {
                        gate: gate.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        symbol_gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .symbol_gate_hits
                    .iter()
                    .map(|((symbol, direction, gate), count)| SymbolGateHit {
                        symbol: symbol.clone(),
                        direction: direction.clone(),
                        gate: gate.clone(),
                        count: count.count,
                        last_detail: count.last_detail.clone(),
                        distance_to_pass: count.distance_to_pass,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        latest_signal_diagnostic: state
            .gap_summary
            .lock()
            .map(|summary| summary.latest_detail.clone())
            .unwrap_or_default(),
    }
}
struct ActivityPositionSnapshot {
    orders: Vec<TraderOrder>,
    auto_positions: Vec<(String, TraderPosition)>,
    open_position_symbols: Vec<String>,
}

fn snapshot_activity_positions(settings: &Settings, today_start: u64) -> ActivityPositionSnapshot {
    let orders = settings
        .orders
        .iter()
        .filter(|order| order.timestamp >= today_start)
        .rev()
        .take(MAX_RETAINED_AUTO_TRADER_ORDERS)
        .cloned()
        .map(|mut order| {
            if order.target_price.is_none() && order.purpose == "stop-loss" {
                if let Some(position) = settings
                    .positions
                    .get(&order.symbol)
                    .filter(|position| position.quantity.abs() > 1e-12)
                {
                    let quantity = position.quantity.abs();
                    let loss_fraction = settings.stop_loss_percent / 100.0;
                    let fee_rate = configured_taker_fee_rate();
                    order.target_price = Some(if position.quantity > 0.0 {
                        (quantity * position.average_entry_price * (1.0 - loss_fraction)
                            + position.entry_fees)
                            / (quantity * (1.0 - fee_rate))
                    } else {
                        (quantity * position.average_entry_price * (1.0 + loss_fraction)
                            - position.entry_fees)
                            / (quantity * (1.0 + fee_rate))
                    });
                }
            }
            if order.quantity > 0.0
                && order.filled_quantity >= order.quantity
                && matches!(order.status.as_str(), "armed" | "resting")
            {
                order.status = "filled".into();
            }
            order
        })
        .collect::<Vec<_>>();
    let auto_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| (symbol.clone(), position.clone()))
        .collect::<Vec<_>>();
    let open_position_symbols = auto_positions
        .iter()
        .map(|(symbol, _)| symbol.clone())
        .collect::<Vec<_>>();
    ActivityPositionSnapshot {
        orders,
        auto_positions,
        open_position_symbols,
    }
}

async fn await_activity_realized_pnl(
    client: &reqwest::Client,
    position_url: &str,
    orders: &mut [TraderOrder],
) -> (
    Option<f64>,
    Option<BTreeMap<String, f64>>,
    Vec<PnlBreakdown>,
) {
    let ledger_pnl = fetch_ledger_realized_pnl(client, position_url).await.ok();
    let (realized_pnl, realized_pnl_by_quote, pnl_breakdown) = if let Some(pnl) = ledger_pnl {
        for order in orders.iter_mut() {
            let realized = if order.purpose == "stop-loss" {
                pnl.realized_pnl_by_barrier_created_at
                    .get(&order.timestamp.to_string())
                    .copied()
            } else if order.order_type.starts_with("Twap") || order.order_type.starts_with("Vwap") {
                order
                    .execution_order_id
                    .as_ref()
                    .and_then(|id| pnl.realized_pnl_by_parent_order.get(id))
                    .copied()
            } else {
                pnl.realized_pnl_by_parent_order
                    .get(&order.timestamp.to_string())
                    .copied()
            };
            order.realized_pnl = realized.unwrap_or(0.0);
        }
        let mut grouped = BTreeMap::<(String, String, String), (f64, usize)>::new();
        for order in orders.iter() {
            if order.realized_pnl.abs() <= 1e-12 {
                continue;
            }
            let direction = if order.side.eq_ignore_ascii_case("sell") {
                "LONG"
            } else {
                "SHORT"
            };
            let total = grouped
                .entry((
                    order.symbol.clone(),
                    direction.to_owned(),
                    order.purpose.clone(),
                ))
                .or_default();
            total.0 += order.realized_pnl;
            total.1 += 1;
        }
        let mut breakdown = grouped
            .into_iter()
            .map(
                |((symbol, direction, purpose), (realized_pnl, outcomes))| PnlBreakdown {
                    symbol,
                    direction,
                    purpose,
                    realized_pnl,
                    outcomes,
                },
            )
            .collect::<Vec<_>>();
        breakdown
            .sort_by(|left, right| right.realized_pnl.abs().total_cmp(&left.realized_pnl.abs()));
        (
            pnl.realized_pnl_all_time_usd,
            Some(pnl.realized_pnl_by_quote),
            breakdown,
        )
    } else {
        for order in orders.iter_mut() {
            order.realized_pnl = 0.0;
        }
        (None, None, Vec::new())
    };
    (realized_pnl, realized_pnl_by_quote, pnl_breakdown)
}

async fn calculate_auto_unrealized_pnl(
    auto_positions: &[(String, TraderPosition)],
) -> Option<(f64, BTreeMap<String, f64>)> {
    let unrealized_pnl = if auto_positions.is_empty() {
        Some((0.0, BTreeMap::new()))
    } else {
        let base = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let mut total = 0.0;
        let mut by_quote = BTreeMap::<String, f64>::new();
        let mut complete = true;
        for (symbol, position) in auto_positions {
            let response = client
                .get(format!("{base}/internal/insights-inputs?symbol={symbol}"))
                .send()
                .await;
            let Ok(response) = response else {
                complete = false;
                break;
            };
            let Ok(inputs) = response.error_for_status() else {
                complete = false;
                break;
            };
            let Ok(inputs) = inputs.json::<serde_json::Value>().await else {
                complete = false;
                break;
            };
            let market = &inputs["markets"][&symbol];
            let mark = if position.quantity > 0.0 {
                market["bids"][0]["price"].as_f64()
            } else {
                market["asks"][0]["price"].as_f64()
            };
            let Some(mark) = mark.filter(|price| price.is_finite() && *price > 0.0) else {
                complete = false;
                break;
            };
            let pnl = (mark - position.average_entry_price) * position.quantity
                - position.entry_fees
                - position.quantity.abs() * mark * configured_taker_fee_rate();
            total += pnl;
            let quote = symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or("USD");
            *by_quote.entry(quote.to_owned()).or_default() += pnl;
        }
        complete.then_some((total, by_quote))
    };
    unrealized_pnl
}

struct ActivityLearningMetrics {
    exit_learning_samples: usize,
    closed_trade_average_return_percent: Option<f64>,
    closed_trade_wins: usize,
    closed_trade_losses: usize,
    closed_trade_flat: usize,
    closed_trade_realized_usd: Option<f64>,
    closed_trade_fees_usd: Option<f64>,
    learning_samples: usize,
    learning_win_rate: Option<f64>,
    preferred_order_type: Option<String>,
    parameter_profile_samples: [usize; 3],
    scout_open_positions: usize,
    scout_learning_samples: usize,
    scout_learning_win_rate: Option<f64>,
    scout_learning_realized_pnl: f64,
}

fn calculate_activity_learning_metrics(
    settings: &Settings,
    today_start: u64,
) -> ActivityLearningMetrics {
    let today_closed_returns = settings
        .closed_trade_returns_percent
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            settings
                .closed_trade_timestamps_ms
                .get(*index)
                .is_some_and(|timestamp| *timestamp >= today_start)
        })
        .map(|(_, value)| *value)
        .collect::<Vec<_>>();
    let exit_learning_samples = today_closed_returns.len();
    let closed_trade_average_return_percent = (exit_learning_samples > 0)
        .then(|| today_closed_returns.iter().sum::<f64>() / exit_learning_samples as f64);
    let closed_trade_wins = today_closed_returns
        .iter()
        .filter(|value| **value > 1e-12)
        .count();
    let closed_trade_losses = today_closed_returns
        .iter()
        .filter(|value| **value < -1e-12)
        .count();
    let closed_trade_flat = exit_learning_samples - closed_trade_wins - closed_trade_losses;
    let today_closed_indices = settings
        .closed_trade_timestamps_ms
        .iter()
        .enumerate()
        .filter(|(_, timestamp)| **timestamp >= today_start)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let closed_trade_realized_usd = (settings.closed_trade_timestamps_ms.len()
        == settings.closed_trade_returns_percent.len()
        && settings.closed_trade_realized_usd.len() == settings.closed_trade_returns_percent.len()
        && !today_closed_indices.is_empty())
    .then(|| {
        today_closed_indices
            .iter()
            .map(|index| settings.closed_trade_realized_usd[*index])
            .sum()
    });
    let closed_trade_fees_usd = (settings.closed_trade_timestamps_ms.len()
        == settings.closed_trade_returns_percent.len()
        && settings.closed_trade_fees_usd.len() == settings.closed_trade_returns_percent.len()
        && !today_closed_indices.is_empty())
    .then(|| {
        today_closed_indices
            .iter()
            .map(|index| settings.closed_trade_fees_usd[*index])
            .sum()
    });
    let today = utc_day();
    let learning_samples = settings
        .learning
        .values()
        .map(|stats| learning_stats_for_day(stats, &today).samples)
        .sum();
    let scout_open_positions = settings
        .positions
        .values()
        .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
        .count();
    let scout_learning = settings
        .learning
        .iter()
        .filter(|(key, _)| key.starts_with("scout:"))
        .map(|(_, stats)| learning_stats_for_day(stats, &today))
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        });
    let scout_learning_win_rate = (scout_learning.samples > 0)
        .then_some(scout_learning.positive as f64 / scout_learning.samples as f64);
    let learning_positive = settings
        .learning
        .values()
        .map(|stats| learning_stats_for_day(stats, &today).positive)
        .sum::<usize>();
    let preferred_order_type = ALL_ORDER_TYPES
        .iter()
        .filter_map(|kind| {
            let suffix = format!(":{kind:?}");
            let stats = settings
                .learning
                .iter()
                .filter(|(key, _)| key.ends_with(&suffix))
                .map(|(_, stats)| learning_stats_for_day(stats, &today))
                .fold(LearningStats::default(), |mut total, stats| {
                    total.samples += stats.samples;
                    total.positive += stats.positive;
                    total.realized_pnl += stats.realized_pnl;
                    total
                });
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && stats.positive as f64 / stats.samples as f64 >= 0.55)
                .then_some((
                    format!("{kind:?}"),
                    stats.realized_pnl / stats.samples as f64,
                ))
        })
        .filter(|(_, pnl)| *pnl > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind);
    let learning_win_rate =
        (learning_samples > 0).then_some(learning_positive as f64 / learning_samples as f64);
    let scout_learning_samples = scout_learning.samples;
    let scout_learning_realized_pnl = scout_learning.realized_pnl;
    let parameter_profile_samples = std::array::from_fn(|profile| {
        settings
            .learning
            .iter()
            .filter(|(key, _)| key.contains(&format!(":P{profile}:")))
            .map(|(_, stats)| learning_stats_for_day(stats, &today).samples)
            .sum()
    });
    ActivityLearningMetrics {
        exit_learning_samples,
        closed_trade_average_return_percent,
        closed_trade_wins,
        closed_trade_losses,
        closed_trade_flat,
        closed_trade_realized_usd,
        closed_trade_fees_usd,
        learning_samples,
        learning_win_rate,
        preferred_order_type,
        parameter_profile_samples,
        scout_open_positions,
        scout_learning_samples,
        scout_learning_win_rate,
        scout_learning_realized_pnl,
    }
}

async fn get_activity(State(state): State<AutoTraderState>) -> Json<ActivityResponse> {
    let today_start = utc_day_start_ms();
    let ActivityPositionSnapshot {
        mut orders,
        auto_positions,
        open_position_symbols,
    } = {
        let settings = state.settings.lock().await;
        snapshot_activity_positions(&settings, today_start)
    };
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let (realized_pnl, realized_pnl_by_quote, pnl_breakdown) =
        await_activity_realized_pnl(&state.client, &position_url, &mut orders).await;
    let unrealized_pnl = calculate_auto_unrealized_pnl(&auto_positions).await;
    let learning = {
        let settings = state.settings.lock().await;
        calculate_activity_learning_metrics(&settings, today_start)
    };
    let processing_times = state.insight_processing_ms.lock().await.clone();
    let insight_processing_latest_ms = processing_times.last().copied();
    let mut sorted_processing_times = processing_times;
    sorted_processing_times.sort_by(f64::total_cmp);
    let insight_processing_samples = sorted_processing_times.len();
    let insight_processing_average_ms = (insight_processing_samples > 0)
        .then(|| sorted_processing_times.iter().sum::<f64>() / insight_processing_samples as f64);
    let insight_processing_p95_ms = (insight_processing_samples > 0).then(|| {
        sorted_processing_times[(95 * insight_processing_samples)
            .div_ceil(100)
            .saturating_sub(1)]
    });
    let lock_wait_samples = state.cycle_lock_wait_ms.lock().await.clone();
    let cycle_lock_wait_ms = lock_wait_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    let lock_hold_samples = state
        .cycle_lock_hold_ms
        .lock()
        .ok()
        .map(|samples| samples.clone())
        .unwrap_or_default();
    let cycle_lock_hold_ms = lock_hold_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    let (unrealized_pnl, unrealized_pnl_by_quote) = unrealized_pnl
        .map(|(total, by_quote)| (Some(total), Some(by_quote)))
        .unwrap_or((None, None));
    Json(ActivityResponse {
        orders,
        open_position_symbols,
        pnl_breakdown,
        realized_pnl,
        realized_pnl_by_quote,
        unrealized_pnl,
        unrealized_pnl_by_quote,
        exit_learning_samples: learning.exit_learning_samples,
        closed_trade_average_return_percent: learning.closed_trade_average_return_percent,
        closed_trade_wins: learning.closed_trade_wins,
        closed_trade_losses: learning.closed_trade_losses,
        closed_trade_flat: learning.closed_trade_flat,
        closed_trade_realized_usd: learning.closed_trade_realized_usd,
        closed_trade_fees_usd: learning.closed_trade_fees_usd,
        learning_samples: learning.learning_samples,
        learning_win_rate: learning.learning_win_rate,
        preferred_order_type: learning.preferred_order_type,
        parameter_profile_samples: learning.parameter_profile_samples,
        scout_open_positions: learning.scout_open_positions,
        scout_max_active_positions: SCOUT_MAX_ACTIVE_POSITIONS,
        scout_learning_samples: learning.scout_learning_samples,
        scout_learning_win_rate: learning.scout_learning_win_rate,
        scout_learning_realized_pnl: learning.scout_learning_realized_pnl,
        insight_processing_latest_ms,
        insight_processing_average_ms,
        insight_processing_p95_ms,
        insight_processing_samples,
        cycle_lock_wait_ms,
        cycle_lock_hold_ms,
    })
}
