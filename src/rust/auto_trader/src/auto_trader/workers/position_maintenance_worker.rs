async fn run_position_maintenance(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_rsocket_url = std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    // Consume fills before classifying disappeared resting orders as inactive.
    reconcile_maker_fills(&position_rsocket_url, state).await?;
    if let Err(error) = reconcile_inactive_execution_orders(state).await {
        crate::service_log!("WARN", "Pending-order reconciliation failed: {}", error.1);
    }
    reconcile_scheduled_fills(state).await?;
    sync_auto_positions(state).await?;
    let _ = maybe_exit_auto_position(state).await?;
    Ok(())
}

#[derive(Deserialize)]
struct LedgerRealizedPnl {
    realized_pnl_by_quote: BTreeMap<String, f64>,
    realized_pnl_usd: Option<f64>,
    realized_pnl_by_parent_order: BTreeMap<String, f64>,
    realized_pnl_by_barrier_created_at: BTreeMap<String, f64>,
    realized_pnl_all_time_by_quote: BTreeMap<String, f64>,
    realized_pnl_all_time_usd: Option<f64>,
}

async fn fetch_ledger_realized_pnl(
) -> Result<LedgerRealizedPnl, (StatusCode, String)> {
    let value = crate::service_rpc::position_get(&format!(
        "/positions/realized-pnl?source=auto&since_ms={}",
        utc_day_start_ms()
    )).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    serde_json::from_value(value).map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

#[derive(Deserialize)]
struct SourcePositionRecord {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    realized_pnl: f64,
    entry_fees: f64,
}

async fn sync_auto_positions(
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let ledger_pnl = fetch_ledger_realized_pnl().await?;
    let rates = state.settings.lock().await.usd_rates.clone();
    let realized_all_time_usd = match ledger_pnl.realized_pnl_all_time_usd {
        Some(value) => value,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                missing_fx_rate_error(
                    "All-time realized P&L",
                    &ledger_pnl.realized_pnl_all_time_by_quote,
                    &rates,
                ),
            ));
        }
    };
    let realized_today_usd = match ledger_pnl.realized_pnl_usd {
        Some(value) => value,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                missing_fx_rate_error("Daily realized P&L", &ledger_pnl.realized_pnl_by_quote, &rates),
            ));
        }
    };
    let positions = crate::service_rpc::position_get("/positions?source=auto")
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch Auto Trader positions: {error}"),
            )
        })?;
    let positions =
        serde_json::from_value::<Vec<SourcePositionRecord>>(positions["positions"].clone())
            .map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Invalid Auto Trader position records: {error}"),
                )
            })?;
    // Serialize the ledger reconciliation with order reservation and fill
    // application. A request remains "submitting" while Position Manager is
    // working; do not mirror its fill here or the response path would apply it
    // a second time.
    let _cycle = acquire_cycle_lock(state, "position_maintenance").await;
    let settings_lock_started = Instant::now();
    let mut settings = state.settings.lock().await;
    record_cycle_lock_scope(
        state,
        "position_maintenance_settings_lock_wait",
        settings_lock_started.elapsed(),
    );
    let reconciliation_started = Instant::now();
    let pending_signal_symbols = settings
        .orders
        .iter()
        .filter(|order| order.status == "submitting")
        .map(|order| order.symbol.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut changed = false;
    let mut source_symbols = std::collections::BTreeSet::new();
    let mut external_closes = Vec::new();
    for source_position in positions {
        if !source_position.quantity.is_finite()
            || !source_position.average_entry_price.is_finite()
            || !source_position.realized_pnl.is_finite()
        {
            continue;
        }
        source_symbols.insert(source_position.symbol.clone());
        if pending_signal_symbols.contains(&source_position.symbol) {
            continue;
        }
        let (realized_delta, close_audit, closed_out, scout_entry) = {
            let position = settings
                .positions
                .entry(source_position.symbol.clone())
                .or_default();
            let prior_quantity = position.quantity;
            let prior_average_entry_price = position.average_entry_price;
            let scout_entry = position.scout_entry;
            let realized_delta = source_position.realized_pnl - position.realized_pnl;
            let closed_quantity = external_close_quantity(prior_quantity, source_position.quantity);
            let close_audit = (prior_quantity.abs() > 1e-12
                && closed_quantity > 1e-12
                && realized_delta.abs() > 1e-12)
                .then(|| external_close_audit_payload(
                    &source_position.symbol,
                    prior_quantity,
                    closed_quantity,
                    prior_average_entry_price,
                    position,
                ));
            let closed_out = prior_quantity.abs() > 1e-12
                && source_position.quantity.abs() <= 1e-12;
            if (position.quantity - source_position.quantity).abs() > 1e-12
                || (position.average_entry_price - source_position.average_entry_price).abs()
                    > 1e-12
                || (position.entry_fees - source_position.entry_fees.abs()).abs() > 1e-12
                || (position.realized_pnl - source_position.realized_pnl).abs() > 1e-12
            {
                position.quantity = source_position.quantity;
                position.average_entry_price = source_position.average_entry_price;
                position.entry_fees = source_position.entry_fees.abs();
                changed = true;
            }
            position.realized_pnl = source_position.realized_pnl;
            (realized_delta, close_audit, closed_out, scout_entry)
        };
        if let Some(audit) = close_audit {
            let strategy_lots = settings.strategy_positions
                .get(&source_position.symbol)
                .map(|lots| lots.iter()
                    .filter(|(_, lot)| lot.quantity.abs() > 1e-12)
                    .map(|(strategy, lot)| (strategy.clone(), lot.clone()))
                    .collect::<Vec<_>>())
                .unwrap_or_default();
            let mut known_exit_ids = settings.orders.iter()
                .filter(|order| !matches!(order.purpose.as_str(), "signal" | "scout"))
                .flat_map(|order| [Some(order.timestamp.to_string()), order.execution_order_id.clone()])
                .flatten()
                .collect::<BTreeSet<_>>();
            for (_, lot) in &strategy_lots {
                known_exit_ids.extend(lot.protective_stops.iter().map(|stop| stop.barrier_id.clone()));
            }
            external_closes.push((audit, realized_delta, strategy_lots, current_time_ms(), known_exit_ids));
        }
        if realized_delta.is_finite() && realized_delta.abs() > 1e-12 {
            // Manual position-manager fills bypass apply_auto_fill; matched
            // strategy-lot audits below provide attributable trade outcomes.
            changed = true;
            if closed_out && !scout_entry {
                mark_symbol_reentry_cooldown(&mut settings, &source_position.symbol);
            }
        }
    }
    // The position manager owns the ledger. Drop cached symbols that no longer
    // exist there (for example after a ledger reset) and mirror its P&L totals.
    let cached_symbols = settings.positions.keys().cloned().collect::<Vec<_>>();
    for symbol in cached_symbols {
        if !source_symbols.contains(&symbol) {
            settings.positions.remove(&symbol);
            changed = true;
        }
    }
    if (settings.realized_pnl - realized_all_time_usd).abs() > 1e-9
        || (settings.daily_realized_pnl - realized_today_usd).abs() > 1e-9
    {
        settings.realized_pnl = realized_all_time_usd;
        settings.daily_realized_pnl = realized_today_usd;
        changed = true;
    }
    if changed {
        settings.used_notional_today = allocated_notional(&settings);
        request_settings_persist(state)?;
    }
    let reconciliation_elapsed = reconciliation_started.elapsed();
    drop(settings);
    drop(_cycle);
    record_cycle_lock_scope(
        state,
        "position_maintenance_ledger_reconciliation",
        reconciliation_elapsed,
    );
    for (mut payload, realized_pnl, strategy_lots, closed_at_ms, known_exit_ids) in external_closes {
        let symbol = payload["symbol"].as_str().unwrap_or_default().to_owned();
        let entry_timestamp = payload["entry_timestamp_ms"].as_u64().unwrap_or_default();
        let now = closed_at_ms;
        let stage = format!("exit:external:{symbol}:{entry_timestamp}");
        payload["stage"] = serde_json::json!(stage);
        payload["timestamp"] = serde_json::json!(now);
        payload["reason"] = serde_json::json!("external-or-manual-close");
        payload["exit_order_type"] = serde_json::json!("external-ledger-fill");
        payload["status"] = serde_json::json!("filled");
        payload["realized_pnl"] = serde_json::json!(realized_pnl);
        payload["duration_ms"] = serde_json::json!((entry_timestamp > 0).then_some(now.saturating_sub(entry_timestamp)));
        let trades = crate::service_rpc::position_get(&format!(
            "/positions/trades?symbol={symbol}&source=auto&since_ms={entry_timestamp}"
        )).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
        let close_side = if payload["entry_position_side"].as_str() == Some("long") { "sell" } else { "buy" };
        let trade_rows = trades["trades"].as_array().cloned().unwrap_or_default();
        let closed_quantity = payload["close_quantity"].as_f64().unwrap_or(0.0);
        if strategy_lots.is_empty() {
            let close_fills = external_close_fills(closed_quantity, &close_side, &trade_rows, closed_at_ms, &known_exit_ids);
            let exit_quantity = close_fills.iter().filter_map(|fill| fill["quantity"].as_f64()).sum::<f64>();
            let exit_notional = close_fills.iter().filter_map(|fill| Some(fill["quantity"].as_f64()? * fill["price"].as_f64()?)).sum::<f64>();
            let exit_fees = close_fills.iter().filter_map(|fill| fill["fee"].as_f64()).map(f64::abs).sum::<f64>();
            payload["exit_filled_quantity"] = serde_json::json!(exit_quantity);
            payload["exit_average_price"] = serde_json::json!((exit_quantity > 0.0).then_some(exit_notional / exit_quantity));
            payload["exit_fees"] = serde_json::json!(exit_fees);
            payload["exit_fills"] = serde_json::json!(close_fills);
            payload["realized_pnl"] = serde_json::json!(realized_pnl);
            payload["strategy"] = serde_json::json!(payload["strategy"].as_str().unwrap_or("manual"));
            if let Some(order) = external_close_order(&payload) {
                let mut settings = state.settings.lock().await;
                settings.orders.push(order);
                if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
                    let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
                    settings.orders.drain(..excess);
                }
                request_settings_persist(state)?;
            }
            queue_auto_trader_audit(state, payload);
        } else {
            let audits = external_strategy_close_audits(
                &symbol,
                closed_quantity,
                &close_side,
                &strategy_lots,
                &trade_rows,
                closed_at_ms,
                &known_exit_ids,
            );
            for audit in audits {
                if audit["trade_closed"].as_bool() == Some(true) {
                    let net_return_percent = audit["trade_realized_pnl"]
                        .as_f64()
                        .zip(audit["entry_context"]["initial_entry_notional"].as_f64())
                        .filter(|(_, notional)| *notional > 0.0)
                        .map(|(realized, notional)| realized / notional * 100.0);
                    if let (Some(strategy), Some(realized), Some(net_return_percent)) = (
                        audit["strategy"].as_str(),
                        audit["trade_realized_pnl"].as_f64(),
                        net_return_percent,
                    ) {
                        let mut settings = state.settings.lock().await;
                        let realized_usd = realized_usd(&settings, &symbol, realized);
                        record_strategy_trade_outcome(
                            &mut settings,
                            &symbol,
                            Some(strategy),
                            audit["learning_context"].as_str(),
                            audit["scout"].as_bool().unwrap_or(false),
                            realized_usd,
                            Some(net_return_percent),
                            audit["strategy_decision"].as_object().map(|_| &audit["strategy_decision"]),
                            audit["trade_id"].as_str(),
                        );
                        request_settings_persist(state)?;
                    }
                }
                let has_known_auto_exit = audit["exit_fills"].as_array().into_iter().flatten().any(|fill| {
                    fill["parent_order_id"].as_str().is_some_and(|parent_id| known_exit_ids.contains(parent_id))
                });
                if !has_known_auto_exit {
                    if let Some(order) = external_close_order(&audit) {
                        let mut settings = state.settings.lock().await;
                        settings.orders.push(order);
                        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
                            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
                            settings.orders.drain(..excess);
                        }
                        request_settings_persist(state)?;
                    }
                }
                queue_auto_trader_audit(state, audit);
            }
        }
    }
    Ok(())
}


async fn reconcile_inactive_execution_orders(
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let response = crate::service_rpc::position_get("/positions/orders")
        .await.map_err(|error| (StatusCode::BAD_GATEWAY, format!("Could not fetch active orders: {error}")))?;
    let active_ids = response["orders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|order| order["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut settings = state.settings.lock().await;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut changed = false;
    let mut completed_orders = Vec::new();
    for order in settings.orders.iter_mut().filter(|order| {
        matches!(order.purpose.as_str(), "signal" | "scout")
            && !is_scheduled_execution_order_type(&order.order_type)
            && !is_maker_execution_order_type(&order.order_type)
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
    }) {
        let Some(id) = order.execution_order_id.as_deref() else {
            continue;
        };
        if active_ids.contains(id) {
            continue;
        }
        let prior_status = order.status.clone();
        order.status = inactive_execution_order_status(order.filled_quantity).into();
        order.failure_reason = Some(if order.expires_at.is_some_and(|expires| expires <= now_ms) {
            "expired_without_full_fill".into()
        } else {
            "removed_from_execution_ledger".into()
        });
        changed |= order.status != prior_status;
        completed_orders.push(order.clone());
    }
    for order in completed_orders {
        let Settings { execution_feedback_outbox, execution_learning_order_ids, .. } = &mut *settings;
        record_execution_order_quality(execution_feedback_outbox, execution_learning_order_ids, &order);
    }
    if changed {
        request_settings_persist(state)?;
    }
    Ok(())
}


fn is_scheduled_execution_order_type(order_type: &str) -> bool {
    order_type.starts_with("Vwap") || order_type.starts_with("Twap")
}

fn inactive_execution_order_status(filled_quantity: f64) -> &'static str {
    if filled_quantity > 1e-12 {
        "partially_filled_closed"
    } else {
        "expired_or_cancelled"
    }
}

#[cfg(test)]
mod inactive_execution_order_tests {
    use super::{inactive_execution_order_status, is_scheduled_execution_order_type};

    #[test]
    fn removes_unfilled_and_partially_filled_orders_from_pending_state() {
        assert_eq!(inactive_execution_order_status(0.0), "expired_or_cancelled");
        assert_eq!(inactive_execution_order_status(0.25), "partially_filled_closed");
    }

    #[test]
    fn scheduled_orders_are_not_checked_against_the_resting_order_ledger() {
        assert!(is_scheduled_execution_order_type("Vwap · P0"));
        assert!(is_scheduled_execution_order_type("Twap · P1"));
        assert!(!is_scheduled_execution_order_type("PostOnlyLimit · P0"));
    }
}
