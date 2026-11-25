#[derive(Default)]
struct ProtectiveStopFillEvidence {
    complete: bool,
    reasons: Vec<String>,
    fills: Vec<serde_json::Value>,
    unprocessed_fills: Vec<serde_json::Value>,
    fill_ids: Vec<String>,
}

fn stable_pm_fill_id(value: &str) -> bool {
    value
        .strip_prefix("pm-fill:v1:")
        .and_then(|identity| identity.split_once(':'))
        .is_some_and(|(epoch, sequence)| {
            !epoch.trim().is_empty() && sequence.parse::<u64>().is_ok()
        })
}

fn protective_stop_fill_evidence(
    barrier: &serde_json::Value,
    trade_records: &[serde_json::Value],
    stop: &ProtectiveStop,
    processed_fill_ids: &[String],
) -> ProtectiveStopFillEvidence {
    let mut evidence = ProtectiveStopFillEvidence::default();
    let barrier_order_id = barrier["id"]
        .as_str()
        .filter(|value| !value.trim().is_empty());
    let client_order_id = barrier["client_order_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty());
    let parent_order_id = barrier["parent_order_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty());
    let Some(order_id) = barrier_order_id else {
        evidence
            .reasons
            .push("protective_barrier_id_missing".into());
        return evidence;
    };
    if order_id != stop.barrier_id {
        evidence
            .reasons
            .push("protective_barrier_id_mismatch".into());
    }
    if client_order_id.is_none()
        || parent_order_id != client_order_id
        || !client_order_id.is_some_and(|id| id.starts_with("auto-stop-"))
    {
        evidence
            .reasons
            .push("protective_barrier_order_identity_missing_or_mismatched".into());
    }

    let matching_rows = trade_records
        .iter()
        .filter(|trade| {
            trade["parent_order_id"].as_str() == Some(order_id)
                || trade["client_order_id"].as_str() == Some(order_id)
        })
        .collect::<Vec<_>>();
    let expected_quantity = barrier["filled_quantity"].as_f64().unwrap_or(f64::NAN);
    if expected_quantity > 1e-12 && matching_rows.is_empty() {
        evidence
            .reasons
            .push("protective_barrier_fill_rows_missing".into());
    }

    let mut fill_ids = std::collections::BTreeSet::new();
    let mut row_ids = std::collections::BTreeSet::new();
    let mut quantity = 0.0;
    let mut notional = 0.0;
    let mut fees = 0.0;
    for trade in matching_rows {
        let fill_id = trade["fill_id"]
            .as_str()
            .filter(|value| stable_pm_fill_id(value));
        let row_id = trade["id"]
            .as_str()
            .filter(|value| !value.trim().is_empty());
        if trade["source"].as_str() != Some("auto")
            || trade["symbol"].as_str() != barrier["symbol"].as_str()
            || trade["side"].as_str() != barrier["side"].as_str()
            || trade["parent_order_id"].as_str() != Some(order_id)
            || trade["client_order_id"].as_str() != Some(order_id)
        {
            evidence
                .reasons
                .push("protective_barrier_fill_order_or_market_identity_mismatch".into());
        }
        let Some(fill_id) = fill_id else {
            evidence
                .reasons
                .push("protective_barrier_fill_id_missing_or_unsupported".into());
            continue;
        };
        if !fill_ids.insert(fill_id.to_owned()) {
            evidence
                .reasons
                .push("protective_barrier_fill_id_duplicate".into());
        }
        let Some(row_id) = row_id else {
            evidence
                .reasons
                .push("protective_barrier_trade_row_id_missing".into());
            continue;
        };
        if !row_ids.insert(row_id.to_owned()) {
            evidence
                .reasons
                .push("protective_barrier_trade_row_id_duplicate".into());
        }
        let fill_quantity = trade["quantity"].as_f64().unwrap_or(f64::NAN);
        let execution_price = trade["execution_price"].as_f64().unwrap_or(f64::NAN);
        let fee = trade["fee"].as_f64().unwrap_or(f64::NAN);
        let fee_currency = trade["fee_currency"]
            .as_str()
            .filter(|value| !value.trim().is_empty());
        if !fill_quantity.is_finite()
            || fill_quantity <= 0.0
            || !execution_price.is_finite()
            || execution_price <= 0.0
        {
            evidence
                .reasons
                .push("protective_barrier_fill_quantity_or_price_invalid".into());
        }
        if trade["fee_known"].as_bool() != Some(true) || !fee.is_finite() || fee_currency.is_none()
        {
            evidence
                .reasons
                .push("protective_barrier_fill_fee_incomplete".into());
        }
        quantity += fill_quantity;
        notional += fill_quantity * execution_price;
        fees += fee.abs();
        evidence.fill_ids.push(fill_id.to_owned());
        evidence.fills.push((*trade).clone());
    }
    evidence.fill_ids.sort();
    let total_notional = barrier["average_execution_price"]
        .as_f64()
        .zip(barrier["filled_quantity"].as_f64())
        .map(|(average, quantity)| average * quantity)
        .unwrap_or(f64::NAN);
    let total_fee = barrier["fee_paid"].as_f64().unwrap_or(f64::NAN).abs();
    let approximately_equal = |left: f64, right: f64| {
        left.is_finite()
            && right.is_finite()
            && (left - right).abs() <= 1e-9 * left.abs().max(right.abs()).max(1.0)
    };
    if !approximately_equal(quantity, expected_quantity)
        || !approximately_equal(notional, total_notional)
        || !approximately_equal(fees, total_fee)
    {
        evidence
            .reasons
            .push("protective_barrier_aggregate_does_not_match_pm_fills".into());
    }

    let processed_set = processed_fill_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if processed_set.len() != processed_fill_ids.len() {
        evidence
            .reasons
            .push("protective_barrier_processed_fill_id_duplicate".into());
    }
    let processed_rows = evidence
        .fills
        .iter()
        .filter(|trade| {
            trade["fill_id"]
                .as_str()
                .is_some_and(|id| processed_set.contains(id))
        })
        .collect::<Vec<_>>();
    if processed_rows.len() != processed_fill_ids.len() {
        evidence
            .reasons
            .push("protective_barrier_processed_fill_id_not_in_pm_ledger".into());
    }
    let processed_quantity = processed_rows
        .iter()
        .map(|trade| trade["quantity"].as_f64().unwrap_or_default())
        .sum::<f64>();
    let processed_notional = processed_rows
        .iter()
        .map(|trade| {
            trade["quantity"].as_f64().unwrap_or_default()
                * trade["execution_price"].as_f64().unwrap_or_default()
        })
        .sum::<f64>();
    let processed_fee = processed_rows
        .iter()
        .map(|trade| trade["fee"].as_f64().unwrap_or_default().abs())
        .sum::<f64>();
    if !approximately_equal(processed_quantity, stop.processed_quantity)
        || !approximately_equal(processed_notional, stop.processed_notional)
        || !approximately_equal(processed_fee, stop.processed_fee)
    {
        evidence
            .reasons
            .push("protective_barrier_processed_progress_mismatch".into());
    }

    evidence.complete = evidence.reasons.is_empty();
    if evidence.complete {
        evidence.unprocessed_fills = evidence
            .fills
            .iter()
            .filter(|trade| {
                trade["fill_id"]
                    .as_str()
                    .is_some_and(|id| !processed_set.contains(id))
            })
            .cloned()
            .collect();
    }
    evidence
}

async fn sync_protective_stops(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let tracked = {
        let settings = state.settings.lock().await;
        settings
            .strategy_positions
            .iter()
            .flat_map(|(symbol, lots)| {
                lots.iter().flat_map(|(strategy, position)| {
                    position.protective_stops.iter().cloned().map({
                        let symbol = symbol.clone();
                        let strategy = strategy.clone();
                        move |stop| (symbol.clone(), strategy.clone(), stop)
                    })
                })
            })
            .collect::<Vec<_>>()
    };
    if tracked.is_empty() {
        return Ok(());
    }
    let barriers: Vec<serde_json::Value> = serde_json::from_value(
        crate::service_rpc::position_get("/positions/barriers")
            .await
            .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?,
    )
    .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let trade_records_response =
        crate::service_rpc::position_get("/positions/trades?source=auto").await;
    let (trade_records, trade_records_error) = match trade_records_response {
        Ok(response) => (
            response["trades"].as_array().cloned().unwrap_or_default(),
            None,
        ),
        Err(error) => {
            crate::service_log!(
                "WARN",
                "Could not fetch PM fills for protective stop attribution: {}",
                error
            );
            (Vec::new(), Some(error.to_string()))
        }
    };
    for (symbol, strategy, stop) in tracked {
        let barrier = barriers
            .iter()
            .find(|barrier| barrier["id"].as_str() == Some(&stop.barrier_id));
        let Some(barrier) = barrier else {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(position) = settings
                .strategy_positions
                .get_mut(&symbol)
                .and_then(|lots| lots.get_mut(&strategy))
            {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings.orders.iter_mut().find(|order| {
                order.timestamp == stop.order_timestamp
                    && order.execution_order_id.as_deref() == Some(stop.barrier_id.as_str())
            }) {
                order.status = "cancelled".into();
            }
            request_settings_persist(state)?;
            continue;
        };
        let total_filled = barrier["filled_quantity"].as_f64().unwrap_or(0.0).max(0.0);
        let total_notional =
            barrier["average_execution_price"].as_f64().unwrap_or(0.0) * total_filled;
        let total_fee = barrier["fee_paid"].as_f64().unwrap_or(0.0).abs();
        let (matching_local_order_count, processed_fill_ids) = {
            let settings = state.settings.lock().await;
            let matching_local_orders = settings
                .orders
                .iter()
                .filter(|order| {
                    order.timestamp == stop.order_timestamp
                        && order.execution_order_id.as_deref() == Some(stop.barrier_id.as_str())
                })
                .collect::<Vec<_>>();
            (
                matching_local_orders.len(),
                matching_local_orders
                    .first()
                    .map(|order| order.processed_fill_trade_ids.clone())
                    .unwrap_or_default(),
            )
        };
        let mut fill_evidence =
            protective_stop_fill_evidence(barrier, &trade_records, &stop, &processed_fill_ids);
        if matching_local_order_count != 1 {
            fill_evidence
                .reasons
                .push("protective_stop_local_order_identity_missing_or_ambiguous".into());
            fill_evidence.complete = false;
            fill_evidence.unprocessed_fills.clear();
        }
        if let Some(error) = trade_records_error.as_ref() {
            fill_evidence
                .reasons
                .push(format!("protective_stop_pm_fill_query_failed:{error}"));
            fill_evidence.complete = false;
            fill_evidence.unprocessed_fills.clear();
        }
        let (delta_quantity, delta_notional, delta_fee) = if fill_evidence.complete {
            fill_evidence.unprocessed_fills.iter().fold(
                (0.0, 0.0, 0.0),
                |(quantity, notional, fee), fill| {
                    let fill_quantity = fill["quantity"].as_f64().unwrap_or_default();
                    let fill_price = fill["execution_price"].as_f64().unwrap_or_default();
                    let fill_fee = fill["fee"].as_f64().unwrap_or_default().abs();
                    (
                        quantity + fill_quantity,
                        notional + fill_quantity * fill_price,
                        fee + fill_fee,
                    )
                },
            )
        } else {
            (
                (total_filled - stop.processed_quantity).max(0.0),
                (total_notional - stop.processed_notional).max(0.0),
                (total_fee - stop.processed_fee).max(0.0),
            )
        };
        let status = barrier["status"].as_str().unwrap_or("unknown");
        if delta_quantity > 1e-12 && delta_notional > 0.0 {
            let side = if barrier["side"].as_str() == Some("buy") {
                "buy"
            } else {
                "sell"
            };
            let fill_price = delta_notional / delta_quantity;
            let mut settings = state.settings.lock().await;
            let entry_decision_id = settings
                .strategy_positions
                .get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .and_then(|position| position.entry_decision_id.clone());
            let entry_price_before = settings
                .strategy_positions
                .get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .map(|position| position.average_entry_price)
                .filter(|price| *price > 0.0)
                .unwrap_or(fill_price);
            let (
                price_path,
                price_path_total_samples,
                price_path_truncated,
                price_path_restart_interrupted,
                position_quantity,
                effective_stop_loss_percent,
                effective_take_profit_percent,
                entry_fees,
                entry_quantity,
                position_entry_timestamp_ms,
                prior_realized_pnl,
                initial_entry_notional,
                prior_total_fees,
                total_entry_fees_paid,
                entry_fill_ids,
                entry_fee_currencies,
                entry_fill_costs_complete,
                entry_context,
                scout_entry,
                strategy_decision,
                trade_id,
                lifecycle_identity,
                exploratory,
            ) = settings
                .strategy_positions
                .get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .map(|position| {
                    (
                        position.price_path.clone(),
                        position.price_path_total_samples,
                        position.price_path_truncated,
                        position.price_path_restart_interrupted,
                        position.quantity,
                        position
                            .stop_loss_percent
                            .unwrap_or(settings.stop_loss_percent),
                        position
                            .take_profit_percent
                            .or(position.horizon_take_profit_percent)
                            .unwrap_or(settings.take_profit_percent),
                        position.entry_fees,
                        position.quantity.abs(),
                        position.entry_timestamp_ms,
                        position.realized_pnl,
                        position.initial_entry_notional,
                        position.total_fees_paid,
                        position.total_entry_fees_paid,
                        position.entry_fill_ids.clone(),
                        position.entry_fee_currencies.clone(),
                        position.entry_fill_costs_complete,
                        position.entry_context.clone(),
                        position.scout_entry,
                        position.strategy_decision.clone(),
                        position.trade_id.clone(),
                        position.lifecycle_identity.clone(),
                        position.exploratory,
                    )
                })
                .unwrap_or((
                    Vec::new(),
                    0,
                    false,
                    false,
                    0.0,
                    settings.stop_loss_percent,
                    settings.take_profit_percent,
                    0.0,
                    0.0,
                    0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    Vec::new(),
                    Vec::new(),
                    false,
                    String::new(),
                    false,
                    None,
                    None,
                    None,
                    false,
                ));
            let (realized_delta, _learned_key) = apply_auto_fill(
                &mut settings.positions,
                &symbol,
                side,
                delta_quantity,
                fill_price,
                delta_fee,
                "StopMarket",
                "stop-loss",
                false,
                None,
            );
            let Settings {
                strategy_positions,
                positions,
                ..
            } = &mut *settings;
            let (strategy_realized_delta, _) = apply_strategy_position_fill(
                strategy_positions,
                positions,
                &symbol,
                &strategy,
                side,
                delta_quantity,
                fill_price,
                delta_fee,
                "StopMarket",
                "stop-loss",
                false,
                None,
                None,
                None,
                None,
                None,
            );
            let realized_delta_usd = realized_usd(&settings, &symbol, realized_delta);
            settings.realized_pnl += realized_delta_usd;
            settings.daily_realized_pnl += realized_delta_usd;
            settings.used_notional_today = allocated_notional(&settings);
            let fully_closed = settings
                .strategy_positions
                .get(&symbol)
                .and_then(|lots| lots.get(&strategy))
                .is_none_or(|position| position.quantity.abs() <= 1e-12);
            let stop_fully_filled = status == "filled"
                && completed_stop_position_fill(
                    "stop-loss",
                    total_filled,
                    stop.quantity,
                    fully_closed,
                );
            let mut outcome_fee_currencies = entry_fee_currencies
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            for fill in &fill_evidence.fills {
                if let Some(currency) = fill["fee_currency"]
                    .as_str()
                    .filter(|currency| !currency.trim().is_empty())
                {
                    outcome_fee_currencies.insert(currency.to_owned());
                }
            }
            let outcome_fees_complete = fill_evidence.complete
                && entry_fill_costs_complete
                && !entry_fill_ids.is_empty()
                && outcome_fee_currencies.len() == 1;
            let outcome_fill_response = serde_json::json!({
                "fee_complete": outcome_fees_complete,
                "fill_evidence": if outcome_fees_complete {
                    fill_evidence.fills.clone()
                } else {
                    Vec::<serde_json::Value>::new()
                },
            });
            let post_stop_cooldown_started = if stop_fully_filled {
                let cooldown_position = TraderPosition {
                    entry_decision_id: entry_decision_id.clone(),
                    strategy_decision: strategy_decision.clone(),
                    ..TraderPosition::default()
                };
                record_post_stop_reentry_cooldown(
                    &mut settings,
                    &symbol,
                    &strategy,
                    &cooldown_position,
                    current_time_ms(),
                )
            } else {
                false
            };
            let trade_realized_pnl = prior_realized_pnl + strategy_realized_delta;
            let entry_notional = if initial_entry_notional > 0.0 {
                initial_entry_notional
            } else {
                delta_quantity * entry_price_before
            };
            if fully_closed
                && !scout_entry
                && settings
                    .positions
                    .get(&symbol)
                    .is_none_or(|position| position.quantity.abs() <= 1e-12)
            {
                mark_symbol_entry_floor_lift(&mut settings, &symbol, &strategy, trade_realized_pnl);
            }
            let mut trade_feedback_enqueued = false;
            if fully_closed && entry_notional > 0.0 {
                let total_trade_pnl = trade_realized_pnl;
                let total_fees = if prior_total_fees > 0.0 {
                    prior_total_fees + delta_fee
                } else {
                    entry_fees + delta_fee
                };
                let fees_usd = realized_usd(&settings, &symbol, total_fees);
                let strategy_realized_usd = realized_usd(&settings, &symbol, total_trade_pnl);
                let replay_context = build_trade_replay_context(
                    &symbol,
                    position_entry_timestamp_ms,
                    barrier["triggered_at"]
                        .as_u64()
                        .unwrap_or_else(current_time_ms),
                    if side == "sell" { "long" } else { "short" },
                    entry_price_before,
                    total_notional / total_filled.max(f64::EPSILON),
                    entry_quantity,
                    total_entry_fees_paid.max(entry_fees),
                    total_fee,
                    entry_walk_forward_evidence(strategy_decision.as_ref()),
                );
                let closed_position = TraderPosition {
                    entry_context: entry_context.clone(),
                    trade_id: trade_id.clone(),
                    lifecycle_identity: lifecycle_identity.clone(),
                    exploratory,
                    scout_entry,
                    entry_fill_ids: entry_fill_ids.clone(),
                    entry_fee_currencies: entry_fee_currencies.clone(),
                    entry_fill_costs_complete,
                    strategy_decision: strategy_decision.clone(),
                    ..TraderPosition::default()
                };
                let replay_context = replay_context.and_then(|context| {
                    let outcome_key = trade_id.as_deref().filter(|value| !value.is_empty())?;
                    Some(attach_authoritative_outcome_lineage(
                        context,
                        &closed_position,
                        &outcome_fill_response,
                        lifecycle_identity.as_ref(),
                        outcome_key,
                    ))
                });
                let authoritative_lineage_complete =
                    lifecycle_identity.as_ref().is_none_or(|identity| {
                        let Some(trade_id) = trade_id.as_deref().filter(|value| !value.is_empty())
                        else {
                            return false;
                        };
                        authoritative_outcome_lineage_is_complete(
                            identity,
                            replay_context.as_ref(),
                            &format!("trade:{trade_id}"),
                        )
                    });
                let mut closed_audit = closed_trade_audit_for_position(
                    &closed_position,
                    &symbol,
                    replay_context.as_ref(),
                );
                if !outcome_fees_complete || !authoritative_lineage_complete {
                    closed_audit.status = ClosedTradeAuditStatus::NeedsAudit;
                    closed_audit.reason = Some(if !outcome_fees_complete {
                        if fill_evidence.reasons.is_empty() {
                            "protective_stop_fill_fee_lineage_incomplete".into()
                        } else {
                            format!(
                                "protective_stop_fill_lineage_incomplete:{}",
                                fill_evidence.reasons.join(",")
                            )
                        }
                    } else {
                        "protective_stop_authoritative_outcome_lineage_incomplete".into()
                    });
                }
                record_closed_trade_outcome(
                    &mut settings,
                    total_trade_pnl / entry_notional * 100.0,
                    strategy_realized_usd,
                    fees_usd,
                    closed_audit,
                );
                trade_feedback_enqueued = record_strategy_trade_outcome(
                    &mut settings,
                    &symbol,
                    Some(&strategy),
                    Some(&entry_context),
                    scout_entry,
                    strategy_realized_usd,
                    Some(total_trade_pnl / entry_notional * 100.0),
                    strategy_decision.as_ref(),
                    trade_id.as_deref(),
                    replay_context,
                );
                adapt_exit_policy(&mut settings);
            }
            if let Some(order) = settings.orders.iter_mut().find(|order| {
                order.timestamp == stop.order_timestamp
                    && order.execution_order_id.as_deref() == Some(stop.barrier_id.as_str())
            }) {
                order.triggered_at = barrier["triggered_at"].as_u64();
                order.filled_quantity = total_filled;
                order.fill_rate = if order.quantity > 0.0 {
                    (total_filled / order.quantity).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                order.average_price = barrier["average_execution_price"].as_f64();
                order.fees_paid = total_fee;
                order.realized_pnl += realized_delta;
                record_strategy_order_realized_pnl(order, strategy_realized_delta);
                order.status = status.to_owned();
                if fill_evidence.complete {
                    for fill_id in &fill_evidence.fill_ids {
                        if !order
                            .processed_fill_trade_ids
                            .iter()
                            .any(|processed| processed == fill_id)
                        {
                            order.processed_fill_trade_ids.push(fill_id.clone());
                        }
                    }
                }
            }
            let Settings {
                strategy_positions,
                positions,
                ..
            } = &mut *settings;
            for position in [
                positions.get_mut(&symbol),
                strategy_positions
                    .get_mut(&symbol)
                    .and_then(|lots| lots.get_mut(&strategy)),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(tracked) = position
                    .protective_stops
                    .iter_mut()
                    .find(|tracked| tracked.barrier_id == stop.barrier_id)
                {
                    tracked.processed_quantity = total_filled;
                    tracked.processed_notional = total_notional;
                    tracked.processed_fee = total_fee;
                }
            }
            request_settings_persist(state)?;
            let cumulative_strategy_pnl = settings
                .orders
                .iter()
                .find(|order| {
                    order.timestamp == stop.order_timestamp
                        && order.execution_order_id.as_deref() == Some(stop.barrier_id.as_str())
                })
                .and_then(|order| order.strategy_realized_pnl)
                .unwrap_or(strategy_realized_delta);
            drop(settings);
            if post_stop_cooldown_started {
                notify_live_profile_updates(state);
            }
            if delta_quantity > 1e-12 || trade_feedback_enqueued || post_stop_cooldown_started {
                if let Err(error) = persist_settings_snapshot_durable(state).await {
                    crate::service_log!(
                        "ERROR",
                        "Could not durably persist closed-trade or post-stop state: {}",
                        error.1
                    );
                }
            }
            if let Some(decision_id) = entry_decision_id {
                let update_timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let price_path_evidence = price_path_excursion_evidence_for_samples(
                    &price_path,
                    price_path_total_samples,
                    price_path_truncated,
                    price_path_restart_interrupted,
                    entry_price_before,
                    position_quantity,
                    position_entry_timestamp_ms,
                    update_timestamp,
                );
                let mut payload = protective_stop_audit_payload(
                    &decision_id,
                    &stop,
                    update_timestamp,
                    &symbol,
                    &strategy,
                    status,
                    barrier,
                    total_filled,
                    total_fee,
                    cumulative_strategy_pnl,
                    entry_price_before,
                    entry_fees,
                    entry_quantity,
                    initial_entry_notional,
                    total_entry_fees_paid,
                    prior_total_fees + delta_fee,
                    position_entry_timestamp_ms,
                    effective_stop_loss_percent,
                    effective_take_profit_percent,
                    &price_path,
                    price_path_evidence,
                    fully_closed,
                );
                payload["barrier_fill_lineage"] = serde_json::json!({
                    "status": if outcome_fees_complete && authoritative_lineage_complete {
                        "complete"
                    } else {
                        "NeedsAudit"
                    },
                    "reasons": fill_evidence.reasons.clone(),
                    "barrier_order_id": stop.barrier_id.clone(),
                    "parent_order_id": barrier["parent_order_id"],
                    "client_order_id": barrier["client_order_id"],
                    "pm_fill_ids": fill_evidence.fill_ids.clone(),
                    "pm_fill_count": fill_evidence.fills.len(),
                    "fees_complete": outcome_fees_complete,
                    "authoritative_lineage_complete": authoritative_lineage_complete,
                    "fee_currency": (outcome_fee_currencies.len() == 1)
                        .then(|| outcome_fee_currencies.iter().next().cloned())
                        .flatten(),
                });
                payload["exit_fill_ids"] = serde_json::json!(if outcome_fees_complete {
                    fill_evidence.fill_ids
                } else {
                    Vec::new()
                });
                if let Some(trade_id) = trade_id.as_deref() {
                    payload["trade_id"] = serde_json::json!(trade_id);
                    payload["entry_context"]["trade_id"] = serde_json::json!(trade_id);
                }
                payload["entry_context"]["learning_context"] = serde_json::json!(entry_context);
                payload["entry_context"]["strategy_decision"] =
                    serde_json::json!(strategy_decision);
                if fully_closed {
                    let mut settings = state.settings.lock().await;
                    queue_post_exit_observation(&mut settings, &payload);
                    request_settings_persist(state)?;
                }
                queue_auto_trader_audit(state, payload);
            }
        }
        if !matches!(status, "armed" | "triggering") {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(position) = settings
                .strategy_positions
                .get_mut(&symbol)
                .and_then(|lots| lots.get_mut(&strategy))
            {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings.orders.iter_mut().find(|order| {
                order.timestamp == stop.order_timestamp
                    && order.execution_order_id.as_deref() == Some(stop.barrier_id.as_str())
            }) {
                order.status = status.to_owned();
            }
            request_settings_persist(state)?;
            drop(settings);
            if let Err(error) = persist_settings_snapshot_durable(state).await {
                crate::service_log!(
                    "ERROR",
                    "Could not durably persist terminal protective-stop state: {}",
                    error.1
                );
            }
        }
    }
    Ok(())
}

fn protective_stop_audit_payload(
    decision_id: &str,
    stop: &ProtectiveStop,
    update_timestamp: u64,
    symbol: &str,
    strategy: &str,
    status: &str,
    barrier: &serde_json::Value,
    total_filled: f64,
    total_fee: f64,
    cumulative_pnl: f64,
    entry_price_before: f64,
    _entry_fees: f64,
    entry_quantity: f64,
    initial_entry_notional: f64,
    total_entry_fees_paid: f64,
    total_fees_paid: f64,
    position_entry_timestamp_ms: u64,
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    price_path: &[TradePriceSample],
    price_path_evidence: serde_json::Value,
    trade_closed: bool,
) -> serde_json::Value {
    serde_json::json!({
        "record_schema_version": 1,
        "decision_id": decision_id,
        "trade_id": decision_id,
        "trade_closed": trade_closed,
        "strategy": strategy,
        "stage": format!("exit:stop:{}", stop.order_timestamp),
        "timestamp": update_timestamp,
        "observed_at": update_timestamp,
        "symbol": symbol,
        "side": barrier["side"],
        "status": status,
        "reason": "stop-loss",
        "exit_order_type": "StopMarket",
        "submitted_at": barrier["triggered_at"],
        "barrier_order_id": stop.barrier_id,
        "exit_order_quantity": stop.quantity,
        "exit_filled_quantity": total_filled,
        "exit_average_price": barrier["average_execution_price"],
        "exit_fees": total_fee,
        "realized_pnl": cumulative_pnl,
        "entry_average_price": entry_price_before,
        "entry_fees_paid": total_entry_fees_paid,
        "entry_timestamp_ms": position_entry_timestamp_ms,
        "entry_context": {
            "trade_id": decision_id,
            "average_price": entry_price_before,
            "fees": total_entry_fees_paid,
            "initial_entry_notional": initial_entry_notional,
            "total_entry_fees_paid": total_entry_fees_paid,
            "total_fees_paid": total_fees_paid,
            "timestamp_ms": position_entry_timestamp_ms,
            "quantity": entry_quantity,
        },
        "duration_ms": (position_entry_timestamp_ms > 0)
            .then_some(update_timestamp.saturating_sub(position_entry_timestamp_ms)),
        "barrier_trigger_price": barrier["trigger_price"],
        "barrier_trigger_direction": barrier["trigger_direction"],
        "effective_stop_loss_percent": effective_stop_loss_percent,
        "effective_take_profit_percent": effective_take_profit_percent,
        "price_path": price_path.iter().map(|sample| serde_json::json!({
            "timestamp": sample.timestamp,
            "bid": sample.bid,
            "ask": sample.ask,
            "source_updated_at_ms": sample.source_updated_at_ms,
            "freshness_contract_version": sample.freshness_contract_version,
            "freshness_max_age_ms": sample.freshness_max_age_ms,
            "source_fresh": sample.source_fresh,
        })).collect::<Vec<_>>(),
        "price_path_evidence": price_path_evidence,
        "net_return_percent": (entry_price_before > 0.0).then_some(
            cumulative_pnl / initial_entry_notional.max(1e-12) * 100.0
        ),
    })
}

#[cfg(test)]
mod protective_stop_audit_tests {
    use super::*;

    fn barrier(status: &str, quantity: f64, average_price: f64, fee: f64) -> serde_json::Value {
        serde_json::json!({
            "id":"barrier-1",
            "symbol":"BTC-USDT",
            "side":"sell",
            "status":status,
            "parent_order_id":"auto-stop-BTC-1000",
            "client_order_id":"auto-stop-BTC-1000",
            "filled_quantity":quantity,
            "average_execution_price":average_price,
            "fee_paid":fee
        })
    }

    fn fill(id: &str, fill_id: &str, quantity: f64, price: f64, fee: f64) -> serde_json::Value {
        serde_json::json!({
            "id":id,
            "fill_id":fill_id,
            "symbol":"BTC-USDT",
            "side":"sell",
            "source":"auto",
            "parent_order_id":"barrier-1",
            "client_order_id":"barrier-1",
            "quantity":quantity,
            "execution_price":price,
            "fee":fee,
            "fee_known":true,
            "fee_currency":"USDT"
        })
    }

    #[test]
    fn pm_barrier_fill_ids_reconcile_partial_full_duplicate_and_restart_refreshes() {
        let stop = ProtectiveStop {
            barrier_id: "barrier-1".into(),
            order_timestamp: 1_000,
            quantity: 0.5,
            ..ProtectiveStop::default()
        };
        let first_fill = fill("pm-row-1", "pm-fill:v1:epoch-1:1", 0.25, 96.0, -0.025);
        let partial = protective_stop_fill_evidence(
            &barrier("partially_filled", 0.25, 96.0, 0.025),
            std::slice::from_ref(&first_fill),
            &stop,
            &[],
        );
        assert!(partial.complete, "{:?}", partial.reasons);
        assert_eq!(partial.fill_ids, vec!["pm-fill:v1:epoch-1:1"]);
        assert_eq!(partial.unprocessed_fills.len(), 1);

        let second_fill = fill("pm-row-2", "pm-fill:v1:epoch-1:2", 0.25, 94.0, -0.025);
        let full_barrier = barrier("filled", 0.5, 95.0, 0.05);
        let stop_after_partial = ProtectiveStop {
            processed_quantity: 0.25,
            processed_notional: 24.0,
            processed_fee: 0.025,
            ..stop.clone()
        };
        let processed = vec!["pm-fill:v1:epoch-1:1".to_owned()];
        let full = protective_stop_fill_evidence(
            &full_barrier,
            &[first_fill.clone(), second_fill.clone()],
            &stop_after_partial,
            &processed,
        );
        assert!(full.complete, "{:?}", full.reasons);
        assert_eq!(full.unprocessed_fills, vec![second_fill.clone()]);

        let serialized_restart_ids =
            serde_json::to_value(["pm-fill:v1:epoch-1:1", "pm-fill:v1:epoch-1:2"]).unwrap();
        let restarted_ids: Vec<String> = serde_json::from_value(serialized_restart_ids).unwrap();
        let stop_after_full = ProtectiveStop {
            processed_quantity: 0.5,
            processed_notional: 47.5,
            processed_fee: 0.05,
            ..stop
        };
        let duplicate_refresh = protective_stop_fill_evidence(
            &full_barrier,
            &[first_fill, second_fill],
            &stop_after_full,
            &restarted_ids,
        );
        assert!(
            duplicate_refresh.complete,
            "{:?}",
            duplicate_refresh.reasons
        );
        assert!(duplicate_refresh.unprocessed_fills.is_empty());
    }

    #[test]
    fn missing_duplicate_or_misparented_barrier_fills_fail_closed() {
        let stop = ProtectiveStop {
            barrier_id: "barrier-1".into(),
            quantity: 0.5,
            ..ProtectiveStop::default()
        };
        let barrier = barrier("filled", 0.5, 95.0, 0.05);
        let good = fill("pm-row-1", "pm-fill:v1:epoch-1:1", 0.5, 95.0, -0.05);
        assert!(!protective_stop_fill_evidence(&barrier, &[], &stop, &[]).complete);

        let duplicate =
            protective_stop_fill_evidence(&barrier, &[good.clone(), good.clone()], &stop, &[]);
        assert!(!duplicate.complete);
        assert!(duplicate
            .reasons
            .iter()
            .any(|reason| reason == "protective_barrier_fill_id_duplicate"));

        let mut misparented = good;
        misparented["parent_order_id"] = serde_json::json!("another-stop");
        let mismatch = protective_stop_fill_evidence(&barrier, &[misparented], &stop, &[]);
        assert!(!mismatch.complete);
        assert!(mismatch.reasons.iter().any(|reason| {
            reason == "protective_barrier_fill_order_or_market_identity_mismatch"
        }));

        let mut unknown_fee = good;
        unknown_fee["fee_known"] = serde_json::json!(false);
        unknown_fee["fee_currency"] = serde_json::Value::Null;
        let incomplete_fee = protective_stop_fill_evidence(&barrier, &[unknown_fee], &stop, &[]);
        assert!(!incomplete_fee.complete);
        assert!(incomplete_fee
            .reasons
            .iter()
            .any(|reason| reason == "protective_barrier_fill_fee_incomplete"));
    }

    #[test]
    fn audit_payload_records_barrier_fill_and_duration() {
        let stop = ProtectiveStop {
            barrier_id: "barrier-1".into(),
            order_timestamp: 1_000,
            quantity: 2.0,
            ..ProtectiveStop::default()
        };
        let path = [TradePriceSample {
            timestamp: 1_500,
            bid: 99.0,
            ask: 101.0,
            ..TradePriceSample::default()
        }];
        let barrier = serde_json::json!({"side":"sell","triggered_at":1200,"average_execution_price":98.0,"trigger_price":99.0,"trigger_direction":"below"});
        let path_evidence = price_path_excursion_evidence_for_samples(
            &path, 1, false, false, 100.0, 2.0, 1_000, 2_000,
        );
        let payload = protective_stop_audit_payload(
            "decision-3",
            &stop,
            2_000,
            "BTC-USDT",
            "strategy-a",
            "filled",
            &barrier,
            2.0,
            0.1,
            -4.1,
            100.0,
            0.2,
            2.0,
            200.0,
            0.2,
            0.3,
            1_000,
            5.0,
            10.0,
            &path,
            path_evidence,
            true,
        );

        assert_eq!(payload["stage"], "exit:stop:1000");
        assert_eq!(payload["barrier_order_id"], "barrier-1");
        assert_eq!(payload["duration_ms"], 1_000);
        assert_eq!(payload["entry_context"]["average_price"], 100.0);
        assert_eq!(payload["entry_context"]["quantity"], 2.0);
        assert_eq!(payload["price_path"][0]["bid"], 99.0);
        assert_eq!(payload["net_return_percent"], -2.05);
    }
}
