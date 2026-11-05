fn is_maker_execution_order_type(order_type: &str) -> bool {
    order_type.starts_with("PostOnlyLimit") || order_type.starts_with("Gtd")
}

fn is_maker_order_reconcilable(order_type: &str, status: &str) -> bool {
    is_maker_execution_order_type(order_type) && matches!(status,
        "resting" | "partially_filled" | "accepted"
        | "expired_or_cancelled" | "partially_filled_closed")
}

#[derive(Clone, Deserialize)]

struct MakerFillRecord {
    id: String,
    symbol: String,
    side: String,
    quantity: f64,
    execution_price: f64,
    fee: f64,
    resting_order_id: Option<String>,
}

async fn reconcile_maker_fills(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let has_pending = state.settings.lock().await.orders.iter().any(|order| {
        order.execution_order_id.is_some()
            && is_maker_order_reconcilable(&order.order_type, &order.status)
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
    let Settings {
        orders, positions, strategy_positions,
        execution_feedback_outbox, execution_learning_order_ids, ..
    } = &mut *settings;
    for order in orders.iter_mut().filter(|order| {
        is_maker_order_reconcilable(&order.order_type, &order.status)
    }) {
        // An inactive order may already have been mirrored into aggregate
        // inventory by sync_auto_positions. Recover its lot attribution without
        // applying that inventory a second time.
        let recovering_inactive = matches!(order.status.as_str(),
            "expired_or_cancelled" | "partially_filled_closed");
        let Some(id) = order.execution_order_id.clone() else {
            continue;
        };
        let pending_fills = records
            .iter()
            .filter(|fill| {
                fill.resting_order_id.as_deref() == Some(id.as_str())
                    && fill.symbol == order.symbol
                    && fill.side == order.side
                    && !order
                        .processed_fill_trade_ids
                        .iter()
                        .any(|processed| processed == &fill.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut fill_updated = false;
        for fill in pending_fills {
            if !fill.quantity.is_finite()
                || fill.quantity <= 0.0
                || !fill.execution_price.is_finite()
                || fill.execution_price <= 0.0
            {
                continue;
            }
            let count_entry = order.filled_quantity <= 1e-12;
            let execution_kind = order
                .order_type
                .split(" · ")
                .next()
                .unwrap_or("PostOnlyLimit");
            let realized = if recovering_inactive {
                0.0
            } else {
                apply_auto_fill(
                positions,
                &order.symbol,
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                execution_kind,
                order.learning_context.as_deref().unwrap_or(&order.symbol),
                count_entry,
                order.horizon_take_profit_percent,
                ).0
            };
            let attribution_changed = positions.get(&order.symbol)
                .is_some_and(|position| position.strategy != order.strategy);
            tag_position_strategy(positions, &order.symbol, order.strategy.as_deref());
            let (strategy_realized, _) = apply_strategy_position_fill(
                strategy_positions,
                positions,
                &order.symbol,
                order.strategy.as_deref().unwrap_or("live-insights"),
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                execution_kind,
                order.learning_context.as_deref().unwrap_or(&order.symbol),
                count_entry,
                order.horizon_take_profit_percent,
                order.decision_id.as_deref(),
                None,
                None,
                None,
            );
            if attribution_changed {
                if let Some(position) = positions.get_mut(&order.symbol) {
                    position.entry_decision_id = order.decision_id.clone();
                    position.strategy_decision = None;
                    position.entry_context = order.learning_context.clone().unwrap_or_default();
                    position.entry_order_type = execution_kind.to_owned();
                    position.entry_order_types = vec![execution_kind.to_owned()];
                }
            }
            order.filled_quantity += fill.quantity;
            order.fill_rate = (order.filled_quantity / order.quantity).clamp(0.0, 1.0);
            order.fees_paid += fill.fee.abs();
            let total_notional = order.average_price.unwrap_or(0.0)
                * (order.filled_quantity - fill.quantity)
                + fill.execution_price * fill.quantity;
            order.average_price = Some(total_notional / order.filled_quantity);
            order.realized_pnl += realized;
            record_strategy_order_realized_pnl(order, strategy_realized);
            order.processed_fill_trade_ids.push(fill.id.clone());
            changed = true;
            fill_updated = true;
        }
        let prior_status = order.status.clone();
        if order.filled_quantity + 1e-12 >= order.quantity {
            order.status = "filled".into();
            order.failure_reason = None;
        } else if active_ids.contains(&id) {
            order.status = if order.filled_quantity > 0.0 {
                "partially_filled"
            } else {
                "resting"
            }
            .into();
        } else if !active_ids.contains(&id) {
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
        }
        if fill_updated && order.decision_id.is_some() {
            queue_auto_trader_audit(state, serde_json::json!({
                "decision_id": order.decision_id,
                "trade_id": order.decision_id,
                "stage": "entry_fill",
                "timestamp": current_time_ms(),
                "symbol": order.symbol,
                "strategy": order.strategy,
                "side": order.side,
                "status": order.status,
                "order_type": order.order_type,
                "order_quantity": order.quantity,
                "filled_quantity": order.filled_quantity,
                "average_price": order.average_price,
                "fees": order.fees_paid,
                "realized_pnl": order.realized_pnl,
                "fill_rate": order.fill_rate,
                "learning_context": order.learning_context,
            }));
        }
        changed |= order.status != prior_status;
        record_execution_order_quality(execution_feedback_outbox, execution_learning_order_ids, order);
    }
    if changed {
        request_settings_persist(state)?;
    }
    Ok(())
}


#[cfg(test)]
mod maker_reconciliation_tests {
    use super::*;

    #[test]
    fn maker_fills_are_recovered_after_order_disappears() {
        for kind in ["PostOnlyLimit ? P0", "Gtd ? P1"] {
            for status in ["resting", "expired_or_cancelled", "partially_filled_closed"] {
                assert!(is_maker_order_reconcilable(kind, status));
            }
            assert!(!is_maker_order_reconcilable(kind, "filled"));
            assert!(!is_maker_order_reconcilable(kind, "failed"));
        }
        assert!(!is_maker_order_reconcilable("Vwap", "resting"));
    }

    #[tokio::test]
    async fn recovers_true_strategy_without_double_counting_mirrored_inventory() {
        let app = Router::new()
            .route("/positions/trades", get(|| async {
                Json(serde_json::json!({"trades": [{
                    "id": "maker-fill-1", "symbol": "BTC-USDT", "side": "buy",
                    "quantity": 1.0, "execution_price": 100.0, "fee": -0.08,
                    "resting_order_id": "resting-1"
                }]}))
            }))
            .route("/positions/orders", get(|| async {
                Json(serde_json::json!({"orders": []}))
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let strategy = "risk-adjusted-trend-portfolio";
        let mut settings = Settings::default();
        let mut order = strategy_cleanup_integration_tests::order(strategy, 42);
        order.side = "buy".into();
        order.order_type = "PostOnlyLimit ? P0".into();
        order.purpose = "signal".into();
        order.status = "expired_or_cancelled".into();
        order.execution_order_id = Some("resting-1".into());
        order.decision_id = Some("current-entry".into());
        settings.orders.push(order);
        settings.positions.insert("BTC-USDT".into(), TraderPosition {
            quantity: 1.0, average_entry_price: 100.0,
            strategy: Some("live-vwap-dislocation".into()),
            entry_decision_id: Some("stale-vwap-entry".into()),
            ..TraderPosition::default()
        });
        let state = strategy_cleanup_integration_tests::test_state(settings);
        let base = format!("http://{address}");
        reconcile_maker_fills(&state.client, &base, &state).await.unwrap();
        // A second pass must not replay the same fill or duplicate its lot.
        reconcile_maker_fills(&state.client, &base, &state).await.unwrap();
        let settings = state.settings.lock().await;
        let aggregate = &settings.positions["BTC-USDT"];
        assert_eq!(aggregate.quantity, 1.0);
        assert_eq!(aggregate.strategy.as_deref(), Some(strategy));
        assert_eq!(aggregate.entry_decision_id.as_deref(), Some("current-entry"));
        assert_eq!(settings.strategy_positions["BTC-USDT"][strategy].quantity, 1.0);
        assert_eq!(settings.orders[0].status, "filled");
        assert_eq!(settings.orders[0].processed_fill_trade_ids, vec!["maker-fill-1"]);
        server.abort();
    }

}
