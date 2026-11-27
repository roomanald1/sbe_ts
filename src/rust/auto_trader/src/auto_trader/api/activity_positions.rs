struct ActivityPositionSnapshot {
    orders: Vec<TraderOrder>,
    lifecycle_links: BTreeMap<String, serde_json::Value>,
    auto_positions: Vec<(String, TraderPosition)>,
    strategy_positions: Vec<OpenStrategyPosition>,
    open_position_symbols: Vec<String>,
    position_exit_policies: Vec<PositionExitPolicy>,
}

fn selected_exit_parameter_source(
    position: &TraderPosition,
    parameter: &str,
) -> String {
    position
        .exit_parameter_provenance
        .as_ref()
        .and_then(|provenance| provenance.get(parameter))
        .and_then(|selection| selection.get("source"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let value_is_missing = match parameter {
                "stop_loss" => position.stop_loss_percent.is_none(),
                "take_profit" => {
                    position.take_profit_percent.is_none()
                        && position.horizon_take_profit_percent.is_none()
                }
                _ => false,
            };
            if value_is_missing {
                "global_fallback".into()
            } else {
                "unknown".into()
            }
        })
}

fn provenance_selected_percent(
    position: &TraderPosition,
    parameter: &str,
) -> Option<f64> {
    position
        .exit_parameter_provenance
        .as_ref()
        .and_then(|provenance| provenance.get(parameter))
        .and_then(|selection| selection.get("selected_percent"))
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
}

fn initial_risk_evidence_status(position: &TraderPosition, symbol: &str) -> &'static str {
    let Some(reference) = position.initial_risk_reference.as_ref() else {
        return "unavailable";
    };
    if reference.status != "complete"
        || reference.schema_version != 1
        || reference.quote_amount.is_none_or(|amount| !amount.is_finite() || amount <= 0.0)
        || reference.fill_ids.is_empty()
    {
        return "incomplete";
    }
    let quote_currency = symbol.split_once('-').map(|(_, quote)| quote);
    if quote_currency.is_none() || reference.quote_currency.as_deref() != quote_currency {
        return "quote_currency_mismatch";
    }
    let identity_complete = position.lifecycle_identity.as_ref().is_some_and(|identity| {
        identity.schema_version == 1
            && !identity.episode_id.trim().is_empty()
            && !identity.opportunity_id.trim().is_empty()
            && position.trade_id.as_deref() == Some(identity.trade_id.as_str())
    });
    if !identity_complete {
        return "canonical_identity_incomplete";
    }
    if reference.fill_ids.len() != position.entry_fill_ids.len()
        || !reference
            .fill_ids
            .iter()
            .all(|fill_id| position.entry_fill_ids.contains(fill_id))
        || !entry_fill_identity_complete(position)
    {
        return "entry_fill_lineage_incomplete";
    }
    if position.entry_fee_currencies.len() != 1
        || position.entry_fee_currencies[0].as_str() != quote_currency.unwrap_or_default()
    {
        return "fee_currency_not_quote";
    }
    "eligible_for_r_multiple"
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
                && remaining_quantity.get(*index).copied().unwrap_or_default() + 1e-12 >= quantity
        })
        .min_by_key(|(_, order)| order.timestamp.saturating_sub(timestamp))
        .map(|(index, _)| index)
}

fn build_closed_ledger_rows(
    orders: &[TraderOrder],
    lifecycle_links: &BTreeMap<String, serde_json::Value>,
    open_positions: &[OpenStrategyPosition],
    carry_positions: &[DatedCarryPosition],
    body: &serde_json::Value,
    realized_by_parent: Option<&BTreeMap<String, f64>>,
) -> Vec<serde_json::Value> {
    let mut order_by_id = BTreeMap::<String, &TraderOrder>::new();
    let mut orders_by_opportunity = BTreeMap::<String, Vec<&TraderOrder>>::new();
    for order in orders {
        order_by_id.insert(order.timestamp.to_string(), order);
        if let Some(id) = order.execution_order_id.as_ref() {
            order_by_id.insert(id.clone(), order);
        }
        if matches!(order.purpose.as_str(), "signal" | "scout") {
            if let Some(opportunity_id) = order
                .decision_id
                .as_ref()
                .and_then(|decision_id| lifecycle_links.get(decision_id))
                .and_then(|identity| identity.get("opportunity_id"))
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
            {
                orders_by_opportunity
                    .entry(opportunity_id.to_owned())
                    .or_default()
                    .push(order);
            }
        }
    }
    let mut groups = BTreeMap::<(String, String, String), Vec<serde_json::Value>>::new();
    let mut entry_groups = std::collections::BTreeSet::new();
    let mut manual_close_groups = std::collections::BTreeSet::new();
    let mut unassigned_auto_trades = BTreeMap::<String, Vec<serde_json::Value>>::new();
    let mut external_close_remaining = orders
        .iter()
        .map(|order| order.filled_quantity.max(0.0))
        .collect::<Vec<_>>();
    for trade in body["trades"].as_array().into_iter().flatten() {
        let parent = trade["parent_order_id"].as_str().unwrap_or_default();
        let Some(symbol) = trade["symbol"].as_str() else {
            continue;
        };
        let side = trade["side"].as_str();
        let is_manual_auto_fill =
            parent.starts_with("execution-check-") || parent.starts_with("auto-manual-close:");
        let exact_opportunity_order = orders_by_opportunity
            .get(parent)
            .filter(|matches| matches.len() == 1)
            .and_then(|matches| matches.first().copied())
            .filter(|order| order.symbol == symbol && Some(order.side.as_str()) == side);
        let order = order_by_id
            .get(parent)
            .copied()
            .or(exact_opportunity_order)
            .or_else(|| {
            let index = external_close_order_for_trade(trade, orders, &external_close_remaining)?;
            let quantity = trade["quantity"].as_f64().unwrap_or_default();
            external_close_remaining[index] = (external_close_remaining[index] - quantity).max(0.0);
            orders.get(index)
        });
        let trade_source = trade["source"].as_str().unwrap_or("auto");
        // Manual Auto orders bypass AutoTrader's strategy order registry, so
        // their fills have no strategy/decision attribution. Keep them for
        // position-cycle reconstruction below, including fills with no order
        // object at all (for example, immediate IOC market orders).
        let Some(order) = order else {
            if trade_source == "auto" && is_manual_auto_fill {
                unassigned_auto_trades
                    .entry(symbol.to_owned())
                    .or_default()
                    .push(trade.clone());
            }
            continue;
        };
        if trade_source != "auto" && (order.strategy.is_none() || order.decision_id.is_none()) {
            continue;
        }
        if order.strategy.is_none() || order.decision_id.is_none() {
            if trade_source != "auto" || !is_manual_auto_fill {
                continue;
            }
            unassigned_auto_trades
                .entry(symbol.to_owned())
                .or_default()
                .push(trade.clone());
            continue;
        }
        let strategy = order.strategy.as_deref().expect("strategy checked above");
        let decision_id = order
            .decision_id
            .as_deref()
            .expect("decision ID checked above");
        let key = (
            strategy.to_owned(),
            decision_id.to_owned(),
            symbol.to_owned(),
        );
        if matches!(order.purpose.as_str(), "signal" | "scout") {
            entry_groups.insert(key.clone());
        }
        if order.purpose == "external-close" {
            manual_close_groups.insert(key.clone());
        }
        groups.entry(key).or_default().push(trade.clone());
    }
    for (symbol, mut trades) in unassigned_auto_trades {
        trades.sort_by_key(|trade| trade["timestamp"].as_u64().unwrap_or_default());
        let mut cycle = Vec::new();
        let mut quantity = 0.0_f64;
        for trade in trades {
            let fill_quantity = trade["quantity"].as_f64().unwrap_or_default();
            let incoming = if trade["side"].as_str() == Some("buy") {
                fill_quantity
            } else {
                -fill_quantity
            };
            if !incoming.is_finite() || incoming.abs() <= 1e-12 {
                continue;
            }
            cycle.push(trade);
            quantity += incoming;
            if quantity.abs() <= 1e-12 {
                let opened_at = cycle
                    .first()
                    .and_then(|fill| fill["timestamp"].as_u64())
                    .unwrap_or_default();
                let decision_id = format!("manual-auto:{symbol}:{opened_at}");
                let key = ("manual".to_owned(), decision_id, symbol.clone());
                entry_groups.insert(key.clone());
                manual_close_groups.insert(key.clone());
                groups.insert(key, std::mem::take(&mut cycle));
            }
        }
        // Any remaining non-flat cycle stays out of closed positions. It is an
        // open inventory position, not a completed trade.
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
        let entry_notional = trades
            .iter()
            .filter(|trade| trade["side"].as_str() == Some(entry_side))
            .map(|trade| {
                trade["quantity"].as_f64().unwrap_or_default()
                    * trade["execution_price"].as_f64().unwrap_or_default()
            })
            .sum::<f64>();
        let entry_price = if quantity > 0.0 {
            entry_notional / quantity
        } else {
            first["execution_price"].as_f64().unwrap_or_default()
        };
        let total_fees = trades
            .iter()
            .map(|trade| trade["fee"].as_f64().unwrap_or_default().abs())
            .sum::<f64>();
        let entry_fees = trades
            .iter()
            .filter(|trade| trade["side"].as_str() == Some(entry_side))
            .map(|trade| trade["fee"].as_f64().unwrap_or_default().abs())
            .sum::<f64>();
        let parent_orders = trades
            .iter()
            .filter_map(|trade| trade["parent_order_id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let realized_pnl = if realized_by_parent.is_some_and(|realized| {
            parent_orders
                .iter()
                .any(|parent| realized.contains_key(*parent))
        }) {
            let realized_by_parent = realized_by_parent.expect("availability checked above");
            parent_orders
                .iter()
                .filter_map(|parent| realized_by_parent.get(*parent))
                .sum::<f64>()
        } else if net.abs() <= 1e-12 {
            // Fallback only for complete round trips when the ledger P&L
            // endpoint is unavailable. Partial cash flows resemble notional,
            // so they must never be shown as realized P&L.
            trades
                .iter()
                .map(|trade| {
                    let notional = trade["quantity"].as_f64().unwrap_or_default()
                        * trade["execution_price"].as_f64().unwrap_or_default();
                    let cashflow = if trade["side"].as_str() == Some("buy") {
                        -notional
                    } else {
                        notional
                    };
                    cashflow - trade["fee"].as_f64().unwrap_or_default().abs()
                })
                .sum::<f64>()
        } else {
            0.0
        };
        let manually_closed =
            manual_close_groups.contains(&(strategy.clone(), decision_id.clone(), symbol.clone()));
        // Provenance comes only from the persisted entry order and its durable
        // lifecycle link. Legacy rows intentionally retain unknown values.
        let entry_order = orders.iter().find(|order| {
            order.decision_id.as_deref() == Some(decision_id.as_str())
                && order.symbol == symbol
                && matches!(order.purpose.as_str(), "signal" | "scout")
        });
        let lifecycle = lifecycle_links.get(&decision_id);
        let worker_identity = entry_order
            .and_then(|order| order.revalidation_snapshot.as_ref())
            .filter(|snapshot| snapshot.event_identity_version.is_some());
        let provenance = serde_json::json!({
            "exploratory": entry_order.map(|order| order.exploratory),
            "purpose": entry_order.map(|order| order.purpose.as_str()),
            "opportunityId": lifecycle.and_then(|identity| identity["opportunity_id"].as_str()),
            "episodeId": lifecycle.and_then(|identity| identity["episode_id"].as_str()),
            "tradeId": lifecycle.and_then(|identity| identity["trade_id"].as_str()),
            "profileId": lifecycle.and_then(|identity| identity["profile_id"].as_str()),
            "profileHash": lifecycle.and_then(|identity| identity["profile_hash"].as_str()),
            "profileVersion": lifecycle.and_then(|identity| identity["profile_version"].as_u64()),
            "executionMode": lifecycle.and_then(|identity| identity["execution_mode"].as_str()),
            "entryPolicy": lifecycle.and_then(|identity| identity["entry_policy"].as_str()),
            "workerEventIdentityVersion": worker_identity.and_then(|snapshot| snapshot.event_identity_version),
            "workerEventId": worker_identity.and_then(|snapshot| snapshot.event_id.as_deref()),
            "workerEpisodeId": worker_identity.and_then(|snapshot| snapshot.episode_id.as_deref()),
        });
        let outcome_source = if entry_order.is_some_and(|order| order.purpose == "scout") {
            "scout"
        } else {
            match lifecycle.and_then(|identity| identity["entry_policy"].as_str()) {
                Some("strict") => "strict",
                Some("exploratory") => "exploratory",
                _ => "unknown",
            }
        };
        let key = format!("{strategy}:{decision_id}:{symbol}");
        rows.push(serde_json::json!({"rowId":format!("ledger-summary:{key}"),"status":"Closed","decisionId":decision_id,"strategyInstanceId":decision_id,"symbol":symbol,"side":entry_side,"openedAt":first["timestamp"],"closedAt":last["timestamp"],"quantity":quantity,"signedQuantity":if entry_side == "buy" {quantity} else {-quantity},"entryPrice":entry_price,"pnl":realized_pnl,"entryFees":entry_fees,"totalFees":total_fees,"estimatedExitFeeRate":0,"estimatedExitSlippageBps":0,"pnlQuoteCurrency":symbol.split_once('-').map(|(_,q)|q).unwrap_or("USD"),"strategy":strategy,"direction":if entry_side == "buy" {"LONG"} else {"SHORT"},"strength":null,"scout":entry_order.is_some_and(|order| order.purpose == "scout"),"source":outcome_source,"provenance":provenance.clone(),"manuallyClosed":manually_closed,"components":{}}));
        for trade in trades {
            let trade_source = trade["source"].as_str().unwrap_or("auto");
            rows.push(serde_json::json!({"rowId":format!("ledger-trade:{}",trade["id"].as_str().unwrap_or_default()),"status":"Closed","decisionId":decision_id,"strategyInstanceId":decision_id,"symbol":symbol,"side":trade["side"],"openedAt":trade["timestamp"],"closedAt":trade["timestamp"],"quantity":trade["quantity"],"signedQuantity":if trade["side"].as_str()==Some("buy"){trade["quantity"].as_f64().unwrap_or_default()}else{-trade["quantity"].as_f64().unwrap_or_default()},"entryPrice":trade["execution_price"],"pnl":0,"entryFees":trade["fee"],"estimatedExitFeeRate":trade["fee_rate"],"estimatedExitSlippageBps":0,"pnlQuoteCurrency":symbol.split_once('-').map(|(_,q)|q).unwrap_or("USD"),"strategy":strategy,"direction":trade["side"],"strength":null,"scout":entry_order.is_some_and(|order| order.purpose == "scout"),"source":outcome_source,"provenance":provenance.clone(),"components":{},"ledgerTrade":trade,"ledgerSource":trade_source}));
        }
    }
    rows.extend(
        carry_positions
            .iter()
            .filter(|position| position.status.starts_with("closed"))
            .filter_map(|position| {
                let pnl = position.net_pnl_usdt.filter(|value| value.is_finite())?;
                let closed_at = position.closed_at_ms?;
                Some(serde_json::json!({
                    "rowId": format!(
                        "ledger-summary:dated-futures-carry:dated-carry:{}:{}",
                        position.future_symbol,
                        position.spot_symbol.replace("USDT", "-USDT"),
                    ),
                    "status": "Closed",
                    "decisionId": format!("dated-carry:{}", position.future_symbol),
                    "strategyInstanceId": format!("dated-carry:{}", position.future_symbol),
                    "symbol": position.spot_symbol.replace("USDT", "-USDT"),
                    "side": "paired",
                    "openedAt": position.entry_ms,
                    "closedAt": closed_at,
                    "quantity": position.quantity,
                    "signedQuantity": position.quantity,
                    "entryPrice": position.spot_entry_price,
                    "pnl": pnl,
                    "netPnlUsd": pnl,
                    "entryFees": position.entry_fees,
                    "estimatedExitFeeRate": 0,
                    "estimatedExitSlippageBps": 0,
                    "pnlQuoteCurrency": "USDT",
                    "strategy": "dated-futures-carry",
                    "direction": "Spot long / future short",
                    "strength": null,
                    "scout": false,
                    "source": "paper-carry",
                    "manuallyClosed": position.status == "closed_manual",
                    "carryDetails": position,
                    "components": {},
                }))
            }),
    );
    rows
}

#[cfg(test)]
mod closed_ledger_rows_tests {
    use super::*;

    #[test]
    fn open_strategy_position_exposes_only_its_persisted_lifecycle_identity() {
        let decision_id = "decision-open";
        let mut settings = Settings::default();
        settings.strategy_positions.insert(
            "DOGE-USDT".into(),
            BTreeMap::from([(
                "live-vwap-dislocation".to_owned(),
                serde_json::from_value(serde_json::json!({
                    "quantity": -2.0,
                    "entry_decision_id": decision_id,
                    "entry_timestamp_ms": 100,
                    "average_entry_price": 0.05,
                    "entry_fees": 0.001,
                    "realized_pnl": 0.0,
                    "entry_order_type": "PostOnlyLimit",
                    "entry_order_types": ["PostOnlyLimit"],
                    "stop_loss_percent": 0.5,
                    "take_profit_percent": 0.75
                }))
                .unwrap(),
            )]),
        );
        settings.canonical_lifecycle_links.insert(
            decision_id.to_owned(),
            serde_json::from_value(serde_json::json!({
                "schema_version": 1,
                "opportunity_id": "opportunity-1",
                "episode_id": "episode-1",
                "trade_id": "trade-1",
                "profile_id": "profile-1",
                "profile_hash": "sha256:v1:profile",
                "profile_version": 2,
                "execution_mode": "paper",
                "entry_policy": "exploratory",
                "source": "auto_trader_entry_reservation"
            }))
            .unwrap(),
        );

        let snapshot = snapshot_activity_positions(&settings, 0);
        let row = serde_json::to_value(&snapshot.strategy_positions[0]).unwrap();
        assert_eq!(row["decision_id"], decision_id);
        assert_eq!(row["lifecycle"]["trade_id"], "trade-1");
        assert_eq!(row["exit_profile_id"], "profile-1");
        assert_eq!(row["exit_profile_version"], 2);
        assert_eq!(row["lifecycle"]["opportunity_id"], "opportunity-1");
        assert_eq!(row["lifecycle"]["episode_id"], "episode-1");
        assert_eq!(row["stop_loss_source"], "unknown");
        assert_eq!(row["selected_stop_loss_percent"], serde_json::Value::Null);
        assert_eq!(row["exit_control_mode"], "protective_stop_and_target");

        settings.canonical_lifecycle_links.clear();
        let legacy = snapshot_activity_positions(&settings, 0);
        assert!(legacy.strategy_positions[0].lifecycle.is_none());
    }

    #[test]
    fn signal_managed_strategy_is_explicitly_inactive_for_fixed_protection() {
        let mut settings = Settings::default();
        settings.strategy_positions.insert(
            "BTC-USDT".into(),
            BTreeMap::from([(
                "risk-adjusted-monthly-momentum".to_owned(),
                TraderPosition {
                    quantity: 1.0,
                    average_entry_price: 100.0,
                    ..TraderPosition::default()
                },
            )]),
        );
        let snapshot = snapshot_activity_positions(&settings, 0);
        let row = serde_json::to_value(&snapshot.strategy_positions[0]).unwrap();
        assert_eq!(row["stop_loss_active"], false);
        assert_eq!(row["take_profit_active"], false);
        assert_eq!(row["exit_control_mode"], "signal_managed");
    }

    #[test]
    fn exit_parameter_source_is_explicit_when_selected_equals_global_fallback() {
        let mut settings = Settings::default();
        let provenance = serde_json::json!({
            "schema_version":1,
            "profile_id":"profile-btc-momentum-v4",
            "profile_version":4,
            "exit_policy_as_of_ms":1_000,
            "risk_appetite_at_selection":50.0,
            "stop_loss":{
                "source":"validated_exit_policy",
                "selected_percent":0.5,
                "profile_percent":0.5,
                "effective_percent":0.5,
                "global_fallback_percent":0.5
            },
            "take_profit":{
                "source":"validated_exit_policy",
                "selected_percent":0.75,
                "profile_percent":0.75,
                "effective_percent":0.75,
                "global_fallback_percent":0.75
            }
        });
        settings.strategy_positions.insert(
            "BTC-USDT".into(),
            BTreeMap::from([(
                "live-momentum".to_owned(),
                TraderPosition {
                    quantity: 1.0,
                    average_entry_price: 100.0,
                    entry_decision_id: Some("decision-profile".into()),
                    trade_id: Some("trade-profile".into()),
                    lifecycle_identity: Some(CanonicalLifecycleIdentity {
                        schema_version: 1,
                        opportunity_id: "opportunity-profile".into(),
                        episode_id: "episode-profile".into(),
                        trade_id: "trade-profile".into(),
                        profile_id: Some("profile-btc-momentum-v4".into()),
                        profile_hash: None,
                        profile_version: Some(4),
                        execution_mode: "paper".into(),
                        entry_policy: "strict".into(),
                        source: "auto_trader_entry_reservation".into(),
                    }),
                    stop_loss_percent: Some(0.5),
                    take_profit_percent: Some(0.75),
                    exit_parameter_provenance: Some(provenance),
                    ..TraderPosition::default()
                },
            )]),
        );
        settings.canonical_lifecycle_links.insert(
            "decision-profile".into(),
            CanonicalLifecycleIdentity {
                schema_version: 1,
                opportunity_id: "opportunity-profile".into(),
                episode_id: "episode-profile".into(),
                trade_id: "trade-profile".into(),
                profile_id: Some("profile-btc-momentum-v4".into()),
                profile_hash: None,
                profile_version: Some(4),
                execution_mode: "paper".into(),
                entry_policy: "strict".into(),
                source: "auto_trader_entry_reservation".into(),
            },
        );
        settings.stop_loss_percent = 0.5;
        settings.take_profit_percent = 0.75;

        let snapshot = snapshot_activity_positions(&settings, 0);
        let row = serde_json::to_value(&snapshot.strategy_positions[0]).unwrap();
        assert_eq!(row["selected_stop_loss_percent"], 0.5);
        assert_eq!(row["selected_take_profit_percent"], 0.75);
        assert_eq!(row["stop_loss_source"], "validated_exit_policy");
        assert_eq!(row["take_profit_source"], "validated_exit_policy");
        assert_eq!(row["effective_stop_loss_percent"], 0.5);
        assert_eq!(row["effective_take_profit_percent"], 0.75);
        assert_eq!(row["fallback_stop_loss_percent"], 0.5);
        assert_eq!(row["fallback_take_profit_percent"], 0.75);
        assert_eq!(row["exit_profile_id"], "profile-btc-momentum-v4");
        assert_eq!(row["exit_profile_version"], 4);
        assert_eq!(row["exit_policy_as_of_ms"], 1_000);
        assert_eq!(row["initial_risk_evidence_status"], "unavailable");
    }

    fn lineage_order(exploratory: bool, with_worker_identity: bool) -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": 100, "symbol": "LINK-USDT", "side": "buy",
            "order_type": "Limit", "purpose": "signal", "exploratory": exploratory,
            "strategy": "risk-adjusted-trend-portfolio", "decision_id": "decision-1",
            "quantity": 1.0, "filled_quantity": 1.0, "status": "filled", "realized_pnl": 0.0,
            "execution_order_id": "entry-order",
            "revalidation_snapshot": if with_worker_identity { serde_json::json!({
                "symbol":"LINK-USDT", "strategy":"risk-adjusted-trend-portfolio", "direction":"LONG",
                "signal_timestamp_ms":99, "profile_id":"profile-a", "profile_version":3,
                "parameter_hash":"hash-a", "event_identity_version":1,
                "event_id":"event-7", "episode_id":"episode-2"
            }) } else { serde_json::Value::Null }
        })).unwrap()
    }

    fn closed_trades() -> serde_json::Value {
        serde_json::json!({"trades":[
            {"id":"fill-1", "source":"auto", "symbol":"LINK-USDT", "side":"buy", "timestamp":100,
             "quantity":1.0, "execution_price":10.0, "fee":0.1, "fee_rate":0.01, "parent_order_id":"entry-order"},
            {"id":"fill-2", "source":"auto", "symbol":"LINK-USDT", "side":"sell", "timestamp":120,
             "quantity":1.0, "execution_price":11.0, "fee":0.1, "fee_rate":0.01, "parent_order_id":"exit-order"}
        ]})
    }

    #[test]
    fn canonical_opportunity_id_joins_entry_fill_with_protective_exit() {
        let entry = serde_json::from_value::<TraderOrder>(serde_json::json!({
            "timestamp":100,"symbol":"DOGE-USDT","side":"sell","order_type":"Limit",
            "purpose":"signal","exploratory":true,"strategy":"live-vwap-dislocation",
            "decision_id":"decision-1","quantity":2.0,"filled_quantity":2.0,
            "status":"filled","realized_pnl":0.0,"execution_order_id":"entry-execution-1"
        })).unwrap();
        let stop = serde_json::from_value::<TraderOrder>(serde_json::json!({
            "timestamp":120,"symbol":"DOGE-USDT","side":"buy","order_type":"StopMarket",
            "purpose":"stop-loss","exploratory":false,"strategy":"live-vwap-dislocation",
            "decision_id":"decision-1","quantity":2.0,"filled_quantity":2.0,
            "status":"filled","realized_pnl":0.0,"execution_order_id":"barrier-1"
        })).unwrap();
        let links = BTreeMap::from([(
            "decision-1".to_owned(),
            serde_json::json!({
                "opportunity_id":"opportunity-1","episode_id":"episode-1",
                "trade_id":"trade-1","profile_id":"profile-1",
                "execution_mode":"paper","entry_policy":"exploratory"
            }),
        )]);
        let trades = serde_json::json!({"trades":[
            {"id":"fill-entry","source":"auto","symbol":"DOGE-USDT","side":"sell",
             "timestamp":100,"quantity":2.0,"execution_price":0.05,"fee":-0.00008,
             "fee_rate":-0.0008,"parent_order_id":"opportunity-1"},
            {"id":"fill-stop","source":"auto","symbol":"DOGE-USDT","side":"buy",
             "timestamp":120,"quantity":2.0,"execution_price":0.051,"fee":-0.000102,
             "fee_rate":-0.001,"parent_order_id":"barrier-1"}
        ]});

        let rows = build_closed_ledger_rows(
            &[entry, stop], &links, &[], &[], &trades, None,
        );
        assert_eq!(rows.len(), 3, "one summary plus two visible fill records");
        let summary = rows
            .iter()
            .find(|row| row["rowId"].as_str().unwrap().starts_with("ledger-summary:"))
            .unwrap();
        assert_eq!(summary["openedAt"], 100);
        assert_eq!(summary["closedAt"], 120);
        assert_eq!(summary["entryPrice"], 0.05);
        assert_eq!(summary["source"], "exploratory");
        assert_eq!(summary["entryFees"], 0.00008);
        assert_eq!(summary["totalFees"], 0.000182);
        assert_eq!(summary["provenance"]["exploratory"], true);
        assert_eq!(summary["provenance"]["tradeId"], "trade-1");
        let fill_parents = rows
            .iter()
            .filter_map(|row| row.get("ledgerTrade"))
            .filter_map(|trade| trade["parent_order_id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(fill_parents, ["barrier-1", "opportunity-1"].into());
    }

    #[test]
    fn closed_activity_rows_project_versioned_exploratory_lineage() {
        let orders = vec![
            lineage_order(true, true),
            serde_json::from_value(serde_json::json!({
                "timestamp":120,"symbol":"LINK-USDT","side":"sell","order_type":"Market",
                "purpose":"external-close","strategy":"risk-adjusted-trend-portfolio",
                "decision_id":"decision-1","quantity":1.0,"filled_quantity":1.0,
                "status":"filled","realized_pnl":0.0,"execution_order_id":"exit-order"
            }))
            .unwrap(),
        ];
        let links = BTreeMap::from([(
            "decision-1".to_owned(),
            serde_json::json!({
                "opportunity_id":"opportunity-1", "episode_id":"canonical-episode-1", "trade_id":"trade-1",
                "profile_id":"profile-a", "profile_hash":"sha256:v1:abc", "profile_version":3,
                "execution_mode":"paper", "entry_policy":"exploratory"
            }),
        )]);
        let rows = build_closed_ledger_rows(&orders, &links, &[], &[], &closed_trades(), None);
        assert_eq!(rows.len(), 3);
        let summary = rows
            .iter()
            .find(|row| {
                row["rowId"]
                    .as_str()
                    .unwrap()
                    .starts_with("ledger-summary:")
            })
            .unwrap();
        let provenance = &summary["provenance"];
        assert_eq!(provenance["exploratory"], true);
        assert_eq!(provenance["purpose"], "signal");
        assert_eq!(provenance["opportunityId"], "opportunity-1");
        assert_eq!(provenance["episodeId"], "canonical-episode-1");
        assert_eq!(provenance["tradeId"], "trade-1");
        assert_eq!(provenance["profileId"], "profile-a");
        assert_eq!(provenance["profileHash"], "sha256:v1:abc");
        assert_eq!(provenance["profileVersion"], 3);
        assert_eq!(provenance["executionMode"], "paper");
        assert_eq!(provenance["entryPolicy"], "exploratory");
        assert_eq!(provenance["workerEventIdentityVersion"], 1);
        assert_eq!(provenance["workerEventId"], "event-7");
        assert_eq!(provenance["workerEpisodeId"], "episode-2");
        assert!((summary["pnl"].as_f64().unwrap() - 0.8).abs() < 1e-12);
        assert!((summary["totalFees"].as_f64().unwrap() - 0.2).abs() < 1e-12);
        assert_eq!(summary["pnlQuoteCurrency"], "USDT");
        assert!(rows
            .iter()
            .filter(|row| row["ledgerTrade"].is_object())
            .all(|row| row["provenance"] == *provenance));
        let entry_fill = rows.iter().find(|row| row["rowId"] == "ledger-trade:fill-1").unwrap();
        assert_eq!(entry_fill["entryFees"], 0.1);
        assert_eq!(entry_fill["ledgerTrade"]["parent_order_id"], "entry-order");
    }

    #[test]
    fn legacy_closed_activity_rows_keep_missing_lineage_unknown() {
        // Legacy orders keep exploratory status from the durable order while
        // absent lifecycle and worker identities remain unknown.
        let orders = vec![
            lineage_order(true, false),
            serde_json::from_value(serde_json::json!({
                "timestamp":120,"symbol":"LINK-USDT","side":"sell","order_type":"Market",
                "purpose":"external-close","strategy":"risk-adjusted-trend-portfolio",
                "decision_id":"decision-1","quantity":1.0,"filled_quantity":1.0,
                "status":"filled","realized_pnl":0.0,"execution_order_id":"exit-order"
            }))
            .unwrap(),
        ];
        let links = BTreeMap::new();
        let rows = build_closed_ledger_rows(&orders, &links, &[], &[], &closed_trades(), None);
        let summary = rows
            .iter()
            .find(|row| {
                row["rowId"]
                    .as_str()
                    .unwrap()
                    .starts_with("ledger-summary:")
            })
            .unwrap();
        let provenance = &summary["provenance"];
        assert_eq!(provenance["exploratory"], true);
        assert_eq!(provenance["opportunityId"], serde_json::Value::Null);
        assert_eq!(provenance["tradeId"], serde_json::Value::Null);
        assert_eq!(provenance["executionMode"], serde_json::Value::Null);
        assert_eq!(
            provenance["workerEventIdentityVersion"],
            serde_json::Value::Null
        );
        assert_eq!(provenance["workerEventId"], serde_json::Value::Null);
        assert_eq!(provenance["workerEpisodeId"], serde_json::Value::Null);
    }

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
        assert_eq!(
            external_close_order_for_trade(&trade, &orders, &[7.0]),
            Some(0)
        );
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
        assert_eq!(
            external_close_order_for_trade(&manual_trade, &orders, &[7.0]),
            None
        );
        assert_eq!(
            external_close_order_for_trade(&wrong_side, &orders, &[7.0]),
            None
        );
    }
}

fn snapshot_activity_positions(settings: &Settings, today_start: u64) -> ActivityPositionSnapshot {
    let today = utc_day();
    let eligible_closed_trade_returns = closed_trade_learning_returns(settings);
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
    let mut strategy_positions = settings
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
                        let (candidate, _) =
                            position_take_profit_candidate(position, settings.take_profit_percent);
                        let learned_exit_policy = position_uses_learned_exit_policy(position);
                        let effective_candidate = if learned_exit_policy {
                            candidate
                        } else {
                            confidence_adjusted_take_profit_percent(
                                &eligible_closed_trade_returns,
                                candidate,
                                effective_stop_loss,
                            )
                        };
                        Some(if learned_exit_policy {
                            effective_candidate
                        } else {
                            age_adjusted_take_profit_percent(
                                Some(strategy.as_str()),
                                position.entry_timestamp_ms,
                                now_ms,
                                effective_candidate,
                            )
                        })
                    } else {
                        None
                    };
                    OpenStrategyPosition {
                        symbol: symbol.clone(),
                        strategy: strategy.clone(),
                        decision_id: position.entry_decision_id.clone(),
                        lifecycle: None,
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
                        effective_stop_loss_percent: stop_loss_active.then_some(effective_stop_loss),
                        effective_take_profit_percent: effective_take_profit,
                        selected_stop_loss_percent: provenance_selected_percent(position, "stop_loss"),
                        selected_take_profit_percent: provenance_selected_percent(position, "take_profit"),
                        stop_loss_source: selected_exit_parameter_source(
                            position,
                            "stop_loss",
                        ),
                        take_profit_source: selected_exit_parameter_source(
                            position,
                            "take_profit",
                        ),
                        fallback_stop_loss_percent: Some(settings.stop_loss_percent),
                        fallback_take_profit_percent: Some(settings.take_profit_percent),
                        exit_profile_id: position
                            .exit_parameter_provenance
                            .as_ref()
                            .and_then(|provenance| provenance.get("profile_id"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| {
                                position
                                    .lifecycle_identity
                                    .as_ref()
                                    .and_then(|identity| identity.profile_id.clone())
                            }),
                        exit_profile_version: position
                            .exit_parameter_provenance
                            .as_ref()
                            .and_then(|provenance| provenance.get("profile_version"))
                            .and_then(serde_json::Value::as_u64)
                            .or_else(|| {
                                position
                                    .lifecycle_identity
                                    .as_ref()
                                    .and_then(|identity| identity.profile_version)
                            }),
                        exit_policy_as_of_ms: position
                            .exit_parameter_provenance
                            .as_ref()
                            .and_then(|provenance| provenance.get("exit_policy_as_of_ms"))
                            .and_then(serde_json::Value::as_u64),
                        exit_parameter_provenance: position.exit_parameter_provenance.clone(),
                        protective_barrier: None,
                        protective_barrier_ids: position.protective_stops.iter().map(|stop| stop.barrier_id.clone()).collect(),
                        initial_risk_reference: position.initial_risk_reference.clone(),
                        initial_risk_evidence_status: initial_risk_evidence_status(position, symbol).into(),
                        horizon_take_profit_percent: position.horizon_take_profit_percent,
                        stop_loss_active,
                        take_profit_active,
                        exit_control_mode: match (stop_loss_active, take_profit_active) {
                            (false, false) => "signal_managed",
                            (true, false) => "protective_stop_only",
                            (false, true) => "fixed_target_only",
                            (true, true) => "protective_stop_and_target",
                        }
                        .into(),
                        carry_details: None,
                    }
                })
        })
        .collect::<Vec<_>>();
    let lifecycle_links = settings
        .canonical_lifecycle_links
        .iter()
        .filter_map(|(key, identity)| {
            serde_json::to_value(identity)
                .ok()
                .map(|value| (key.clone(), value))
        })
        .collect::<BTreeMap<_, _>>();
    for position in &mut strategy_positions {
        position.lifecycle = position
            .decision_id
            .as_ref()
            .and_then(|decision_id| lifecycle_links.get(decision_id).cloned());
        if position.exit_profile_id.is_none() {
            position.exit_profile_id = position
                .lifecycle
                .as_ref()
                .and_then(|identity| identity["profile_id"].as_str())
                .map(str::to_owned);
        }
        if position.exit_profile_version.is_none() {
            position.exit_profile_version = position
                .lifecycle
                .as_ref()
                .and_then(|identity| identity["profile_version"].as_u64());
        }
    }
    let open_position_symbols = auto_positions
        .iter()
        .map(|(symbol, _)| symbol.clone())
        .collect::<Vec<_>>();
    let position_exit_policies = auto_positions
        .iter()
        .map(|(symbol, position)| {
            let (take_profit_candidate, _) =
                position_take_profit_candidate(position, settings.take_profit_percent);
            let stop_loss_percent = position
                .stop_loss_percent
                .unwrap_or(settings.stop_loss_percent);
            let take_profit_percent = confidence_adjusted_take_profit_percent(
                &eligible_closed_trade_returns,
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
                source: "global_fallback".into(),
                is_global_fallback: true,
            }
        })
        .collect();
    ActivityPositionSnapshot {
        orders,
        lifecycle_links,
        auto_positions,
        strategy_positions,
        open_position_symbols,
        position_exit_policies,
    }
}

fn activity_realized_pnl(
    orders: &mut [TraderOrder],
    ledger_pnl: Option<&LedgerRealizedPnl>,
) -> (
    Option<f64>,
    Option<BTreeMap<String, f64>>,
    Vec<PnlBreakdown>,
) {
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
            Some(pnl.realized_pnl_by_quote.clone()),
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
