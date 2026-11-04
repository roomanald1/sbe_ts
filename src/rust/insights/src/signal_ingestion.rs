fn encode_market_levels(levels: &[(f64, f64)]) -> Vec<serde_json::Value> {
    levels
        .iter()
        .filter(|(price, size)| {
            price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
        })
        .map(|(price, size)| serde_json::json!({"price": price, "size": size}))
        .collect()
}

fn market_value(
    timestamp: u64,
    best_bid: f64,
    best_ask: f64,
    bids: &[(f64, f64)],
    asks: &[(f64, f64)],
) -> Option<serde_json::Value> {
    (best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid).then(
        || {
            serde_json::json!({
                "bids": encode_market_levels(bids),
                "asks": encode_market_levels(asks),
                "updated_at_ms": timestamp,
            })
        },
    )
}


#[derive(serde::Deserialize)]
struct ScoutShadowRegistration {
    decision_id: String,
    symbol: String,
    timestamp: u64,
    direction: String,
    entry_price: f64,
    strategy: String,
    exit_scenario: String,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    profit_protection_activation_percent: f64,
    profit_protection_giveback_percent: f64,
    profit_protection_minimum_lock_percent: f64,
    #[serde(default)]
    scout_parameter_key: Option<String>,
    #[serde(default)]
    scout_parameter_value: Option<f64>,
    #[serde(default)]
    scout_minimum_hold_days: Option<f64>,
    #[serde(default)]
    scout_maximum_hold_days: Option<f64>,
    #[serde(default)]
    scout_comparison_id: Option<String>,
    #[serde(default)]
    scout_comparison_role: Option<String>,
}

async fn register_scout_shadow(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(sample): Json<ScoutShadowRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    let now = now_ms();
    if sample.decision_id.is_empty() || sample.decision_id.len() > 240
        || sample.symbol.is_empty() || sample.symbol.len() > 32
        || sample.timestamp > now + 5_000 || now.saturating_sub(sample.timestamp) > 7 * 86_400_000
        || !sample.entry_price.is_finite() || sample.entry_price <= 0.0
        || !sample.stop_loss_percent.is_finite() || sample.stop_loss_percent <= 0.0
        || !sample.take_profit_percent.is_finite() || sample.take_profit_percent <= 0.0
        || !sample.profit_protection_activation_percent.is_finite() || sample.profit_protection_activation_percent < 0.0
        || !sample.profit_protection_giveback_percent.is_finite() || sample.profit_protection_giveback_percent < 0.0
        || !sample.profit_protection_minimum_lock_percent.is_finite() || sample.profit_protection_minimum_lock_percent < 0.0
        || sample.strategy.is_empty()
        || !matches!(sample.exit_scenario.as_str(), "strategy" | "tight" | "wide")
        || sample.scout_minimum_hold_days.is_some_and(|days| !days.is_finite() || !(1.0..=365.0).contains(&days))
        || sample.scout_maximum_hold_days.is_some_and(|days| !days.is_finite() || !(1.0..=365.0).contains(&days))
        || !matches!(sample.direction.as_str(), "bullish" | "bearish")
        || sample.scout_comparison_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 240)
        || sample.scout_comparison_role.as_deref().is_some_and(|role| !matches!(role, "baseline" | "variant"))
        || sample.scout_comparison_id.is_some() != sample.scout_comparison_role.is_some()
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid scout shadow sample".to_owned()));
    }
    let mut engines = runtime.engines.lock().await;
    let engine = engines.entry(sample.symbol.clone()).or_default();
    let decision_id = sample.decision_id.clone();
    let symbol = sample.symbol.clone();
    let strategy = sample.strategy.clone();
    let exit_scenario = sample.exit_scenario.clone();
    let timestamp = sample.timestamp;
    if !engine.track_scout_shadow(
        sample.decision_id, sample.symbol, sample.strategy,
        sample.timestamp, sample.direction, sample.entry_price,
        sample.stop_loss_percent, sample.take_profit_percent,
        sample.profit_protection_activation_percent,
        sample.profit_protection_giveback_percent,
        sample.profit_protection_minimum_lock_percent,
        sample.scout_parameter_key,
        sample.scout_parameter_value,
        sample.scout_minimum_hold_days,
        sample.scout_maximum_hold_days,
    ) {
        crate::service_log!("INFO", "Scout shadow registration ignored as duplicate: sample_id={} symbol={}", decision_id, symbol);
        return Ok(StatusCode::NO_CONTENT);
    }
    crate::service_log!("INFO", "Scout shadow registered: sample_id={} symbol={} strategy={} exit_scenario={} age_ms={}", decision_id, symbol, strategy, exit_scenario, now.saturating_sub(timestamp));
    Ok(StatusCode::ACCEPTED)
}

#[derive(serde::Deserialize)]
struct ScoutStrategyStatusUpdate {
    active_strategy_symbols: Vec<(String, String)>,
    #[serde(default)]
    active_parameter_profiles: Vec<(String, String, String, String)>,
    enabled_strategies: Vec<String>,
}

async fn update_scout_strategy_status(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(update): Json<ScoutStrategyStatusUpdate>,
) -> StatusCode {
    let active = update.active_strategy_symbols.into_iter().collect::<BTreeSet<_>>();
    let active_parameter_profiles = update.active_parameter_profiles.into_iter().collect::<BTreeSet<_>>();
    let enabled = update.enabled_strategies.into_iter().collect::<BTreeSet<_>>();
    let mut engines = runtime.engines.lock().await;
    for engine in engines.values_mut() {
        engine.update_scout_strategy_state(&active, &active_parameter_profiles, &enabled);
    }
    StatusCode::NO_CONTENT
}

async fn ingest_signal_event(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(request): Json<SignalEventRequest>,
) -> Result<Json<Option<SignalSnapshot>>, (StatusCode, String)> {
    if request.symbol.is_empty() || !request.symbol.is_ascii() || request.symbol.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signal symbol".to_owned()));
    }
    // Nonblocking snapshot read: market feed handling never waits on RSS/model work.
    let current_news = Arc::clone(&runtime.news.borrow());
    let current_news_weight = runtime
        .news_learning
        .try_lock()
        .map(|learning| learning.weight)
        .unwrap_or(0.0);
    let mut shadow_learning_outcomes = Vec::new();
    let (snapshot, records, market, event_timestamp) = {
        let event_timestamp = match &request.event {
            SignalFeedEvent::Price { timestamp, .. }
            | SignalFeedEvent::Trade { timestamp, .. }
            | SignalFeedEvent::Book { timestamp, .. } => *timestamp,
        };
        // Preserve the original event ordering and atomic BTC/ETH context snapshot.
        // The sharded caches below are independent of this stateful event sequence.
        let mut engines = runtime.engines.lock().await;
        let btc_context = engines.get("BTC-USDT").map_or((None, None), |reference| {
            reference.return_context(event_timestamp)
        });
        let eth_context = engines.get("ETH-USDT").map_or((None, None), |reference| {
            reference.return_context(event_timestamp)
        });
        let engine = engines.entry(request.symbol.clone()).or_default();
        let mut records = Vec::new();
        let mut market = None;
        let snapshot = match request.event {
            SignalFeedEvent::Price {
                timestamp,
                price,
                volatility_1m,
            } => {
                let (price_count_before, previous_price) = engine.price_state();
                let outcomes = engine.on_price(timestamp, price, volatility_1m);
                let (price_count_after, latest_price) = engine.price_state();
                let accepted = price.is_finite()
                    && price > 0.0
                    && latest_price == Some((timestamp, price))
                    && (price_count_after > price_count_before || previous_price != latest_price);
                if accepted {
                    runtime.latest_prices.insert(request.symbol.clone(), (timestamp, price));
                }
                if !accepted && signal_diagnostic_due(&runtime, &request.symbol, timestamp) {
                    let reason = if !price.is_finite() || price <= 0.0 {
                        "invalid_price"
                    } else {
                        "out_of_order_timestamp"
                    };
                    crate::service_log!(
                        "WARN",
                        "Signal price sample rejected: symbol={} reason={} timestamp={} previous_timestamp={:?} price={} volatility_1m={}",
                        request.symbol,
                        reason,
                        timestamp,
                        previous_price.map(|sample| sample.0),
                        price,
                        volatility_1m
                    );
                }
                for outcome in outcomes {
                    if outcome.scout_shadow_id.is_some() {
                        crate::service_log!("INFO", "Scout shadow completed: sample_id={} symbol={} exit_reason={} age_seconds={}", outcome.scout_shadow_id.as_deref().unwrap_or("unknown"), outcome.evaluation.symbol, outcome.scout_exit_reason.unwrap_or("unknown"), outcome.evaluation.horizon_seconds);
                        shadow_learning_outcomes.push(outcome.evaluation.clone());
                    }
                    if outcome.legacy.horizon_seconds == 60 {
                        if let Ok(mut learning) = runtime.news_learning.try_lock() {
                            if let Some(vote) = learning.pending.remove(&(
                                outcome.legacy.symbol.clone(),
                                outcome.legacy.signal_timestamp,
                            )) {
                                learning.samples += 1;
                                if vote * outcome.legacy.return_percent > 0.0 {
                                    learning.correct += 1;
                                }
                                if learning.samples >= 30 {
                                    let accuracy =
                                        learning.correct as f64 / learning.samples as f64;
                                    learning.weight =
                                        (0.20 * ((accuracy - 0.45) / 0.20)).clamp(0.0, 0.20);
                                }
                            }
                        }
                    }
                    records.push(StoredSignalRecord::Outcome(outcome.legacy));
                    records.push(StoredSignalRecord::Evaluation(outcome.evaluation));
                }
                let should_compute_snapshot = accepted && engine.live_snapshot_due(timestamp);
                let should_capture_research =
                    should_compute_snapshot && engine.research_sample_is_due(timestamp);
                let mut snapshot = should_compute_snapshot
                    .then(|| {
                        engine.snapshot_with_reasons(
                            &request.symbol,
                            timestamp,
                            should_capture_research,
                        )
                    })
                    .flatten();
                if should_compute_snapshot
                    && snapshot.is_none()
                    && signal_diagnostic_due(
                        &runtime,
                        &format!("missing:{}", request.symbol),
                        timestamp,
                    )
                {
                    crate::service_log!(
                        "INFO",
                        "Signal snapshot not ready: symbol={} timestamp={} diagnostics={}",
                        request.symbol,
                        timestamp,
                        engine.input_diagnostic(timestamp)
                    );
                } else if should_compute_snapshot
                    && snapshot.is_some()
                    && signal_diagnostic_due(
                        &runtime,
                        &format!("generated:{}", request.symbol),
                        timestamp,
                    )
                {
                    let age_ms = now_ms().saturating_sub(timestamp);
                    crate::service_log!(
                        "INFO",
                        "Signal snapshot generated: symbol={} event_timestamp={} age_ms={} diagnostics={}",
                        request.symbol,
                        timestamp,
                        age_ms,
                        engine.input_diagnostic(timestamp)
                    );
                }
                if let Some(snapshot) = snapshot.as_mut() {
                    let news_vote =
                        apply_news_signal(snapshot, &current_news, now_ms(), current_news_weight);
                    market =
                        runtime
                            .market_cache
                            .get(&request.symbol)
                            .and_then(|cached| {
                                let (book_timestamp, value) = cached.value();
                                (snapshot.timestamp >= *book_timestamp
                            && snapshot.timestamp.saturating_sub(*book_timestamp) <= 5_000)
                            .then(|| serde_json::json!({ request.symbol.clone(): value.clone() }))
                            });
                    if should_capture_research {
                        engine.research_sample_due(snapshot.timestamp);
                        let features = engine.research_features(
                            &request.symbol,
                            snapshot.timestamp,
                            btc_context,
                            eth_context,
                        );
                        if let Some(book_snapshot) =
                            engine.book_snapshot(&request.symbol, snapshot.timestamp)
                        {
                            records.push(StoredSignalRecord::BookSnapshot(book_snapshot));
                        }
                        records.push(StoredSignalRecord::ResearchFeatures(features));
                        engine.track(snapshot);
                        if let Some(vote) = news_vote {
                            if let Ok(mut learning) = runtime.news_learning.try_lock() {
                                learning
                                    .pending
                                    .insert((snapshot.symbol.clone(), snapshot.timestamp), vote);
                            }
                        }
                        let mut persisted_snapshot = snapshot.clone();
                        // Granular live-only variants are used by AutoTrader and
                        // are not needed in the long-term research snapshot store.
                        persisted_snapshot.components.momentum_by_window.clear();
                        persisted_snapshot.components.trend_volatility_units_by_window.clear();
                        persisted_snapshot.components.trade_flow_by_window.clear();
                        persisted_snapshot.components.book_imbalance_by_depth.clear();
                        persisted_snapshot.components.vwap_dislocation_by_window.clear();
                        persisted_snapshot.components.absorption_pressure_by_window.clear();
                        records.push(StoredSignalRecord::Snapshot(persisted_snapshot));
                    }
                }
                snapshot
            }
            SignalFeedEvent::Trade {
                timestamp,
                price,
                size,
                is_buy,
            } => {
                engine.on_trade(timestamp, price, size, is_buy);
                None
            }
            SignalFeedEvent::Book {
                timestamp,
                best_bid,
                best_ask,
                bid_sizes,
                ask_sizes,
                bid_levels,
                ask_levels,
            } => {
                engine.on_book(
                    timestamp,
                    best_bid,
                    best_ask,
                    &bid_sizes,
                    &ask_sizes,
                    &bid_levels,
                    &ask_levels,
                );
                if let Some(value) =
                    market_value(timestamp, best_bid, best_ask, &bid_levels, &ask_levels)
                {
                    runtime
                        .market_cache
                        .insert(request.symbol.clone(), (timestamp, value.clone()));
                    market = Some(serde_json::json!({ request.symbol.clone(): value }));
                }
                None
            }
        };
        (snapshot, records, market, event_timestamp)
    };
    if snapshot.is_none() {
        if let Some(markets) = market.as_ref() {
            if let Some(mut dashboard) = runtime.signal_push_cache.get_mut(&request.symbol) {
                dashboard["markets"] = markets.clone();
                drop(dashboard);
                let mut should_notify = false;
                runtime
                    .market_push_at
                    .entry(request.symbol.clone())
                    .and_modify(|last| {
                        if event_timestamp < *last
                            || event_timestamp.saturating_sub(*last)
                                >= LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS
                        {
                            *last = event_timestamp;
                            should_notify = true;
                        }
                    })
                    .or_insert_with(|| {
                        should_notify = true;
                        event_timestamp
                    });
                if should_notify {
                    runtime.signal_push_notify.notify_one();
                }
            }
        }
    }
    if !records.is_empty() {
        let store = runtime.store.clone();
        tokio::spawn(async move {
            if let Err(error) = store.append_records(records).await {
                crate::service_log!(
                    "ERROR",
                    "Could not persist signal event off hot path: {error}"
                );
            }
        });
    }
    if !shadow_learning_outcomes.is_empty() {
        let store = runtime.store.clone();
        tokio::spawn(async move {
            publish_shadow_learning_outcomes(&store, shadow_learning_outcomes).await;
        });
    }
    if let Some(snapshot) = snapshot.as_ref() {
        let markets = market.unwrap_or_else(|| serde_json::json!({}));
        // Historical counterfactual analysis scans a large database. Signal
        // ingestion only needs the latest completed result; dashboard reads
        // schedule any refresh in the background.
        let missed_opportunity_bands = runtime.store.cached_missed_opportunity_bands();
        let dashboard = serde_json::json!({
            "best": [snapshot],
            "walkForward": runtime.store.cached_walk_forward(),
            "missedOpportunityBands": missed_opportunity_bands,
            "markets": markets
        });
        runtime
            .signal_push_cache
            .insert(snapshot.symbol.clone(), dashboard);
        runtime.signal_push_notify.notify_one();
        let mut ui_snapshot = snapshot.clone();
        ui_snapshot.components.momentum_by_window.clear();
        ui_snapshot.components.trend_volatility_units_by_window.clear();
        ui_snapshot.components.trade_flow_by_window.clear();
        ui_snapshot.components.book_imbalance_by_depth.clear();
        ui_snapshot.components.vwap_dislocation_by_window.clear();
        ui_snapshot.components.absorption_pressure_by_window.clear();
        let snapshot_json = serde_json::to_value(ui_snapshot).unwrap_or(serde_json::Value::Null);
        runtime
            .ui_snapshot_cache
            .insert(snapshot.symbol.clone(), snapshot_json);
        runtime.ui_snapshot_notify.notify_one();
    }
    Ok(Json(snapshot))
}

async fn publish_shadow_learning_outcomes(store: &SignalStore, outcomes: Vec<SignalEvaluation>) {
    let _permit = SHADOW_LEARNING_DELIVERY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .expect("shadow learning delivery semaphore remains open");
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base =
        std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".to_owned());
    for evaluation in outcomes {
        let decisions = match store
            .load_shadow_decisions_near(
                evaluation.symbol.clone(),
                evaluation.signal_timestamp,
                evaluation.signal_timestamp.saturating_add(1),
            )
            .await
        {
            Ok(decisions) => decisions,
            Err(error) => {
                crate::service_log!(
                    "WARN",
                    "Could not load shadow decisions for learning: {error}"
                );
                continue;
            }
        };
        for decision in decisions {
            let Some(signal_timestamp) = decision["signal_timestamp"].as_u64() else {
                continue;
            };
            if decision["status"] != "shadowed"
                || decision["scout"] != true
                || signal_timestamp > evaluation.signal_timestamp
                || evaluation.signal_timestamp.saturating_sub(signal_timestamp) > 10_000
            {
                continue;
            }
            let direction = match decision["direction"].as_str() {
                Some("bullish") => "bullish",
                Some("bearish") => "bearish",
                _ => continue,
            };
            let Some(decision_id) = decision["decision_id"].as_str() else {
                continue;
            };
            let Some(learning_context) = decision["shadow_learning_context"].as_str() else {
                continue;
            };
            let Some(notional_usd) = decision["shadow_notional_usd"].as_f64() else {
                continue;
            };
            let net_return_percent = if direction == "bullish" {
                evaluation.long_net_percent
            } else {
                evaluation.short_net_percent
            };
            let Some(net_return_percent) = net_return_percent else {
                continue;
            };
            let simulated_net_pnl_usd = notional_usd * net_return_percent / 100.0;
            if !simulated_net_pnl_usd.is_finite() {
                continue;
            }
            let payload = serde_json::json!({
                "decision_id": decision_id,
                "learning_context": learning_context,
                "simulated_net_pnl_usd": simulated_net_pnl_usd,
                "net_return_percent": net_return_percent,
                "signal_timestamp_ms": evaluation.signal_timestamp,
                "evaluated_timestamp_ms": evaluation.evaluated_timestamp,
                "taker_fee_rate": evaluation.taker_fee_rate,
                "symbol": evaluation.symbol.as_str(),
                "strategy": decision["strategy"].as_str().unwrap_or(""),
                "strategy_parameters": decision.get("strategy_parameters"),
                "exit_scenario": decision["shadow_exit_scenario"].as_str().unwrap_or("strategy"),
                "sample_group_id": decision["shadow_sample_group_id"].as_str().unwrap_or(""),
                "scout_parameter_key": decision.get("scout_parameter_key"),
                "scout_parameter_value": decision.get("scout_parameter_value"),
                "scout_parameter_profile_symbol": decision.get("scout_parameter_profile_symbol"),
                "scout_comparison_id": decision.get("scout_comparison_id"),
                "scout_comparison_role": decision.get("scout_comparison_role"),
            });
            for attempt in 0..3 {
                match client
                    .post(format!(
                        "{base}/internal/auto-trader/shadow-learning-outcome"
                    ))
                    .timeout(Duration::from_secs(2))
                    .json(&payload)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                {
                    Ok(response) => {
                        crate::service_log!("INFO", "Scout learning outcome delivered: sample_id={} symbol={} scenario={} status={} attempt={}", decision_id, evaluation.symbol, decision["shadow_exit_scenario"].as_str().unwrap_or("strategy"), response.status(), attempt + 1);
                        break;
                    }
                    Err(error) if attempt == 2 => crate::service_log!(
                        "WARN",
                        "Could not deliver fee-aware shadow Scout learning outcome sample_id={} symbol={}: {error}",
                        decision_id,
                        evaluation.symbol
                    ),
                    Err(_) => tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await,
                }
            }
        }
    }
}
