use std::io::Write;

fn compress_live_strategy_update(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn ui_snapshot_json(snapshot: &SignalSnapshot) -> serde_json::Value {
    let components = &snapshot.components;
    serde_json::json!({
        "symbol": &snapshot.symbol,
        "timestamp": snapshot.timestamp,
        "direction": &snapshot.direction,
        "score": snapshot.score,
        "strength": snapshot.strength,
        "components": {
            "trend_1m_percent": components.trend_1m_percent,
            "momentum_10s_percent": components.momentum_10s_percent,
            "trade_flow_imbalance": components.trade_flow_imbalance,
            "book_imbalance": components.book_imbalance,
            "volatility_1m_percent": components.volatility_1m_percent,
            "trend_5m_percent": components.trend_5m_percent,
            "trend_5m_volatility_units": components.trend_5m_volatility_units,
            "mispricing_vs_vwap_bps": components.mispricing_vs_vwap_bps,
            "pressure_without_price_move": components.pressure_without_price_move,
            "aggressor_cost_bps": components.aggressor_cost_bps,
            "momentum_by_window": [],
            "trend_volatility_units_by_window": [],
            "trade_flow_by_window": [],
            "book_imbalance_by_depth": [],
            "vwap_dislocation_by_window": [],
            "absorption_pressure_by_window": [],
        },
        "reasons": &snapshot.reasons,
        "news_attribution": &snapshot.news_attribution,
        "stale": snapshot.stale,
        "ranging_probability": snapshot.ranging_probability,
        "range_lower_bound": snapshot.range_lower_bound,
        "range_upper_bound": snapshot.range_upper_bound,
        "range_location": snapshot.range_location,
        "evidence_coverage": snapshot.evidence_coverage,
        "estimated_accuracy": snapshot.estimated_accuracy,
    })
}

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
    admit_runtime_symbol(&runtime, &sample.symbol)
        .await
        .map_err(|()| (StatusCode::TOO_MANY_REQUESTS, "Insights active-symbol capacity reached".to_owned()))?;
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

// Keep this conversion structural instead of serializing SignalSnapshot into a
// generic JSON value and deserializing it again for every live feed update.
fn strategy_signal_from_snapshot(
    snapshot: &SignalSnapshot,
) -> strategy::StrategyMarketSignal {
    let components = &snapshot.components;
    strategy::StrategyMarketSignal {
        symbol: snapshot.symbol.clone(),
        direction: snapshot.direction.clone(),
        strength: snapshot.strength,
        timestamp: snapshot.timestamp,
        stale: snapshot.stale,
        score: snapshot.score,
        ranging_probability: snapshot.ranging_probability,
        range_location: snapshot.range_location,
        components: strategy::SignalComponents {
            momentum_10s_percent: components.momentum_10s_percent,
            trend_5m_volatility_units: components.trend_5m_volatility_units,
            trade_flow_imbalance: components.trade_flow_imbalance,
            book_imbalance: components.book_imbalance,
            volatility_1m_percent: components.volatility_1m_percent,
            mispricing_vs_vwap_bps: components.mispricing_vs_vwap_bps,
            pressure_without_price_move: components.pressure_without_price_move,
            aggressor_cost_bps: components.aggressor_cost_bps,
            momentum_by_window: components
                .momentum_by_window
                .iter()
                .map(|value| strategy::WindowedSignalValue {
                    window_seconds: value.window_seconds,
                    value: value.value,
                    sample_count: value.sample_count,
                    span_ms: value.span_ms,
                })
                .collect(),
            trend_volatility_units_by_window: components
                .trend_volatility_units_by_window
                .iter()
                .map(|value| strategy::WindowedSignalValue {
                    window_seconds: value.window_seconds,
                    value: value.value,
                    sample_count: value.sample_count,
                    span_ms: value.span_ms,
                })
                .collect(),
            trade_flow_by_window: components
                .trade_flow_by_window
                .iter()
                .map(|value| strategy::WindowedSignalValue {
                    window_seconds: value.window_seconds,
                    value: value.value,
                    sample_count: value.sample_count,
                    span_ms: value.span_ms,
                })
                .collect(),
            book_imbalance_by_depth: components
                .book_imbalance_by_depth
                .iter()
                .map(|value| strategy::DepthSignalValue {
                    depth_levels: value.depth_levels,
                    value: value.value,
                })
                .collect(),
            vwap_dislocation_by_window: components
                .vwap_dislocation_by_window
                .iter()
                .map(|value| strategy::WindowedSignalValue {
                    window_seconds: value.window_seconds,
                    value: value.value,
                    sample_count: value.sample_count,
                    span_ms: value.span_ms,
                })
                .collect(),
            absorption_pressure_by_window: components
                .absorption_pressure_by_window
                .iter()
                .map(|value| strategy::AbsorptionPressureValue {
                    window_seconds: value.window_seconds,
                    replenishment_window_ms: value.replenishment_window_ms,
                    touch_tolerance_bps: value.touch_tolerance_bps,
                    value: value.value,
                })
                .collect(),
        },
        strategy_candidate: false,
        strategy: None,
        strategy_decision: None,
        market_regime: None,
        market_state: None,
        news_attribution: snapshot
            .news_attribution
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
        entry_gate: None,
    }
}

async fn ingest_signal_event(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(request): Json<SignalEventRequest>,
) -> Result<Json<Option<SignalSnapshot>>, (StatusCode, String)> {
    if request.symbol.is_empty() || !request.symbol.is_ascii() || request.symbol.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signal symbol".to_owned()));
    }
    admit_runtime_symbol(&runtime, &request.symbol)
        .await
        .map_err(|()| (StatusCode::TOO_MANY_REQUESTS, "Insights active-symbol capacity reached".to_owned()))?;
    let event_kind = match &request.event {
        SignalFeedEvent::Price { .. } => "price",
        SignalFeedEvent::Trade { .. } => "trade",
        SignalFeedEvent::Book { .. } => "book",
    };
    let event_processing_started = Instant::now();
    // Nonblocking snapshot read: market feed handling never waits on RSS/model work.
    let current_news = Arc::clone(&runtime.news.borrow());
    let current_news_weight = runtime
        .news_learning
        .try_lock()
        .map(|learning| learning.weight)
        .unwrap_or(0.0);
    let mut scout_markouts = Vec::<(String, SignalEvaluation, bool, Option<String>)>::new();
    let (snapshot, records) = {
        let event_timestamp = match &request.event {
            SignalFeedEvent::Price { timestamp, .. }
            | SignalFeedEvent::Trade { timestamp, .. }
            | SignalFeedEvent::Book { timestamp, .. } => *timestamp,
        };
        let engine = runtime
            .engines
            .entry(request.symbol.clone())
            .or_insert_with(|| Arc::new(Mutex::new(SignalEngine::default())))
            .clone();
        let engine_wait_started = Instant::now();
        let mut engine = engine.lock().await;
        crate::latency::record_engine_lock_wait(engine_wait_started.elapsed());
        let mut records = Vec::new();
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
                let engine_work_started = Instant::now();
                let outcomes = engine.on_price(timestamp, price, volatility_1m);
                crate::latency::record_engine_work("price", engine_work_started.elapsed());
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
                                outcome.strategy.clone(),
                            ));
                        }
                    }
                    if outcome.legacy.horizon_seconds == 60 {
                        if let Ok(mut learning) = runtime.news_learning.try_lock() {
                            prune_news_pending(&mut learning, now_ms());
                            if let Some((vote, _)) = learning.pending.remove(&(
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
                    accepted && engine.research_sample_is_due(timestamp);
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
                    // News is a weak, slowly refreshed signal. Recompute its
                    // attribution on the research cadence instead of cloning
                    // article metadata into every two-second live snapshot.
                    let news_vote = should_capture_research
                        .then(|| {
                            apply_news_signal(
                                snapshot,
                                &current_news,
                                now_ms(),
                                current_news_weight,
                            )
                        })
                        .flatten();
                    if should_capture_research {
                        engine.research_sample_due(snapshot.timestamp);
                        // Reference returns are only consumed by research samples.
                        // Avoid two DashMap lookups and return-history scans for every
                        // market event; same-symbol context comes from the locked engine.
                        let btc_context = if request.symbol == "BTC-USDT" {
                            engine.return_context(event_timestamp)
                        } else {
                            runtime
                                .engines
                                .get("BTC-USDT")
                                .and_then(|entry| {
                                    entry
                                        .value()
                                        .try_lock()
                                        .ok()
                                        .map(|reference| reference.return_context(event_timestamp))
                                })
                                .unwrap_or((None, None))
                        };
                        let eth_context = if request.symbol == "ETH-USDT" {
                            engine.return_context(event_timestamp)
                        } else {
                            runtime
                                .engines
                                .get("ETH-USDT")
                                .and_then(|entry| {
                                    entry
                                        .value()
                                        .try_lock()
                                        .ok()
                                        .map(|reference| reference.return_context(event_timestamp))
                                })
                                .unwrap_or((None, None))
                        };
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
                                prune_news_pending(&mut learning, now_ms());
                                learning
                                    .pending
                                    .insert((snapshot.symbol.clone(), snapshot.timestamp), (vote, now_ms()));
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
                let engine_work_started = Instant::now();
                engine.on_trade(timestamp, price, size, is_buy);
                crate::latency::record_engine_work("trade", engine_work_started.elapsed());
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
                let engine_work_started = Instant::now();
                engine.on_book(
                    timestamp,
                    best_bid,
                    best_ask,
                    &bid_sizes,
                    &ask_sizes,
                    &bid_levels,
                    &ask_levels,
                );
                crate::latency::record_engine_work("book", engine_work_started.elapsed());
                if best_bid.is_finite()
                    && best_ask.is_finite()
                    && best_bid > 0.0
                    && best_ask >= best_bid
                {
                    runtime
                        .market_cache
                        .insert(
                            request.symbol.clone(),
                            CachedMarket {
                                timestamp,
                                best_bid,
                                best_ask,
                                bid_levels,
                                ask_levels,
                            },
                        );
                }
                None
            }
        };
        (snapshot, records)
    };
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
        let market_value = runtime
            .market_cache
            .get(&snapshot.symbol)
            .and_then(|entry| {
                let cached = entry.value();
                if snapshot.timestamp.saturating_sub(cached.timestamp) > 5_000 {
                    return None;
                }
                market_value(
                    cached.timestamp,
                    cached.best_bid,
                    cached.best_ask,
                    &cached.bid_levels,
                    &cached.ask_levels,
                )
            });
        let signal_encoding_started = Instant::now();
        let update = strategy::StrategyInsightUpdate {
            contract_version: strategy::STRATEGY_INSIGHT_UPDATE_VERSION,
            event_id: format!("{}:{}", snapshot.symbol, snapshot.timestamp),
            event_time_ms: snapshot.timestamp,
            signal: Some(strategy_signal_from_snapshot(snapshot)),
            market: market_value,
            taker_fee_rate: Some(insights::signals::configured_signal_taker_fee_rate()),
            daily_snapshot: None,
            dated_carry_scan: None,
            dated_carry_candidate: None,
            dated_carry_settlement: None,
        };
        if let Ok(encoded) = serde_json::to_vec(&update) {
            let compressed = compress_live_strategy_update(&encoded);
            crate::latency::record_strategy_update_encoding(
                signal_encoding_started.elapsed(),
                encoded.len(),
                compressed.as_ref().map_or(0, Vec::len),
            );
            let encoded = Bytes::from(encoded);
            runtime.latest_strategy_signal_updates.insert(snapshot.symbol.clone(), encoded.clone());
            let _ = runtime.strategy_signal_events.send(encoded.clone());
            let _ = runtime.legacy_strategy_signal_events.send(encoded);
            match compressed {
                Ok(compressed) => {
                    let compressed = Bytes::from(compressed);
                    runtime
                        .latest_compressed_strategy_signal_updates
                        .insert(snapshot.symbol.clone(), compressed.clone());
                    let _ = runtime.compressed_strategy_signal_events.send(compressed);
                }
                Err(error) => crate::service_log!(
                    "ERROR",
                    "Could not compress live strategy signal update: {error}"
                ),
            }
        }
        let snapshot_json = ui_snapshot_json(snapshot);
        runtime
            .ui_snapshot_cache
            .insert(snapshot.symbol.clone(), snapshot_json);
        runtime.ui_snapshot_notify.notify_one();
    }
    crate::latency::record_event_processing(event_kind, event_processing_started.elapsed());
    Ok(Json(snapshot))
}

async fn publish_scout_markouts(
    outcomes: Vec<(String, SignalEvaluation, bool, Option<String>)>,
) {
    let _permit = SCOUT_LEARNING_DELIVERY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .expect("scout learning delivery semaphore remains open");
    let learning_rsocket_url = std::env::var("LEARNING_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10010".to_owned());
    for (decision_id, evaluation, observation_only, strategy_name) in outcomes {
        let strategy_target = strategy_name
            .as_deref()
            .and_then(strategy::Strategy::from_name);
        let payload = serde_json::json!({
            "decision_id": decision_id,
            "strategy": strategy_name,
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
            let response = strategy::request_scout_worker_route(
                &learning_rsocket_url,
                &strategy::ScoutCoordinatorRouteCommand {
                    request_id: format!("scout-markout:{decision_id}:{}", evaluation.evaluated_timestamp),
                    strategy: strategy_target,
                    method: strategy::ScoutWorkerHttpMethod::Post,
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
                    "ERROR",
                    "Could not deliver scout markout sample_id={} symbol={}: status {} body={}",
                    decision_id,
                    evaluation.symbol,
                    response.status,
                    response.body
                ),
                Err(error) if attempt == 2 => crate::service_log!(
                    "ERROR",
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

    const INGEST_BENCHMARK_SAMPLES: usize = 128;
    const INGEST_BENCHMARK_WARMUP_EVENTS: usize = 40;
    const INGEST_BENCHMARK_START_MS: u64 = 1_800_000_000_000;
    const INGEST_BENCHMARK_STEP_MS: u64 = 2_000;

    fn benchmark_event(index: usize) -> Vec<u8> {
        let timestamp = INGEST_BENCHMARK_START_MS + index as u64 * INGEST_BENCHMARK_STEP_MS;
        raw_price_event(timestamp, 100.0 + index as f64 * 0.01)
    }

    fn raw_price_event(timestamp_ms: u64, price: f64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(56);
        bytes.extend_from_slice(&48_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&9_u16.to_le_bytes());
        let mut symbol = [b' '; 32];
        symbol[..8].copy_from_slice(b"BTC-USDT");
        bytes.extend_from_slice(&symbol);
        bytes.extend_from_slice(&price.to_le_bytes());
        bytes.extend_from_slice(&timestamp_ms.to_le_bytes());
        bytes
    }

    fn benchmark_input_hash() -> String {
        // FNV-1a keeps this fixture hash stable across Rust versions and hosts.
        let mut hash = 0xcbf29ce484222325_u64;
        let mut add = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        };
        add(b"insights-raw-price-to-strategy-signal-v1");
        add(&(INGEST_BENCHMARK_WARMUP_EVENTS as u64).to_le_bytes());
        add(&(INGEST_BENCHMARK_SAMPLES as u64).to_le_bytes());
        add(&INGEST_BENCHMARK_START_MS.to_le_bytes());
        add(&INGEST_BENCHMARK_STEP_MS.to_le_bytes());
        for index in 0..INGEST_BENCHMARK_WARMUP_EVENTS + INGEST_BENCHMARK_SAMPLES {
            add(&benchmark_event(index));
        }
        format!("fnv1a64:{hash:016x}")
    }

    fn benchmark_runtime() -> (
        Arc<SignalRuntime>,
        tokio::sync::mpsc::Receiver<Vec<StoredSignalRecord>>,
    ) {
        let (research_record_sender, research_records) = tokio::sync::mpsc::channel(256);
        let (signal_event_sender, _) = tokio::sync::mpsc::channel(1);
        let (news_tx, news) = tokio::sync::watch::channel(Arc::new(NewsFeedSnapshot::default()));
        drop(news_tx);
        let (enriched_market_events, _) = broadcast::channel(256);
        let (strategy_signal_events, _) = broadcast::channel(256);
        let (compressed_strategy_signal_events, _) = broadcast::channel(256);
        let (strategy_maintenance_events, _) = broadcast::channel(16);
        let (legacy_strategy_signal_events, _) = broadcast::channel(256);
        (
            Arc::new(SignalRuntime {
                active_symbols: DashMap::new(),
                symbol_admission_gate: Mutex::new(()),
                max_active_symbols: 1,
                symbol_idle_ttl_ms: 60_000,
                engines: DashMap::new(),
                research_record_sender,
                signal_event_sender,
                signal_event_shards: Arc::new(std::sync::RwLock::new(Vec::new())),
                market_cache: Arc::new(DashMap::new()),
                ui_snapshot_cache: Arc::new(DashMap::new()),
                ui_snapshot_notify: Arc::new(Notify::new()),
                news,
                news_learning: Mutex::new(NewsLearning::default()),
                snapshot_diagnostic_at: DashMap::new(),
                latest_prices: DashMap::new(),
                rolling_volatility: DashMap::new(),
                enriched_market_events,
                strategy_signal_events,
                compressed_strategy_signal_events,
                strategy_maintenance_events,
                legacy_strategy_signal_events,
                latest_strategy_signal_updates: Arc::new(DashMap::new()),
                latest_compressed_strategy_signal_updates: Arc::new(DashMap::new()),
            }),
            research_records,
        )
    }

    fn exact_percentile(sorted_ns: &[u128], percentile_basis_points: usize) -> u128 {
        let rank = sorted_ns
            .len()
            .saturating_mul(percentile_basis_points)
            .div_ceil(10_000)
            .max(1);
        sorted_ns[rank.saturating_sub(1)]
    }

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

    #[tokio::test]
    async fn raw_market_ingress_to_strategy_signal_benchmark_fixture() {
        let (runtime, _research_records) = benchmark_runtime();
        let mut published = runtime.strategy_signal_events.subscribe();
        let mut elapsed_ns = Vec::with_capacity(INGEST_BENCHMARK_SAMPLES);
        let input_hash = benchmark_input_hash();

        // The directional feature requires a 60-second return window. Prime
        // that state through the same raw decoder and production handler, but
        // exclude warm-up events and their possible publications from timing.
        for index in 0..INGEST_BENCHMARK_WARMUP_EVENTS {
            let request = decode_signal_event(&benchmark_event(index))
                .expect("deterministic warm-up event decodes through production codec");
            let result = ingest_signal_event(
                axum::extract::State(Arc::clone(&runtime)),
                Json(request),
            )
            .await;
            assert!(result.is_ok(), "production ingress should accept warm-up event");
        }
        while published.try_recv().is_ok() {}

        let workload_started = Instant::now();
        for index in INGEST_BENCHMARK_WARMUP_EVENTS
            ..INGEST_BENCHMARK_WARMUP_EVENTS + INGEST_BENCHMARK_SAMPLES
        {
            let raw_event = benchmark_event(index);
            let timestamp = INGEST_BENCHMARK_START_MS + index as u64 * INGEST_BENCHMARK_STEP_MS;
            // The measured path begins with the real feed codec payload and uses
            // the production decoder, ingestion/evaluation handler and publisher.
            let ingress_started = Instant::now();
            let mut request = decode_signal_event(&raw_event)
                .expect("deterministic raw market event decodes through production codec");
            request.queued_at = Some(ingress_started);
            let result = ingest_signal_event(
                axum::extract::State(Arc::clone(&runtime)),
                Json(request),
            )
            .await;
            assert!(result.is_ok(), "production market ingress should accept fixture event");
            let update = published
                .recv();
            let update = tokio::time::timeout(Duration::from_secs(1), update)
                .await
                .expect("production signal publication should complete within one second")
                .expect("accepted raw price event publishes a strategy signal update");
            let value: serde_json::Value = serde_json::from_slice(&update).unwrap();
            assert_eq!(value["event_id"], format!("BTC-USDT:{timestamp}"));
            assert_eq!(value["signal"]["timestamp"], timestamp);
            assert!(!value["signal"].is_null());
            elapsed_ns.push(ingress_started.elapsed().as_nanos());
        }

        let workload_elapsed = workload_started.elapsed();
        elapsed_ns.sort_unstable();
        let signals_per_second = elapsed_ns.len() as f64 / workload_elapsed.as_secs_f64();
        println!(
            "INSIGHTS_RAW_INGRESS_BENCHMARK {{\"input_config_hash\":\"{input_hash}\",\"warmup_event_count\":{},\"event_count\":{},\"published_signal_count\":{},\"elapsed_ms\":{},\"signals_per_second\":{},\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"p99_9_ns\":{},\"scope\":\"raw market decode + feature evaluation + local StrategyMarketSignal update publication\",\"excludes\":\"downstream AutoTrader risk/order-ready, exchange submission, acknowledgement and fills\"}}",
            INGEST_BENCHMARK_WARMUP_EVENTS,
            INGEST_BENCHMARK_SAMPLES,
            elapsed_ns.len(),
            workload_elapsed.as_secs_f64() * 1_000.0,
            signals_per_second,
            exact_percentile(&elapsed_ns, 5_000),
            exact_percentile(&elapsed_ns, 9_500),
            exact_percentile(&elapsed_ns, 9_900),
            exact_percentile(&elapsed_ns, 9_990),
        );
        assert_eq!(elapsed_ns.len(), INGEST_BENCHMARK_SAMPLES);
        assert_eq!(benchmark_input_hash(), input_hash);
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
