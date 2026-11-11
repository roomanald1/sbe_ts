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
                                refresh_order_usd_rate_snapshot();
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
    let router = Router::new()
        .route("/internal/persistence-timing", get(get_persistence_timing))
        .route("/positions", get(get_positions))
        .route("/positions/realized-pnl", get(get_realized_pnl))
        .route("/positions/fee-summary", get(get_fee_summary))
        .route("/positions/account", get(get_account))
        .route("/account", get(get_account))
        .route("/positions/account/topups", post(top_up_account))
        .route("/positions/account/leverage", post(set_account_leverage))
        .route("/positions/dated-carry/ledger", post(sync_dated_carry_ledger))
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
        .with_state(Arc::clone(&store))
        .merge(crate::users::router());
    start_position_rsocket(router.clone(), Arc::clone(&store));
    Ok(router)
}

async fn sync_dated_carry_ledger(
    State(store): State<Arc<PositionStore>>,
    Json(positions): Json<Vec<DatedCarryLedgerPosition>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if positions.iter().any(|position| {
        !position.quantity.is_finite()
            || position.quantity <= 0.0
            || !position.spot_entry_price.is_finite()
            || position.spot_entry_price <= 0.0
            || !position.future_entry_price.is_finite()
            || position.future_entry_price <= 0.0
            || position.spot_symbol.is_empty()
            || position.future_symbol.is_empty()
    }) {
        return Err((StatusCode::BAD_REQUEST, "Invalid dated-carry ledger position".into()));
    }
    let mut ledger = store.ledger.lock().await;
    ledger.dated_carry_positions = positions;
    store.persist(&ledger).await.map_err(|error| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not persist dated-carry ledger: {error}"))
    })?;
    Ok(Json(serde_json::json!({
        "status": "recorded",
        "positions": ledger.dated_carry_positions.len(),
    })))
}

fn position_ui_events(
    store: Arc<PositionStore>,
) -> impl futures_util::Stream<Item = Result<serde_json::Value, anyhow::Error>> + Send {
    let updates = store.state_updates.subscribe();
    let rates_refresh = tokio::time::interval(Duration::from_secs(30));
    let hedge_refresh = tokio::time::interval(Duration::from_secs(15));
    futures_util::stream::unfold(
        (store, updates, None, false, rates_refresh, hedge_refresh),
        |(store, mut updates, mut previous, initialized, mut rates_refresh, mut hedge_refresh)| async move {
            if !initialized {
                let mut snapshot = position_ui_snapshot(&store).await;
                snapshot["hedge_candidates"] = hedge_candidates_snapshot(&store).await;
                let event = serde_json::json!({ "type": "snapshot", "state": snapshot });
                return Some((Ok(event), (store, updates, Some(snapshot), true, rates_refresh, hedge_refresh)));
            }
            loop {
                tokio::select! {
                    update = updates.recv() => match update {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            let mut snapshot = position_ui_snapshot(&store).await;
                            snapshot["hedge_candidates"] = previous.as_ref().and_then(|old: &serde_json::Value| old.get("hedge_candidates")).cloned().unwrap_or(serde_json::Value::Null);
                            let mut changes = serde_json::Map::new();
                            let mut auto_trade_appends = Vec::new();
                            if let (Some(current), Some(old)) = (snapshot.as_object(), previous.as_ref().and_then(serde_json::Value::as_object)) {
                                for (key, value) in current {
                                    if key == "auto_trades" {
                                        let current_trades = value.get("trades").and_then(serde_json::Value::as_array);
                                        let previous_trades = old.get(key).and_then(|value| value.get("trades")).and_then(serde_json::Value::as_array);
                                        if let (Some(current_trades), Some(previous_trades)) = (current_trades, previous_trades) {
                                            let previous_last_id = previous_trades.last().and_then(|trade| trade.get("id"));
                                            if let Some(last_index) = previous_last_id.and_then(|id| current_trades.iter().position(|trade| trade.get("id") == Some(id))) {
                                                auto_trade_appends.extend(current_trades[last_index + 1..].iter().cloned());
                                                continue;
                                            }
                                            if current_trades.last().and_then(|trade| trade.get("id")) == previous_last_id { continue; }
                                        }
                                    }
                                    if old.get(key) != Some(value) { changes.insert(key.clone(), value.clone()); }
                                }
                            }
                            previous = Some(snapshot);
                            if !changes.is_empty() || !auto_trade_appends.is_empty() {
                                let event = serde_json::json!({ "type": "delta", "changes": changes, "auto_trade_appends": auto_trade_appends });
                                return Some((Ok(event), (store, updates, previous, true, rates_refresh, hedge_refresh)));
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    _ = rates_refresh.tick() => {
                        let rates = serde_json::to_value(latest_stream_usd_rates()).unwrap_or(serde_json::Value::Null);
                        let account = account_ui_snapshot(&store, &rates).await;
                        let mut changes = serde_json::Map::new();
                        if previous.as_ref().and_then(|p| p.get("usd_rates")) != Some(&rates) { changes.insert("usd_rates".into(), rates.clone()); }
                        if previous.as_ref().and_then(|p| p.get("account")) != Some(&account) { changes.insert("account".into(), account.clone()); }
                        if let Some(old) = previous.as_mut() { old["usd_rates"] = rates; old["account"] = account; }
                        if !changes.is_empty() {
                            return Some((Ok(serde_json::json!({ "type": "delta", "changes": changes })), (store, updates, previous, true, rates_refresh, hedge_refresh)));
                        }
                    },
                    _ = hedge_refresh.tick() => {
                        let hedge = hedge_candidates_snapshot(&store).await;
                        if previous.as_ref().and_then(|p| p.get("hedge_candidates")) != Some(&hedge) {
                            if let Some(old) = previous.as_mut() { old["hedge_candidates"] = hedge.clone(); }
                            return Some((Ok(serde_json::json!({ "type": "delta", "changes": { "hedge_candidates": hedge } })), (store, updates, previous, true, rates_refresh, hedge_refresh)));
                        }
                    },
                }
            }
        },
    )
}

async fn position_ui_snapshot(store: &Arc<PositionStore>) -> serde_json::Value {
    let (positions, auto_positions, orders, trades, auto_trades, account) = {
    let (ledger, _, _, _) = snapshot_position_ledger(&store.ledger).await;
        let positions = cached_position_summary(store, &ledger, None);
        let auto_positions = cached_position_summary(store, &ledger, Some("auto"));
        let orders = OrdersResponse { orders: ledger.orders.clone(), barrier_orders: ledger.barrier_orders.clone() };
        const STREAM_TRADE_LIMIT: usize = 256;
        let mut recent_trades = ledger.trades.iter().rev().take(STREAM_TRADE_LIMIT).cloned().map(|mut trade| { trade.fee = signed_fee_cashflow(trade.fee); trade }).collect::<Vec<_>>();
        recent_trades.reverse();
        let trades = TradesResponse { trades: recent_trades, total: None, next_offset: None, has_older: None };
        let mut recent_auto_trades = ledger.trades.iter().rev().filter(|trade| trade.source == "auto").take(STREAM_TRADE_LIMIT).cloned().map(|mut trade| { trade.fee = signed_fee_cashflow(trade.fee); trade }).collect::<Vec<_>>();
        recent_auto_trades.reverse();
        let auto_trades = TradesResponse { trades: recent_auto_trades, total: None, next_offset: None, has_older: None };
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
    let account = {
        let ledger = store.ledger.lock().await;
        account_snapshot_with_rates(store, &ledger, &rates).ok()
    };
    serde_json::to_value(account).unwrap_or(serde_json::Value::Null)
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

#[derive(Clone, serde::Deserialize)]
struct PositionRpcRequest {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<serde_json::Value>,
}

struct PositionRSocketResponder {
    router: Router,
    store: Arc<PositionStore>,
}

impl RSocket for PositionRSocketResponder {
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where Self: 'async_trait, 'life0: 'async_trait {
        Box::pin(async { Err(anyhow::anyhow!("RSocket metadata push is unsupported")) })
    }

    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where Self: 'async_trait, 'life0: 'async_trait {
        Box::pin(async { Err(anyhow::anyhow!("RSocket fire-and-forget is unsupported")) })
    }

    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<Option<Payload>>> + Send + 'async_trait>>
    where Self: 'async_trait, 'life0: 'async_trait {
        let router = self.router.clone();
        Box::pin(async move {
            let bytes = request.data().ok_or_else(|| anyhow::anyhow!("RSocket request data is required"))?;
            let rpc: PositionRpcRequest = serde_json::from_slice(bytes.as_ref())?;
            let method = rpc.method.parse::<axum::http::Method>()?;
            if !is_position_rpc_path(&rpc.path) {
                anyhow::bail!("unsupported Position Management RSocket path");
            }
            let body = rpc.body.map(|value| serde_json::to_vec(&value)).transpose()?.unwrap_or_default();
            let request = Request::builder()
                .method(method)
                .uri(&rpc.path)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))?;
            use tower::ServiceExt;
            let response = router.oneshot(request).await?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_owned(), value.to_owned()))
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await?;
            let response = serde_json::json!({
                "status": status,
                "headers": headers,
                "body": serde_json::from_slice::<serde_json::Value>(&body)
                    .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&body).into_owned()))
            });
            Ok(Some(Payload::new(Some(Bytes::from(serde_json::to_vec(&response)?)), None)))
        })
    }

    fn request_stream(
        &self,
        request: Payload,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
        let is_ui_stream = request.data().is_some_and(|data| data.as_ref() == b"position-ui:*");
        if !is_ui_stream {
            return Box::pin(futures_util::stream::once(async {
                Err(anyhow::anyhow!("unsupported Position Management RSocket stream"))
            }));
        }
        Box::pin(position_ui_events(Arc::clone(&self.store)).map(|event| {
            let event = event?;
            Ok(Payload::new(Some(Bytes::from(serde_json::to_vec(&event)?)), None))
        }))
    }

    fn request_channel(&self, _requests: Flux<Result<Payload, anyhow::Error>>) -> Flux<Result<Payload, anyhow::Error>> {
        futures_util::stream::once(async { Err(anyhow::anyhow!("RSocket request-channel is unsupported")) }).boxed()
    }
}

fn is_position_rpc_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    (path.starts_with("/positions") && !path.starts_with("/positions/auto-trader"))
        || matches!(path, "/account" | "/account/topups")
        || path.starts_with("/users/")
        || matches!(
            path,
            "/internal/insights-inputs"
                | "/internal/market-book"
                | "/internal/persistence-timing"
        )
        || (path.starts_with("/internal/market-book/") && path.ends_with("/subscribe"))
}

fn start_position_rsocket(router: Router, store: Arc<PositionStore>) {
    let port = std::env::var("POSITION_MANAGEMENT_RSOCKET_PORT").unwrap_or_else(|_| "10005".to_owned());
    tokio::spawn(async move {
        let address = format!("0.0.0.0:{port}");
        crate::service_log!("INFO", "Position Management RSocket server listening on ws://{address}");
        if let Err(error) = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(PositionRSocketResponder { router: router.clone(), store: Arc::clone(&store) }))
            }))
            .serve()
            .await
        {
            crate::service_log!("ERROR", "Position Management RSocket server stopped: {error:#}");
        }
    });
}

#[cfg(test)]
mod position_rsocket_contract_tests {
    use super::{PositionRpcRequest, is_position_rpc_path};

    #[test]
    fn accepts_only_position_management_rpc_routes() {
        for route in [
            "/positions?source=auto",
            "/positions/account",
            "/positions/trades?limit=100",
            "/account/topups",
            "/users/1",
            "/internal/insights-inputs?symbol=BTC-USDT",
            "/internal/market-book?symbol=BTC-USDT",
            "/internal/market-book/BTC-USDT/subscribe",
            "/internal/persistence-timing",
        ] {
            assert!(is_position_rpc_path(route), "unexpectedly rejected {route}");
        }
        for route in [
            "/positions/auto-trader",
            "/positions/auto-trader/activity",
            "/signals/stream",
            "/health",
            "/internal/other",
        ] {
            assert!(!is_position_rpc_path(route), "unexpectedly accepted {route}");
        }
    }

    #[test]
    fn rpc_request_contract_carries_method_path_query_and_body() {
        let request: PositionRpcRequest = serde_json::from_str(
            r#"{"method":"POST","path":"/positions/account/topups?dry_run=false","body":{"amount":25.0}}"#,
        ).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/positions/account/topups?dry_run=false");
        assert_eq!(request.body.unwrap()["amount"], 25.0);
    }
}
