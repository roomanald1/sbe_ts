async fn execute_portfolio_hedge(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    candidate: (String, String, f64, f64, f64, f64),
    now_ms: u64,
    portfolio_shock_risk: f64,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let (symbol, side, requested_notional, price, quote_rate, _) = candidate;
    // Candidate and rate reads, plus correlation/risk calculations above, are
    // read-only. Keep them off the shared cycle lock so a slow position-manager
    // response cannot delay a signal or position-maintenance decision. Acquire
    // the lock only when we are ready to reserve the hedge and run its
    // state-changing stop/order sequence.
    let _cycle = acquire_cycle_lock(state, "hedge_evaluation").await;
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let notional =
        requested_notional.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but the daily hedge budget is exhausted",
        )));
    }
    let timestamp = now_ms;
    let quantity = notional / (price * quote_rate);
    settings.used_notional_today = allocated_notional(&settings);
    settings.last_hedge_at.insert(symbol.clone(), timestamp);
    settings.orders.push(new_portfolio_hedge_order(
        timestamp, &symbol, &side, quantity, notional,
    ));
    if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
        let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
        settings.orders.drain(..excess);
    }
    request_settings_persist(state)?;
    drop(settings);
    if !cancel_protective_stops(client, base, state, &symbol, None).await? {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_stop_active".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        request_settings_persist(state)?;
        return Ok(Some(skipped(
            "Could not cancel the existing protective stop; deferred portfolio hedge",
        )));
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        settings.last_hedge_at.remove(&symbol);
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        request_settings_persist(state)?;
        drop(settings);
        ensure_protective_stops(client, base, state).await?;
        return Ok(Some(skipped(
            "Auto Trader was disabled before hedge submission",
        )));
    }
    let response = match submit_configured_order(
        client,
        base,
        &symbol,
        &side,
        quantity,
        notional,
        price,
        price,
        AutoOrderType::Ioc,
        ExecutionParameters::default(),
        timestamp.to_string(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".into();
            }
            request_settings_persist(state)?;
            drop(settings);
            ensure_protective_stops(client, base, state).await?;
            return Err(error);
        }
    };
    let mut settings = state.settings.lock().await;
    let filled = response
        .get("filled_quantity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let average_price = response
        .get("average_execution_price")
        .and_then(serde_json::Value::as_f64);
    let fee = response
        .get("estimated_fee")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        .abs();
    let execution_learning = settings
        .learning
        .entry("portfolio-hedge:execution".into())
        .or_default();
    record_learning_outcome(
        execution_learning,
        if filled >= quantity * 0.8 { 1.0 } else { 0.0 },
    );
    let (realized_pnl, _) = if filled > 0.0 {
        average_price
            .map(|execution| {
                apply_auto_fill(
                    &mut settings.positions,
                    &symbol,
                    &side,
                    filled,
                    execution,
                    fee,
                    "Ioc",
                    "portfolio-hedge",
                    true,
                    None,
                )
            })
            .unwrap_or_default()
    } else {
        (0.0, None)
    };
    let realized_pnl_usd = realized_usd(&settings, &symbol, realized_pnl);
    settings.realized_pnl += realized_pnl_usd;
    settings.daily_realized_pnl += realized_pnl_usd;
    let status = if response
        .get("barrier_order")
        .is_some_and(|value| !value.is_null())
    {
        "armed"
    } else if response.get("order").is_some_and(|value| !value.is_null()) {
        "resting"
    } else if filled > 0.0 {
        "filled"
    } else {
        "accepted"
    };
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.average_price = average_price;
        order.realized_pnl = realized_pnl;
        order.status = status.into();
    }
    settings.used_notional_today = allocated_notional(&settings);
    request_settings_persist(state)?;
    drop(settings);
    ensure_protective_stops(client, base, state).await?;
    Ok(Some(CycleResult {
        status: "hedge_submitted",
        reason: Some(format!(
            "Portfolio 1% shock risk was USD {:.2}; submitted a risk-reducing hedge",
            portfolio_shock_risk
        )),
        symbol: Some(symbol),
        side: Some(side),
        quantity: Some(quantity),
        order_type: Some("Ioc hedge".into()),
        order_response: Some(response),
    }))
}


fn new_portfolio_hedge_order(
    timestamp: u64,
    symbol: &str,
    side: &str,
    quantity: f64,
    notional: f64,
) -> TraderOrder {
    TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        order_type: "Ioc".into(),
        purpose: "portfolio-hedge".into(),
        strategy: Some("portfolio-hedge".into()),
        decision_id: None,
        quantity,
        filled_quantity: 0.0,
        fill_rate: 0.0,
        average_price: None,
        fees_paid: 0.0,
        reserved_notional: notional,
        execution_order_id: None,
        processed_fill_trade_ids: Vec::new(),
        learning_context: None,
        book_bid_depth_10bps: None,
        book_ask_depth_10bps: None,
        book_depth_imbalance_10bps: None,
        estimated_sweep_bps: None,
        estimated_ioc_all_in_bps: None,
        decision_bid: None,
        decision_ask: None,
        decision_mid: None,
        submitted_limit_price: None,
        target_price: None,
        horizon_take_profit_percent: None,
        limit_to_mid_bps: None,
        limit_to_crossing_quote_bps: None,
        expires_at: None,
        failure_reason: None,
        stage_timings_ms: BTreeMap::new(),
        status: "submitting".into(),
        realized_pnl: 0.0,
    }
}


#[cfg(test)]
mod portfolio_hedge_order_tests {
    use super::*;

    #[test]
    fn portfolio_hedge_order_has_expected_identity_and_reservation() {
        let order = new_portfolio_hedge_order(99, "ETH-USD", "buy", 3.0, 9_000.0);

        assert_eq!(order.timestamp, 99);
        assert_eq!(order.symbol, "ETH-USD");
        assert_eq!(order.side, "buy");
        assert_eq!(order.order_type, "Ioc");
        assert_eq!(order.purpose, "portfolio-hedge");
        assert_eq!(order.strategy.as_deref(), Some("portfolio-hedge"));
        assert_eq!(order.decision_id, None);
        assert_eq!(order.quantity, 3.0);
        assert_eq!(order.reserved_notional, 9_000.0);
        assert_eq!(order.status, "submitting");
        assert_eq!(order.filled_quantity, 0.0);
        assert_eq!(order.fill_rate, 0.0);
    }
}
