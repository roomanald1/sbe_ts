pub async fn router() -> anyhow::Result<Router> {
    let spot_symbols = Arc::new(crate::feed_client::spot_symbols().await?);
    let store = Arc::new(PositionStore::load(Arc::clone(&spot_symbols)).await?);
    let mut symbols = store
        .ledger
        .lock()
        .await
        .orders
        .iter()
        .map(|order| order.symbol.clone())
        .collect::<std::collections::HashSet<_>>();
    symbols.extend(
        store
            .ledger
            .lock()
            .await
            .barrier_orders
            .iter()
            .filter(|order| order.status == "armed")
            .map(|order| order.symbol.clone()),
    );
    symbols.extend(
        summarize_positions(&store.ledger.lock().await.trades)
            .positions
            .into_iter()
            .filter(|position| position.quantity.abs() > EPSILON)
            .map(|position| position.symbol),
    );
    for symbol in symbols {
        store.ensure_subscription(&symbol);
    }
    let barrier_store = Arc::clone(&store);
    tokio::spawn(async move {
        barrier_order_loop(barrier_store).await;
    });
    let expiry_store = Arc::clone(&store);
    tokio::spawn(async move {
        expire_orders_loop(expiry_store).await;
    });
    tokio::spawn(async move {
        loop {
            match crate::feed_client::stream("usd-rates:*".to_owned()).await {
                Ok(mut stream) => {
                    while let Some(item) = stream.next().await {
                        match item.and_then(|bytes| {
                            serde_json::from_slice::<UsdRatesResponse>(&bytes)
                                .map_err(anyhow::Error::from)
                        }) {
                            Ok(snapshot)
                                if snapshot
                                    .rates
                                    .values()
                                    .any(|rate| rate.is_finite() && *rate > 0.0) =>
                            {
                                let cache =
                                    USD_RATE_STREAM_CACHE.get_or_init(|| StdRwLock::new(None));
                                if let Ok(mut current) = cache.write() {
                                    *current = Some(snapshot);
                                }
                            }
                            Ok(_) => crate::service_log!(
                                "WARN",
                                "Ignoring empty or invalid FeedHandlers USD rate snapshot"
                            ),
                            Err(error) => crate::service_log!(
                                "WARN",
                                "Ignoring invalid FeedHandlers USD rate update: {error}"
                            ),
                        }
                    }
                }
                Err(error) => crate::service_log!(
                    "ERROR",
                    "FeedHandlers USD rate stream disconnected: {error:#}"
                ),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let sample_store = Arc::clone(&store);
    // Keep the REST fallback cache warm so order capacity checks do not need
    // to wait on an OKX request when the USD-rate feed is temporarily stale.
    tokio::spawn(async move {
        loop {
            if let Err((status, error)) = get_usd_rates().await {
                crate::service_log!(
                    "WARN",
                    "Could not refresh USD conversion rates ({status}): {error}"
                );
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    tokio::spawn(async move {
        loop {
            match crate::feed_client::stream("tickers:*".to_owned()).await {
                Ok(mut stream) => {
                    while let Some(Ok(bytes)) = stream.next().await {
                        for message in bytes.chunks_exact(SYMBOL_PRICE_MESSAGE_LENGTH) {
                            let instrument = String::from_utf8_lossy(&message[8..40])
                                .trim_end_matches([' ', '\0'])
                                .to_owned();
                            let price =
                                f64::from_le_bytes(message[40..48].try_into().unwrap_or_default());
                            if price.is_finite() && price > 0.0 {
                                if let Ok(timestamp) = now_ms() {
                                    sample_store
                                        .record_ticker_sample(instrument, price, timestamp)
                                        .await;
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    crate::service_log!("ERROR", "FeedHandlers ticker API disconnected: {error:#}")
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Ok(Router::new()
        .route("/positions/stream", get(position_ui_stream))
        .route("/internal/persistence-timing", get(get_persistence_timing))
        .route("/positions", get(get_positions))
        .route("/positions/realized-pnl", get(get_realized_pnl))
        .route("/positions/account", get(get_account))
        .route("/account", get(get_account))
        .route("/positions/account/topups", post(top_up_account))
        .route("/positions/account/leverage", post(set_account_leverage))
        .route("/account/topups", post(top_up_account))
        .route("/positions/usd-rates", get(get_usd_rates))
        .route("/positions/fee-rates", get(get_fee_rates))
        .route("/positions/hedge-candidates", get(get_hedge_candidates))
        .route("/internal/insights-inputs", get(get_insights_inputs))
        .route("/internal/market-book", get(get_market_book))
        .route(
            "/internal/market-book/{symbol}/subscribe",
            post(subscribe_market_book),
        )
        .route("/positions/trades", get(get_position_trades))
        .route("/positions/vwap", get(get_vwap_runs).post(start_vwap))
        .route("/positions/twap", post(start_twap))
        .route("/positions/vwap/{id}", axum::routing::delete(cancel_vwap))
        .route(
            "/positions/orders",
            get(get_orders)
                .post(submit_order)
                .layer(middleware::from_fn(order_handler_timing)),
        )
        .route("/positions/barriers", get(get_barrier_orders))
        .route(
            "/positions/barriers/profit-target",
            post(create_profit_barrier),
        )
        .route("/positions/close-profitable", post(close_profitable))
        .route(
            "/positions/orders/{id}",
            axum::routing::delete(cancel_order),
        )
        .with_state(store))
}

async fn position_ui_stream(
    State(store): State<Arc<PositionStore>>,
    websocket: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    websocket.on_upgrade(move |socket| async move {
        use axum::extract::ws::Message;
        use futures_util::{SinkExt, StreamExt};

        let mut updates = store.state_updates.subscribe();
        let (mut sender, mut receiver) = socket.split();
        let mut rates_refresh = tokio::time::interval(Duration::from_secs(30));
        rates_refresh.tick().await;
        let mut hedge_refresh = tokio::time::interval(Duration::from_secs(15));
        hedge_refresh.tick().await;
        let mut previous = position_ui_snapshot(&store).await;
        previous["hedge_candidates"] = hedge_candidates_snapshot(&store).await;
        let Ok(initial) = serde_json::to_string(&serde_json::json!({ "type": "snapshot", "state": previous })) else { return };
        if sender.send(Message::Text(initial.into())).await.is_err() { return; }
        loop {
            tokio::select! {
                update = updates.recv() => match update {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        let snapshot = position_ui_snapshot(&store).await;
                        let mut snapshot = snapshot;
                        snapshot["hedge_candidates"] = previous["hedge_candidates"].clone();
                        let mut changes = serde_json::Map::new();
                        let mut auto_trade_appends = Vec::new();
                        if let (Some(current), Some(old)) = (snapshot.as_object(), previous.as_object()) {
                            for (key, value) in current {
                                if key == "auto_trades" {
                                    let current_trades = value.get("trades").and_then(serde_json::Value::as_array);
                                    let previous_trades = old.get(key).and_then(|value| value.get("trades")).and_then(serde_json::Value::as_array);
                                    if let (Some(current_trades), Some(previous_trades)) = (current_trades, previous_trades) {
                                        if current_trades.starts_with(previous_trades) {
                                            auto_trade_appends.extend(current_trades[previous_trades.len()..].iter().cloned());
                                            continue;
                                        }
                                    }
                                }
                                if old.get(key) != Some(value) {
                                    changes.insert(key.clone(), value.clone());
                                }
                            }
                        }
                        previous = snapshot;
                        if !changes.is_empty() || !auto_trade_appends.is_empty() {
                            let Ok(payload) = serde_json::to_string(&serde_json::json!({ "type": "delta", "changes": changes, "auto_trade_appends": auto_trade_appends })) else { break };
                            if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = rates_refresh.tick() => {
                    let rates = serde_json::to_value(latest_stream_usd_rates()).unwrap_or(serde_json::Value::Null);
                    let account = account_ui_snapshot(&store, &rates).await;
                    let mut changes = serde_json::Map::new();
                    if previous.get("usd_rates") != Some(&rates) {
                        changes.insert("usd_rates".into(), rates.clone());
                    }
                    if previous.get("account") != Some(&account) {
                        changes.insert("account".into(), account.clone());
                    }
                    previous["usd_rates"] = rates;
                    previous["account"] = account;
                    if !changes.is_empty() {
                        let payload = serde_json::json!({ "type": "delta", "changes": changes }).to_string();
                        if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                    }
                },
                _ = hedge_refresh.tick() => {
                    let hedge = hedge_candidates_snapshot(&store).await;
                    if previous.get("hedge_candidates") != Some(&hedge) {
                        previous["hedge_candidates"] = hedge.clone();
                        let payload = serde_json::json!({ "type": "delta", "changes": { "hedge_candidates": hedge } }).to_string();
                        if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                    }
                },
                incoming = receiver.next() => if incoming.is_none() { break; },
            }
        }
    })
}

async fn position_ui_snapshot(store: &Arc<PositionStore>) -> serde_json::Value {
    let (positions, auto_positions, orders, trades, auto_trades, account) = {
        let ledger = store.ledger.lock().await;
        let positions = cached_position_summary(store, &ledger, None);
        let auto_positions = cached_position_summary(store, &ledger, Some("auto"));
        let orders = OrdersResponse { orders: ledger.orders.clone(), barrier_orders: ledger.barrier_orders.clone() };
        let trades = TradesResponse { trades: ledger.trades.iter().cloned().map(|mut trade| { trade.fee = signed_fee_cashflow(trade.fee); trade }).collect() };
        let auto_trades = TradesResponse { trades: ledger.trades.iter()
            .filter(|trade| trade.source == "auto")
            .cloned()
            .map(|mut trade| { trade.fee = signed_fee_cashflow(trade.fee); trade })
            .collect() };
        let rates = latest_stream_usd_rates().map(|rates| rates.rates).unwrap_or_default();
        let account = account_snapshot_with_rates(store, &ledger, &rates).ok();
        (positions, auto_positions, orders, trades, auto_trades, account)
    };
    let Json(vwap_runs) = get_vwap_runs(State(Arc::clone(store))).await;
    let Json(fee_rates) = get_fee_rates().await;
    serde_json::json!({
        "positions": positions,
        "auto_positions": auto_positions,
        "orders": orders,
        "trades": trades,
        "auto_trades": auto_trades,
        "account": account,
        "fee_rates": fee_rates,
        "vwap_runs": vwap_runs,
        "usd_rates": latest_stream_usd_rates(),
    })
}

async fn hedge_candidates_snapshot(store: &Arc<PositionStore>) -> serde_json::Value {
    let Json(response) = get_hedge_candidates(State(Arc::clone(store))).await;
    serde_json::to_value(response).unwrap_or(serde_json::Value::Null)
}

async fn account_ui_snapshot(store: &Arc<PositionStore>, rates: &serde_json::Value) -> serde_json::Value {
    let rates = rates.get("rates").cloned().and_then(|rates| serde_json::from_value(rates).ok()).unwrap_or_default();
    let ledger = store.ledger.lock().await;
    serde_json::to_value(account_snapshot_with_rates(store, &ledger, &rates).ok()).unwrap_or(serde_json::Value::Null)
}

async fn order_handler_timing(request: Request<Body>, next: Next) -> Response {
    let started = Instant::now();
    let mut response = next.run(request).await;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if let Ok(value) = elapsed_ms.to_string().parse() {
        response.headers_mut().insert(
            HeaderName::from_static("x-position-manager-handler-ms"),
            value,
        );
    }
    response
}


#[derive(Serialize)]
struct PersistenceTimingSummary {
    latest_ms: f64,
    average_ms: f64,
    p95_ms: f64,
    samples: usize,
}

async fn get_persistence_timing(
    State(store): State<Arc<PositionStore>>,
) -> Json<BTreeMap<String, PersistenceTimingSummary>> {
    let samples = store
        .persistence_timings_ms
        .lock()
        .map(|samples| samples.clone())
        .unwrap_or_default();
    Json(samples.into_iter().filter_map(|(phase, values)| {
        if values.is_empty() { return None; }
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        Some((phase, PersistenceTimingSummary {
            latest_ms: *values.last().unwrap_or(&0.0),
            average_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p95_ms: sorted[(95 * sorted.len()).div_ceil(100).saturating_sub(1)],
            samples: sorted.len(),
        }))
    }).collect())
}
