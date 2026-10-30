async fn submit_barrier_order(
    store: Arc<PositionStore>,
    symbol: String,
    request: NewOrder,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    store.ensure_subscription(&symbol);
    let trailing = matches!(&request.order_type, OrderType::TrailingStop);
    let (trigger_price, trigger_direction, trailing_amount, trailing_percent, extreme_price) =
        if trailing {
            let amount = request
                .trailing_amount
                .filter(|value| value.is_finite() && *value > 0.0);
            let percent = request
                .trailing_percent
                .filter(|value| value.is_finite() && *value > 0.0 && *value < 1.0);
            if amount.is_some() == percent.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Trailing stop needs exactly one positive amount or percentage below 100%"
                        .into(),
                ));
            }
            let book = store.book(&symbol).ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Live book required to arm trailing stop".into(),
            ))?;
            let current = match request.side {
                TradeSide::Sell => book.bids.first().map(|l| l.price),
                TradeSide::Buy => book.asks.first().map(|l| l.price),
            }
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Live trigger reference price unavailable".into(),
            ))?;
            let distance = amount.unwrap_or(current * percent.unwrap());
            let initial = if matches!(request.side, TradeSide::Sell) {
                current - distance
            } else {
                current + distance
            };
            if !initial.is_finite() || initial <= 0.0 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Trailing distance produces an invalid trigger price".into(),
                ));
            }
            (
                initial,
                if matches!(request.side, TradeSide::Sell) {
                    BarrierDirection::Below
                } else {
                    BarrierDirection::Above
                },
                amount,
                percent,
                Some(current),
            )
        } else {
            (
                request
                    .trigger_price
                    .filter(|price| price.is_finite() && *price > 0.0)
                    .ok_or((
                        StatusCode::BAD_REQUEST,
                        "Trigger price must be finite and greater than zero".to_owned(),
                    ))?,
                request.trigger_direction.clone().ok_or((
                    StatusCode::BAD_REQUEST,
                    "Trigger direction must be above or below".to_owned(),
                ))?,
                None,
                None,
                None,
            )
        };
    let execution_type = if matches!(&request.order_type, OrderType::StopLimit) {
        "limit"
    } else {
        "market"
    };
    let stop_limit_price = if execution_type == "limit" {
        Some(
            request
                .limit_price
                .filter(|p| p.is_finite() && *p > 0.0)
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    "Stop-limit requires a positive limit price".into(),
                ))?,
        )
    } else {
        None
    };
    let timestamp = now_ms().map_err(internal_error)?;
    let mut ledger = store.ledger.try_lock().map_err(|_| {
        (StatusCode::SERVICE_UNAVAILABLE, "Position ledger busy".to_owned())
    })?;
    if let Some(response) = request
        .client_order_id
        .as_deref()
        .and_then(|id| existing_order_response(&ledger, id))
    {
        return Ok(Json(response));
    }
    ensure_paper_capacity(
        &store,
        &ledger,
        &symbol,
        &request.side,
        request.quantity,
        stop_limit_price.unwrap_or(trigger_price),
        configured_fee_rates().taker,
    )
    .await?;
    let barrier_order = BarrierOrder {
        id: format!("barrier-{timestamp}-{}", ledger.barrier_orders.len()),
        symbol,
        side: request.side,
        quantity: request.quantity,
        remaining_quantity: request.quantity,
        filled_quantity: 0.0,
        trigger_price,
        trigger_direction,
        created_at: timestamp,
        triggered_at: None,
        retry_after: None,
        status: "armed".to_owned(),
        average_execution_price: None,
        fee_paid: 0.0,
        error: None,
        order_kind: if trailing {
            "trailing-stop"
        } else if execution_type == "limit" {
            "stop-limit"
        } else if matches!(&request.order_type, OrderType::StopMarket) {
            "stop-market"
        } else {
            "barrier"
        }
        .to_owned(),
        execution_type: execution_type.to_owned(),
        limit_price: stop_limit_price,
        trailing_amount,
        trailing_percent,
        extreme_price,
        source: request.source.clone(),
        parent_order_id: request.client_order_id.clone(),
    };
    ledger.barrier_orders.push(barrier_order.clone());
    if let Err(error) = store.persist(&ledger).await {
        ledger.barrier_orders.pop();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist barrier order: {error}"),
        ));
    }
    return Ok(Json(OrderResponse {
        estimated_fee: 0.0,
        filled_quantity: 0.0,
        remaining_quantity: request.quantity,
        average_execution_price: None,
        order: None,
        barrier_order: Some(barrier_order),
        processing_timings_ms: BTreeMap::new(),
    }));
}
