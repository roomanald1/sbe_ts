async fn submit_position_exit_order(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: String,
    strategy_key: String,
    position: TraderPosition,
    bid: f64,
    ask: f64,
    reason: &str,
    return_fraction: f64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let is_long = position.quantity > 0.0;
    let entry_notional = position.quantity.abs() * position.average_entry_price;
    let entry_decision_id = position.entry_decision_id.clone();
    let protected_quantity = position
        .protective_stops
        .iter()
        .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
        .sum::<f64>();
    if reason == "stop-loss" && protected_quantity + 1e-12 >= position.quantity.abs() {
        // The server-side barrier is already watching this price; don't race it with a second IOC.
        return Ok(None);
    }
    if !cancel_protective_stops(client, base, state, &symbol, Some(&strategy_key)).await? {
        // A stop is already triggering or could not be cancelled. Let its server-side execution settle.
        return Ok(None);
    }
    let side = if is_long { "sell" } else { "buy" };
    let quantity = position.quantity.abs();
    let notional = quantity * (bid + ask) / 2.0;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        settings.used_notional_today = allocated_notional(&settings);
        settings.orders.push(new_position_exit_order(
            timestamp, &symbol, side, reason, &position, quantity, notional,
        ));
    }
    let submit_started = Instant::now();
    let response = match submit_configured_order(
        client,
        base,
        &symbol,
        side,
        quantity,
        notional,
        bid,
        ask,
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
            return Err(error);
        }
    };
    let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
    let mut settings = state.settings.lock().await;
    let filled = response["filled_quantity"].as_f64().unwrap_or(0.0);
    let average_price = response["average_execution_price"].as_f64();
    let fee = response["estimated_fee"].as_f64().unwrap_or(0.0).abs();
    let (realized_pnl, _learned_key) = if filled > 0.0 {
        average_price
            .map(|price| {
                apply_auto_fill(
                    &mut settings.positions,
                    &symbol,
                    side,
                    filled,
                    price,
                    fee,
                    "Ioc",
                    reason,
                    false,
                    None,
                )
            })
            .unwrap_or_default()
    } else {
        (0.0, None)
    };
    let mut strategy_realized_pnl = 0.0;
    if filled > 0.0 {
        if let Some(price) = average_price {
            let Settings { strategy_positions, positions, .. } = &mut *settings;
            (strategy_realized_pnl, _) = apply_strategy_position_fill(
                strategy_positions,
                positions,
                &symbol,
                &strategy_key,
                &side,
                filled,
                price,
                fee,
                "Ioc",
                reason,
                false,
                None,
                None,
                None,
                None,
                None,
            );
        }
    }
    let realized_pnl_usd = realized_usd(&settings, &symbol, realized_pnl);
    settings.realized_pnl += realized_pnl_usd;
    settings.daily_realized_pnl += realized_pnl_usd;
    if filled + 1e-12 >= quantity {
        let entry_notional = if position.initial_entry_notional > 0.0 {
            position.initial_entry_notional
        } else {
            position.quantity.abs() * position.average_entry_price
        };
        if entry_notional > 0.0 {
            let total_trade_pnl = position.realized_pnl + strategy_realized_pnl;
            let total_fees = if position.total_fees_paid > 0.0 {
                position.total_fees_paid + fee
            } else {
                position.entry_fees + fee
            };
            let fees_usd = realized_usd(&settings, &symbol, total_fees);
            let strategy_realized_usd = realized_usd(&settings, &symbol, total_trade_pnl);
            record_closed_trade_outcome(
                &mut settings,
                total_trade_pnl / entry_notional * 100.0,
                strategy_realized_usd,
                fees_usd,
            );
            record_strategy_trade_outcome(
                &mut settings,
                &symbol,
                position.strategy.as_deref(),
                Some(&position.entry_context),
                position.scout_entry,
                strategy_realized_usd,
            );
            adapt_exit_policy(&mut settings);
        }
    }
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.fill_rate = if quantity > 0.0 {
            (filled / quantity).clamp(0.0, 1.0)
        } else {
            0.0
        };
        order.average_price = average_price;
        order.fees_paid = fee;
        order.realized_pnl = realized_pnl;
        record_strategy_order_realized_pnl(order, strategy_realized_pnl);
        order.status = if filled + 1e-12 >= quantity {
            "filled"
        } else if filled > 0.0 {
            "partially_filled_closed"
        } else {
            "expired_or_cancelled"
        }
        .into();
    }
    request_settings_persist(state)?;
    drop(settings);
    if let Some(decision_id) = entry_decision_id {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let payload = position_exit_audit_payload(
            &decision_id, now, timestamp, &symbol, reason, side, &response, &position,
            quantity, filled, average_price, fee, strategy_realized_pnl, entry_notional,
            return_fraction, bid, ask, submit_response_ms, stop_loss_percent,
            take_profit_percent,
        );
        queue_auto_trader_audit(state, payload);
    }
    Ok(Some(CycleResult {
        status: "order_submitted",
        reason: Some(format!(
            "Auto-trader {reason} at {:.4}%",
            return_fraction * 100.0
        )),
        symbol: Some(symbol),
        side: Some(side.into()),
        quantity: Some(quantity),
        order_type: Some("Ioc exit".into()),
        order_response: Some(response),
    }))
}


#[cfg(test)]
mod position_exit_order_tests {
    use super::*;

    #[test]
    fn position_exit_order_preserves_strategy_and_entry_decision() {
        let position = TraderPosition {
            strategy: Some("momentum-alpha".into()),
            entry_decision_id: Some("decision-123".into()),
            ..TraderPosition::default()
        };

        let order = new_position_exit_order(
            42, "BTC-USD", "sell", "take-profit", &position, 1.25, 50_000.0,
        );

        assert_eq!(order.timestamp, 42);
        assert_eq!(order.symbol, "BTC-USD");
        assert_eq!(order.side, "sell");
        assert_eq!(order.order_type, "Ioc");
        assert_eq!(order.purpose, "take-profit");
        assert_eq!(order.strategy.as_deref(), Some("momentum-alpha"));
        assert_eq!(order.decision_id.as_deref(), Some("decision-123"));
        assert_eq!(order.quantity, 1.25);
        assert_eq!(order.reserved_notional, 50_000.0);
        assert_eq!(order.status, "submitting");
        assert_eq!(order.filled_quantity, 0.0);
        assert_eq!(order.fill_rate, 0.0);
    }

    #[test]
    fn position_exit_order_uses_live_insights_fallback() {
        let position = TraderPosition::default();

        let order = new_position_exit_order(
            7, "ETH-USD", "buy", "stop-loss", &position, 2.0, 6_000.0,
        );

        assert_eq!(order.strategy.as_deref(), Some("live-insights"));
        assert_eq!(order.decision_id, None);
    }
}
