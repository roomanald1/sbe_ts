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

fn valid_scout_sample(sample: &ScoutScoutRegistration, now: u64) -> bool {
    let has_single_comparison = sample.scout_comparison_id.is_some();
    let has_comparison_list = !sample.scout_comparison_ids.is_empty();
    let valid_comparison_shape = match sample.scout_comparison_role.as_deref() {
        None => !has_single_comparison && !has_comparison_list,
        Some("baseline") => has_single_comparison ^ has_comparison_list,
        Some("variant") => has_single_comparison && !has_comparison_list,
        Some(_) => false,
    };
    !(sample.decision_id.is_empty() || sample.decision_id.len() > 240
        || sample.symbol.is_empty() || sample.symbol.len() > 32
        || sample.timestamp > now.saturating_add(5_000) || now.saturating_sub(sample.timestamp) > 7 * 86_400_000
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
        || sample.scout_maximum_duration_ms.is_some_and(|duration| duration == 0 || duration > 7 * 86_400_000)
        || !matches!(sample.direction.as_str(), "bullish" | "bearish")
        || sample.scout_comparison_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 240)
        || sample.scout_comparison_ids.len() > 128
        || sample.scout_comparison_ids.iter().any(|id| id.is_empty() || id.len() > 240)
        || sample.scout_comparison_ids.iter().collect::<std::collections::BTreeSet<_>>().len() != sample.scout_comparison_ids.len()
        || !valid_comparison_shape)
}


#[derive(serde::Deserialize)]
struct ScoutScoutRegistration {
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
    scout_maximum_duration_ms: Option<u64>,
    #[serde(default)]
    scout_comparison_id: Option<String>,
    #[serde(default)]
    scout_comparison_ids: Vec<String>,
    #[serde(default)]
    scout_comparison_role: Option<String>,
}

async fn register_scout(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(sample): Json<ScoutScoutRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Scouts owns the durable instance and outcome state. Insights keeps only
    // this transient price-path tracker so market exits stay beside feed data.
    let now = now_ms();
    if !valid_scout_sample(&sample, now) {
        return Err((StatusCode::BAD_REQUEST, "Invalid scout scout sample".to_owned()));
    }
    let engine = runtime
        .engines
        .entry(sample.symbol.clone())
        .or_insert_with(|| Arc::new(Mutex::new(SignalEngine::default())))
        .clone();
    let mut engine = engine.lock().await;
    let decision_id = sample.decision_id.clone();
    let symbol = sample.symbol.clone();
    let strategy = sample.strategy.clone();
    let exit_scenario = sample.exit_scenario.clone();
    let timestamp = sample.timestamp;
    if !engine.track_scout_with_maximum_duration(
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
        sample.scout_maximum_duration_ms,
    ) {
        crate::service_log!("INFO", "Scout scout registration ignored as duplicate: sample_id={} symbol={}", decision_id, symbol);
        return Ok(StatusCode::NO_CONTENT);
    }
    crate::service_log!("INFO", "Scout scout registered: sample_id={} symbol={} strategy={} exit_scenario={} age_ms={}", decision_id, symbol, strategy, exit_scenario, now.saturating_sub(timestamp));
    Ok(StatusCode::ACCEPTED)
}

#[derive(serde::Deserialize)]
struct CounterfactualRegistration {
    symbol: String,
    timestamp: u64,
    direction: String,
    entry_price: f64,
}

async fn register_counterfactual(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(sample): Json<CounterfactualRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    let now = now_ms();
    if sample.symbol.is_empty()
        || sample.symbol.len() > 32
        || sample.timestamp > now.saturating_add(5_000)
        || now.saturating_sub(sample.timestamp) > 7 * 86_400_000
        || !matches!(sample.direction.as_str(), "bullish" | "bearish")
        || !sample.entry_price.is_finite()
        || sample.entry_price <= 0.0
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid counterfactual sample".to_owned()));
    }
    let engine = runtime
        .engines
        .entry(sample.symbol.clone())
        .or_insert_with(|| Arc::new(Mutex::new(SignalEngine::default())))
        .clone();
    let accepted = engine.lock().await.track_counterfactual(
        sample.symbol,
        sample.timestamp,
        sample.direction,
        sample.entry_price,
    );
    Ok(if accepted { StatusCode::ACCEPTED } else { StatusCode::NO_CONTENT })
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
    let engines = runtime
        .engines
        .iter()
        .map(|entry| Arc::clone(entry.value()))
        .collect::<Vec<_>>();
    for engine in engines {
        engine.lock().await.update_scout_strategy_state(
            &active,
            &active_parameter_profiles,
            &enabled,
        );
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
    let mut scout_markouts = Vec::<(String, SignalEvaluation, bool)>::new();
    let (snapshot, records, market, event_timestamp) = {
        let event_timestamp = match &request.event {
            SignalFeedEvent::Price { timestamp, .. }
            | SignalFeedEvent::Trade { timestamp, .. }
            | SignalFeedEvent::Book { timestamp, .. } => *timestamp,
        };
        // Capture BTC/ETH context opportunistically without taking another symbol's
        // engine lock. Research samples may omit context while a reference is busy.
        let btc_engine = runtime
            .engines
            .get("BTC-USDT")
            .map(|entry| Arc::clone(entry.value()));
        let btc_context = btc_engine
            .and_then(|engine| engine.try_lock().ok().map(|engine| engine.return_context(event_timestamp)))
            .unwrap_or((None, None));
        let eth_engine = runtime
            .engines
            .get("ETH-USDT")
            .map(|entry| Arc::clone(entry.value()));
        let eth_context = eth_engine
            .and_then(|engine| engine.try_lock().ok().map(|engine| engine.return_context(event_timestamp)))
            .unwrap_or((None, None));
        let engine = runtime
            .engines
            .entry(request.symbol.clone())
            .or_insert_with(|| Arc::new(Mutex::new(SignalEngine::default())))
            .clone();
        let engine_wait_started = Instant::now();
        let mut engine = engine.lock().await;
        crate::latency::record_engine_lock_wait(engine_wait_started.elapsed());
        let mut records = Vec::new();
        let mut market = None;
        let snapshot = match request.event {
            SignalFeedEvent::Price {
                timestamp,
                price,
            } => {
                let (volatility_1m, volatility_5m) = runtime
                    .rolling_volatility
                    .entry(request.symbol.clone())
                    .or_default()
                    .update(timestamp, price);
                if let Some(raw) = request.raw_payload.as_ref() {
                    if let Some(enriched) = enrich_raw_price(raw, volatility_1m, volatility_5m) {
                        let _ = runtime.enriched_market_events.send(enriched);
                    }
                }
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
                    if outcome.scout_id.is_some() {
                        crate::service_log!("INFO", "Scout observation completed: sample_id={} symbol={} observation_only={} exit_reason={} age_seconds={}", outcome.scout_id.as_deref().unwrap_or("unknown"), outcome.evaluation.symbol, outcome.scout_observation_only, outcome.scout_exit_reason.unwrap_or("horizon"), outcome.evaluation.horizon_seconds);
                        if let Some(sample_id) = outcome.scout_id.clone() {
                            scout_markouts.push((
                                sample_id,
                                outcome.evaluation.clone(),
                                outcome.scout_observation_only,
                            ));
                        }
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
                let should_compute_snapshot = accepted
                    && if crate::research_only() {
                        engine.research_sample_is_due(timestamp)
                    } else {
                        engine.live_snapshot_due(timestamp)
                    };
                let should_capture_research =
                    should_compute_snapshot && engine.research_sample_is_due(timestamp);
                let snapshot_started = Instant::now();
                let mut snapshot = should_compute_snapshot
                    .then(|| {
                        engine.snapshot_with_reasons(
                            &request.symbol,
                            timestamp,
                            should_capture_research,
                        )
                    })
                    .flatten();
                if should_compute_snapshot {
                    crate::latency::record_snapshot_compute(snapshot_started.elapsed());
                }
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
    if snapshot.is_none() && !crate::research_only() {
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
        if let Err(error) = runtime.research_record_sender.try_send(records) {
            let dropped_records = error.into_inner().len();
            crate::latency::record_dropped_research_records(dropped_records);
            crate::service_log!(
                "WARN",
                "Research record queue is full; dropped a batch of {dropped_records} records"
            );
        }
    }
    if !scout_markouts.is_empty() {
        tokio::spawn(async move {
            publish_scout_markouts(scout_markouts).await;
        });
    }
    if let Some(snapshot) = snapshot.as_ref() {
        if !crate::research_only() {
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
        }
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

async fn publish_scout_markouts(
    outcomes: Vec<(String, SignalEvaluation, bool)>,
) {
    let _permit = SCOUT_LEARNING_DELIVERY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .expect("scout learning delivery semaphore remains open");
    let learning_rsocket_url = std::env::var("LEARNING_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10010".to_owned());
    for (decision_id, evaluation, observation_only) in outcomes {
        let payload = serde_json::json!({
            "decision_id": decision_id,
            "symbol": evaluation.symbol,
            "signal_timestamp_ms": evaluation.signal_timestamp,
            "evaluated_timestamp_ms": evaluation.evaluated_timestamp,
            "taker_fee_rate": evaluation.taker_fee_rate,
            "observation_only": observation_only,
            "horizon_seconds": evaluation.horizon_seconds,
            "reference_entry_price": evaluation.reference_entry_price,
            "reference_exit_price": evaluation.reference_exit_price,
            "entry_bid": evaluation.entry_bid,
            "entry_ask": evaluation.entry_ask,
            "exit_bid": evaluation.exit_bid,
            "exit_ask": evaluation.exit_ask,
            "long_gross_percent": evaluation.long_gross_percent,
            "short_gross_percent": evaluation.short_gross_percent,
            "long_net_percent": evaluation.long_net_percent,
            "short_net_percent": evaluation.short_net_percent,
            "path_high": evaluation.path_high,
            "path_low": evaluation.path_low,
            "long_time_to_positive_net_seconds": evaluation.long_time_to_positive_net_seconds,
            "short_time_to_positive_net_seconds": evaluation.short_time_to_positive_net_seconds,
        });
        for attempt in 0..3 {
            let response = strategies::request_scout_worker_route(
                &learning_rsocket_url,
                &strategies::ScoutCoordinatorRouteCommand {
                    request_id: format!("scout-markout:{decision_id}:{}", evaluation.evaluated_timestamp),
                    strategy: None,
                    method: strategies::ScoutWorkerHttpMethod::Post,
                    path: "/scout-markout".into(),
                    payload: payload.clone(),
                },
            )
            .await;
            match response {
                Ok(response) if response.status < 400 => {
                    crate::service_log!("INFO", "Scout markout delivered: sample_id={} symbol={} status={} attempt={}", decision_id, evaluation.symbol, response.status, attempt + 1);
                    break;
                }
                Ok(response) if attempt == 2 => crate::service_log!(
                    "WARN",
                    "Could not deliver scout markout sample_id={} symbol={}: status {}",
                    decision_id,
                    evaluation.symbol,
                    response.status
                ),
                Err(error) if attempt == 2 => crate::service_log!(
                    "WARN",
                    "Could not deliver scout markout sample_id={} symbol={}: {error}",
                    decision_id,
                    evaluation.symbol
                ),
                Err(_) => tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await,
                Ok(_) => tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await,
            }
        }
    }
}

#[cfg(test)]
mod signal_ingestion_tests {
    use super::*;

    fn sample() -> ScoutScoutRegistration {
        serde_json::from_value(serde_json::json!({
            "decision_id":"decision-1", "symbol":"BTC-USDT", "timestamp":1000,
            "direction":"bullish", "entry_price":100.0, "strategy":"trend_following",
            "exit_scenario":"strategy", "stop_loss_percent":1.0, "take_profit_percent":2.0,
            "profit_protection_activation_percent":0.5, "profit_protection_giveback_percent":0.2,
            "profit_protection_minimum_lock_percent":0.1
        })).unwrap()
    }

    #[test]
    fn market_levels_filters_invalid_prices_and_sizes() {
        let encoded = encode_market_levels(&[
            (100.0, 2.0), (0.0, 1.0), (-1.0, 1.0), (101.0, 0.0),
            (102.0, f64::INFINITY), (f64::NAN, 1.0),
        ]);
        assert_eq!(encoded, vec![serde_json::json!({"price":100.0,"size":2.0})]);
    }

    #[test]
    fn market_value_requires_a_valid_ordered_quote_and_keeps_valid_levels() {
        assert!(market_value(1, 0.0, 1.0, &[], &[]).is_none());
        assert!(market_value(1, 2.0, 1.0, &[], &[]).is_none());
        assert!(market_value(1, f64::NAN, 2.0, &[], &[]).is_none());
        let value = market_value(42, 99.0, 100.0, &[(99.0, 1.0)], &[(100.0, 2.0)]).unwrap();
        assert_eq!(value["updated_at_ms"], 42);
        assert_eq!(value["bids"][0], serde_json::json!({"price":99.0,"size":1.0}));
        assert_eq!(value["asks"][0], serde_json::json!({"price":100.0,"size":2.0}));
    }

    #[test]
    fn scout_validation_accepts_valid_sample_and_rejects_field_boundaries() {
        let mut value = sample();
        assert!(valid_scout_sample(&value, 1_000));
        value.entry_price = f64::NAN;
        assert!(!valid_scout_sample(&value, 1_000));

        let mut value = sample();
        value.timestamp = 1_000 + 5_001;
        assert!(!valid_scout_sample(&value, 1_000));
        value.timestamp = 0;
        assert!(!valid_scout_sample(&value, 7 * 86_400_000 + 1));

        let mut value = sample();
        value.scout_comparison_id = Some("pair".into());
        assert!(!valid_scout_sample(&value, 1_000));
        value.scout_comparison_role = Some("baseline".into());
        assert!(valid_scout_sample(&value, 1_000));
        value.scout_comparison_role = Some("unknown".into());
        assert!(!valid_scout_sample(&value, 1_000));

        let mut value = sample();
        value.scout_comparison_ids = vec!["pair-a".into(), "pair-b".into()];
        value.scout_comparison_role = Some("baseline".into());
        assert!(valid_scout_sample(&value, 1_000));
        value.scout_comparison_role = Some("variant".into());
        assert!(!valid_scout_sample(&value, 1_000));
        value.scout_comparison_role = Some("baseline".into());
        value.scout_comparison_ids.push("pair-a".into());
        assert!(!valid_scout_sample(&value, 1_000));
    }
}
