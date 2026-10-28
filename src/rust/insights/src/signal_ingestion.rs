async fn ingest_signal_event(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(request): Json<SignalEventRequest>,
) -> Result<Json<Option<SignalSnapshot>>, (StatusCode, String)> {
    if request.symbol.is_empty() || !request.symbol.is_ascii() || request.symbol.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signal symbol".to_owned()));
    }
    // Nonblocking snapshot read: market feed handling never waits on RSS/model work.
    let current_news = Arc::clone(&runtime.news.borrow());
    let current_news_weight = runtime.news_learning.try_lock().map(|learning| learning.weight).unwrap_or(0.0);
    let (snapshot, records, market) = {
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
                if !accepted && signal_diagnostic_due(&runtime, &request.symbol, timestamp)
                {
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
                    if outcome.legacy.horizon_seconds == 60 {
                        if let Ok(mut learning) = runtime.news_learning.try_lock() {
                            if let Some(vote) = learning.pending.remove(&(outcome.legacy.symbol.clone(), outcome.legacy.signal_timestamp)) {
                                learning.samples += 1;
                                if vote * outcome.legacy.return_percent > 0.0 { learning.correct += 1; }
                                if learning.samples >= 30 {
                                    let accuracy = learning.correct as f64 / learning.samples as f64;
                                    learning.weight = (0.20 * ((accuracy - 0.45) / 0.20)).clamp(0.0, 0.20);
                                }
                            }
                        }
                    }
                    records.push(StoredSignalRecord::Outcome(outcome.legacy));
                    records.push(StoredSignalRecord::Evaluation(outcome.evaluation));
                }
                let should_compute_snapshot = accepted && engine.live_snapshot_due(timestamp);
                let mut snapshot = should_compute_snapshot
                    .then(|| engine.snapshot(&request.symbol, timestamp))
                    .flatten();
                if should_compute_snapshot && snapshot.is_none()
                    && signal_diagnostic_due(&runtime, &format!("missing:{}", request.symbol), timestamp)
                {
                    crate::service_log!(
                        "WARN",
                        "Signal snapshot unavailable: symbol={} timestamp={} diagnostics={}",
                        request.symbol,
                        timestamp,
                        engine.input_diagnostic(timestamp)
                    );
                } else if should_compute_snapshot
                    && snapshot.is_some()
                    && signal_diagnostic_due(&runtime, &format!("generated:{}", request.symbol), timestamp)
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
                let news_vote;
                if let Some(snapshot) = snapshot.as_mut() {
                    news_vote = apply_news_signal(snapshot, &current_news, now_ms(), current_news_weight);
                    // The pushed decision needs the latest available market cache on
                    // every price event, regardless of the research sampling cadence.
                    if let Some(book_snapshot) =
                        engine.book_snapshot(&request.symbol, snapshot.timestamp)
                    {
                        let levels = |levels: Vec<(f64, f64)>| {
                            levels.into_iter()
                            .map(|(price, size)| serde_json::json!({"price": price, "size": size}))
                            .collect::<Vec<_>>()
                        };
                        market = Some(serde_json::json!({
                            request.symbol.clone(): {
                                "bids": levels(book_snapshot.bid_levels.clone()),
                                "asks": levels(book_snapshot.ask_levels.clone()),
                                "updated_at_ms": book_snapshot.timestamp
                            }
                        }));
                        if engine.research_sample_due(snapshot.timestamp) {
                            let features = engine.research_features(
                                &request.symbol,
                                snapshot.timestamp,
                                btc_context,
                                eth_context,
                            );
                            records.push(StoredSignalRecord::BookSnapshot(book_snapshot));
                            records.push(StoredSignalRecord::ResearchFeatures(features));
                            engine.track(snapshot);
                            if let Some(vote) = news_vote {
                                if let Ok(mut learning) = runtime.news_learning.try_lock() {
                                    learning.pending.insert((snapshot.symbol.clone(), snapshot.timestamp), vote);
                                }
                            }
                            records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
                        }
                    } else if engine.research_sample_due(snapshot.timestamp) {
                        // Preserve feature and snapshot samples when a fresh book is
                        // unavailable; depth data is optional in the research schema.
                        let features = engine.research_features(
                            &request.symbol,
                            snapshot.timestamp,
                            btc_context,
                            eth_context,
                        );
                        records.push(StoredSignalRecord::ResearchFeatures(features));
                        engine.track(snapshot);
                        if let Some(vote) = news_vote { if let Ok(mut learning) = runtime.news_learning.try_lock() { learning.pending.insert((snapshot.symbol.clone(), snapshot.timestamp), vote); } }
                        records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
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
                None
            }
        };
        (snapshot, records, market)
    };
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
        let snapshot_json = serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null);
        runtime
            .ui_snapshot_cache
            .insert(snapshot.symbol.clone(), snapshot_json);
        runtime.ui_snapshot_notify.notify_one();
    }
    Ok(Json(snapshot))
}

