async fn submit_order(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let mut request = request;
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
    let prepared = prepare_market_match(&store, &symbol, &request, &options).await?;
    let book = prepared.book;
    let mut matched = prepared.matched;
    let decision_timestamp = prepared.decision_timestamp;
    let decision_price = prepared.decision_price;
    let decision_mid_price = prepared.decision_mid_price;
    let mut processing_timings_ms = prepared.processing_timings_ms;
    let levels = match request.side {
        TradeSide::Buy => &book.asks,
        TradeSide::Sell => &book.bids,
    };
    let timestamp = now_ms().map_err(internal_error)?;
    // Reduce-only orders cannot consume additional buying power, so they do
    // not need an FX lookup before matching or ledger persistence.
    let capacity_rates = if request.reduce_only {
        None
    } else {
        let rates_started = Instant::now();
        let rates = get_usd_rates().await?.0.rates;
        processing_timings_ms.insert(
            "capacity_rate_lookup".to_owned(),
            rates_started.elapsed().as_secs_f64() * 1_000.0,
        );
        Some(rates)
    };
    let ledger_lock_started = Instant::now();
    let mut ledger = store.ledger.lock().await;
    processing_timings_ms.insert(
        "ledger_lock_wait".to_owned(),
        ledger_lock_started.elapsed().as_secs_f64() * 1_000.0,
    );
    if request.reduce_only {
        let current_quantity = summarize_positions_from(
            ledger.trades.iter().filter(|trade| trade.source == request.source),
        )
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
            return Ok(Json(OrderResponse {
                estimated_fee: 0.0,
                filled_quantity: 0.0,
                remaining_quantity: 0.0,
                average_execution_price: None,
                order: None,
                barrier_order: None,
                processing_timings_ms,
            }));
        }
        request.quantity = request.quantity.min(current_quantity.abs());
        if request.quantity <= EPSILON {
            return Ok(Json(OrderResponse {
                estimated_fee: 0.0,
                filled_quantity: 0.0,
                remaining_quantity: 0.0,
                average_execution_price: None,
                order: None,
                barrier_order: None,
                processing_timings_ms,
            }));
        }
        // Recompute while holding the ledger lock so simultaneous stop barriers
        // see the quantity left after the preceding barrier fill.
        matched = match_book(&request.side, request.quantity, limit_price, &book);
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
    let fills = matched
        .fills
        .into_iter()
        .map(|fill| (fill.quantity, fill.price))
        .collect::<Vec<_>>();
    let capacity_started = Instant::now();
    let reserve_price = if matches!(request.side, TradeSide::Buy) {
        levels
            .iter()
            .map(|level| level.price)
            .fold(0.0_f64, f64::max)
    } else {
        reference_price.unwrap_or(0.0)
    };
    if !request.reduce_only {
        ensure_paper_capacity_with_rates(
            &store,
            &ledger,
            capacity_rates.as_ref().expect("capacity rates loaded for opening order"),
            &symbol,
            &request.side,
            request.quantity,
            reserve_price,
            taker_fee_rate,
            Some(&mut processing_timings_ms),
        )
        .await?;
    }
    processing_timings_ms.insert(
        "capacity_check".to_owned(),
        capacity_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let start = ledger.trades.len();
    for (index, (quantity, execution_price)) in fills.into_iter().enumerate() {
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{}", start),
            symbol: symbol.clone(),
            side: request.side.clone(),
            quantity,
            execution_price,
            reference_price,
            timestamp,
            source: request.source.clone(),
            parent_order_id: request.client_order_id.clone(),
            fee: -quantity * execution_price * taker_fee_rate.abs(),
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
        let auto_quantity = summarize_positions_from(
            ledger.trades.iter().filter(|trade| trade.source == "auto"),
        )
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
        };
        ledger.orders.push(order.clone());
        Some(order)
    } else {
        None
    };
    let persistence_started = Instant::now();
    if let Err(error) = store.persist(&ledger).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist paper order: {error}"),
        ));
    }
    processing_timings_ms.insert(
        "persistence_enqueue".to_owned(),
        persistence_started.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(Json(OrderResponse {
        filled_quantity,
        estimated_fee: ledger.trades[start..]
            .iter()
            .map(|trade| trade.fee.abs())
            .sum::<f64>(),
        remaining_quantity: remaining,
        average_execution_price,
        order,
        barrier_order: None,
        processing_timings_ms,
    }))
}


