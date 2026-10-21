async fn acquire_cycle_lock(state: &AutoTraderState, worker: &str) -> CycleLockGuard {
    let wait_started = Instant::now();
    let guard = state.cycle_lock.clone().lock_owned().await;
    record_cycle_lock_wait(state, worker, wait_started.elapsed()).await;
    CycleLockGuard {
        _guard: guard,
        hold_samples: state.cycle_lock_hold_ms.clone(),
        worker: worker.to_owned(),
        acquired_at: Instant::now(),
    }
}

async fn record_cycle_lock_wait(state: &AutoTraderState, worker: &str, elapsed: Duration) {
    let mut samples = state.cycle_lock_wait_ms.lock().await;
    let values = samples.entry(worker.to_owned()).or_default();
    values.push(elapsed.as_secs_f64() * 1000.0);
    if values.len() > 500 {
        values.remove(0);
    }
}

fn summarize_lock_wait(values: &[f64]) -> Option<CycleLockWaitSummary> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(CycleLockWaitSummary {
        latest_ms: *values.last()?,
        average_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p95_ms: sorted[(95 * sorted.len()).div_ceil(100).saturating_sub(1)],
        samples: sorted.len(),
    })
}

async fn run_cycle(
    state: &AutoTraderState,
    dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let processing_started = Instant::now();
    if let Err(error) = refresh_usd_rates(state).await {
        crate::service_log!(
            "WARN",
            "USD allocation rates refresh failed; using cached rates: {}",
            error.1
        );
    }
    let result = run_cycle_inner(state, dashboard).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    record_cycle_gap_summary(state, &result);
    let mut message = cycle_decision_message(&result);
    let mut settings = state.settings.lock().await;
    if !settings.last_risk_note.is_empty() {
        message.push_str(" · ");
        message.push_str(&settings.last_risk_note);
    }
    settings.last_decision = message;
    settings.last_decision_at = now;
    // The last decision is informational. Keep it in memory immediately and
    // let the next durable settings write include it, rather than writing the
    // full state file on every signal cycle.
    drop(settings);
    let mut processing_times = state.insight_processing_ms.lock().await;
    processing_times.push(processing_started.elapsed().as_secs_f64() * 1000.0);
    if processing_times.len() > 500 {
        processing_times.remove(0);
    }
    result
}

fn cycle_decision_message(result: &Result<CycleResult, (StatusCode, String)>) -> String {
    match result {
        Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted" => {
            format!(
                "{}: {} {} {}",
                cycle.status,
                cycle.side.as_deref().unwrap_or(""),
                cycle.symbol.as_deref().unwrap_or(""),
                cycle.order_type.as_deref().unwrap_or("")
            )
        }
        Ok(cycle) => cycle
            .reason
            .clone()
            .unwrap_or_else(|| cycle.status.to_owned()),
        Err((_, error)) => format!("Cycle error: {error}"),
    }
}

fn record_cycle_gap_summary(
    state: &AutoTraderState,
    result: &Result<CycleResult, (StatusCode, String)>,
) {
    let traded = matches!(&result, Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted");
    if let Ok(mut summary) = state.gap_summary.lock() {
        let day = utc_day();
        if summary.utc_day != day {
            *summary = GapSummary {
                utc_day: day.clone(),
                ..GapSummary::default()
            };
        }
        if traded {
            *summary = GapSummary {
                utc_day: day,
                ..GapSummary::default()
            };
            if let Ok(cycle) = &result {
                if cycle.status == "order_submitted"
                    && cycle.order_type.as_deref() != Some("Ioc exit")
                {
                    if let (Some(symbol), Some(side)) = (&cycle.symbol, &cycle.side) {
                        let direction = match side.as_str() {
                            "buy" => "BULLISH",
                            "sell" => "BEARISH",
                            other => other,
                        };
                        summary.symbol_gate_hits.insert(
                            (symbol.clone(), direction.to_owned(), "Readiness".to_owned()),
                            SymbolGateCount {
                                count: 0,
                                last_detail:
                                    "All entry, risk, and execution gates passed; order submitted."
                                        .to_owned(),
                                distance_to_pass: Some(0.0),
                            },
                        );
                    }
                }
            }
        } else {
            summary
                .symbol_gate_hits
                .retain(|(_, _, gate), _| gate != "Readiness");
            summary.decisions = summary.decisions.saturating_add(1);
            let raw_reason = match &result {
                Ok(cycle) => cycle.reason.as_deref().unwrap_or(cycle.status),
                Err((_, error)) => error.as_str(),
            };
            let reason = gap_reason_category(raw_reason);
            *summary.reasons.entry(reason).or_default() += 1;
            for gate in gate_hits_for_reason(raw_reason) {
                *summary.gate_hits.entry(gate).or_default() += 1;
            }
            for (symbol, direction, gate, detail, distance_to_pass) in
                symbol_gate_hits_for_reason(raw_reason)
            {
                let entry = summary
                    .symbol_gate_hits
                    .entry((symbol, direction, gate))
                    .or_default();
                entry.count = entry.count.saturating_add(1);
                entry.last_detail = detail;
                entry.distance_to_pass = distance_to_pass;
            }
            if let Ok(cycle) = &result {
                if let (Some(symbol), Some(direction)) = (&cycle.symbol, &cycle.side) {
                    for gate in gate_hits_for_reason(raw_reason) {
                        let distance_to_pass = gate_distance_to_pass(&gate, raw_reason);
                        let entry = summary
                            .symbol_gate_hits
                            .entry((symbol.clone(), direction.clone(), gate))
                            .or_default();
                        entry.count = entry.count.saturating_add(1);
                        entry.last_detail = raw_reason.to_owned();
                        entry.distance_to_pass = distance_to_pass;
                    }
                }
            }
            summary.latest_detail = raw_reason.to_owned();
        }
    }

}

async fn run_position_maintenance(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
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
    #[serde(default)]
    fees_paid: f64,
}

async fn sync_auto_positions(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let ledger_pnl = fetch_ledger_realized_pnl(client, base).await?;
    let realized_all_time_usd = ledger_pnl.realized_pnl_all_time_usd.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "All-time realized P&L is unavailable from the position ledger because an FX rate is missing".into(),
    ))?;
    let realized_today_usd = ledger_pnl.realized_pnl_usd.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Daily realized P&L is unavailable from the position ledger because an FX rate is missing"
            .into(),
    ))?;
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
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    let mut source_symbols = std::collections::BTreeSet::new();
    for source_position in positions {
        if !source_position.quantity.is_finite()
            || !source_position.average_entry_price.is_finite()
            || !source_position.realized_pnl.is_finite()
        {
            continue;
        }
        source_symbols.insert(source_position.symbol.clone());
        let (prior_quantity, prior_average_entry_price, realized_delta, learning_key) = {
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
            (
                prior_quantity,
                prior_average_entry_price,
                realized_delta,
                learning_key,
            )
        };
        if realized_delta.is_finite() && realized_delta.abs() > 1e-12 {
            // Manual position-manager fills bypass apply_auto_fill. Use the
            // ledger delta for close-out learning; account totals are mirrored below.
            let realized_delta_usd =
                realized_usd(&settings, &source_position.symbol, realized_delta);
            changed = true;
            if prior_quantity.abs() > 1e-12 && source_position.quantity.abs() <= 1e-12 {
                let entry_notional = prior_quantity.abs() * prior_average_entry_price;
                if entry_notional > 0.0 {
                    let fees_usd = realized_usd(
                        &settings,
                        &source_position.symbol,
                        source_position.fees_paid.max(0.0),
                    );
                    record_closed_trade_outcome(
                        &mut settings,
                        realized_delta / entry_notional * 100.0,
                        realized_delta_usd,
                        fees_usd,
                    );
                    adapt_exit_policy(&mut settings);
                }
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
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(())
}

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
        orders, positions, ..
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
            let (realized, learned) = apply_auto_fill(
                positions,
                &order.symbol,
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                "PostOnlyLimit",
                order.learning_context.as_deref().unwrap_or(&order.symbol),
                order.horizon_take_profit_percent,
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

async fn reconcile_scheduled_fills(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let is_reconcilable = |order: &&TraderOrder| {
        (order.order_type.starts_with("Twap") || order.order_type.starts_with("Vwap"))
            && (matches!(
                order.status.as_str(),
                "accepted" | "partially_filled" | "resting"
            ) || (order.status == "failed"
                && order.failure_reason.as_deref().is_some_and(|reason| {
                    reason.contains("Scheduled run is missing from the position manager")
                })))
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
            learning,
            realized_pnl,
            daily_realized_pnl,
            ..
        } = &mut *settings;
        for order in orders.iter_mut().filter(|order| {
            (order.order_type.starts_with("Twap") || order.order_type.starts_with("Vwap"))
                && (matches!(
                    order.status.as_str(),
                    "accepted" | "partially_filled" | "resting"
                ) || (order.status == "failed"
                    && order.failure_reason.as_deref().is_some_and(|reason| {
                        reason.contains("Scheduled run is missing from the position manager")
                    })))
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
                let (realized, learned) = apply_auto_fill(
                    positions,
                    &order.symbol,
                    &order.side,
                    qty,
                    price,
                    fee,
                    "ScheduledExecution",
                    order.learning_context.as_deref().unwrap_or(&order.symbol),
                    order.horizon_take_profit_percent,
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
                    let (realized, learned) = apply_auto_fill(
                        positions,
                        &order.symbol,
                        &order.side,
                        delta_quantity,
                        price,
                        fee,
                        "ScheduledExecution",
                        order.learning_context.as_deref().unwrap_or(&order.symbol),
                        order.horizon_take_profit_percent,
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
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(())
}

async fn run_hedge_evaluation(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    let _ = maybe_place_portfolio_hedge(&client, &position_url, state).await?;
    Ok(())
}

async fn fresh_subscribed_market_book(
    client: &reqwest::Client,
    position_url: &str,
    symbol: &str,
) -> Option<serde_json::Value> {
    // The GET handler ensures the subscription itself, so a separate POST here
    // only adds a network round trip before the first book poll.
    let market_url = format!("{position_url}/internal/market-book?symbol={symbol}");
    for _ in 0..4 {
        if let Ok(response) = client
            .get(&market_url)
            .timeout(Duration::from_millis(500))
            .send()
            .await
            && let Ok(response) = response.error_for_status()
            && let Ok(market) = response.json::<serde_json::Value>().await
        {
            let updated_at = market
                .get("updated_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            if updated_at > 0 && now_ms.saturating_sub(updated_at) <= 5_000 {
                return Some(market);
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    None
}
