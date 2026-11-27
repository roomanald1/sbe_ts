#[derive(Clone, Copy, Debug, PartialEq)]
struct ProfileProtectionTarget {
    net_return_percent: f64,
    reason: &'static str,
}

#[derive(Clone, Debug)]
struct ProfileTriggerOverride {
    trigger_price: f64,
    target_net_return_percent: f64,
    reason: &'static str,
    policy_source: String,
    profile_id: String,
    profile_version: u64,
}

fn profile_protective_barrier_target_return_percent(
    position: &TraderPosition,
) -> Option<ProfileProtectionTarget> {
    let stop_percent = position
        .stop_loss_percent
        .filter(|value| value.is_finite() && *value > 0.0)?;
    let mut target = -stop_percent;
    let mut reason = "initial-stop-floor";
    let trailing_activation = profile_exit_parameter(position, "trailing_activation_percent")
        .filter(|value| *value > 0.0);
    let trailing_distance =
        profile_exit_parameter(position, "trailing_distance_percent").filter(|value| *value > 0.0);
    if let (Some(activation), Some(distance), Some(peak)) = (
        trailing_activation,
        trailing_distance,
        position
            .trailing_peak_return_percent
            .filter(|value| value.is_finite()),
    ) {
        if peak >= activation && peak - distance > target {
            target = peak - distance;
            reason = "trailing-stop";
        }
    }
    if position.breakeven_protection_activated && target < 0.0 {
        target = 0.0;
        reason = "breakeven-protection";
    }
    (target > -stop_percent + 1e-9).then_some(ProfileProtectionTarget {
        net_return_percent: target,
        reason,
    })
}

fn protective_barrier_trigger_for_net_return(
    position: &TraderPosition,
    net_return_percent: f64,
) -> Option<f64> {
    let quantity = position.quantity.abs();
    let entry_price = position.average_entry_price;
    let entry_fees = position.entry_fees;
    if !quantity.is_finite()
        || quantity <= 0.0
        || !entry_price.is_finite()
        || entry_price <= 0.0
        || !entry_fees.is_finite()
        || entry_fees < 0.0
        || !net_return_percent.is_finite()
    {
        return None;
    }
    let entry_notional = quantity * entry_price;
    let target_fraction = net_return_percent / 100.0;
    let fee_rate = configured_taker_fee_rate();
    let trigger = if position.quantity > 0.0 {
        (target_fraction * entry_notional + entry_notional + entry_fees)
            / (quantity * (1.0 - fee_rate))
    } else {
        (entry_notional - entry_fees - target_fraction * entry_notional)
            / (quantity * (1.0 + fee_rate))
    };
    (trigger.is_finite() && trigger > 0.0).then_some(trigger)
}

fn profile_trigger_override(
    position: &TraderPosition,
    target: ProfileProtectionTarget,
) -> Option<ProfileTriggerOverride> {
    let trigger_price =
        protective_barrier_trigger_for_net_return(position, target.net_return_percent)?;
    let provenance = position.exit_parameter_provenance.as_ref()?;
    let profile_id = provenance["profile_id"].as_str()?.to_owned();
    let profile_version = provenance["profile_version"].as_u64()?;
    let source_key = if target.reason == "trailing-stop" {
        "trailing_protection"
    } else {
        "breakeven_protection"
    };
    let policy_source = provenance[source_key]["source"].as_str()?.to_owned();
    Some(ProfileTriggerOverride {
        trigger_price,
        target_net_return_percent: target.net_return_percent,
        reason: target.reason,
        policy_source,
        profile_id,
        profile_version,
    })
}

const MAX_RETAINED_PROTECTION_ADJUSTMENTS: usize = 64;

fn append_protection_adjustment(
    provenance: &mut serde_json::Value,
    adjustment: serde_json::Value,
) -> bool {
    let Some(object) = provenance.as_object_mut() else {
        return false;
    };
    let request_id = adjustment["request_id"].as_str();
    if request_id.is_some_and(|request_id| {
        object
            .get("protection_adjustments")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|existing| existing["request_id"].as_str() == Some(request_id))
    }) {
        return false;
    }
    let too_many = object
        .get("protection_adjustments")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|adjustments| adjustments.len() >= MAX_RETAINED_PROTECTION_ADJUSTMENTS);
    if too_many {
        if let Some(adjustments) = object
            .get_mut("protection_adjustments")
            .and_then(serde_json::Value::as_array_mut)
        {
            adjustments.remove(0);
        }
        let dropped = object
            .get("protection_adjustments_dropped")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
            .saturating_add(1);
        object.insert(
            "protection_adjustments_dropped".into(),
            serde_json::json!(dropped),
        );
    }
    let Some(adjustments) = object
        .entry("protection_adjustments")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
    else {
        return false;
    };
    adjustments.push(adjustment);
    true
}

async fn reconcile_protection_adjustments_from_barrier(
    state: &AutoTraderState,
    symbol: &str,
    strategy: &str,
    position: &TraderPosition,
    barrier: &serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    let (Some(barrier_id), Some(lifecycle), Some(position_lifecycle)) = (
        barrier["id"].as_str(),
        barrier.get("lifecycle"),
        position.lifecycle_identity.as_ref(),
    ) else {
        return Ok(());
    };
    if barrier["source"].as_str() != Some("auto")
        || barrier["parent_order_id"]
            .as_str()
            .is_none_or(|parent| !parent.starts_with("auto-stop-"))
        || barrier["symbol"].as_str() != Some(symbol)
        || lifecycle["trade_id"].as_str() != Some(position_lifecycle.trade_id.as_str())
        || lifecycle["profile_id"].as_str() != position_lifecycle.profile_id.as_deref()
        || lifecycle["profile_version"].as_u64() != position_lifecycle.profile_version
    {
        return Ok(());
    }
    let Some(rows) = barrier["trigger_adjustments"].as_array() else {
        return Ok(());
    };
    let provenance_identity_matches =
        position
            .exit_parameter_provenance
            .as_ref()
            .is_some_and(|provenance| {
                provenance["profile_id"].as_str() == position_lifecycle.profile_id.as_deref()
                    && provenance["profile_version"].as_u64() == position_lifecycle.profile_version
            });
    if !provenance_identity_matches {
        return Ok(());
    }
    let matching = rows
        .iter()
        .filter(|row| {
            row["barrier_id"].as_str() == Some(barrier_id)
                && row["symbol"].as_str() == Some(symbol)
                && row["lifecycle_trade_id"].as_str() == Some(position_lifecycle.trade_id.as_str())
                && row["profile_id"].as_str() == position_lifecycle.profile_id.as_deref()
                && row["profile_version"].as_u64() == position_lifecycle.profile_version
                && row["request_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("autotrader-protection-v1:"))
                && row["reason"].as_str().is_some_and(|reason| {
                    matches!(reason, "trailing-stop" | "breakeven-protection")
                })
                && row["expected_trigger_price"]
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value > 0.0)
                && row["new_trigger_price"]
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value > 0.0)
                && row["adjusted_at_ms"]
                    .as_u64()
                    .is_some_and(|timestamp| timestamp > 0)
        })
        .cloned()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok(());
    }
    let current_trigger = barrier["trigger_price"].as_f64();
    let mut changed = false;
    {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings
            .strategy_positions
            .get_mut(symbol)
            .and_then(|lots| lots.get_mut(strategy))
            .filter(|current| {
                (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            })
        {
            let Some(provenance) = current.exit_parameter_provenance.as_mut() else {
                return Ok(());
            };
            for row in matching {
                changed |= append_protection_adjustment(
                    provenance,
                    serde_json::json!({
                        "schema_version": 1,
                        "request_id": row["request_id"],
                        "barrier_id": barrier_id,
                        "reason": row["reason"],
                        "policy_source": row["policy_source"],
                        "profile_id": row["profile_id"],
                        "profile_version": row["profile_version"],
                        "target_net_return_percent": row["target_net_return_percent"],
                        "previous_trigger_price": row["expected_trigger_price"],
                        "trigger_price": row["new_trigger_price"],
                        "updated_at_ms": row["adjusted_at_ms"],
                        "recovered_from_durable_pm_barrier": true,
                    }),
                );
            }
        }
        if let (Some(current_trigger), Some(order)) = (
            current_trigger,
            settings.orders.iter_mut().find(|order| {
                order.symbol == symbol
                    && order.strategy.as_deref() == Some(strategy)
                    && order.execution_order_id.as_deref() == Some(barrier_id)
            }),
        ) {
            if order.target_price != Some(current_trigger) {
                order.target_price = Some(current_trigger);
                changed = true;
            }
        }
    }
    if changed {
        persist_settings_snapshot_durable(state).await?;
    }
    Ok(())
}

async fn adjust_protective_barrier_trigger(
    state: &AutoTraderState,
    symbol: &str,
    strategy: &str,
    position: &TraderPosition,
    barrier: &serde_json::Value,
    target: ProfileProtectionTarget,
) -> Result<(), (StatusCode, String)> {
    if barrier["status"].as_str() != Some("armed") {
        return Ok(());
    }
    let (Some(barrier_id), Some(current_trigger), Some(side), Some(quantity)) = (
        barrier["id"].as_str(),
        barrier["trigger_price"].as_f64(),
        barrier["side"].as_str(),
        barrier["quantity"].as_f64(),
    ) else {
        return Ok(());
    };
    let Some(next_trigger) =
        protective_barrier_trigger_for_net_return(position, target.net_return_percent)
    else {
        return Ok(());
    };
    let protective_move = match (side, barrier["trigger_direction"].as_str()) {
        ("sell", Some("below")) => next_trigger > current_trigger + 1e-12,
        ("buy", Some("above")) => next_trigger < current_trigger - 1e-12,
        _ => false,
    };
    if !protective_move {
        return Ok(());
    }
    let Some(parent_order_id) = barrier["parent_order_id"].as_str() else {
        return Ok(());
    };
    let Some(profile_id) = barrier["lifecycle"]["profile_id"].as_str() else {
        return Ok(());
    };
    let Some(profile_version) = barrier["lifecycle"]["profile_version"].as_u64() else {
        return Ok(());
    };
    let provenance_profile_id = position
        .exit_parameter_provenance
        .as_ref()
        .and_then(|value| value["profile_id"].as_str());
    let provenance_profile_version = position
        .exit_parameter_provenance
        .as_ref()
        .and_then(|value| value["profile_version"].as_u64());
    if provenance_profile_id != Some(profile_id)
        || provenance_profile_version != Some(profile_version)
    {
        return Ok(());
    }
    let policy_source = position
        .exit_parameter_provenance
        .as_ref()
        .and_then(|value| {
            if target.reason == "trailing-stop" {
                value["trailing_protection"]["source"].as_str()
            } else {
                value["breakeven_protection"]["source"].as_str()
            }
        })
        .filter(|value| !value.trim().is_empty());
    let Some(policy_source) = policy_source else {
        return Ok(());
    };
    let lifecycle_trade_id = barrier["lifecycle"]["trade_id"].as_str();
    let request_id = format!(
        "autotrader-protection-v1:{barrier_id}:{:016x}:{:016x}",
        current_trigger.to_bits(),
        next_trigger.to_bits()
    );
    let request = serde_json::json!({
        "request_id": request_id.clone(),
        "symbol": symbol,
        "side": side,
        "quantity": quantity,
        "expected_parent_order_id": parent_order_id,
        "expected_lifecycle_trade_id": lifecycle_trade_id,
        "profile_id": profile_id,
        "profile_version": profile_version,
        "policy_source": policy_source,
        "reason": target.reason,
        "target_net_return_percent": target.net_return_percent,
        "expected_trigger_price": current_trigger,
        "new_trigger_price": next_trigger,
    });
    let response = crate::service_rpc::position_request(
        "POST",
        &format!("/positions/orders/{barrier_id}/trigger"),
        Some(request),
    )
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "Protective barrier trigger update rejected ({status}): {}",
                response.body
            ),
        ));
    }
    let applied_trigger = response.body["trigger_price"]
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(next_trigger);
    let adjustment = serde_json::json!({
        "schema_version": 1,
        "request_id": request_id,
        "barrier_id": barrier_id,
        "reason": target.reason,
        "target_net_return_percent": target.net_return_percent,
        "previous_trigger_price": current_trigger,
        "trigger_price": applied_trigger,
        "profile_id": position.exit_parameter_provenance.as_ref()
            .and_then(|value| value["profile_id"].as_str()),
        "profile_version": position.exit_parameter_provenance.as_ref()
            .and_then(|value| value["profile_version"].as_u64()),
        "policy_source": policy_source,
        "updated_at_ms": response.body["adjusted_at_ms"].as_u64().unwrap_or_else(current_time_ms),
    });
    {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings
            .strategy_positions
            .get_mut(symbol)
            .and_then(|lots| lots.get_mut(strategy))
            .filter(|current| {
                (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            })
        {
            if let Some(provenance) = current.exit_parameter_provenance.as_mut() {
                append_protection_adjustment(provenance, adjustment);
            }
        }
        if let Some(order) = settings.orders.iter_mut().find(|order| {
            order.symbol == symbol
                && order.strategy.as_deref() == Some(strategy)
                && order.execution_order_id.as_deref() == Some(barrier_id)
        }) {
            order.target_price = Some(applied_trigger);
        }
    }
    persist_settings_snapshot_durable(state).await?;
    Ok(())
}

async fn ensure_protective_stops(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let barriers: Vec<serde_json::Value> = serde_json::from_value(
        crate::service_rpc::position_get("/positions/barriers")
            .await
            .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?,
    )
    .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    reconcile_local_stop_order_statuses(state, &barriers).await?;
    let (lots, stop_strategies) = {
        let settings = state.settings.lock().await;
        let lots = if settings.strategy_positions.is_empty() {
            settings
                .positions
                .iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| {
                    (
                        symbol.clone(),
                        position
                            .strategy
                            .clone()
                            .unwrap_or_else(|| "unattributed".into()),
                        position.clone(),
                    )
                })
                .collect::<Vec<_>>()
        } else {
            settings
                .strategy_positions
                .iter()
                .flat_map(|(symbol, strategies)| {
                    strategies
                        .iter()
                        .filter(|(_, position)| position.quantity.abs() > 1e-12)
                        .map(|(strategy, position)| {
                            (symbol.clone(), strategy.clone(), position.clone())
                        })
                })
                .collect::<Vec<_>>()
        };
        let stop_strategies = if settings.strategy_positions.is_empty() {
            settings
                .positions
                .iter()
                .flat_map(|(symbol, position)| {
                    position.protective_stops.iter().map(move |stop| {
                        (
                            (symbol.clone(), stop.barrier_id.clone()),
                            position
                                .strategy
                                .clone()
                                .unwrap_or_else(|| "unattributed".into()),
                        )
                    })
                })
                .collect::<BTreeMap<_, _>>()
        } else {
            settings
                .strategy_positions
                .iter()
                .flat_map(|(symbol, strategies)| {
                    strategies
                        .iter()
                        .filter(|(_, position)| position.quantity.abs() > 1e-12)
                        .flat_map(move |(strategy, position)| {
                            position.protective_stops.iter().map(move |stop| {
                                ((symbol.clone(), stop.barrier_id.clone()), strategy.clone())
                            })
                        })
                })
                .collect::<BTreeMap<_, _>>()
        };
        (lots, stop_strategies)
    };

    // Position Manager survives AutoTrader state resets. Barriers from old
    // strategy lots must be removed before they can act on a later position
    // in the same symbol. A triggering barrier is left alone and blocks new
    // coverage for that symbol until its fill can be reconciled.
    let mut blocked_symbols = BTreeSet::new();
    for barrier in barriers.iter().filter(|barrier| {
        barrier["source"].as_str() == Some("auto")
            && barrier["parent_order_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("auto-stop-"))
            && matches!(barrier["status"].as_str(), Some("armed" | "triggering"))
    }) {
        let symbol = barrier["symbol"].as_str().unwrap_or_default();
        let barrier_id = barrier["id"].as_str().unwrap_or_default();
        if stop_strategies.contains_key(&(symbol.to_owned(), barrier_id.to_owned())) {
            continue;
        }
        if barrier["status"].as_str() == Some("triggering") {
            blocked_symbols.insert(symbol.to_owned());
            continue;
        }
        let created_at = barrier["created_at"].as_u64().unwrap_or_default();
        if !cancel_protective_barrier(state, symbol, barrier_id, created_at, None).await? {
            blocked_symbols.insert(symbol.to_owned());
            let mut settings = state.settings.lock().await;
            settings.last_risk_note = format!(
                "Orphan auto-stop {barrier_id} for {symbol} could not be canceled; stop coverage is deferred"
            );
        }
    }
    for (symbol, strategy, position) in lots {
        if blocked_symbols.contains(&symbol) {
            continue;
        }
        for stop in &position.protective_stops {
            if let Some(barrier) = barriers
                .iter()
                .find(|barrier| barrier["id"].as_str() == Some(stop.barrier_id.as_str()))
            {
                if let Err((_, error)) = reconcile_protection_adjustments_from_barrier(
                    state, &symbol, &strategy, &position, barrier,
                )
                .await
                {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!(
                        "Could not persist recovered protection history for {symbol} ({strategy}): {error}"
                    );
                }
            }
        }
        let uses_protective_stops = Strategy::from_name(&strategy)
            .map(Strategy::uses_protective_stops)
            .unwrap_or(true);
        if !uses_protective_stops {
            for stop in &position.protective_stops {
                if !cancel_protective_barrier(
                    state,
                    &symbol,
                    &stop.barrier_id,
                    stop.order_timestamp,
                    Some(&strategy),
                )
                .await?
                {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!(
                        "Could not cancel obsolete stop for {symbol} ({strategy}); retrying"
                    );
                }
            }
            continue;
        }
        let desired_quantity = position.quantity.abs();
        let active = barriers
            .iter()
            .filter(|barrier| {
                if barrier["symbol"].as_str() != Some(symbol.as_str())
                    || barrier["source"].as_str() != Some("auto")
                    || !barrier["parent_order_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("auto-stop-"))
                    || !matches!(barrier["status"].as_str(), Some("armed" | "triggering"))
                {
                    return false;
                }
                let barrier_id = barrier["id"].as_str().unwrap_or_default();
                stop_strategies
                    .get(&(symbol.clone(), barrier_id.to_owned()))
                    .is_some_and(|owner| owner == &strategy)
            })
            .collect::<Vec<_>>();
        if active
            .iter()
            .any(|barrier| barrier["status"].as_str() == Some("triggering"))
        {
            continue;
        }
        let protection_target = profile_protective_barrier_target_return_percent(&position);
        let trigger_override =
            protection_target.and_then(|target| profile_trigger_override(&position, target));
        if let Some(target) = protection_target {
            for barrier in &active {
                if let Err((_, error)) = adjust_protective_barrier_trigger(
                    state, &symbol, &strategy, &position, barrier, target,
                )
                .await
                {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!(
                        "Profile protective threshold update failed for {symbol} ({strategy}); existing stop remains armed: {error}"
                    );
                }
            }
        }
        let expected_side = if position.quantity > 0.0 {
            "sell"
        } else {
            "buy"
        };
        let covered_quantity = active
            .iter()
            .filter(|barrier| barrier["side"].as_str() == Some(expected_side))
            .map(|barrier| {
                barrier["remaining_quantity"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .max(0.0)
            })
            .sum::<f64>();
        let wrong_side = active
            .iter()
            .any(|barrier| barrier["side"].as_str() != Some(expected_side));
        let tolerance = (desired_quantity * 1e-6).max(1e-12);
        let coverage_mismatch =
            wrong_side || (covered_quantity - desired_quantity).abs() > tolerance;
        if !wrong_side && desired_quantity > covered_quantity + tolerance && !active.is_empty() {
            let uncovered_quantity = desired_quantity - covered_quantity;
            let uncovered_fees = position.entry_fees * (uncovered_quantity / desired_quantity);
            if let Err((_, error)) = arm_protective_stop(
                state,
                &symbol,
                &strategy,
                if position.quantity > 0.0 {
                    "buy"
                } else {
                    "sell"
                },
                uncovered_quantity,
                position.average_entry_price,
                uncovered_fees,
                position.stop_loss_percent,
                trigger_override.clone(),
            )
            .await
            {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note =
                    format!("Additional protective stop failed for {symbol} ({strategy}): {error}");
            }
            continue;
        }
        if coverage_mismatch && !active.is_empty() {
            let mut cancelled_all = true;
            for barrier in &active {
                let Some(id) = barrier["id"].as_str() else {
                    cancelled_all = false;
                    break;
                };
                let created_at = barrier["created_at"].as_u64().unwrap_or_default();
                if !cancel_protective_barrier(state, &symbol, id, created_at, Some(&strategy))
                    .await?
                {
                    cancelled_all = false;
                    break;
                }
            }
            if !cancelled_all {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!(
                    "Could not replace stop protection for {symbol} ({strategy}); no replacement added"
                );
                continue;
            }
        }
        if desired_quantity <= 1e-12 || (!active.is_empty() && !coverage_mismatch) {
            continue;
        }
        if let Err((_, error)) = arm_protective_stop(
            state,
            &symbol,
            &strategy,
            if position.quantity > 0.0 {
                "buy"
            } else {
                "sell"
            },
            desired_quantity,
            position.average_entry_price,
            position.entry_fees,
            position.stop_loss_percent,
            trigger_override.clone(),
        )
        .await
        {
            let mut settings = state.settings.lock().await;
            settings.last_risk_note = format!(
                "Protective stop failed for {symbol} ({strategy}): {error}; exit monitor remains active"
            );
        }
    }
    Ok(())
}

async fn reconcile_local_stop_order_statuses(
    state: &AutoTraderState,
    barriers: &[serde_json::Value],
) -> Result<(), (StatusCode, String)> {
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    for order in settings
        .orders
        .iter_mut()
        .filter(|order| order.purpose == "stop-loss")
    {
        let barrier = barriers.iter().find(|barrier| {
            barrier["symbol"].as_str() == Some(order.symbol.as_str())
                && barrier["created_at"].as_u64() == Some(order.timestamp)
                && barrier["source"].as_str() == Some("auto")
                && barrier["parent_order_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("auto-stop-"))
        });
        let next_status = if let Some(barrier) = barrier {
            let status = barrier["status"].as_str().unwrap_or("cancelled");
            if status == "partially_filled" {
                "partially_filled_closed"
            } else {
                status
            }
        } else if matches!(
            order.status.as_str(),
            "armed" | "triggering" | "partially_filled"
        ) {
            "cancelled"
        } else {
            continue;
        };
        if order.status != next_status {
            order.status = next_status.to_owned();
            changed = true;
        }
        if let Some(barrier) = barrier {
            let filled = barrier["filled_quantity"]
                .as_f64()
                .unwrap_or(order.filled_quantity);
            let average_price = barrier["average_execution_price"]
                .as_f64()
                .or(order.average_price);
            let fee = barrier["fee_paid"]
                .as_f64()
                .unwrap_or(order.fees_paid)
                .abs();
            if (order.filled_quantity - filled).abs() > 1e-12
                || order.average_price != average_price
                || (order.fees_paid - fee).abs() > 1e-12
                || order.triggered_at != barrier["triggered_at"].as_u64()
            {
                order.filled_quantity = filled;
                order.fill_rate = if order.quantity > 0.0 {
                    (filled / order.quantity).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                order.average_price = average_price;
                order.fees_paid = fee;
                order.triggered_at = barrier["triggered_at"].as_u64();
                changed = true;
            }
        }
    }
    if changed {
        request_settings_persist(state)?;
    }
    Ok(())
}

#[cfg(test)]
mod profile_protective_barrier_adjustment_tests {
    use super::*;

    fn position(quantity: f64, strategy: &str) -> TraderPosition {
        TraderPosition {
            quantity,
            average_entry_price: 100.0,
            entry_fees: 1.0,
            stop_loss_percent: Some(0.5),
            strategy: Some(strategy.to_owned()),
            ..TraderPosition::default()
        }
    }

    #[test]
    fn selected_trailing_and_breakeven_raise_the_live_stop_floor_monotonically() {
        let mut position = position(1.0, "live-momentum");
        position.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": {
                "trailing_activation_percent":0.30,
                "trailing_distance_percent":0.10,
                "breakeven_activation_percent":0.25
            }
        }));
        position.trailing_peak_return_percent = Some(0.35);
        let trailing_target = profile_protective_barrier_target_return_percent(&position).unwrap();
        assert!((trailing_target.net_return_percent - 0.25).abs() < 1e-12);
        assert_eq!(trailing_target.reason, "trailing-stop");
        position.breakeven_protection_activated = true;
        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["trailing_distance_percent"] =
            serde_json::json!(0.50);
        let breakeven_target = profile_protective_barrier_target_return_percent(&position).unwrap();
        assert!((breakeven_target.net_return_percent - 0.0).abs() < 1e-12);
        assert_eq!(breakeven_target.reason, "breakeven-protection");
        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["trailing_distance_percent"] =
            serde_json::json!(0.10);
        position.trailing_peak_return_percent = Some(0.80);
        let raised_target = profile_protective_barrier_target_return_percent(&position).unwrap();
        assert!((raised_target.net_return_percent - 0.70).abs() < 1e-12);
        assert_eq!(raised_target.reason, "trailing-stop");
    }

    #[test]
    fn trigger_conversion_matches_net_fee_aware_long_and_short_targets() {
        let long = position(1.0, "live-momentum");
        let short = position(-1.0, "live-momentum");
        for (position, target) in [(&long, 0.0), (&short, 0.0), (&long, -0.5), (&short, -0.5)] {
            let trigger = protective_barrier_trigger_for_net_return(position, target).unwrap();
            let observed =
                net_exit_return_fraction(position, trigger, configured_taker_fee_rate()) * 100.0;
            assert!((observed - target).abs() < 1e-9, "{observed} != {target}");
            if position.quantity > 0.0 {
                assert!(trigger > 0.0);
            } else {
                assert!(trigger > 0.0);
            }
        }
    }

    #[test]
    fn bootstrap_or_invalid_trailing_values_do_not_move_the_existing_barrier() {
        let mut position = position(1.0, "live-momentum");
        position.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": {
                "trailing_activation_percent":0.0,
                "trailing_distance_percent":0.0,
                "breakeven_activation_percent":0.0
            }
        }));
        position.trailing_peak_return_percent = Some(1.0);
        assert!(profile_protective_barrier_target_return_percent(&position).is_none());
        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["trailing_distance_percent"] =
            serde_json::json!(100.0);
        assert!(profile_protective_barrier_target_return_percent(&position).is_none());
    }
}
