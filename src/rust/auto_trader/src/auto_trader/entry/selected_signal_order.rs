struct SelectedSignalOrderRequest<'a> {
    state: &'a AutoTraderState,
    dashboard: &'a Dashboard,
    signal: &'a Signal,
    exploratory: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    context_threshold_refresh_ms: f64,
    cycle_started_at: Instant,
    now: u64,
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_cache_lookup_ms: f64,
    market_http_fetch_ms: f64,
}

fn portfolio_reduction_quantity(settings: &Settings, symbol: &str, strategy: &str) -> f64 {
    settings.strategy_positions.get(symbol).and_then(|lots| lots.get(strategy))
        .or_else(|| settings.positions.get(symbol).filter(|position| {
            settings.strategy_positions.is_empty() && position.strategy.as_deref() == Some(strategy)
        }))
        .map(|position| position.quantity.max(0.0))
        .unwrap_or(0.0)
}

struct SignalSettingsSnapshot {
    settings: Settings,
    quote_rate: f64,
    settings_lock_wait_ms: f64,
    ioc_fallback_for: Option<u64>,
}

async fn snapshot_signal_settings(
    state: &AutoTraderState,
    signal: &Signal,
    now_ms: u64,
) -> Result<SignalSettingsSnapshot, String> {
    let signal_key = signal.symbol.as_str();
    let settings_lock_started = Instant::now();
    let mut settings = state.settings.lock().await;
    let settings_lock_wait = settings_lock_started.elapsed().as_secs_f64() * 1_000.0;
    record_cycle_lock_scope(state, "signal_settings_lock_wait", settings_lock_started.elapsed());
    reset_daily_budget(&mut settings);
    let quote_rate = quote_usd_rate(&settings, signal_key);
    let strategy_key = signal.strategy.as_deref().unwrap_or("unattributed");
    let block_reason = signal_position_block_reason(&settings, signal, signal_key, strategy_key, now_ms);
    let ioc_fallback_for = match block_reason {
        Some(reason) if reason.starts_with("Entry already pending") => {
            pending_maker_entry_for_signal(&settings, signal, signal_key, strategy_key, now_ms)
                .filter(|(_, strength)| signal.strength.is_finite() && signal.strength > *strength)
                .map(|(timestamp, _)| timestamp)
                .map(Some)
                .ok_or(reason)
        }
        Some(reason) => Err(reason),
        None => Ok(None),
    }?;
    if let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) {
        let profile = learned_strategy_parameter_profile(&settings, signal_key, strategy);
        queue_strategy_profile_registration(&mut settings, signal_key, strategy, &profile);
    }
    let snapshot = settings.clone();
    drop(settings);
    Ok(SignalSettingsSnapshot {
        settings: snapshot,
        quote_rate,
        settings_lock_wait_ms: settings_lock_wait,
        ioc_fallback_for,
    })
}

async fn submit_selected_signal(
    request: SelectedSignalOrderRequest<'_>,
) -> Result<CycleResult, (StatusCode, String)> {
    if is_stablecoin_pair(&request.signal.symbol) {
        return Ok(skipped_for_signal(
            request.signal,
            "Stablecoin-to-stablecoin pairs are excluded from directional entries",
        ));
    }
    let SelectedSignalOrderRequest {
        state,
        dashboard,
        signal,
        exploratory,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        context_threshold_refresh_ms,
        cycle_started_at,
        now,
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    } = request;
    // The settings mutex serializes the gate/profile update and snapshot below.
    // Don't also hold the shared cycle lock while scanning and cloning Settings:
    // order reservation rechecks the gate under the settings mutex before writing.
    let decision_id = signal_decision_id(signal);
    let now_ms = now.saturating_mul(1000);
    let SignalSettingsSnapshot {
        settings: settings_snapshot,
        quote_rate,
        settings_lock_wait_ms,
        ioc_fallback_for,
    } = match snapshot_signal_settings(state, signal, now_ms).await {
        Ok(snapshot) => snapshot,
        Err(reason) => return Ok(skipped_for_signal(signal, &reason)),
    };
    let recent_exploratory_exit_fee = signal
        .strategy
        .as_deref()
        .and_then(|strategy| {
            recent_exploratory_exit_fee_rate(
                &settings_snapshot,
                signal.symbol.as_str(),
                strategy,
                now_ms,
            )
        });
    let signal_key = signal.symbol.clone();
    let strategy_key = signal.strategy.as_deref().unwrap_or("unattributed");
    let execution_plan_started = Instant::now();
    let plan = match prepare_signal_execution_plan(
        state,
        settings_snapshot,
        signal,
        dashboard,
        &market,
        now_ms,
        quote_rate,
        exploratory,
        ioc_fallback_for.is_some(),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    )
    .await
    {
        Ok(plan) => plan,
        Err(reason) => return Ok(skipped_for_signal(signal, &reason)),
    };
    let execution_plan_ms = execution_plan_started.elapsed().as_secs_f64() * 1_000.0;
    let SignalExecutionPlan {
        effective_stop_loss_percent,
        effective_take_profit_percent,
        horizon_take_profit_percent,
        mut notional,
        minimum_book_order,
        stop_headroom_multiplier,
        deep_book,
        deep_book_ms,
        side,
        order_type,
        profile,
        execution_params,
        learning_context,
        execution_learning_ms,
        imported_execution_learning,
        exploration_risk_adjustment,
    } = plan;
    if exploratory || imported_execution_learning {
        let mut settings = state.settings.lock().await;
        if exploratory {
            if !settings.last_risk_note.is_empty() {
                settings.last_risk_note.push_str(" · ");
            }
            settings.last_risk_note.push_str(
                "small paper-exploration size; no reliable same-side walk-forward edge yet",
            );
        }
        if imported_execution_learning {
            settings.execution_learning_imported = true;
        }
    }
    // Apply the live spread and estimated sweep cost before submitting.
    // Strategy-specific Scout returns already include their modeled fees.
    let Some(assessment) = deep_book else {
        return Ok(skipped_for_signal(
            signal,
            "Cannot estimate execution slippage because the order book is unavailable",
        ));
    };
    // `assess_deep_book` uses infinity when the visible book cannot fill the
    // planned order. Reject that condition before calculating/formatting the
    // spread-and-sweep threshold, otherwise the UI reports a misleading
    // `inf%` execution cost instead of the actual book-capacity failure.
    if !assessment.impact_bps.is_finite() {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Deep book cannot fill a {} {side} order: available capacity {:.2} USD; minimum order {:.2} USD",
                signal.symbol,
                assessment.notional_cap / quote_rate,
                minimum_book_order
            ),
        ));
    }
    if assessment.impact_bps > DEEP_BOOK_BAND_BPS {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Deep book cannot support a {} {side} order: estimated impact {:.2} bps exceeds {:.0} bps",
                signal.symbol, assessment.impact_bps, DEEP_BOOK_BAND_BPS
            ),
        ));
    }
    let spread_bps = (ask - bid) / mid * 10_000.0;
    let strategy_edge = signal.strategy.as_deref().and_then(Strategy::from_name)
        .filter(|kind| !kind.is_dated_carry())
        .and_then(|kind| validated_strategy_entry_edge(signal, kind).ok());
    let contextual_net_edge = exploratory_contextual_net_edge(&exploration_risk_adjustment);
    let reducing_rebalance = signal.direction == "bearish"
        && is_portfolio_rebalance_signal(signal, signal.strategy.as_deref().and_then(Strategy::from_name));
    let (edge_percent, edge_source, fee_bps, fee_assumption, edge_is_net) =
        if let Some(edge) = strategy_edge {
            (
                edge,
                "measured strategy Scout net edge",
                0.0,
                "walk-forward net edge already includes execution costs".to_owned(),
                true,
            )
        } else if let Some(edge) = contextual_net_edge {
            (
                edge,
                "recent contextual net expectancy lower bound",
                0.0,
                "realized contextual net returns already include execution costs".to_owned(),
                true,
            )
        } else if exploratory {
            let (fee_bps, fee_assumption) = recent_exploratory_exit_fee
                .map(|(exit_fee_rate, samples)| {
                    (
                        (configured_maker_fee_rate() + exit_fee_rate) * 10_000.0,
                        format!("exploratory: maker entry; observed exit fee mix from {samples} completed trades"),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        exploratory_round_trip_fee_bps(
                            configured_maker_fee_rate(),
                            configured_taker_fee_rate(),
                        ),
                        "exploratory: maker entry; 50% maker / 50% taker exit prior".to_owned(),
                    )
                });
            (
                effective_take_profit_percent,
                "strategy take-profit payoff ceiling (not a measured edge)",
                fee_bps,
                fee_assumption,
                false,
            )
        } else {
            (
                effective_take_profit_percent,
                "strategy take-profit payoff ceiling (not a measured edge)",
                2.0 * configured_taker_fee_rate() * 10_000.0,
                "strict/urgent: taker entry and taker exit".to_owned(),
                false,
            )
        };
    let incremental_cost_percent = (spread_bps + 2.0 * assessment.impact_bps + fee_bps) / 100.0;
    let execution_cost_cushion =
        time_adjusted_edge_cost_cushion(now, MIN_EDGE_TO_INCREMENTAL_COST_MULTIPLIER);
    let edge_cost_threshold = if edge_is_net {
        0.0
    } else {
        incremental_cost_percent * execution_cost_cushion
    };
    if !reducing_rebalance && (!edge_percent.is_finite()
        || !incremental_cost_percent.is_finite()
        || edge_percent <= edge_cost_threshold)
    {
        let rejection_reason = if edge_is_net {
            format!(
                "{edge_source} ({edge_percent:.3}%) is not positive after fees and execution costs ({fee_assumption})"
            )
        } else {
            format!(
                "{edge_source} ({edge_percent:.3}%) is below the {:.1}x estimated spread, sweep, and fee cost ({edge_cost_threshold:.3}%; fee assumption: {fee_assumption}, maker {:.2} bps, taker {:.2} bps)",
                execution_cost_cushion,
                configured_maker_fee_rate() * 10_000.0,
                configured_taker_fee_rate() * 10_000.0,
            )
        };
        return Ok(skipped_for_signal(
            signal,
            &rejection_reason,
        ));
    }
    let deep_book_note = if let Some(assessment) = deep_book {
        if !matches!(order_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
            notional = notional.min(assessment.notional_cap / quote_rate);
        }
        if notional < minimum_book_order {
            return Ok(skipped(&format!(
                "Deep-book capacity for {} ({:.2} USD) is below the minimum paper order ({:.2} USD)",
                signal.symbol,
                assessment.notional_cap / quote_rate,
                minimum_book_order
            )));
        }
        Some(format!(
            "deep book: {:.0} quote liquidity within {:.0} bps; order cap {:.0} at {:.0}% participation; estimated sweep {:.2} bps",
            assessment.available_within_band,
            DEEP_BOOK_BAND_BPS,
            assessment.notional_cap,
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            assessment.impact_bps
        ))
    } else {
        None
    };
    if let Some(note) = deep_book_note {
        let mut settings = state.settings.lock().await;
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings.last_risk_note.push_str(&note);
    }
    let now_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let timestamp = next_unique_order_timestamp(now_timestamp);
    let max_signal_age_ms = signal_max_age_seconds(signal) * 1_000;
    if timestamp.saturating_sub(signal.timestamp) > max_signal_age_ms {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Signal became stale before order creation (age {}ms; max {}ms)",
                timestamp.saturating_sub(signal.timestamp),
                max_signal_age_ms
            ),
        ));
    }
    let mut stage_timings_ms = BTreeMap::new();
    stage_timings_ms.insert("settings_lock_wait".to_owned(), settings_lock_wait_ms);
    stage_timings_ms.insert(
        "execution_plan_local".to_owned(),
        (execution_plan_ms - deep_book_ms - execution_learning_ms).max(0.0),
    );
    stage_timings_ms.insert("execution_learning_lookup".to_owned(), execution_learning_ms);
    stage_timings_ms.insert("signal_selection_total".to_owned(), signal_selection_ms);
    stage_timings_ms.insert(
        "signal_selection_local".to_owned(),
        (signal_selection_ms - context_threshold_refresh_ms).max(0.0),
    );
    stage_timings_ms.insert(
        "context_threshold_refresh".to_owned(),
        context_threshold_refresh_ms,
    );
    stage_timings_ms.insert("market_cache_lookup".to_owned(), market_cache_lookup_ms);
    stage_timings_ms.insert("market_http_fetch".to_owned(), market_http_fetch_ms);
    stage_timings_ms.insert("deep_book_local".to_owned(), deep_book_ms);
    stage_timings_ms.insert(
        "signal_queue_wait".to_owned(),
        dashboard.queue_wait_ms.unwrap_or_default(),
    );
    stage_timings_ms.insert(
        "cycle_to_order_record".to_owned(),
        cycle_started_at.elapsed().as_secs_f64() * 1000.0,
    );
    stage_timings_ms.insert(
        "signal_age_at_order_record".to_owned(),
        timestamp.saturating_sub(signal.timestamp) as f64,
    );
    let is_post_only = matches!(order_type, AutoOrderType::PostOnlyLimit);
    let submitted_limit_price = is_post_only.then_some(if side == "buy" { bid } else { ask });
    let limit_to_mid_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let limit_to_crossing_quote_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (ask - price) / mid * 10_000.0
        } else {
            (price - bid) / mid * 10_000.0
        }
    });
    let expires_at = is_post_only.then_some(timestamp.saturating_add(execution_params.post_only_timeout_minutes * 60_000));
    let estimated_ioc_all_in_bps =
        deep_book.map(|assessment| assessment.impact_bps + configured_taker_fee_rate() * 10_000.0);
    let cycle_guard = acquire_cycle_lock(state, "signal").await;
    let reservation_cycle_lock_started = Instant::now();
    let quantity = {
        let reservation_lock_started = Instant::now();
        let mut settings = state.settings.lock().await;
        stage_timings_ms.insert(
            "reservation_lock_wait".to_owned(),
            reservation_lock_started.elapsed().as_secs_f64() * 1_000.0,
        );
        record_cycle_lock_scope(
            state,
            "signal_reservation_settings_lock_wait",
            reservation_lock_started.elapsed(),
        );
        let order_reservation_started = Instant::now();
        reset_daily_budget(&mut settings);
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name);
        if !paper_learning_exploration_active(&settings, now) && !exploratory {
            if let Some(reason) = learned_pause_block_reason(
            &settings,
            strategy,
            signal,
            reducing_rebalance,
            &signal.direction,
            now,
            ) {
                return Ok(skipped_for_signal(signal, &format!("Strategy entry paused: {reason}")));
            }
        }
        let current_pending_maker = pending_maker_entry_for_signal(
            &settings,
            signal,
            &signal.symbol,
            strategy_key,
            now_ms,
        );
        if ioc_fallback_for.is_some()
            && !current_pending_maker.is_some_and(|(timestamp, strength)| {
                Some(timestamp) == ioc_fallback_for
                    && signal.strength.is_finite()
                    && signal.strength > strength
            })
        {
            return Ok(skipped_for_signal(
                signal,
                "Resting maker entry resolved before the stronger-signal IOC fallback",
            ));
        }
        let block_reason = signal_position_block_reason(&settings, signal, &signal.symbol, strategy_key, now_ms);
        if let Some(reason) = block_reason {
            let fallback_still_valid = ioc_fallback_for.is_some_and(|pending_timestamp| {
                reason.starts_with("Entry already pending")
                    && current_pending_maker.is_some_and(|(timestamp, strength)| {
                        timestamp == pending_timestamp
                            && signal.strength.is_finite()
                            && signal.strength > strength
                    })
            });
            if !fallback_still_valid {
                return Ok(skipped_for_signal(signal, &reason));
            }
        }
        let allocated = allocated_notional(&settings);
        let category = strategy_category(signal.strategy.as_deref());
        let category_allocated = allocated_notional_for_category(&settings, Some(category));
        // Cap gross open and pending exposure for this symbol across all strategies.
        let symbol_remaining_budget = settings.max_order_notional
            - allocated_notional_for_symbol(&settings, &signal.symbol);
        let strategy_remaining_budget = Strategy::from_name(strategy_key)
            .map(|kind| match strategy_category(Some(kind.as_str())) {
                StrategyCategory::Daily if !kind.is_live_insight() && !kind.is_dated_carry() => {
                    signal_weighted_strategy_budget(&settings, dashboard, kind, StrategyCategory::Daily, now_ms)
                        - allocated_notional_for_strategy(&settings, strategy_key)
                }
                StrategyCategory::Intraday if kind.is_live_insight() => {
                    signal_weighted_strategy_budget(&settings, dashboard, kind, StrategyCategory::Intraday, now_ms)
                        - allocated_notional_for_strategy(&settings, strategy_key)
                }
                _ => f64::INFINITY,
            })
            .unwrap_or(f64::INFINITY);
        let remaining_budget = if reducing_rebalance { f64::INFINITY } else { (settings.daily_notional_limit - allocated)
            .min(category_notional_limit(&settings, category) - category_allocated)
            .min(strategy_remaining_budget)
            .min(symbol_remaining_budget)
            .max(0.0) };
        notional = notional.min(remaining_budget / stop_headroom_multiplier);
        if reducing_rebalance {
            notional = notional.min(portfolio_reduction_quantity(&settings, &signal.symbol, strategy_key) * mid * quote_rate);
        }
        if notional < minimum_book_order {
            return Ok(skipped_for_signal(
                signal,
                if stop_headroom_multiplier > 1.0 {
                    "Insufficient notional headroom for both the scheduled entry and its protective stop"
                } else {
                    "Remaining daily notional budget is below the minimum order size"
                },
            ));
        }
        let quantity = notional / (mid * quote_rate);
        stage_timings_ms.insert(
            "order_reservation_record_local".to_owned(),
            order_reservation_started.elapsed().as_secs_f64() * 1_000.0,
        );
        settings.used_notional_today = allocated;
        settings
            .last_signal_at
            .insert(signal_key.clone(), timestamp);
        settings
            .last_signal_direction
            .insert(signal_key.clone(), signal.direction.clone());
        settings
            .last_signal_strength
            .insert(signal_key, signal.strength);
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: signal.symbol.clone(),
            side: side.to_owned(),
            order_type: format!("{order_type:?} · P{profile}"),
            purpose: "signal".into(),
            exploratory,
            strategy: Some(
                signal
                    .strategy
                    .clone()
                    .unwrap_or_else(|| "unattributed".into()),
            ),
            decision_id: Some(decision_id.clone()),
            strength: signal.strength,
            ioc_fallback_for,
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: notional,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: Some(learning_context.clone()),
            book_bid_depth_10bps: deep_book.map(|assessment| assessment.bid_depth_10bps),
            book_ask_depth_10bps: deep_book.map(|assessment| assessment.ask_depth_10bps),
            book_depth_imbalance_10bps: deep_book
                .map(|assessment| assessment.depth_imbalance_10bps),
            estimated_sweep_bps: deep_book.map(|assessment| assessment.impact_bps),
            estimated_ioc_all_in_bps,
            decision_bid: Some(bid),
            decision_ask: Some(ask),
            decision_mid: Some(mid),
            submitted_limit_price,
            target_price: None,
            horizon_take_profit_percent,
            limit_to_mid_bps,
            limit_to_crossing_quote_bps,
            expires_at,
            failure_reason: None,
            stage_timings_ms: stage_timings_ms.clone(),
            status: "submitting".to_owned(),
            realized_pnl: 0.0,
            strategy_realized_pnl: None,
            strategy_realized_pnl_today: None,
            strategy_realized_pnl_day: None,
        });
        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
            settings.orders.drain(..excess);
        }
        let settings_persist_started = Instant::now();
        request_settings_persist(state)?;
        stage_timings_ms.insert(
            "order_record_persist_notify".to_owned(),
            settings_persist_started.elapsed().as_secs_f64() * 1_000.0,
        );
        quantity
    };
    record_cycle_lock_scope(
        state,
        "signal_order_reservation",
        reservation_cycle_lock_started.elapsed(),
    );
    // The pending order and notional reservation are durable-in-memory now.
    // Other cycles can proceed while Position Manager handles the request.
    drop(cycle_guard);
    let submission = SelectedOrderExecution {
        state: state.clone(),
        signal: signal.clone(),
        exploratory,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        side: side.to_owned(),
        quantity,
        notional,
        bid,
        ask,
        mid,
        order_type,
        execution_params,
        timestamp,
        decision_id: decision_id.clone(),
        learning_context: learning_context.clone(),
        exploration_risk_adjustment,
        horizon_take_profit_percent,
        stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps,
        submitted_limit_price,
        limit_to_mid_bps,
        limit_to_crossing_quote_bps,
        expires_at,
    };
    let worker_state = state.clone();
    tokio::spawn(async move {
        match submit_and_record_selected_order(submission).await {
            Ok(result) if result.status == "order_submitted" => {
                record_cycle_gap_summary(&worker_state, &Ok(result));
            }
            Ok(_) => {}
            Err((status, error)) => crate::service_log!(
                "ERROR",
                "Asynchronous signal order submission failed ({status}): {error}"
            ),
        }
    });
    Ok(CycleResult {
        status: "order_submitting",
        reason: Some("Order reserved locally and queued for Position Manager submission".into()),
        symbol: Some(signal.symbol.clone()),
        side: Some(side.to_owned()),
        quantity: Some(quantity),
        order_type: Some(format!("{order_type:?}")),
        order_response: None,
        filled_quantity: None,
        entry_diagnostic: None,
    })
}

// Lower-activity calendar periods get a larger measured-edge cushion. Calendar
// time changes execution caution only and never creates a directional signal.
fn time_adjusted_edge_cost_cushion(now_seconds: u64, base: f64) -> f64 {
    let days_since_epoch = now_seconds / 86_400;
    let weekday = (days_since_epoch + 4) % 7; // Sunday = 0, Saturday = 6
    let hour_utc = (now_seconds % 86_400) / 3_600;
    if weekday == 0 || weekday == 6 || hour_utc < 8 {
        base.max(2.0)
    } else {
        base
    }
}

fn exploratory_contextual_net_edge(
    adjustment: &ExplorationRiskAdjustment,
) -> Option<f64> {
    const MIN_CONTEXT_EDGE_SAMPLES: usize = 6;
    let samples = adjustment.sample_count;
    let lower_bound = adjustment.weighted_net_return_lcb_percent?;
    if samples < MIN_CONTEXT_EDGE_SAMPLES || !lower_bound.is_finite() {
        return None;
    }
    Some(lower_bound)
}

fn recent_exploratory_exit_fee_rate(
    settings: &Settings,
    symbol: &str,
    strategy: &str,
    now_ms: u64,
) -> Option<(f64, usize)> {
    const FEE_HISTORY_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
    const MIN_COMPLETED_TRADES: usize = 5;
    let mut completed_trades = BTreeSet::new();
    let mut total_notional = 0.0;
    let mut weighted_fee = 0.0;
    for order in &settings.orders {
        if !order.exploratory
            || order.symbol != symbol
            || order.strategy.as_deref() != Some(strategy)
            || order.purpose == "signal"
            || !matches!(
                order.status.as_str(),
                "filled" | "partially_filled_closed" | "expired_or_cancelled"
            )
            || order.timestamp > now_ms
            || now_ms.saturating_sub(order.timestamp) > FEE_HISTORY_WINDOW_MS
        {
            continue;
        }
        let Some(decision_id) = order.decision_id.as_deref() else {
            continue;
        };
        let Some(notional) = order
            .average_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .map(|price| order.filled_quantity * price)
            .filter(|notional| notional.is_finite() && *notional > 0.0)
        else {
            continue;
        };
        let fee_rate = if order.order_type.starts_with("PostOnlyLimit")
            || order.order_type.starts_with("Gtd")
        {
            configured_maker_fee_rate()
        } else {
            configured_taker_fee_rate()
        };
        completed_trades.insert(decision_id.to_owned());
        total_notional += notional;
        weighted_fee += notional * fee_rate;
    }
    if completed_trades.len() < MIN_COMPLETED_TRADES || total_notional <= 0.0 {
        return None;
    }
    Some((weighted_fee / total_notional, completed_trades.len()))
}

fn exploratory_round_trip_fee_bps(maker_rate: f64, taker_rate: f64) -> f64 {
    // Use a balanced prior until this strategy/symbol has enough completed
    // exploratory exit paths to estimate its observed mix.
    (maker_rate + (maker_rate + taker_rate) / 2.0) * 10_000.0
}

#[cfg(test)]
mod selected_signal_order_fee_tests {
    use super::*;

    fn exploratory_exit_order(
        timestamp: u64,
        decision_id: &str,
        order_type: &str,
        quantity: f64,
        average_price: f64,
    ) -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": timestamp,
            "symbol": "BTC-USDT",
            "side": "sell",
            "order_type": order_type,
            "purpose": "take-profit",
            "exploratory": true,
            "strategy": "live-momentum",
            "decision_id": decision_id,
            "quantity": quantity,
            "filled_quantity": quantity,
            "average_price": average_price,
            "status": "filled",
            "realized_pnl": 0.0
        }))
        .unwrap()
    }

    #[test]
    fn exploratory_fee_uses_maker_entry_and_mixed_exit_paths() {
        assert!((exploratory_round_trip_fee_bps(0.0008, 0.001) - 17.0).abs() < 1e-9);
    }

    #[test]
    fn immediate_stop_loss_exit_costs_taker_fee() {
        let maker_entry_and_stop_exit_bps: f64 = (0.0008 + 0.001) * 10_000.0;
        assert!((maker_entry_and_stop_exit_bps - 18.0).abs() < 1e-9);
        assert!(maker_entry_and_stop_exit_bps > 2.0 * 0.0008 * 10_000.0);
    }

    #[test]
    fn strict_or_urgent_path_keeps_two_taker_fees_and_measured_edge_adds_none() {
        let strict_taker_fee_bps = 2.0 * configured_taker_fee_rate() * 10_000.0;
        let measured_edge_fee_bps = 0.0;
        assert!((strict_taker_fee_bps - 20.0).abs() < 1e-9);
        assert_eq!(measured_edge_fee_bps, 0.0);
    }

    #[test]
    fn contextual_exploration_net_edge_uses_lower_bound_and_requires_six_samples() {
        let adjustment = ExplorationRiskAdjustment {
            scale: 1.0,
            sample_count: 6,
            weighted_net_return_percent: Some(0.6),
            weighted_net_return_lcb_percent: Some(0.4),
            regime_scoped: false,
        };
        assert!(
            (exploratory_contextual_net_edge(&adjustment).unwrap() - 0.4).abs() < 1e-12
        );

        let insufficient = ExplorationRiskAdjustment {
            sample_count: 5,
            ..adjustment
        };
        assert_eq!(exploratory_contextual_net_edge(&insufficient), None);
    }

    #[test]
    fn recent_exit_fee_estimate_uses_completed_strategy_exit_mix() {
        let now = 10 * 24 * 60 * 60 * 1_000;
        let mut settings = Settings::default();
        for (index, order_type, notional) in [
            (0, "PostOnlyLimit · exit", 1.0),
            (1, "Ioc", 3.0),
            (2, "Ioc", 3.0),
            (3, "Ioc", 3.0),
            (4, "Ioc", 3.0),
        ] {
            settings.orders.push(exploratory_exit_order(
                now - index * 1_000,
                &format!("decision-{index}"),
                order_type,
                notional / 100.0,
                100.0,
            ));
        }
        let (exit_fee_rate, sample_count) = recent_exploratory_exit_fee_rate(
            &settings,
            "BTC-USDT",
            "live-momentum",
            now,
        )
        .unwrap();
        assert_eq!(sample_count, 5);
        assert!((exit_fee_rate - (0.0128 / 13.0)).abs() < 1e-12);
    }
}
