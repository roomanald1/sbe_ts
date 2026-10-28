async fn run_position_maintenance(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    if let Err(error) = reconcile_inactive_execution_orders(&client, &position_url, state).await {
        crate::service_log!("WARN", "Pending-order reconciliation failed: {}", error.1);
    }
    reconcile_maker_fills(&client, &position_url, state).await?;
    reconcile_scheduled_fills(&client, &position_url, state).await?;
    sync_auto_positions(&client, &position_url, state).await?;
    let _ = maybe_exit_auto_position(&client, &position_url, state).await?;
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
    client: &reqwest::Client,
    base: &str,
) -> Result<LedgerRealizedPnl, (StatusCode, String)> {
    client
        .get(format!(
            "{base}/positions/realized-pnl?source=auto&since_ms={}",
            utc_day_start_ms()
        ))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json::<LedgerRealizedPnl>()
        .await
        .map_err(upstream_error)
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
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let ledger_pnl = fetch_ledger_realized_pnl(client, base).await?;
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
    let positions = client
        .get(format!("{base}/positions?source=auto"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch Auto Trader positions: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Auto Trader positions lookup failed: {error}"),
            )
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid Auto Trader positions response: {error}"),
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
    let mut settings = state.settings.lock().await;
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
        let (prior_quantity, realized_delta, learning_key, close_audit) = {
            let position = settings
                .positions
                .entry(source_position.symbol.clone())
                .or_default();
            let prior_quantity = position.quantity;
            let prior_average_entry_price = position.average_entry_price;
            let realized_delta = source_position.realized_pnl - position.realized_pnl;
            let learning_key = format!(
                "{}:{}",
                if position.entry_context.is_empty() {
                    &source_position.symbol
                } else {
                    &position.entry_context
                },
                if position.entry_order_type.is_empty() {
                    "ManualClose"
                } else {
                    &position.entry_order_type
                },
            );
            let close_audit = (prior_quantity.abs() > 1e-12
                && source_position.quantity.abs() <= 1e-12)
                .then(|| external_close_audit_payload(
                    &source_position.symbol,
                    prior_quantity,
                    prior_average_entry_price,
                    position,
                ));
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
            (prior_quantity, realized_delta, learning_key, close_audit)
        };
        if let Some(audit) = close_audit {
            if audit["decision_id"].as_str().is_some() {
                external_closes.push((audit, realized_delta));
            }
        }
        if realized_delta.is_finite() && realized_delta.abs() > 1e-12 {
            // Manual position-manager fills bypass apply_auto_fill. Use the
            // ledger delta for close-out learning; account totals are mirrored below.
            let realized_delta_usd =
                realized_usd(&settings, &source_position.symbol, realized_delta);
            changed = true;
            if prior_quantity.abs() > 1e-12 && source_position.quantity.abs() <= 1e-12 {
                // This is a symbol-level reconciliation delta, not one matched
                // round trip. The old quantity may be a tiny residual and
                // source fees are cumulative, so do not write a trade return.
                let stats = settings.learning.entry(learning_key).or_default();
                record_learning_outcome(stats, realized_delta_usd);
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
    drop(settings);
    drop(_cycle);
    for (mut payload, realized_pnl) in external_closes {
        let symbol = payload["symbol"].as_str().unwrap_or_default().to_owned();
        let entry_timestamp = payload["entry_timestamp_ms"].as_u64().unwrap_or_default();
        let now = current_time_ms();
        let stage = format!("exit:external:{symbol}:{entry_timestamp}");
        payload["stage"] = serde_json::json!(stage);
        payload["timestamp"] = serde_json::json!(now);
        payload["reason"] = serde_json::json!("external-or-manual-close");
        payload["exit_order_type"] = serde_json::json!("external-ledger-fill");
        payload["status"] = serde_json::json!("filled");
        payload["realized_pnl"] = serde_json::json!(realized_pnl);
        payload["duration_ms"] = serde_json::json!((entry_timestamp > 0).then_some(now.saturating_sub(entry_timestamp)));
        store_execution_audit_until_saved(&client, &payload).await;
        let trades_url = format!(
            "{base}/positions/trades?symbol={symbol}&source=auto&since_ms={entry_timestamp}"
        );
        let trades = client
            .get(trades_url)
            .send()
            .await
            .map_err(upstream_error)?
            .error_for_status()
            .map_err(upstream_error)?
            .json::<serde_json::Value>()
            .await
            .map_err(upstream_error)?;
        let close_side = if payload["entry_position_side"].as_str() == Some("long") { "sell" } else { "buy" };
        let close_fills = trades["trades"].as_array().into_iter().flatten()
            .filter(|trade| trade["side"].as_str() == Some(close_side))
            .filter_map(|trade| {
                let quantity = trade["quantity"].as_f64()?;
                let price = trade["execution_price"].as_f64()?;
                Some(serde_json::json!({
                    "timestamp": trade["timestamp"], "quantity": quantity,
                    "price": price, "fee": trade["fee"], "parent_order_id": trade["parent_order_id"]
                }))
            })
            .collect::<Vec<_>>();
        let exit_quantity = close_fills.iter().filter_map(|fill| fill["quantity"].as_f64()).sum::<f64>();
        let exit_notional = close_fills.iter().filter_map(|fill| {
            Some(fill["quantity"].as_f64()? * fill["price"].as_f64()?)
        }).sum::<f64>();
        let exit_fees = close_fills.iter().filter_map(|fill| fill["fee"].as_f64()).map(f64::abs).sum::<f64>();
        payload["exit_filled_quantity"] = serde_json::json!(exit_quantity);
        payload["exit_average_price"] = serde_json::json!((exit_quantity > 0.0).then_some(exit_notional / exit_quantity));
        payload["exit_fees"] = serde_json::json!(exit_fees);
        payload["exit_fills"] = serde_json::json!(close_fills);
        payload["realized_pnl"] = serde_json::json!(realized_pnl);
        store_execution_audit_until_saved(&client, &payload).await;
    }
    Ok(())
}


async fn reconcile_inactive_execution_orders(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let response = client
        .get(format!("{base}/positions/orders"))
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Unable to fetch active orders: {error}")))?
        .error_for_status()
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Active order lookup failed: {error}")))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid active order response: {error}")))?;
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
    for order in settings.orders.iter_mut().filter(|order| {
        matches!(order.purpose.as_str(), "signal" | "scout")
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
    }
    if changed {
        request_settings_persist(state)?;
    }
    Ok(())
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
    use super::inactive_execution_order_status;

    #[test]
    fn removes_unfilled_and_partially_filled_orders_from_pending_state() {
        assert_eq!(inactive_execution_order_status(0.0), "expired_or_cancelled");
        assert_eq!(inactive_execution_order_status(0.25), "partially_filled_closed");
    }
}
