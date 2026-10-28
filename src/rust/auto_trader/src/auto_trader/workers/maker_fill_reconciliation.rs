#[derive(Clone, Deserialize)]

struct MakerFillRecord {
    id: String,
    symbol: String,
    side: String,
    quantity: f64,
    execution_price: f64,
    fee: f64,
    fee_liquidity: String,
    resting_order_id: Option<String>,
}

async fn reconcile_maker_fills(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let has_pending = state.settings.lock().await.orders.iter().any(|order| {
        order.execution_order_id.is_some()
            && order.order_type.starts_with("PostOnlyLimit")
            && order.status != "filled"
            && order.status != "cancelled"
    });
    if !has_pending {
        return Ok(());
    }
    let fills = client
        .get(format!("{base}/positions/trades"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch maker fills: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Maker fill lookup failed: {error}"),
            )
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid maker fill response: {error}"),
            )
        })?;
    let records = serde_json::from_value::<Vec<MakerFillRecord>>(fills["trades"].clone()).map_err(
        |error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid maker trade records: {error}"),
            )
        },
    )?;
    let open = client
        .get(format!("{base}/positions/orders"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch resting orders: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Resting order lookup failed: {error}"),
            )
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid resting order response: {error}"),
            )
        })?;
    let active_ids = open["orders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect::<std::collections::HashSet<_>>();
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    let mut learning_updates = Vec::new();
    let Settings {
        orders, positions, strategy_positions, ..
    } = &mut *settings;
    for order in orders.iter_mut().filter(|order| {
        order.order_type.starts_with("PostOnlyLimit")
            && matches!(
                order.status.as_str(),
                "resting" | "partially_filled" | "accepted"
            )
    }) {
        let Some(id) = order.execution_order_id.as_deref() else {
            continue;
        };
        let pending_fills = records
            .iter()
            .filter(|fill| {
                fill.resting_order_id.as_deref() == Some(id)
                    && fill.symbol == order.symbol
                    && fill.side == order.side
                    && fill.fee_liquidity == "maker"
                    && !order
                        .processed_fill_trade_ids
                        .iter()
                        .any(|processed| processed == &fill.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        for fill in pending_fills {
            if !fill.quantity.is_finite()
                || fill.quantity <= 0.0
                || !fill.execution_price.is_finite()
                || fill.execution_price <= 0.0
            {
                continue;
            }
            let count_entry = order.filled_quantity <= 1e-12;
            let (realized, learned) = apply_auto_fill(
                positions,
                &order.symbol,
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                "PostOnlyLimit",
                order.learning_context.as_deref().unwrap_or(&order.symbol),
                count_entry,
                order.horizon_take_profit_percent,
            );
            tag_position_strategy(positions, &order.symbol, order.strategy.as_deref());
            apply_strategy_position_fill(
                strategy_positions,
                positions,
                &order.symbol,
                order.strategy.as_deref().unwrap_or("live-insights"),
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                "PostOnlyLimit",
                order.learning_context.as_deref().unwrap_or(&order.symbol),
                count_entry,
                order.horizon_take_profit_percent,
                order.decision_id.as_deref(),
                None,
                None,
                None,
            );
            order.filled_quantity += fill.quantity;
            order.fill_rate = (order.filled_quantity / order.quantity).clamp(0.0, 1.0);
            order.fees_paid += fill.fee.abs();
            let total_notional = order.average_price.unwrap_or(0.0)
                * (order.filled_quantity - fill.quantity)
                + fill.execution_price * fill.quantity;
            order.average_price = Some(total_notional / order.filled_quantity);
            order.realized_pnl += realized;
            order.processed_fill_trade_ids.push(fill.id.clone());
            if let Some(kind) = learned {
                learning_updates.push((kind, order.symbol.clone(), realized));
            }
            changed = true;
        }
        let prior_status = order.status.clone();
        let mut unfilled_post_only_removal = false;
        if order.filled_quantity + 1e-12 >= order.quantity {
            order.status = "filled".into();
        } else if active_ids.contains(id) {
            order.status = if order.filled_quantity > 0.0 {
                "partially_filled"
            } else {
                "resting"
            }
            .into();
        } else if !active_ids.contains(id) {
            order.status = if order.filled_quantity > 0.0 {
                "partially_filled_closed"
            } else {
                "expired_or_cancelled"
            }
            .into();
            order.failure_reason = Some(
                if order.expires_at.is_some_and(|expires| {
                    expires
                        <= SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                }) {
                    "expired_without_full_fill".to_owned()
                } else {
                    "removed_before_expiry".to_owned()
                },
            );
            unfilled_post_only_removal =
                order.filled_quantity <= 1e-12 && prior_status != "expired_or_cancelled";
        }
        if unfilled_post_only_removal {
            if let Some(context) = order.learning_context.as_deref() {
                learning_updates.push((
                    format!("{context}:PostOnlyLimit"),
                    order.symbol.clone(),
                    0.0,
                ));
            }
        }
        changed |= order.status != prior_status;
    }
    for (kind, symbol, realized) in learning_updates {
        let realized_usd = realized_usd(&settings, &symbol, realized);
        let stats = settings.learning.entry(kind).or_default();
        record_learning_outcome(stats, realized_usd);
        settings.realized_pnl += realized_usd;
        settings.daily_realized_pnl += realized_usd;
    }
    if changed {
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(())
}

