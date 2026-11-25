async fn run_position_maintenance(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_rsocket_url = std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    recover_submitting_entry_orders(state).await;
    // Consume fills before classifying disappeared resting orders as inactive.
    reconcile_maker_fills(&position_rsocket_url, state)
        .await
        .map_err(|(status, error)| (status, format!("Maker-fill reconciliation: {error}")))?;
    if let Err(error) = revalidate_resting_signal_orders(state).await {
        crate::service_log!("WARN", "Resting-order revalidation failed: {}", error.1);
    }
    if let Err(error) = reconcile_inactive_execution_orders(state).await {
        crate::service_log!("WARN", "Pending-order reconciliation failed: {}", error.1);
    }
    reconcile_scheduled_fills(state)
        .await
        .map_err(|(status, error)| (status, format!("Scheduled-fill reconciliation: {error}")))?;
    sync_auto_positions(state)
        .await
        .map_err(|(status, error)| (status, format!("Position-ledger sync: {error}")))?;
    let _ = maybe_exit_auto_position(state)
        .await
        .map_err(|(status, error)| (status, format!("Exit evaluation: {error}")))?;
    Ok(())
}

#[derive(Deserialize)]
struct PersistedEntryRecoveryContext {
    schema_version: u16,
    signal: Signal,
    exploratory: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    side: String,
    quantity: f64,
    notional: f64,
    bid: f64,
    ask: f64,
    mid: f64,
    order_type: AutoOrderType,
    execution_parameters: PersistedExecutionParameters,
    timestamp: u64,
    decision_id: String,
    lifecycle_identity: CanonicalLifecycleIdentity,
    learning_context: String,
    exploration_risk_adjustment: PersistedExplorationRiskAdjustment,
    horizon_take_profit_percent: Option<f64>,
    stage_timings_ms: BTreeMap<String, f64>,
    deep_book: Option<PersistedDeepBookAssessment>,
    estimated_ioc_all_in_bps: Option<f64>,
    submitted_limit_price: Option<f64>,
    limit_to_mid_bps: Option<f64>,
    limit_to_crossing_quote_bps: Option<f64>,
    expires_at: Option<u64>,
}

#[derive(Deserialize)]
struct PersistedExecutionParameters {
    post_only_timeout_minutes: u64,
    vwap_duration_minutes: u64,
    vwap_participation_rate: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}

#[derive(Deserialize)]
struct PersistedExplorationRiskAdjustment {
    scale: f64,
    sample_count: usize,
    weighted_net_return_percent: Option<f64>,
    weighted_net_return_lcb_percent: Option<f64>,
    regime_scoped: bool,
}

#[derive(Deserialize)]
struct PersistedDeepBookAssessment {
    available_within_band: f64,
    notional_cap: f64,
    impact_bps: f64,
    bid_depth_10bps: f64,
    ask_depth_10bps: f64,
    depth_imbalance_10bps: f64,
}

fn recovery_execution_from_order(
    state: &AutoTraderState,
    order: &TraderOrder,
) -> Result<SelectedOrderExecution, String> {
    let (request, value) = persisted_recovery_parts(order.revalidation_snapshot.as_ref())?;
    let request = request.clone();
    let context: PersistedEntryRecoveryContext = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid stored response-application context: {error}"))?;
    if context.schema_version != 1
        || context.timestamp != order.timestamp
        || Some(context.decision_id.as_str()) != order.decision_id.as_deref()
        || context.signal.symbol != order.symbol
        || context.side != order.side
        || (context.quantity - order.quantity).abs() > 1e-12
        || !persisted_request_matches_context(&request, &context)
    {
        return Err("stored submission request does not match its durable reservation".to_owned());
    }
    let params = context.execution_parameters;
    let risk = context.exploration_risk_adjustment;
    let deep_book = context.deep_book.map(|book| DeepBookAssessment {
        available_within_band: book.available_within_band,
        notional_cap: book.notional_cap,
        impact_bps: book.impact_bps,
        bid_depth_10bps: book.bid_depth_10bps,
        ask_depth_10bps: book.ask_depth_10bps,
        depth_imbalance_10bps: book.depth_imbalance_10bps,
    });
    Ok(SelectedOrderExecution {
        state: state.clone(),
        signal: context.signal,
        exploratory: context.exploratory,
        counterfactual_stop_loss_percent: context.counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent: context.counterfactual_take_profit_percent,
        effective_stop_loss_percent: context.effective_stop_loss_percent,
        effective_take_profit_percent: context.effective_take_profit_percent,
        side: context.side,
        quantity: context.quantity,
        notional: context.notional,
        bid: context.bid,
        ask: context.ask,
        mid: context.mid,
        order_type: context.order_type,
        execution_params: ExecutionParameters {
            post_only_timeout_minutes: params.post_only_timeout_minutes,
            vwap_duration_minutes: params.vwap_duration_minutes,
            vwap_participation_rate: params.vwap_participation_rate,
            limit_offset_bps: params.limit_offset_bps,
            trigger_offset_bps: params.trigger_offset_bps,
            trailing_distance_percent: params.trailing_distance_percent,
        },
        timestamp: context.timestamp,
        decision_id: context.decision_id,
        lifecycle_identity: context.lifecycle_identity,
        learning_context: context.learning_context,
        exploration_risk_adjustment: ExplorationRiskAdjustment {
            scale: risk.scale,
            sample_count: risk.sample_count,
            weighted_net_return_percent: risk.weighted_net_return_percent,
            weighted_net_return_lcb_percent: risk.weighted_net_return_lcb_percent,
            regime_scoped: risk.regime_scoped,
        },
        horizon_take_profit_percent: context.horizon_take_profit_percent,
        stage_timings_ms: context.stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps: context.estimated_ioc_all_in_bps,
        submitted_limit_price: context.submitted_limit_price,
        limit_to_mid_bps: context.limit_to_mid_bps,
        limit_to_crossing_quote_bps: context.limit_to_crossing_quote_bps,
        expires_at: context.expires_at,
        persisted_request: request,
        is_recovery: true,
    })
}

fn persisted_request_matches_context(
    request: &PersistedEntryRequest,
    context: &PersistedEntryRecoveryContext,
) -> bool {
    let lifecycle = &request.body["lifecycle"];
    let expected_path = if matches!(context.order_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
        "/positions/vwap"
    } else {
        "/positions/orders"
    };
    request.path == expected_path
        && request.body.get("symbol").and_then(serde_json::Value::as_str)
            == Some(context.signal.symbol.as_str())
        && request.body.get("side").and_then(serde_json::Value::as_str)
            == Some(context.side.as_str())
        && request
            .body
            .get("quantity")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|quantity| (quantity - context.quantity).abs() <= 1e-12)
        && request.body.get("client_order_id").and_then(serde_json::Value::as_str)
            == Some(context.lifecycle_identity.opportunity_id.as_str())
        && lifecycle.get("opportunity_id").and_then(serde_json::Value::as_str)
            == Some(context.lifecycle_identity.opportunity_id.as_str())
        && lifecycle.get("episode_id").and_then(serde_json::Value::as_str)
            == Some(context.lifecycle_identity.episode_id.as_str())
        && lifecycle.get("trade_id").and_then(serde_json::Value::as_str)
            == Some(context.lifecycle_identity.trade_id.as_str())
}

fn persisted_recovery_parts<'a>(
    snapshot: Option<&'a RestingOrderRevalidationSnapshot>,
) -> Result<(&'a PersistedEntryRequest, &'a serde_json::Value), String> {
    let snapshot = snapshot
        .ok_or_else(|| "legacy submitting order has no request snapshot".to_owned())?;
    let request = snapshot
        .submission_request
        .as_ref()
        .ok_or_else(|| "legacy submitting order has no exact Position Management request".to_owned())?;
    let context = snapshot
        .submission_recovery_context
        .as_ref()
        .ok_or_else(|| "submitting order has no response-application context".to_owned())?;
    if !persisted_entry_request_is_replayable(request) {
        return Err("stored request is unsupported or fails its idempotency contract".to_owned());
    }
    Ok((request, context))
}

async fn recover_submitting_entry_orders(state: &AutoTraderState) {
    let orders = {
        let settings = state.settings.lock().await;
        settings
            .orders
            .iter()
            .filter(|order| order.purpose == "signal" && order.status == "submitting")
            .cloned()
            .collect::<Vec<_>>()
    };
    for order in orders {
        if !state.enabled.load(Ordering::Relaxed) {
            let mut settings = state.settings.lock().await;
            if let Some(current) = settings.orders.iter_mut().find(|candidate| {
                candidate.timestamp == order.timestamp && candidate.status == "submitting"
            }) {
                current.failure_reason = Some(
                    "recovery_deferred: Auto Trader is disabled; prior dispatch state is ambiguous"
                        .to_owned(),
                );
                let _ = request_settings_persist(state);
            }
            drop(settings);
            let _ = persist_settings_snapshot_durable(state).await;
            continue;
        }
        let execution = match recovery_execution_from_order(state, &order) {
            Ok(execution) => execution,
            Err(reason) => {
                mark_submission_unknown(state, order.timestamp, &reason).await;
                continue;
            }
        };
        if let Err((status, error)) = submit_and_record_selected_order(execution).await {
            crate::service_log!(
                "WARN",
                "Durable entry replay remains pending for {} ({status}): {error}",
                order.symbol
            );
        }
    }
}

async fn mark_submission_unknown(state: &AutoTraderState, timestamp: u64, reason: &str) {
    let mut settings = state.settings.lock().await;
    if let Some(current) = settings.orders.iter_mut().find(|candidate| {
        candidate.timestamp == timestamp && candidate.status == "submitting"
    }) {
        current.status = "unknown_submission".to_owned();
        current.failure_reason = Some(format!("recovery_not_replayed: {reason}"));
        let _ = request_settings_persist(state);
    }
    drop(settings);
    let _ = persist_settings_snapshot_durable(state).await;
}

const RESTING_SIGNAL_REVALIDATION_MAX_AGE_MS: u64 = 10_000;
const RESTING_SIGNAL_REVALIDATION_MAX_CANCELS_PER_CYCLE: usize = 32;

async fn revalidate_resting_signal_orders(
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let now_ms = current_time_ms();
    let candidates = {
        let settings = state.settings.lock().await;
        settings
            .orders
            .iter()
            .filter(|order| {
                order.purpose == "signal"
                    && is_post_only_entry_order(&order.order_type)
                    && matches!(
                        order.status.as_str(),
                        "accepted" | "resting" | "partially_filled"
                    )
                    && order.execution_order_id.is_some()
                    && order.failure_reason.is_none()
                    && order
                        .expires_at
                        .is_some_and(|expires_at| now_ms < expires_at)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let reports = state.latest_worker_decisions.lock().await.clone();
    let invalidated = candidates
        .iter()
        .filter_map(|order| {
            let snapshot = order.revalidation_snapshot.as_ref()?;
            let strategy = order.strategy.as_deref()?;
            let profile_id = snapshot.profile_id.as_deref()?;
            let report = reports.get(&(
                order.symbol.clone(),
                strategy.to_owned(),
                profile_id.to_owned(),
            ))?;
            let reason = resting_order_invalidation_reason(order, report, now_ms)?;
            Some((order.execution_order_id.clone()?, reason))
        })
        .take(RESTING_SIGNAL_REVALIDATION_MAX_CANCELS_PER_CYCLE)
        .collect::<Vec<_>>();

    for (execution_order_id, reason) in invalidated {
        let path = format!("/positions/orders/{execution_order_id}");
        let response = crate::service_rpc::position_request("DELETE", &path, None)
            .await
            .map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!(
                        "Unable to cancel invalidated resting order {execution_order_id}: {error}"
                    ),
                )
            })?;
        // A concurrent fill/cancel can remove the order before this request.
        // Treat not-found as an idempotent race and let the next fill pass
        // reconcile the final quantity before the order becomes inactive.
        if !is_idempotent_cancel_response(response.status) {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "Position Management rejected cancellation for {execution_order_id}: {}",
                    response.status
                ),
            ));
        }
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings.orders.iter_mut().find(|order| {
            order.execution_order_id.as_deref() == Some(execution_order_id.as_str())
                && matches!(
                    order.status.as_str(),
                    "accepted" | "resting" | "partially_filled"
                )
        }) {
            order.failure_reason = Some(format!("revalidation_cancel_requested: {reason}"));
            request_settings_persist(state)?;
            drop(settings);
            persist_settings_snapshot_durable(state).await?;
        }
    }
    Ok(())
}

fn is_idempotent_cancel_response(status: u16) -> bool {
    (200..300).contains(&status) || status == 404
}

fn is_post_only_entry_order(order_type: &str) -> bool {
    order_type.starts_with("PostOnlyLimit")
}

fn resting_order_invalidation_reason(
    order: &TraderOrder,
    report: &strategy::LiveStrategyDecisionReport,
    now_ms: u64,
) -> Option<String> {
    let snapshot = order.revalidation_snapshot.as_ref()?;
    let strategy = Strategy::from_name(order.strategy.as_deref()?)?;
    let max_age = RESTING_SIGNAL_REVALIDATION_MAX_AGE_MS;
    let worker_identity_is_fresh = snapshot.event_identity_version
        == Some(strategy::LIVE_STRATEGY_EVENT_ID_VERSION)
        && snapshot
            .event_id
            .as_deref()
            .is_some_and(|event_id| !event_id.trim().is_empty())
        && snapshot
            .episode_id
            .as_deref()
            .is_some_and(|episode_id| !episode_id.trim().is_empty())
        && report.event_identity_version == strategy::LIVE_STRATEGY_EVENT_ID_VERSION
        && !report.event_id.trim().is_empty()
        && !report.episode_id.trim().is_empty()
        && snapshot.event_id.as_deref() != Some(report.event_id.as_str());
    if report.symbol != order.symbol
        || report.strategy.as_str() != strategy.as_str()
        || snapshot.profile_id.as_deref() != Some(report.profile_id.as_str())
        || snapshot.profile_version != Some(report.profile_version)
        || snapshot.parameter_hash.as_deref() != Some(report.parameter_hash.as_str())
        || report.event_time_ms <= order.timestamp
        || now_ms.saturating_sub(report.event_time_ms) > max_age
        || report.signal.timestamp <= snapshot.signal_timestamp_ms
        || now_ms.saturating_sub(report.signal.timestamp) > max_age
        || report.signal.stale
        || report.signal.symbol != snapshot.symbol
        || report.signal.strategy.as_deref() != Some(snapshot.strategy.as_str())
        || !worker_identity_is_fresh
    {
        return None;
    }
    if snapshot.episode_id.as_deref() != Some(report.episode_id.as_str()) {
        return Some("fresh worker report belongs to a new trading thesis episode".to_owned());
    }
    if report.signal.direction != snapshot.direction {
        return Some(format!(
            "fresh signal direction changed from {} to {}",
            snapshot.direction, report.signal.direction
        ));
    }
    if report.decision != strategy::LiveStrategyDecision::Reject {
        return None;
    }
    let reason = report
        .reason
        .clone()
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| {
            strategy_entry_rejection_reason(
                &report.signal,
                strategy,
                &signal_strategy_parameters(&report.signal),
            )
        });
    Some(reason)
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

async fn fetch_ledger_realized_pnl() -> Result<LedgerRealizedPnl, (StatusCode, String)> {
    let value = crate::service_rpc::position_get(&format!(
        "/positions/realized-pnl?source=auto&since_ms={}",
        utc_day_start_ms()
    ))
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
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

async fn sync_auto_positions(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
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
                missing_fx_rate_error(
                    "Daily realized P&L",
                    &ledger_pnl.realized_pnl_by_quote,
                    &rates,
                ),
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
        .filter(|order| matches!(order.status.as_str(), "submitting" | "unknown_submission"))
        .map(|order| order.symbol.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut changed = false;
    let mut source_symbols = std::collections::BTreeSet::new();
    let mut external_closes = Vec::new();
    // The order IDs are shared by every external-close reconciliation. Build
    // this index once instead of rescanning the retained order history for
    // each changed position while the global settings and cycle locks are held.
    let mut known_exit_order_ids = None;
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
                .then(|| {
                    external_close_audit_payload(
                        &source_position.symbol,
                        prior_quantity,
                        closed_quantity,
                        prior_average_entry_price,
                        position,
                    )
                });
            let closed_out =
                prior_quantity.abs() > 1e-12 && source_position.quantity.abs() <= 1e-12;
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
            known_exit_order_ids.get_or_insert_with(|| {
                settings
                    .orders
                    .iter()
                    .filter(|order| !matches!(order.purpose.as_str(), "signal" | "scout"))
                    .flat_map(|order| {
                        [
                            Some(order.timestamp.to_string()),
                            order.execution_order_id.clone(),
                        ]
                    })
                    .flatten()
                    .collect::<BTreeSet<_>>()
            });
            let strategy_lots = settings
                .strategy_positions
                .get(&source_position.symbol)
                .map(|lots| {
                    lots.iter()
                        .filter(|(_, lot)| lot.quantity.abs() > 1e-12)
                        .map(|(strategy, lot)| (strategy.clone(), lot.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if closed_out && !scout_entry {
                for (strategy, lot) in &strategy_lots {
                    if !lot.scout_entry {
                        mark_symbol_entry_floor_lift(
                            &mut settings,
                            &source_position.symbol,
                            strategy,
                            lot.realized_pnl,
                        );
                    }
                }
            }
            let mut known_barrier_ids = BTreeSet::new();
            for (_, lot) in &strategy_lots {
                known_barrier_ids.extend(
                    lot.protective_stops
                        .iter()
                        .map(|stop| stop.barrier_id.clone()),
                );
            }
            external_closes.push((
                audit,
                realized_delta,
                strategy_lots,
                current_time_ms(),
                known_barrier_ids,
            ));
        }
        if realized_delta.is_finite() && realized_delta.abs() > 1e-12 {
            // Manual position-manager fills bypass apply_auto_fill; matched
            // strategy-lot audits below provide attributable trade outcomes.
            changed = true;
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
    let known_exit_order_ids = known_exit_order_ids.unwrap_or_default();
    for (mut payload, realized_pnl, strategy_lots, closed_at_ms, known_barrier_ids) in
        external_closes
    {
        let known_exit_ids = KnownExitIds {
            order_ids: &known_exit_order_ids,
            barrier_ids: &known_barrier_ids,
        };
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
        payload["duration_ms"] =
            serde_json::json!((entry_timestamp > 0).then_some(now.saturating_sub(entry_timestamp)));
        let trades = crate::service_rpc::position_get(&format!(
            "/positions/trades?symbol={symbol}&source=auto&since_ms={entry_timestamp}"
        ))
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
        let close_side = if payload["entry_position_side"].as_str() == Some("long") {
            "sell"
        } else {
            "buy"
        };
        let trade_rows = trades["trades"].as_array().cloned().unwrap_or_default();
        let closed_quantity = payload["close_quantity"].as_f64().unwrap_or(0.0);
        if strategy_lots.is_empty() {
            let close_fills = external_close_fills(
                closed_quantity,
                &close_side,
                &trade_rows,
                closed_at_ms,
                &known_exit_ids,
            );
            let exit_quantity = close_fills
                .iter()
                .filter_map(|fill| fill["quantity"].as_f64())
                .sum::<f64>();
            let exit_notional = close_fills
                .iter()
                .filter_map(|fill| Some(fill["quantity"].as_f64()? * fill["price"].as_f64()?))
                .sum::<f64>();
            let exit_fees = close_fills
                .iter()
                .filter_map(|fill| fill["fee"].as_f64())
                .map(f64::abs)
                .sum::<f64>();
            payload["exit_filled_quantity"] = serde_json::json!(exit_quantity);
            payload["exit_average_price"] =
                serde_json::json!((exit_quantity > 0.0).then_some(exit_notional / exit_quantity));
            payload["exit_fees"] = serde_json::json!(exit_fees);
            payload["exit_fills"] = serde_json::json!(close_fills);
            payload["realized_pnl"] = serde_json::json!(realized_pnl);
            payload["strategy"] =
                serde_json::json!(payload["strategy"].as_str().unwrap_or("manual"));
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
                        let trade_feedback_enqueued = record_strategy_trade_outcome(
                            &mut settings,
                            &symbol,
                            Some(strategy),
                            audit["learning_context"].as_str(),
                            audit["scout"].as_bool().unwrap_or(false),
                            realized_usd,
                            Some(net_return_percent),
                            audit["strategy_decision"]
                                .as_object()
                                .map(|_| &audit["strategy_decision"]),
                            audit["trade_id"].as_str(),
                            replay_context_from_close_audit(&audit),
                        );
                        request_settings_persist(state)?;
                        drop(settings);
                        if trade_feedback_enqueued {
                            if let Err(error) = persist_settings_snapshot_durable(state).await {
                                crate::service_log!(
                                    "ERROR",
                                    "Could not durably persist closed-trade Learning outcome: {}",
                                    error.1
                                );
                            }
                        }
                    }
                }
                let has_known_auto_exit =
                    audit["exit_fills"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|fill| {
                            fill["parent_order_id"]
                                .as_str()
                                .is_some_and(|parent_id| known_exit_ids.contains(parent_id))
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
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Could not fetch active orders: {error}"),
            )
        })?;
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
            && matches!(
                order.status.as_str(),
                "submitting" | "accepted" | "resting" | "partially_filled"
            )
    }) {
        let Some(id) = order.execution_order_id.as_deref() else {
            continue;
        };
        if active_ids.contains(id) {
            continue;
        }
        let prior_status = order.status.clone();
        order.status = inactive_execution_order_status(order.filled_quantity).into();
        order.failure_reason = Some(
            if order.expires_at.is_some_and(|expires| expires <= now_ms) {
                "expired_without_full_fill".into()
            } else {
                "removed_from_execution_ledger".into()
            },
        );
        changed |= order.status != prior_status;
        completed_orders.push(order.clone());
    }
    for order in completed_orders {
        let Settings {
            execution_feedback_outbox,
            execution_learning_order_ids,
            ..
        } = &mut *settings;
        record_execution_order_quality(
            execution_feedback_outbox,
            execution_learning_order_ids,
            &order,
        );
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
    use super::{
        inactive_execution_order_status, is_idempotent_cancel_response, is_post_only_entry_order,
        is_scheduled_execution_order_type, resting_order_invalidation_reason,
    };
    use serde_json::json;

    fn order(snapshot: bool, filled: f64) -> super::TraderOrder {
        let revalidation_snapshot = snapshot.then(|| {
            json!({
                "symbol": "BTC-USDT", "strategy": "live-momentum",
                "direction": "bullish", "signal_timestamp_ms": 1_000,
                "profile_id": "profile-a", "profile_version": 3, "parameter_hash": "profile-a",
                "event_identity_version": 1, "event_id": "event-1", "episode_id": "episode-1"
            })
        });
        serde_json::from_value(json!({
            "timestamp": 1_100, "symbol": "BTC-USDT", "side": "buy",
            "order_type": "PostOnlyLimit · P0", "purpose": "signal", "strategy": "live-momentum",
            "quantity": 1.0, "filled_quantity": filled, "status": if filled > 0.0 { "partially_filled" } else { "resting" },
            "realized_pnl": 0.0, "revalidation_snapshot": revalidation_snapshot
        })).unwrap()
    }

    fn report(
        decision: strategy::LiveStrategyDecision,
        event_time: u64,
        signal_time: u64,
    ) -> strategy::LiveStrategyDecisionReport {
        serde_json::from_value(json!({
            "contract_version": 1, "event_id": "event-2", "event_identity_version": 1,
            "episode_id": "episode-1", "strategy": "live-momentum",
            "symbol": "BTC-USDT", "profile_id": "profile-a", "profile_version": 3,
            "parameter_hash": "profile-a", "event_time_ms": event_time,
            "decision": decision, "reason": "fresh evaluator rejection", "walk_forward_evidence": null,
            "signal": { "symbol": "BTC-USDT", "direction": "bullish", "strength": 0.4,
                "timestamp": signal_time, "stale": false, "score": 0.2,
                "strategy_candidate": true, "strategy": "live-momentum" }
        })).unwrap()
    }

    #[test]
    fn removes_unfilled_and_partially_filled_orders_from_pending_state() {
        assert_eq!(inactive_execution_order_status(0.0), "expired_or_cancelled");
        assert_eq!(
            inactive_execution_order_status(0.25),
            "partially_filled_closed"
        );
    }

    #[test]
    fn scheduled_orders_are_not_checked_against_the_resting_order_ledger() {
        assert!(is_scheduled_execution_order_type("Vwap · P0"));
        assert!(is_scheduled_execution_order_type("Twap · P1"));
        assert!(!is_scheduled_execution_order_type("PostOnlyLimit · P0"));
    }

    #[test]
    fn only_fresh_same_profile_rejection_invalidates_the_original_opportunity() {
        let now = 5_000;
        let candidate = order(true, 0.0);
        let snapshot =
            serde_json::to_value(candidate.revalidation_snapshot.as_ref().unwrap()).unwrap();
        assert!(snapshot.get("signal").is_none());
        assert_eq!(snapshot["signal_timestamp_ms"], 1_000);
        assert_eq!(
            resting_order_invalidation_reason(
                &candidate,
                &report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400),
                now
            )
            .as_deref(),
            Some("fresh evaluator rejection")
        );
        let mut same_event_retry =
            report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400);
        same_event_retry.event_id = "event-1".to_owned();
        same_event_retry.episode_id = "episode-1".to_owned();
        assert!(
            resting_order_invalidation_reason(&candidate, &same_event_retry, now).is_none(),
            "an exact source-event retry is not fresh revalidation evidence"
        );
        let mut legacy_identity = report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400);
        legacy_identity.event_identity_version = 0;
        assert!(resting_order_invalidation_reason(&candidate, &legacy_identity, now).is_none());
        let mut reset_report = report(strategy::LiveStrategyDecision::Enter, 4_500, 4_400);
        reset_report.episode_id = "episode-2".to_owned();
        assert_eq!(
            resting_order_invalidation_reason(&candidate, &reset_report, now).as_deref(),
            Some("fresh worker report belongs to a new trading thesis episode")
        );
        assert!(
            resting_order_invalidation_reason(
                &candidate,
                &report(strategy::LiveStrategyDecision::Enter, 4_500, 4_400),
                now
            )
            .is_none()
        );
        assert!(
            resting_order_invalidation_reason(
                &candidate,
                &report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400),
                20_000
            )
            .is_none()
        );
        assert!(
            resting_order_invalidation_reason(
                &order(false, 0.0),
                &report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400),
                now
            )
            .is_none()
        );
        let mut stale_report = report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400);
        stale_report.signal.stale = true;
        assert!(resting_order_invalidation_reason(&candidate, &stale_report, now).is_none());
        let mut unrelated_report = report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400);
        unrelated_report.profile_version += 1;
        assert!(resting_order_invalidation_reason(&candidate, &unrelated_report, now).is_none());

        let mut reversed_report = report(strategy::LiveStrategyDecision::Enter, 4_500, 4_400);
        reversed_report.signal.direction = "bearish".to_owned();
        assert_eq!(
            resting_order_invalidation_reason(&candidate, &reversed_report, now).as_deref(),
            Some("fresh signal direction changed from bullish to bearish")
        );
    }

    #[test]
    fn partial_fill_keeps_its_fill_attribution_while_invalid_remainder_is_cancelable() {
        let candidate = order(true, 0.25);
        assert_eq!(candidate.filled_quantity, 0.25);
        assert_eq!(candidate.status, "partially_filled");
        assert!(
            resting_order_invalidation_reason(
                &candidate,
                &report(strategy::LiveStrategyDecision::Reject, 4_500, 4_400),
                5_000
            )
            .is_some()
        );
    }

    #[test]
    fn cancellation_not_found_is_an_idempotent_race_but_conflict_is_not() {
        assert!(is_idempotent_cancel_response(204));
        assert!(is_idempotent_cancel_response(404));
        assert!(!is_idempotent_cancel_response(409));
    }

    #[test]
    fn revalidation_only_selects_post_only_entry_orders() {
        assert!(is_post_only_entry_order("PostOnlyLimit · P0"));
        assert!(!is_post_only_entry_order("Gtd · P0"));
        assert!(!is_post_only_entry_order("Ioc · P0"));
    }
}

#[cfg(test)]
mod submitting_entry_recovery_contract_tests {
    use super::{PersistedEntryRequest, persisted_recovery_parts};
    use serde_json::json;

    fn request(path: &str) -> PersistedEntryRequest {
        PersistedEntryRequest {
            schema_version: 1,
            method: "POST".to_owned(),
            path: path.to_owned(),
            body: json!({
                "source": "auto",
                "client_order_id": "autotrader:v1:opportunity:epoch:1",
                "lifecycle": {"schema_version": 1}
            }),
        }
    }

    #[test]
    fn exact_supported_pm_request_and_context_survive_settings_round_trip() {
        let original = json!({
            "symbol": "BTC-USDT", "strategy": "live-momentum", "direction": "bullish",
            "signal_timestamp_ms": 1200, "profile_id": "profile-a", "profile_version": 1,
            "parameter_hash": "profile-a", "event_identity_version": 1,
            "event_id": "event-a", "episode_id": "episode-a",
            "submission_request": request("/positions/orders"),
            "submission_recovery_context": {"schema_version": 1, "decision_id": "decision-a"}
        });
        let snapshot: super::RestingOrderRevalidationSnapshot =
            serde_json::from_value(original.clone()).unwrap();
        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized["submission_request"], original["submission_request"]);
        assert_eq!(
            serialized["submission_recovery_context"],
            original["submission_recovery_context"]
        );
        let snapshot: super::RestingOrderRevalidationSnapshot =
            serde_json::from_value(serialized).unwrap();
        let (stored_request, context) = persisted_recovery_parts(Some(&snapshot)).unwrap();
        assert_eq!(stored_request.path, "/positions/orders");
        assert_eq!(stored_request.body["client_order_id"], "autotrader:v1:opportunity:epoch:1");
        assert_eq!(context["decision_id"], "decision-a");
    }

    #[test]
    fn legacy_or_unsupported_requests_are_never_replayed() {
        let legacy: super::RestingOrderRevalidationSnapshot = serde_json::from_value(json!({
            "symbol": "BTC-USDT", "strategy": "live-momentum", "direction": "bullish",
            "signal_timestamp_ms": 1200
        })).unwrap();
        assert!(persisted_recovery_parts(Some(&legacy)).is_err());
        assert!(persisted_recovery_parts(None).is_err());

        let unsupported: super::RestingOrderRevalidationSnapshot = serde_json::from_value(json!({
            "symbol": "BTC-USDT", "strategy": "live-momentum", "direction": "bullish",
            "signal_timestamp_ms": 1200,
            "submission_request": {"schema_version": 1, "method": "POST", "path": "/positions/unknown", "body": {}},
            "submission_recovery_context": {"schema_version": 1}
        })).unwrap();
        assert!(persisted_recovery_parts(Some(&unsupported)).is_err());
    }

    #[test]
    fn both_supported_entry_endpoints_carry_a_stable_idempotency_key() {
        for path in ["/positions/orders", "/positions/vwap"] {
            let request = request(path);
            assert!(super::persisted_entry_request_is_replayable(&request));
        }
        let mut missing_key = request("/positions/orders");
        missing_key.body.as_object_mut().unwrap().remove("client_order_id");
        assert!(!super::persisted_entry_request_is_replayable(&missing_key));
    }
}
