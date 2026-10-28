async fn get_orders(State(store): State<Arc<PositionStore>>) -> Json<OrdersResponse> {
    let ledger = store.ledger.lock().await;
    Json(OrdersResponse {
        orders: ledger.orders.clone(),
        barrier_orders: ledger.barrier_orders.clone(),
    })
}
async fn get_barrier_orders(State(store): State<Arc<PositionStore>>) -> Json<Vec<BarrierOrder>> {
    Json(store.ledger.lock().await.barrier_orders.clone())
}

#[derive(Deserialize)]
struct ProfitBarrierRequest {
    symbol: String,
    target_profit_usd: f64,
}

async fn create_profit_barrier(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<ProfitBarrierRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !request.target_profit_usd.is_finite() || request.target_profit_usd <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Target profit must be a finite USD amount greater than zero".into(),
        ));
    }
    let rates = get_usd_rates().await?.0.rates;
    let quote = symbol
        .split_once('-')
        .map(|(_, quote)| quote)
        .unwrap_or_default();
    let rate = *rates.get(quote).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("No OKX USD conversion rate for {quote}"),
    ))?;
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions;
    let position = positions
        .into_iter()
        .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
    let quantity = position.quantity.abs();
    let fee_rate = position.estimated_exit_fee_rate.abs();
    let denominator = position.quantity - quantity * fee_rate;
    if !denominator.is_finite() || denominator.abs() <= EPSILON {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot calculate a profit trigger for this position and fee rate".into(),
        ));
    }
    let trigger_price = (request.target_profit_usd / rate
        + position.average_entry_price * position.quantity
        - position.entry_fees)
        / denominator;
    if !trigger_price.is_finite() || trigger_price <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Target profit produces an invalid trigger price".into(),
        ));
    }
    let side = if position.quantity > 0.0 {
        TradeSide::Sell
    } else {
        TradeSide::Buy
    };
    let trigger_direction = if position.quantity > 0.0 {
        BarrierDirection::Above
    } else {
        BarrierDirection::Below
    };
    store.ensure_subscription(&symbol);
    let timestamp = now_ms().map_err(internal_error)?;
    let mut ledger = store.ledger.lock().await;
    ensure_paper_capacity(
        &store,
        &ledger,
        &symbol,
        &side,
        quantity,
        trigger_price,
        configured_fee_rates().taker,
    )
    .await?;
    let barrier_order = BarrierOrder {
        id: format!("barrier-{timestamp}-{}", ledger.barrier_orders.len()),
        symbol,
        side,
        quantity,
        remaining_quantity: quantity,
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
        order_kind: "profit-target".to_owned(),
        execution_type: "market".to_owned(),
        limit_price: None,
        trailing_amount: None,
        trailing_percent: None,
        extreme_price: None,
        source: default_trade_source(),
        parent_order_id: None,
    };
    ledger.barrier_orders.push(barrier_order.clone());
    if let Err(error) = store.persist(&ledger).await {
        ledger.barrier_orders.pop();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist profit barrier: {error}"),
        ));
    }
    Ok(Json(OrderResponse {
        estimated_fee: 0.0,
        filled_quantity: 0.0,
        remaining_quantity: quantity,
        average_execution_price: None,
        order: None,
        barrier_order: Some(barrier_order),
        processing_timings_ms: BTreeMap::new(),
    }))
}

#[derive(Deserialize)]
struct CloseProfitableRequest {
    symbol: String,
}

async fn close_profitable(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<CloseProfitableRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    let (position, source) = {
        let ledger = store.ledger.lock().await;
        let auto_positions = summarize_positions_from(
            ledger.trades.iter().filter(|trade| trade.source == "auto"),
        )
        .positions;
        if let Some(position) = auto_positions
            .into_iter()
            .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        {
            // Attribute a manual close to the AutoTrader inventory it closes.
            // AutoTrader reconciles its state from source=auto; tagging this fill
            // manual would leave a stale open position and block future entries.
            (position, "auto".to_owned())
        } else {
            let positions = summarize_positions(&ledger.trades).positions;
            let position = positions
                .into_iter()
                .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
                .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
            (position, default_trade_source())
        }
    };
    store.ensure_subscription(&symbol);
    let side = if position.quantity > 0.0 {
        TradeSide::Sell
    } else {
        TradeSide::Buy
    };
    let quantity = position.quantity.abs();
    submit_order(
        State(store),
        Json(NewOrder {
            symbol: symbol.clone(),
            side: side.clone(),
            order_type: OrderType::Ioc,
            quantity,
            limit_price: None,
            measure_execution_quality: true,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
            source,
            client_order_id: None,
            // A manual close must not reserve buying power or open/reverse a
            // position if inventory changed after the close quantity was read.
            reduce_only: true,
        }),
    )
    .await
    .map_err(|(status, error)| {
        (
            status,
            format!(
                "Close IOC {symbol} {} {quantity} failed: {error}",
                match side {
                    TradeSide::Buy => "buy",
                    TradeSide::Sell => "sell",
                }
            ),
        )
    })
}

