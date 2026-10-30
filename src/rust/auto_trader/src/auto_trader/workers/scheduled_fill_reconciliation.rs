fn is_scheduled_order_reconcilable(
    order_type: &str,
    status: &str,
    failure_reason: Option<&str>,
) -> bool {
    let scheduled = order_type.starts_with("Twap") || order_type.starts_with("Vwap");
    scheduled
        && (matches!(status, "accepted" | "partially_filled" | "resting")
            || (status == "failed"
                && failure_reason.is_some_and(|reason| {
                    reason.contains("Scheduled run is missing from the position manager")
                }))
            || (matches!(status, "expired_or_cancelled" | "partially_filled_closed")
                && failure_reason == Some("removed_from_execution_ledger")))
}

#[cfg(test)]
mod scheduled_order_reconciliation_tests {
    use super::is_scheduled_order_reconcilable;

    #[test]
    fn recovers_scheduled_orders_misclassified_by_resting_order_reconciliation() {
        assert!(is_scheduled_order_reconcilable(
            "Vwap · P0", "expired_or_cancelled", Some("removed_from_execution_ledger")
        ));
        assert!(is_scheduled_order_reconcilable(
            "Twap · P1", "partially_filled_closed", Some("removed_from_execution_ledger")
        ));
        assert!(!is_scheduled_order_reconcilable(
            "PostOnlyLimit · P0", "expired_or_cancelled", Some("removed_from_execution_ledger")
        ));
    }
}

async fn reconcile_scheduled_fills(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let is_reconcilable = |order: &&TraderOrder| {
        is_scheduled_order_reconcilable(
            &order.order_type,
            &order.status,
            order.failure_reason.as_deref(),
        )
    };
    let has_pending = state
        .settings
        .lock()
        .await
        .orders
        .iter()
        .filter(is_reconcilable)
        .next()
        .is_some();
    if !has_pending {
        return Ok(());
    }
    let (trades_response, runs_response) = tokio::try_join!(
        client.get(format!("{base}/positions/trades")).send(),
        client.get(format!("{base}/positions/vwap")).send(),
    )
    .map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Unable to fetch Auto Trader executions: {error}"),
        )
    })?;
    let trades_response = trades_response.error_for_status().map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Auto Trader trade ledger lookup failed: {error}"),
        )
    })?;
    let runs_response = runs_response.error_for_status().map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Scheduled execution status lookup failed: {error}"),
        )
    })?;
    let trades = trades_response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid Auto Trader trade ledger: {error}"),
            )
        })?;
    let runs = runs_response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid scheduled execution status: {error}"),
            )
        })?;
    let trade_rows = trades["trades"].as_array().cloned().unwrap_or_default();
    let run_rows = runs["runs"].as_array().cloned().unwrap_or_default();
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    let mut learning_updates = Vec::new();
    {
        let usd_rates = settings.usd_rates.clone();
        let Settings {
            orders,
            positions,
            strategy_positions,
            learning,
            realized_pnl,
            daily_realized_pnl,
            ..
        } = &mut *settings;
        for order in orders.iter_mut().filter(|order| {
            is_scheduled_order_reconcilable(
                &order.order_type,
                &order.status,
                order.failure_reason.as_deref(),
            )
        }) {
            if order.execution_order_id.is_none() {
                let candidate = run_rows
                    .iter()
                    .filter(|run| {
                        run["symbol"].as_str() == Some(order.symbol.as_str())
                            && run["side"].as_str() == Some(order.side.as_str())
                            && run["quantity"].as_f64().is_some_and(|qty| {
                                (qty - order.quantity).abs() <= order.quantity.abs() * 1e-9
                            })
                            && run["started_at"]
                                .as_u64()
                                .is_some_and(|at| at.abs_diff(order.timestamp) <= 120_000)
                    })
                    .min_by_key(|run| {
                        run["started_at"]
                            .as_u64()
                            .unwrap_or_default()
                            .abs_diff(order.timestamp)
                    });
                if let Some(id) = candidate.and_then(|run| run["id"].as_str()) {
                    order.execution_order_id = Some(id.to_owned());
                    changed = true;
                } else {
                    order.status = "failed".into();
                    order.failure_reason =
                        Some("Scheduled run could not be found in the position manager".into());
                    changed = true;
                    continue;
                }
            }
            let Some(run_id) = order.execution_order_id.as_deref() else {
                continue;
            };
            let run = run_rows.iter().find(|run| run["id"].as_str() == Some(run_id)).cloned().unwrap_or_else(|| {
                let matching = trade_rows.iter().filter(|fill| {
                    fill["source"].as_str() == Some("auto")
                        && fill["parent_order_id"].as_str() == Some(run_id)
                        && fill["symbol"].as_str() == Some(order.symbol.as_str())
                        && fill["side"].as_str() == Some(order.side.as_str())
                }).collect::<Vec<_>>();
                let executed_quantity = matching.iter().map(|fill| fill["quantity"].as_f64().unwrap_or(0.0).max(0.0)).sum::<f64>();
                let notional = matching.iter().map(|fill| fill["quantity"].as_f64().unwrap_or(0.0).max(0.0) * fill["execution_price"].as_f64().unwrap_or(0.0).max(0.0)).sum::<f64>();
                serde_json::json!({
                    "id": run_id,
                    "status": "interrupted",
                    "executed_quantity": executed_quantity,
                    "execution_vwap": if executed_quantity > 0.0 { notional / executed_quantity } else { 0.0 },
                    "error": "Scheduled run record is missing; tagged fills were reconciled and no further execution is assumed."
                })
            });
            let fills = trade_rows
                .iter()
                .filter(|fill| {
                    fill["source"].as_str() == Some("auto")
                        && fill["parent_order_id"].as_str() == Some(run_id)
                        && fill["symbol"].as_str() == Some(order.symbol.as_str())
                        && fill["side"].as_str() == Some(order.side.as_str())
                        && fill["id"].as_str().is_some_and(|id| {
                            !order.processed_fill_trade_ids.iter().any(|seen| seen == id)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            let has_tagged_fills = !fills.is_empty();
            for fill in fills {
                let Some(id) = fill["id"].as_str().map(str::to_owned) else {
                    continue;
                };
                let qty = fill["quantity"].as_f64().unwrap_or(0.0);
                let price = fill["execution_price"].as_f64().unwrap_or(0.0);
                let fee = fill["fee"].as_f64().unwrap_or(0.0).abs();
                if !qty.is_finite() || qty <= 0.0 || !price.is_finite() || price <= 0.0 {
                    continue;
                }
                let count_entry = order.filled_quantity <= 1e-12;
                let (realized, learned) = apply_auto_fill(
                    positions,
                    &order.symbol,
                    &order.side,
                    qty,
                    price,
                    fee,
                    "ScheduledExecution",
                    order.learning_context.as_deref().unwrap_or(&order.symbol),
                    count_entry,
                    order.horizon_take_profit_percent,
                );
                tag_position_strategy(positions, &order.symbol, order.strategy.as_deref());
                let (strategy_realized, _) = apply_strategy_position_fill(
                    strategy_positions,
                    positions,
                    &order.symbol,
                    order.strategy.as_deref().unwrap_or("live-insights"),
                    &order.side,
                    qty,
                    price,
                    fee,
                    "ScheduledExecution",
                    order.learning_context.as_deref().unwrap_or(&order.symbol),
                    count_entry,
                    order.horizon_take_profit_percent,
                    order.decision_id.as_deref(),
                    None,
                    None,
                    None,
                );
                let prior_quantity = order.filled_quantity;
                order.filled_quantity = (prior_quantity + qty).min(order.quantity);
                order.fill_rate = (order.filled_quantity / order.quantity).clamp(0.0, 1.0);
                order.average_price = Some(
                    (order.average_price.unwrap_or(0.0) * prior_quantity + price * qty)
                        / order.filled_quantity,
                );
                order.fees_paid += fee;
                order.realized_pnl += realized;
                record_strategy_order_realized_pnl(order, strategy_realized);
                order.processed_fill_trade_ids.push(id);
                let realized_usd = realized * quote_usd_rate_for_map(&usd_rates, &order.symbol);
                *realized_pnl += realized_usd;
                *daily_realized_pnl += realized_usd;
                if let Some(kind) = learned {
                    learning_updates.push((kind, realized_usd));
                }
                changed = true;
            }
            // Compatibility for schedules created before ledger source/parent tags were added.
            if !has_tagged_fills {
                let target_filled = run["executed_quantity"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, order.quantity);
                let delta_quantity = (target_filled - order.filled_quantity).max(0.0);
                let price = run["execution_vwap"].as_f64().unwrap_or(0.0);
                if delta_quantity > 1e-12 && price.is_finite() && price > 0.0 {
                    let fee = delta_quantity * price * configured_taker_fee_rate();
                    let count_entry = order.filled_quantity <= 1e-12;
                    let (realized, learned) = apply_auto_fill(
                        positions,
                        &order.symbol,
                        &order.side,
                        delta_quantity,
                        price,
                        fee,
                        "ScheduledExecution",
                        order.learning_context.as_deref().unwrap_or(&order.symbol),
                        count_entry,
                        order.horizon_take_profit_percent,
                    );
                    tag_position_strategy(positions, &order.symbol, order.strategy.as_deref());
                    let (strategy_realized, _) = apply_strategy_position_fill(
                        strategy_positions,
                        positions,
                        &order.symbol,
                        order.strategy.as_deref().unwrap_or("live-insights"),
                        &order.side,
                        delta_quantity,
                        price,
                        fee,
                        "ScheduledExecution",
                        order.learning_context.as_deref().unwrap_or(&order.symbol),
                        count_entry,
                        order.horizon_take_profit_percent,
                        order.decision_id.as_deref(),
                        None,
                        None,
                        None,
                    );
                    let previous_notional =
                        order.filled_quantity * order.average_price.unwrap_or(price);
                    let next_quantity = order.filled_quantity + delta_quantity;
                    order.average_price =
                        Some((previous_notional + delta_quantity * price) / next_quantity);
                    order.filled_quantity = next_quantity;
                    order.fill_rate = (next_quantity / order.quantity).clamp(0.0, 1.0);
                    order.fees_paid += fee;
                    order.realized_pnl += realized;
                    record_strategy_order_realized_pnl(order, strategy_realized);
                    let realized_usd = realized * quote_usd_rate_for_map(&usd_rates, &order.symbol);
                    *realized_pnl += realized_usd;
                    *daily_realized_pnl += realized_usd;
                    if let Some(kind) = learned {
                        learning_updates.push((kind, realized_usd));
                    }
                    changed = true;
                }
            }
            let next_status = match run["status"].as_str().unwrap_or("running") {
                "running" => {
                    if order.filled_quantity > 0.0 {
                        "partially_filled"
                    } else {
                        "accepted"
                    }
                }
                "completed" if order.filled_quantity + 1e-12 >= order.quantity => "filled",
                "completed" if order.filled_quantity > 1e-12 => "partially_filled_closed",
                "completed" => "expired_or_cancelled",
                "cancelled" if order.filled_quantity > 1e-12 => "partially_filled_closed",
                "cancelled" => "cancelled",
                "failed" => "failed",
                "interrupted" => "interrupted",
                _ => "accepted",
            };
            if order.status != next_status {
                order.status = next_status.to_owned();
                changed = true;
            }
            if let Some(error) = run["error"].as_str() {
                if order.failure_reason.as_deref() != Some(error) {
                    order.failure_reason = Some(error.to_owned());
                    changed = true;
                }
            }
        }
        for (kind, realized) in learning_updates {
            let stats = learning.entry(kind).or_default();
            record_learning_outcome(stats, realized);
        }
    }
    if changed {
        settings.used_notional_today = allocated_notional(&settings);
        request_settings_persist(state)?;
    }
    Ok(())
}
