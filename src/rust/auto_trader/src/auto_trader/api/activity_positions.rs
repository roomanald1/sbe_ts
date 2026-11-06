struct ActivityPositionSnapshot {
    orders: Vec<TraderOrder>,
    auto_positions: Vec<(String, TraderPosition)>,
    strategy_positions: Vec<OpenStrategyPosition>,
    open_position_symbols: Vec<String>,
    position_exit_policies: Vec<PositionExitPolicy>,
}

fn external_close_order_for_trade<'a>(
    trade: &serde_json::Value,
    orders: &'a [TraderOrder],
    remaining_quantity: &[f64],
) -> Option<usize> {
    if trade["source"].as_str().unwrap_or("auto") != "auto" {
        return None;
    }
    let symbol = trade["symbol"].as_str()?;
    let side = trade["side"].as_str()?;
    let timestamp = trade["timestamp"].as_u64()?;
    let quantity = trade["quantity"].as_f64()?;
    orders
        .iter()
        .enumerate()
        .filter(|(index, order)| {
            order.purpose == "external-close"
                && order.symbol == symbol
                && order.side == side
                && order.timestamp >= timestamp
                && remaining_quantity.get(*index).copied().unwrap_or_default()
                    + 1e-12
                    >= quantity
        })
        .min_by_key(|(_, order)| order.timestamp.saturating_sub(timestamp))
        .map(|(index, _)| index)
}

async fn fetch_closed_ledger_rows(
    client: &reqwest::Client,
    position_url: &str,
    today_start: u64,
    orders: &[TraderOrder],
    open_positions: &[OpenStrategyPosition],
) -> Vec<serde_json::Value> {
    let Ok(response) = client
        .get(format!(
            "{position_url}/positions/trades?since_ms={today_start}"
        ))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    else {
        return Vec::new();
    };
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    let mut order_by_id = BTreeMap::<String, &TraderOrder>::new();
    for order in orders {
        order_by_id.insert(order.timestamp.to_string(), order);
        if let Some(id) = order.execution_order_id.as_ref() {
            order_by_id.insert(id.clone(), order);
        }
    }
    let mut groups = BTreeMap::<(String, String, String), Vec<serde_json::Value>>::new();
    let mut entry_groups = std::collections::BTreeSet::new();
    let mut manual_close_groups = std::collections::BTreeSet::new();
    let mut external_close_remaining = orders
        .iter()
        .map(|order| order.filled_quantity.max(0.0))
        .collect::<Vec<_>>();
    for trade in body["trades"].as_array().into_iter().flatten() {
        let parent = trade["parent_order_id"].as_str().unwrap_or_default();
        let order = order_by_id.get(parent).copied().or_else(|| {
            let index = external_close_order_for_trade(trade, orders, &external_close_remaining)?;
            let quantity = trade["quantity"].as_f64().unwrap_or_default();
            external_close_remaining[index] =
                (external_close_remaining[index] - quantity).max(0.0);
            orders.get(index)
        });
        let trade_source = trade["source"].as_str().unwrap_or("auto");
        let Some(order) = order else {
            continue;
        };
        if trade_source != "auto" && (order.strategy.is_none() || order.decision_id.is_none()) {
            continue;
        }
        let (Some(strategy), Some(decision_id), Some(symbol)) = (
            order.strategy.as_ref(),
            order.decision_id.as_ref(),
            trade["symbol"].as_str(),
        ) else {
            continue;
        };
        let key = (strategy.clone(), decision_id.clone(), symbol.to_owned());
        if matches!(order.purpose.as_str(), "signal" | "scout") {
            entry_groups.insert(key.clone());
        }
        if order.purpose == "external-close" {
            manual_close_groups.insert(key.clone());
        }
        groups.entry(key).or_default().push(trade.clone());
    }
    let open = open_positions
        .iter()
        .filter_map(|position| {
            position.decision_id.as_ref().map(|id| {
                (
                    position.strategy.clone(),
                    id.clone(),
                    position.symbol.clone(),
                )
            })
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut rows = Vec::new();
    for ((strategy, decision_id, symbol), mut trades) in groups {
        trades.sort_by_key(|trade| trade["timestamp"].as_u64().unwrap_or_default());
        let net = trades
            .iter()
            .map(|trade| {
                if trade["side"].as_str() == Some("buy") {
                    trade["quantity"].as_f64().unwrap_or_default()
                } else {
                    -trade["quantity"].as_f64().unwrap_or_default()
                }
            })
            .sum::<f64>();
        // Keep open lots out of closed-trade rows even if the strategy snapshot
        // and fill history disagree about the decision ID.
        let key = (strategy.clone(), decision_id.clone(), symbol.clone());
        if open.contains(&key) || (entry_groups.contains(&key) && net.abs() > 1e-12) {
            continue;
        }
        let first = trades.first().cloned().unwrap_or_default();
        let last = trades.last().cloned().unwrap_or_default();
        let entry_side = first["side"].as_str().unwrap_or("buy");
        let quantity = trades
            .iter()
            .filter(|trade| trade["side"].as_str() == Some(entry_side))
            .map(|trade| trade["quantity"].as_f64().unwrap_or_default())
            .sum::<f64>();
        let manually_closed = manual_close_groups.contains(&(
            strategy.clone(),
            decision_id.clone(),
            symbol.clone(),
        ));
        let key = format!("{strategy}:{decision_id}:{symbol}");
        rows.push(serde_json::json!({"rowId":format!("ledger-summary:{key}"),"status":"Closed","decisionId":decision_id,"strategyInstanceId":decision_id,"symbol":symbol,"side":entry_side,"openedAt":first["timestamp"],"closedAt":last["timestamp"],"quantity":quantity,"signedQuantity":if entry_side == "buy" {quantity} else {-quantity},"entryPrice":first["execution_price"],"pnl":0,"entryFees":0,"estimatedExitFeeRate":0,"estimatedExitSlippageBps":0,"pnlQuoteCurrency":symbol.split_once('-').map(|(_,q)|q).unwrap_or("USD"),"strategy":strategy,"direction":if entry_side == "buy" {"LONG"} else {"SHORT"},"strength":null,"scout":false,"source":"unknown","manuallyClosed":manually_closed,"components":{}}));
        for trade in trades {
            let trade_source = trade["source"].as_str().unwrap_or("auto");
            rows.push(serde_json::json!({"rowId":format!("ledger-trade:{}",trade["id"].as_str().unwrap_or_default()),"status":"Closed","decisionId":decision_id,"strategyInstanceId":decision_id,"symbol":symbol,"side":trade["side"],"openedAt":trade["timestamp"],"closedAt":trade["timestamp"],"quantity":trade["quantity"],"signedQuantity":if trade["side"].as_str()==Some("buy"){trade["quantity"].as_f64().unwrap_or_default()}else{-trade["quantity"].as_f64().unwrap_or_default()},"entryPrice":trade["execution_price"],"pnl":0,"entryFees":trade["fee"],"estimatedExitFeeRate":trade["fee_rate"],"estimatedExitSlippageBps":0,"pnlQuoteCurrency":symbol.split_once('-').map(|(_,q)|q).unwrap_or("USD"),"strategy":strategy,"direction":trade["side"],"strength":null,"scout":false,"source":"unknown","components":{},"ledgerTrade":trade,"ledgerSource":trade_source}));
        }
    }
    rows
}

#[cfg(test)]
mod closed_ledger_rows_tests {
    use super::*;

    fn external_close_order() -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": 120,
            "symbol": "LINK-USDT",
            "side": "sell",
            "order_type": "ExternalLedgerClose",
            "purpose": "external-close",
            "strategy": "risk-adjusted-trend-portfolio",
            "decision_id": "entry-1",
            "quantity": 7.0,
            "filled_quantity": 7.0,
            "average_price": 10.9,
            "status": "filled",
            "realized_pnl": 0.3
        }))
        .unwrap()
    }

    #[test]
    fn attributes_parentless_auto_close_fills_to_external_close_order() {
        let orders = vec![external_close_order()];
        let trade = serde_json::json!({
            "source": "auto",
            "symbol": "LINK-USDT",
            "side": "sell",
            "timestamp": 119,
            "quantity": 7.0
        });
        assert_eq!(external_close_order_for_trade(&trade, &orders, &[7.0]), Some(0));
    }

    #[test]
    fn does_not_attribute_manual_fills_or_wrong_side_to_auto_close() {
        let orders = vec![external_close_order()];
        let manual_trade = serde_json::json!({
            "source": "manual", "symbol": "LINK-USDT", "side": "sell",
            "timestamp": 119, "quantity": 7.0
        });
        let wrong_side = serde_json::json!({
            "source": "auto", "symbol": "LINK-USDT", "side": "buy",
            "timestamp": 119, "quantity": 7.0
        });
        assert_eq!(external_close_order_for_trade(&manual_trade, &orders, &[7.0]), None);
        assert_eq!(external_close_order_for_trade(&wrong_side, &orders, &[7.0]), None);
    }
}

fn snapshot_activity_positions(settings: &Settings, today_start: u64) -> ActivityPositionSnapshot {
    let today = utc_day();
    let orders = settings
        .orders
        .iter()
        // A protective order can be armed on a prior day and execute today.
        // Keep it so the position ledger's since-today barrier P&L is attributed.
        .filter(|order| {
            order.timestamp >= today_start
                || order.purpose == "stop-loss"
                || order.strategy_realized_pnl_day.as_deref() == Some(today.as_str())
        })
        .rev()
        .take(MAX_RETAINED_AUTO_TRADER_ORDERS)
        .cloned()
        .map(|mut order| {
            if order.target_price.is_none() && order.purpose == "stop-loss" {
                if let Some(position) = settings
                    .positions
                    .get(&order.symbol)
                    .filter(|position| position.quantity.abs() > 1e-12)
                {
                    let quantity = position.quantity.abs();
                    let loss_fraction = settings.stop_loss_percent / 100.0;
                    let fee_rate = configured_taker_fee_rate();
                    order.target_price = Some(if position.quantity > 0.0 {
                        (quantity * position.average_entry_price * (1.0 - loss_fraction)
                            + position.entry_fees)
                            / (quantity * (1.0 - fee_rate))
                    } else {
                        (quantity * position.average_entry_price * (1.0 + loss_fraction)
                            - position.entry_fees)
                            / (quantity * (1.0 + fee_rate))
                    });
                }
            }
            if order.quantity > 0.0
                && order.filled_quantity >= order.quantity
                && matches!(order.status.as_str(), "armed" | "resting")
            {
                order.status = "filled".into();
            }
            order
        })
        .collect::<Vec<_>>();
    let auto_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| {
            let mut position = position.clone();
            if position.strategy.is_none() {
                // Reconciliation can leave zero-quantity strategy lots behind. They must
                // not make an otherwise unambiguous live position lose its attribution.
                let active_lots = settings
                    .strategy_positions
                    .get(symbol)
                    .into_iter()
                    .flat_map(|lots| lots.iter())
                    .filter(|(_, lot)| lot.quantity.abs() > 1e-12)
                    .collect::<Vec<_>>();
                if let [(strategy, lot)] = active_lots.as_slice() {
                    position.strategy = Some((*strategy).clone());
                    if position.entry_decision_id.is_none() {
                        position.entry_decision_id = lot.entry_decision_id.clone();
                    }
                }
            }
            (symbol.clone(), position)
        })
        .collect::<Vec<_>>();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let strategy_positions = settings
        .strategy_positions
        .iter()
        .flat_map(|(symbol, lots)| {
            lots.iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(strategy, position)| {
                    let strategy_kind = Strategy::from_name(strategy);
                    let stop_loss_active =
                        strategy_kind.map_or(true, Strategy::uses_protective_stops);
                    let take_profit_active =
                        strategy_kind.map_or(true, Strategy::uses_fixed_take_profit);
                    let effective_stop_loss = position
                        .stop_loss_percent
                        .unwrap_or(settings.stop_loss_percent);
                    let effective_take_profit = if take_profit_active {
                        let candidate = position
                            .take_profit_percent
                            .or_else(|| {
                                position
                                    .horizon_take_profit_percent
                                    .map(|target| target.max(settings.take_profit_percent))
                            })
                            .unwrap_or(settings.take_profit_percent);
                        Some(age_adjusted_take_profit_percent(
                            Some(strategy.as_str()),
                            position.entry_timestamp_ms,
                            now_ms,
                            confidence_adjusted_take_profit_percent(
                                &settings.closed_trade_returns_percent,
                                candidate,
                                effective_stop_loss,
                            ),
                        ))
                    } else {
                        None
                    };
                    OpenStrategyPosition {
                        symbol: symbol.clone(),
                        strategy: strategy.clone(),
                        decision_id: position.entry_decision_id.clone(),
                        quantity: position.quantity,
                        average_entry_price: position.average_entry_price,
                        entry_fees: -position.entry_fees.abs(),
                        entry_timestamp_ms: position.entry_timestamp_ms,
                        entry_order_type: position.entry_order_type.clone(),
                        entry_order_types: if position.entry_order_types.is_empty()
                            && !position.entry_order_type.is_empty()
                        {
                            vec![
                                position
                                    .entry_order_type
                                    .split(" · ")
                                    .next()
                                    .unwrap_or(&position.entry_order_type)
                                    .to_owned(),
                            ]
                        } else {
                            position.entry_order_types.clone()
                        },
                        stop_loss_percent: stop_loss_active.then_some(effective_stop_loss),
                        take_profit_percent: effective_take_profit,
                        horizon_take_profit_percent: position.horizon_take_profit_percent,
                        stop_loss_active,
                        take_profit_active,
                        carry_details: None,
                    }
                })
        })
        .collect::<Vec<_>>();
    let open_position_symbols = auto_positions
        .iter()
        .map(|(symbol, _)| symbol.clone())
        .collect::<Vec<_>>();
    let position_exit_policies = auto_positions
        .iter()
        .map(|(symbol, position)| {
            let take_profit_candidate = position
                .take_profit_percent
                .or_else(|| {
                    position
                        .horizon_take_profit_percent
                        .map(|target| target.max(settings.take_profit_percent))
                })
                .unwrap_or(settings.take_profit_percent);
            let stop_loss_percent = position
                .stop_loss_percent
                .unwrap_or(settings.stop_loss_percent);
            let take_profit_percent = confidence_adjusted_take_profit_percent(
                &settings.closed_trade_returns_percent,
                take_profit_candidate,
                stop_loss_percent,
            );
            PositionExitPolicy {
                symbol: symbol.clone(),
                stop_loss_percent,
                take_profit_percent: age_adjusted_take_profit_percent(
                    position.strategy.as_deref(),
                    position.entry_timestamp_ms,
                    now_ms,
                    take_profit_percent,
                ),
            }
        })
        .collect();
    ActivityPositionSnapshot {
        orders,
        auto_positions,
        strategy_positions,
        open_position_symbols,
        position_exit_policies,
    }
}

async fn await_activity_realized_pnl(
    client: &reqwest::Client,
    position_url: &str,
    orders: &mut [TraderOrder],
) -> (
    Option<f64>,
    Option<BTreeMap<String, f64>>,
    Vec<PnlBreakdown>,
) {
    let ledger_pnl = fetch_ledger_realized_pnl(client, position_url).await.ok();
    let (realized_pnl, realized_pnl_by_quote, pnl_breakdown) = if let Some(pnl) = ledger_pnl {
        for order in orders.iter_mut() {
            let realized = if order.purpose == "stop-loss" {
                // Barrier IDs are the ledger parent_order_id for stop fills.
                // Timestamp keys collide when multiple stops are armed in the
                // same millisecond, incorrectly repeating their combined P&L.
                order
                    .execution_order_id
                    .as_ref()
                    .and_then(|id| pnl.realized_pnl_by_parent_order.get(id))
                    .copied()
            } else if order.order_type.starts_with("Twap") || order.order_type.starts_with("Vwap") {
                order
                    .execution_order_id
                    .as_ref()
                    .and_then(|id| pnl.realized_pnl_by_parent_order.get(id))
                    .copied()
            } else {
                let key = order.timestamp.to_string();
                pnl.realized_pnl_by_parent_order.get(&key).copied()
            };
            order.realized_pnl = realized.unwrap_or(0.0);
        }
        let mut grouped =
            BTreeMap::<(String, String, String, String, Option<String>), (f64, usize)>::new();
        for order in orders.iter() {
            let today = utc_day();
            let strategy_realized =
                if order.strategy_realized_pnl_day.as_deref() == Some(today.as_str()) {
                    order.strategy_realized_pnl_today.unwrap_or_default()
                } else if order.purpose == "stop-loss" {
                    // For legacy or pre-midnight barriers, `realized_pnl` has just
                    // been replaced with the ledger's since-UTC-midnight amount.
                    order.realized_pnl
                } else {
                    order.strategy_realized_pnl.unwrap_or(order.realized_pnl)
                };
            if strategy_realized.abs() <= 1e-12 {
                continue;
            }
            let direction = if order.side.eq_ignore_ascii_case("sell") {
                "LONG"
            } else {
                "SHORT"
            };
            let total = grouped
                .entry((
                    order.symbol.clone(),
                    direction.to_owned(),
                    order.purpose.clone(),
                    order
                        .strategy
                        .clone()
                        .unwrap_or_else(|| "legacy-unknown".into()),
                    order.decision_id.clone(),
                ))
                .or_default();
            total.0 += strategy_realized;
            total.1 += 1;
        }
        let mut breakdown = grouped
            .into_iter()
            .map(
                |(
                    (symbol, direction, purpose, strategy, decision_id),
                    (realized_pnl, outcomes),
                )| PnlBreakdown {
                    symbol,
                    direction,
                    purpose,
                    strategy,
                    decision_id,
                    realized_pnl,
                    outcomes,
                },
            )
            .collect::<Vec<_>>();
        breakdown
            .sort_by(|left, right| right.realized_pnl.abs().total_cmp(&left.realized_pnl.abs()));
        (
            pnl.realized_pnl_all_time_usd,
            Some(pnl.realized_pnl_by_quote),
            breakdown,
        )
    } else {
        for order in orders.iter_mut() {
            order.realized_pnl = 0.0;
        }
        (None, None, Vec::new())
    };
    (realized_pnl, realized_pnl_by_quote, pnl_breakdown)
}
