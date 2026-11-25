// Queue behind ordinary ledger readers/writers instead of rejecting on contention.
async fn lock_order_ledger(
    ledger: &Mutex<Ledger>,
) -> Result<tokio::sync::MutexGuard<'_, Ledger>, (StatusCode, String)> {
    // A transient ledger lock must queue orders, not reject an already-filled
    // entry's reduce-only close and leave the position open.
    Ok(ledger.lock().await)
}

async fn submit_order(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    submit_order_inner(store, request, None).await
}

// The test route exercises the same handler and durable ledger path as the
// production endpoint while supplying a deterministic Paper Exchange match.
#[cfg(test)]
async fn submit_order_with_test_match(
    State((store, prepared)): State<(Arc<PositionStore>, PreparedMarketMatch)>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    submit_order_inner(store, request, Some(prepared)).await
}

async fn submit_order_inner(
    store: Arc<PositionStore>,
    mut request: NewOrder,
    prepared_override: Option<PreparedMarketMatch>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !matches!(request.source.as_str(), "manual" | "auto") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Trade source must be manual or auto".into(),
        ));
    }
    if !request.quantity.is_finite() || request.quantity <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Quantity must be finite and greater than zero".into(),
        ));
    }
    if let Some(client_order_id) = request.client_order_id.as_deref() {
        // Resolve known IDs before calling the matcher. Waiting here matters:
        // skipping this check on transient ledger contention could rematch a
        // successful no-fill request and turn a retry into a later fill.
        let ledger = store.ledger.lock().await;
        if let Some(response) = existing_order_response(&store, &ledger, client_order_id) {
            let needs_durable_commit = store.client_order_commit_pending(client_order_id);
            drop(ledger);
            if needs_durable_commit {
                store
                    .persist_durable_snapshot(Some(client_order_id))
                    .await
                    .map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unable to persist paper order: {error}"),
                        )
                    })?;
            }
            return Ok(Json(response));
        }
    }
    if matches!(
        &request.order_type,
        OrderType::Barrier | OrderType::StopMarket | OrderType::StopLimit | OrderType::TrailingStop
    ) {
        return submit_barrier_order(store, symbol, request).await;
    }
    let options = validate_market_order_options(&request)?;
    let maker_fee_rate = options.maker_fee_rate;
    let taker_fee_rate = options.taker_fee_rate;
    let is_post_only = options.post_only;
    let is_limit = options.limit;
    let is_fok = options.fill_or_kill;
    let limit_price = options.limit_price;
    let mut processing_timings_ms = BTreeMap::new();
    // Reduce-only orders cannot consume additional buying power, so they do
    // not need an FX lookup before matching or ledger persistence.
    let capacity_rates = if request.reduce_only {
        None
    } else {
        let rates_started = Instant::now();
        let rates = cached_usd_rates_for_order().await?;
        processing_timings_ms.insert(
            "capacity_rate_lookup".to_owned(),
            rates_started.elapsed().as_secs_f64() * 1_000.0,
        );
        Some(rates)
    };
    // Paper matching runs in the Paper Exchange service and can perform
    // network I/O; do not hold the position ledger lock while waiting on it.
    let prepared = match prepared_override {
        Some(prepared) => prepared,
        None => prepare_market_match(&symbol, &request, &options).await?,
    };
    let mut matched = prepared.matched;
    let decision_timestamp = prepared.decision_timestamp;
    let decision_price = prepared.decision_price;
    let decision_mid_price = prepared.decision_mid_price;
    processing_timings_ms.extend(prepared.processing_timings_ms);
    let ledger_lock_started = Instant::now();
    let mut ledger = lock_order_ledger(&store.ledger).await?;
    processing_timings_ms.insert(
        "ledger_lock_wait".to_owned(),
        ledger_lock_started.elapsed().as_secs_f64() * 1_000.0,
    );
    if let Some(response) = request
        .client_order_id
        .as_deref()
        .and_then(|id| existing_order_response(&store, &ledger, id))
    {
        let client_order_id = request.client_order_id.as_deref().unwrap();
        let needs_durable_commit = store.client_order_commit_pending(client_order_id);
        drop(ledger);
        if needs_durable_commit {
            store
                .persist_durable_snapshot(Some(client_order_id))
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Unable to persist paper order: {error}"),
                    )
                })?;
        }
        return Ok(Json(response));
    }
    let timestamp = now_ms().map_err(internal_error)?;
    if request.reduce_only {
        let current_quantity = indexed_position_summary(&store, &ledger, Some(&request.source))
            .positions
            .into_iter()
            .find(|position| position.symbol == symbol)
            .map(|position| position.quantity)
            .unwrap_or(0.0);
        let closes_position = match (&request.side, current_quantity) {
            (TradeSide::Buy, quantity) if quantity < -EPSILON => true,
            (TradeSide::Sell, quantity) if quantity > EPSILON => true,
            _ => false,
        };
        if !closes_position {
            // A stale protective barrier must never open or reverse a position.
            let durable_idempotency_record =
                remember_empty_no_fill_response(&mut ledger, request.client_order_id.as_deref());
            if durable_idempotency_record {
                let client_order_id = request.client_order_id.as_deref().unwrap();
                store.mark_client_order_commit_pending(client_order_id);
                drop(ledger);
                store
                    .persist_durable_snapshot(Some(client_order_id))
                    .await
                    .map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unable to persist paper order: {error}"),
                        )
                    })?;
            }
            return Ok(Json(OrderResponse {
                estimated_fee: 0.0,
                fee_complete: true,
                unknown_fee_fills: 0,
                filled_quantity: 0.0,
                remaining_quantity: 0.0,
                average_execution_price: None,
                order: None,
                barrier_order: None,
                fill_evidence: Vec::new(),
                processing_timings_ms,
            }));
        }
        request.quantity = request.quantity.min(current_quantity.abs());
        if request.quantity <= EPSILON {
            let durable_idempotency_record =
                remember_empty_no_fill_response(&mut ledger, request.client_order_id.as_deref());
            if durable_idempotency_record {
                let client_order_id = request.client_order_id.as_deref().unwrap();
                store.mark_client_order_commit_pending(client_order_id);
                drop(ledger);
                store
                    .persist_durable_snapshot(Some(client_order_id))
                    .await
                    .map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unable to persist paper order: {error}"),
                        )
                    })?;
            }
            return Ok(Json(OrderResponse {
                estimated_fee: 0.0,
                fee_complete: true,
                unknown_fee_fills: 0,
                filled_quantity: 0.0,
                remaining_quantity: 0.0,
                average_execution_price: None,
                order: None,
                barrier_order: None,
                fill_evidence: Vec::new(),
                processing_timings_ms,
            }));
        }
        // Recompute while holding the ledger lock so simultaneous stop barriers
        // see the quantity left after the preceding barrier fill.
        limit_match_quantity(&mut matched, request.quantity);
    }
    if is_fok && matched.remaining_quantity > EPSILON {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "FOK order cancelled: only {} of {} was available",
                matched.filled_quantity, request.quantity
            ),
        ));
    }
    let reference_price = matched.reference_price;
    let remaining = matched.remaining_quantity;
    let filled_quantity = matched.filled_quantity;
    let average_execution_price = matched.average_execution_price;
    let reserve_price = if matches!(request.side, TradeSide::Buy) {
        matched.worst_ask.unwrap_or(0.0)
    } else {
        reference_price.unwrap_or(0.0)
    };
    let fills = matched
        .fills
        .into_iter()
        .map(|fill| (fill.quantity, fill.price))
        .collect::<Vec<_>>();
    let capacity_started = Instant::now();
    if !request.reduce_only {
        ensure_paper_capacity_with_rates(
            &store,
            &ledger,
            &capacity_rates
                .as_ref()
                .expect("capacity rates loaded for opening order")
                .rates,
            &symbol,
            &request.side,
            request.quantity,
            reserve_price,
            taker_fee_rate,
            Some(&mut processing_timings_ms),
        )?;
    }
    processing_timings_ms.insert(
        "capacity_check".to_owned(),
        capacity_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let start = ledger.trades.len();
    for (index, (quantity, execution_price)) in fills.into_iter().enumerate() {
        let fill_id = allocate_fill_id(&mut ledger, timestamp);
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{}", start),
            fill_id: Some(fill_id),
            symbol: symbol.clone(),
            side: request.side.clone(),
            quantity,
            execution_price,
            reference_price,
            timestamp,
            source: request.source.clone(),
            parent_order_id: request.client_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            lifecycle: request.lifecycle.clone(),
            fee: -quantity * execution_price * taker_fee_rate.abs(),
            fee_known: true,
            fee_currency: symbol
                .split_once('-')
                .map(|(_, quote)| quote.to_owned())
                .unwrap_or_default(),
            fee_rate: taker_fee_rate,
            fee_liquidity: "taker".to_owned(),
            resting_order_id: None,
            decision_timestamp,
            decision_price,
            decision_mid_price,
            slippage: decision_price.map(|price| {
                if matches!(request.side, TradeSide::Sell) {
                    execution_price - price
                } else {
                    price - execution_price
                }
            }),
            slippage_bps: decision_price.filter(|price| *price > 0.0).map(|price| {
                let favorable_difference = if matches!(request.side, TradeSide::Sell) {
                    execution_price - price
                } else {
                    price - execution_price
                };
                favorable_difference / price * 10_000.0
            }),
        });
    }
    // Protective AutoTrader barriers are siblings of the position they guard.
    // Retire them as soon as the authoritative auto ledger becomes flat, rather
    // than waiting for the next AutoTrader maintenance pass to notice.
    if request.source == "auto" {
        let auto_quantity = indexed_position_summary(&store, &ledger, Some("auto"))
            .positions
            .into_iter()
            .find(|position| position.symbol == symbol)
            .map(|position| position.quantity)
            .unwrap_or(0.0);
        if auto_quantity.abs() <= EPSILON {
            for barrier in &mut ledger.barrier_orders {
                if barrier.symbol == symbol
                    && barrier.source == "auto"
                    && barrier.status == "armed"
                    && barrier
                        .parent_order_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with("auto-stop-"))
                {
                    barrier.status = "cancelled".to_owned();
                    barrier.retry_after = None;
                }
            }
        }
    }
    let order = if is_limit && remaining > EPSILON {
        let order = RestingOrder {
            id: format!("order-{timestamp}-{}", ledger.orders.len()),
            symbol,
            side: request.side,
            quantity: request.quantity,
            remaining_quantity: remaining,
            limit_price: limit_price.unwrap(),
            timestamp,
            maker_fee_rate,
            estimated_fee: remaining * limit_price.unwrap() * -maker_fee_rate,
            expires_at: request.expires_at,
            time_in_force: if request.expires_at.is_some() {
                "gtd".to_owned()
            } else {
                "gtc".to_owned()
            },
            post_only: is_post_only,
            source: request.source.clone(),
            parent_order_id: request.client_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            lifecycle: request.lifecycle.clone(),
            reduce_only: request.reduce_only,
        };
        ledger.orders.push(order.clone());
        Some(order)
    } else {
        None
    };
    let (estimated_fee, unknown_fee_fills) =
        fee_total_and_unknown_count(ledger.trades[start..].iter());
    let fee_complete = unknown_fee_fills == 0;
    let is_no_fill_without_resting_order = filled_quantity <= EPSILON && order.is_none();
    if is_no_fill_without_resting_order {
        if let Some(client_order_id) = request.client_order_id.as_ref() {
            remember_no_fill_order_response(
                &mut ledger,
                NoFillOrderResponse {
                    client_order_id: client_order_id.clone(),
                    estimated_fee,
                    fee_complete,
                    unknown_fee_fills,
                    filled_quantity,
                    remaining_quantity: remaining,
                    average_execution_price,
                },
            );
        }
    }
    let fill_evidence = request
        .client_order_id
        .as_deref()
        .map_or_else(Vec::new, |client_id| {
            ledger
                .trades
                .iter()
                .filter(|trade| {
                    trade.client_order_id.as_deref() == Some(client_id)
                        || trade.parent_order_id.as_deref() == Some(client_id)
                })
                .cloned()
                .collect()
        });
    let persistence_started = Instant::now();
    if request.client_order_id.is_some() {
        let client_order_id = request.client_order_id.as_deref().unwrap();
        store.mark_client_order_commit_pending(client_order_id);
        drop(ledger);
        store
            .persist_durable_snapshot(Some(client_order_id))
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to persist paper order: {error}"),
                )
            })?;
    } else if let Err(error) = store.persist(&ledger).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist paper order: {error}"),
        ));
    }
    processing_timings_ms.insert(
        if request.client_order_id.is_some() {
            "persistence_durable_commit"
        } else {
            "persistence_enqueue"
        }
        .to_owned(),
        persistence_started.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(Json(OrderResponse {
        filled_quantity,
        estimated_fee,
        fee_complete,
        unknown_fee_fills,
        remaining_quantity: remaining,
        average_execution_price,
        order,
        barrier_order: None,
        fill_evidence,
        processing_timings_ms,
    }))
}

fn existing_order_response(
    store: &PositionStore,
    ledger: &Ledger,
    client_order_id: &str,
) -> Option<OrderResponse> {
    let order = ledger
        .orders
        .iter()
        .find(|order| order.parent_order_id.as_deref() == Some(client_order_id))
        .cloned();
    let barrier_order = ledger
        .barrier_orders
        .iter()
        .find(|order| order.parent_order_id.as_deref() == Some(client_order_id))
        .cloned();
    let fills = with_ledger_trade_index(store, ledger, |index| {
        index
            .by_parent
            .get(client_order_id)
            .into_iter()
            .flatten()
            .map(|&offset| &ledger.trades[offset])
            .collect::<Vec<_>>()
    });
    if order.is_none() && barrier_order.is_none() && fills.is_empty() {
        return ledger
            .no_fill_order_responses
            .iter()
            .rev()
            .find(|record| record.client_order_id == client_order_id)
            .map(|record| OrderResponse {
                estimated_fee: record.estimated_fee,
                fee_complete: record.fee_complete,
                unknown_fee_fills: record.unknown_fee_fills,
                filled_quantity: record.filled_quantity,
                remaining_quantity: record.remaining_quantity,
                average_execution_price: record.average_execution_price,
                order: None,
                barrier_order: None,
                fill_evidence: Vec::new(),
                processing_timings_ms: BTreeMap::new(),
            });
    }
    let filled_quantity = fills.iter().map(|trade| trade.quantity).sum::<f64>();
    let filled_notional = fills
        .iter()
        .map(|trade| trade.quantity * trade.execution_price)
        .sum::<f64>();
    let (estimated_fee, unknown_fee_fills) = fee_total_and_unknown_count(fills.iter().copied());
    Some(OrderResponse {
        estimated_fee,
        fee_complete: unknown_fee_fills == 0,
        unknown_fee_fills,
        filled_quantity,
        remaining_quantity: order
            .as_ref()
            .map(|order| order.remaining_quantity)
            .or_else(|| barrier_order.as_ref().map(|order| order.remaining_quantity))
            .unwrap_or(0.0),
        average_execution_price: (filled_quantity > EPSILON)
            .then_some(filled_notional / filled_quantity),
        order,
        barrier_order,
        fill_evidence: fills.into_iter().cloned().collect(),
        processing_timings_ms: BTreeMap::new(),
    })
}

fn remember_empty_no_fill_response(ledger: &mut Ledger, client_order_id: Option<&str>) -> bool {
    if let Some(client_order_id) = client_order_id {
        remember_no_fill_order_response(
            ledger,
            NoFillOrderResponse {
                client_order_id: client_order_id.to_owned(),
                estimated_fee: 0.0,
                fee_complete: true,
                unknown_fee_fills: 0,
                filled_quantity: 0.0,
                remaining_quantity: 0.0,
                average_execution_price: None,
            },
        );
        true
    } else {
        false
    }
}

fn fee_total_and_unknown_count<'a>(
    trades: impl Iterator<Item = &'a PositionTrade>,
) -> (f64, usize) {
    trades.fold((0.0, 0), |(known_total, unknown_count), trade| {
        if trade.fee_known {
            (known_total + trade.fee.abs(), unknown_count)
        } else {
            (known_total, unknown_count + 1)
        }
    })
}

#[cfg(test)]
mod order_ledger_contention_tests {
    use super::*;

    fn response_store(ledger: Ledger, path: PathBuf) -> PositionStore {
        let (state_updates, _) = broadcast::channel(8);
        PositionStore {
            path,
            ledger: Arc::new(Mutex::new(ledger)),
            persistence_notify: Arc::new(Notify::new()),
            persistence_write_lock: Arc::new(Mutex::new(())),
            pending_client_order_commits: StdMutex::new(HashSet::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            market_marks: StdRwLock::new(HashMap::new()),
            market_mark_pollers: StdMutex::new(HashSet::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn historical_order_fee_total_marks_unknown_and_keeps_known_zero() {
        let legacy: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"legacy","symbol":"BTC-USDT","side":"buy","quantity":1.0,
            "execution_price":100.0,"timestamp":1
        }))
        .unwrap();
        let known_zero: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"zero","symbol":"BTC-USDT","side":"sell","quantity":1.0,
            "execution_price":100.0,"timestamp":2,"fee":0.0,"fee_currency":"USDT","fee_known":true
        }))
        .unwrap();
        let (fee_total, unknown_count) =
            fee_total_and_unknown_count([&legacy, &known_zero].into_iter());
        assert_eq!(fee_total, 0.0);
        assert_eq!(unknown_count, 1);
        let response = OrderResponse {
            estimated_fee: fee_total,
            fee_complete: unknown_count == 0,
            unknown_fee_fills: unknown_count,
            filled_quantity: 2.0,
            remaining_quantity: 0.0,
            average_execution_price: Some(100.0),
            order: None,
            barrier_order: None,
            fill_evidence: vec![],
            processing_timings_ms: BTreeMap::new(),
        };
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["estimated_fee"], 0.0);
        assert_eq!(json["fee_complete"], false);
        assert_eq!(json["unknown_fee_fills"], 1);
    }

    #[tokio::test]
    async fn submission_waits_for_a_busy_ledger() {
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let held = ledger.lock().await;
        let pending_ledger = Arc::clone(&ledger);
        let pending = tokio::spawn(async move { lock_order_ledger(&pending_ledger).await.is_ok() });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        drop(held);
        assert!(pending.await.unwrap());
    }

    #[tokio::test]
    async fn lifecycle_fill_submitted_through_pm_route_survives_sqlite_restart() {
        use axum::{body::to_bytes, routing::post};
        use tower::ServiceExt;

        let path = std::env::temp_dir().join(format!(
            "position-management-lifecycle-restart-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let lifecycle = serde_json::json!({
            "schema_version": 1,
            "opportunity_id": "opp-v35-42",
            "episode_id": "episode-v35-42",
            "trade_id": "trade-v35-42",
            "profile_id": "profile-7",
            "profile_hash": "sha256:profile-hash",
            "profile_version": 7,
            "execution_mode": "paper",
            "entry_policy": "explore",
            "source": "auto"
        });
        let legacy: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "legacy-position-fill",
            "symbol": "BTC-USDT",
            "side": "buy",
            "quantity": 0.25,
            "execution_price": 100.0,
            "timestamp": 1,
            "source": "auto"
        }))
        .unwrap();
        assert_eq!(legacy.fill_id, None);
        assert_eq!(legacy.lifecycle, None);

        let mut ledger = Ledger::default();
        // This historical row supplies the open quantity for the reduce-only
        // order and deliberately has no retrospectively inferred identities.
        ledger.trades.push(legacy);
        let store = Arc::new(response_store(ledger, path.clone()));
        let prepared = PreparedMarketMatch {
            matched: MatcherResponse {
                reference_price: Some(110.0),
                best_bid: Some(109.0),
                best_ask: Some(111.0),
                worst_ask: Some(111.0),
                book_updated_at_ms: 2,
                fills: vec![MatcherFill {
                    quantity: 0.25,
                    price: 110.0,
                }],
                remaining_quantity: 0.0,
                filled_quantity: 0.25,
                average_execution_price: Some(110.0),
            },
            decision_timestamp: None,
            decision_price: None,
            decision_mid_price: None,
            processing_timings_ms: BTreeMap::new(),
        };
        let submit_router = Router::new()
            .route("/positions/orders", post(submit_order_with_test_match))
            .with_state((Arc::clone(&store), prepared));
        let request = Request::builder()
            .method("POST")
            .uri("/positions/orders")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "symbol": "BTC-USDT",
                    "side": "sell",
                    "order_type": "market",
                    "quantity": 0.25,
                    "source": "auto",
                    "client_order_id": "auto-close-v35-42",
                    "reduce_only": true,
                    "lifecycle": lifecycle
                }))
                .unwrap(),
            ))
            .unwrap();
        let response = submit_router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // The normal request handler performs the durable client-order commit.
        let before_restart = store.ledger.lock().await;
        let submitted = before_restart
            .trades
            .iter()
            .find(|trade| trade.client_order_id.as_deref() == Some("auto-close-v35-42"))
            .expect("PM route appends the requested lifecycle fill");
        let stable_fill_id = submitted
            .fill_id
            .clone()
            .expect("PM allocates a durable fill identity");
        assert!(stable_fill_id.starts_with("pm-fill:v1:"));
        assert_eq!(
            submitted.lifecycle.as_ref().unwrap().trade_id,
            "trade-v35-42"
        );
        drop(before_restart);

        let restored = load_sqlite_ledger(&path).unwrap();
        let restarted_store = Arc::new(response_store(restored, path.clone()));
        let history_router = position_router(Arc::clone(&restarted_store));
        let history_request = Request::builder()
            .method("GET")
            .uri("/positions/trades?source=auto")
            .body(Body::empty())
            .unwrap();
        let history_response = history_router.oneshot(history_request).await.unwrap();
        assert_eq!(history_response.status(), StatusCode::OK);
        let history_body = to_bytes(history_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let history: serde_json::Value = serde_json::from_slice(&history_body).unwrap();
        let trades = history["trades"].as_array().unwrap();
        let recovered = trades
            .iter()
            .find(|trade| trade["client_order_id"] == "auto-close-v35-42")
            .expect("restarted PM trade-history projection includes the submitted fill");
        assert_eq!(recovered["fill_id"], stable_fill_id);
        assert_eq!(recovered["lifecycle"], lifecycle);

        let legacy_projection = trades
            .iter()
            .find(|trade| trade["id"] == "legacy-position-fill")
            .expect("legacy fill remains in trade history");
        assert!(legacy_projection.get("fill_id").is_none());
        assert!(legacy_projection.get("lifecycle").is_none());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn no_fill_client_order_id_replays_after_durable_sqlite_restart() {
        let path = std::env::temp_dir().join(format!(
            "position-management-no-fill-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let record = NoFillOrderResponse {
            client_order_id: "retry-me".to_owned(),
            estimated_fee: 0.0,
            fee_complete: true,
            unknown_fee_fills: 0,
            filled_quantity: 0.0,
            remaining_quantity: 0.75,
            average_execution_price: None,
        };
        let mut ledger = Ledger::default();
        remember_no_fill_order_response(&mut ledger, record);

        let store = response_store(ledger.clone(), path.clone());
        let before_restart = existing_order_response(&store, &ledger, "retry-me").unwrap();
        assert_eq!(before_restart.filled_quantity, 0.0);
        assert_eq!(before_restart.remaining_quantity, 0.75);
        assert!(before_restart.order.is_none());
        assert!(before_restart.barrier_order.is_none());

        store.mark_client_order_commit_pending("retry-me");
        store
            .persist_durable_snapshot(Some("retry-me"))
            .await
            .unwrap();
        let restored = load_sqlite_ledger(&path).unwrap();
        let restarted_store = response_store(restored.clone(), path.clone());
        let after_restart =
            existing_order_response(&restarted_store, &restored, "retry-me").unwrap();
        assert_eq!(
            after_restart.filled_quantity,
            before_restart.filled_quantity
        );
        assert_eq!(
            after_restart.remaining_quantity,
            before_restart.remaining_quantity
        );
        assert_eq!(after_restart.estimated_fee, before_restart.estimated_fee);
        assert_eq!(after_restart.fee_complete, before_restart.fee_complete);
        assert!(after_restart.order.is_none());
        assert!(after_restart.barrier_order.is_none());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn durable_client_order_records_replay_fills_resting_and_barrier_orders() {
        let path = std::env::temp_dir().join(format!(
            "position-management-durable-orders-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let fill: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"fill-1", "fill_id":"pm-fill:v1:2:1", "symbol":"BTC-USDT", "side":"buy", "quantity":0.25,
            "execution_price":40000.0, "timestamp":10, "parent_order_id":"filled-id",
            "client_order_id":"filled-client-id",
            "fee":-1.0, "fee_known":true, "fee_currency":"USDT"
        }))
        .unwrap();
        let resting: RestingOrder = serde_json::from_value(serde_json::json!({
            "id":"resting-1", "symbol":"BTC-USDT", "side":"buy", "quantity":1.0,
            "remaining_quantity":0.5, "limit_price":39900.0, "timestamp":11,
            "parent_order_id":"resting-id", "client_order_id":"resting-client-id"
        }))
        .unwrap();
        let barrier: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id":"barrier-1", "symbol":"BTC-USDT", "side":"sell", "quantity":0.25,
            "remaining_quantity":0.25, "filled_quantity":0.0, "trigger_price":39000.0,
            "trigger_direction":"below", "created_at":12, "status":"armed",
            "parent_order_id":"barrier-id", "client_order_id":"barrier-client-id"
        }))
        .unwrap();
        let mut ledger = Ledger::default();
        ledger.trades.push(fill);
        ledger.orders.push(resting);
        ledger.barrier_orders.push(barrier);

        let store = response_store(ledger, path.clone());
        store.persist_durable_snapshot(None).await.unwrap();
        let restored = load_sqlite_ledger(&path).unwrap();
        let restarted_store = response_store(restored.clone(), path.clone());

        let filled = existing_order_response(&restarted_store, &restored, "filled-id").unwrap();
        assert_eq!(filled.filled_quantity, 0.25);
        assert_eq!(filled.fill_evidence.len(), 1);
        assert_eq!(
            filled.fill_evidence[0].fill_id.as_deref(),
            Some("pm-fill:v1:2:1")
        );
        assert_eq!(filled.fill_evidence[0].fee_currency, "USDT");
        assert!(filled.fill_evidence[0].fee_known);
        let response_json = serde_json::to_value(&filled).unwrap();
        assert_eq!(
            response_json["fill_evidence"][0]["fill_id"],
            "pm-fill:v1:2:1"
        );
        assert_eq!(
            restored.trades[0].client_order_id.as_deref(),
            Some("filled-client-id")
        );
        let resting = existing_order_response(&restarted_store, &restored, "resting-id").unwrap();
        assert_eq!(resting.remaining_quantity, 0.5);
        assert!(resting.order.is_some());
        assert_eq!(
            restored.orders[0].client_order_id.as_deref(),
            Some("resting-client-id")
        );
        let barrier = existing_order_response(&restarted_store, &restored, "barrier-id").unwrap();
        assert_eq!(barrier.remaining_quantity, 0.25);
        assert!(barrier.barrier_order.is_some());
        assert_eq!(
            restored.barrier_orders[0].client_order_id.as_deref(),
            Some("barrier-client-id")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn failed_durable_commit_keeps_client_order_pending_for_retry() {
        let path = std::env::temp_dir().join(format!(
            "position-management-durable-retry-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut ledger = Ledger::default();
        remember_no_fill_order_response(
            &mut ledger,
            NoFillOrderResponse {
                client_order_id: "retry-durable".to_owned(),
                estimated_fee: 0.0,
                fee_complete: true,
                unknown_fee_fills: 0,
                filled_quantity: 0.0,
                remaining_quantity: 1.0,
                average_execution_price: None,
            },
        );
        let store = response_store(ledger, path.clone());
        store.mark_client_order_commit_pending("retry-durable");
        std::fs::create_dir(&path).unwrap();

        assert!(
            store
                .persist_durable_snapshot(Some("retry-durable"))
                .await
                .is_err()
        );
        assert!(store.client_order_commit_pending("retry-durable"));

        std::fs::remove_dir(&path).unwrap();
        store
            .persist_durable_snapshot(Some("retry-durable"))
            .await
            .unwrap();
        assert!(!store.client_order_commit_pending("retry-durable"));
        let restored = load_sqlite_ledger(&path).unwrap();
        assert_eq!(
            restored.no_fill_order_responses[0].client_order_id,
            "retry-durable"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
