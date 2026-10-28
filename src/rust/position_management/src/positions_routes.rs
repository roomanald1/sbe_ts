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

