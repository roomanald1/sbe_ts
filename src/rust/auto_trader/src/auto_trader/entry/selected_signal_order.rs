struct SelectedSignalOrderRequest<'a> {
    state: &'a AutoTraderState,
    client: &'a reqwest::Client,
    position_url: &'a str,
    dashboard: &'a Dashboard,
    signal: &'a Signal,
    exploratory: bool,
    scout: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    now: u64,
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_cache_lookup_ms: f64,
    market_http_fetch_ms: f64,
}

async fn submit_selected_signal(
    request: SelectedSignalOrderRequest<'_>,
) -> Result<CycleResult, (StatusCode, String)> {
    if is_stablecoin_pair(&request.signal.symbol) {
        return Ok(skipped_for_signal(
            request.signal,
            "Stablecoin-to-stablecoin pairs are excluded from directional and scout entries",
        ));
    }
    let SelectedSignalOrderRequest {
        state,
        client,
        position_url,
        dashboard,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        now,
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    } = request;
    let cycle_guard = acquire_cycle_lock(state, "signal").await;
    let cycle_started = Instant::now();
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
    let decision_id = signal_decision_id(signal);
    let signal_key = signal.symbol.clone();
    let now_ms = now.saturating_mul(1000);
    let settings_lock_started = Instant::now();
    let mut settings = state.settings.lock().await;
    let settings_lock_wait_ms = settings_lock_started.elapsed().as_secs_f64() * 1_000.0;
    reset_daily_budget(&mut settings);
    let quote_rate = quote_usd_rate(&settings, &signal.symbol);
    let strategy_key = signal.strategy.as_deref().unwrap_or("live-insights");
    if let Some(reason) =
        signal_position_block_reason(&settings, signal, &signal_key, strategy_key, now_ms)
    {
        return Ok(skipped_for_signal(signal, &reason));
    }
    let execution_plan_started = Instant::now();
    let plan = match prepare_signal_execution_plan(
        state,
        settings,
        signal,
        dashboard,
        &market,
        now_ms,
        quote_rate,
        exploratory,
        scout,
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
    } = plan;
    // Apply the live spread and estimated sweep cost to every strategy before
    // submitting. Walk-forward returns are already net of taker fees; when no
    // same-side evidence exists, use the configured strategy target and include
    // the estimated taker fee in the cost floor.
    let Some(assessment) = deep_book else {
        return Ok(skipped_for_signal(
            signal,
            "Cannot estimate execution slippage because the order book is unavailable",
        ));
    };
    let spread_bps = (ask - bid) / mid * 10_000.0;
    let side_edge = evidence.and_then(|item| {
        if signal.direction == "bullish" {
            item.all_signal_long.average_net_return_percent
        } else {
            item.all_signal_short.average_net_return_percent
        }
    });
    let (edge_percent, edge_source, fee_bps) =
        if let Some(edge) = side_edge.filter(|v| v.is_finite()) {
            (edge, "measured walk-forward edge", 0.0)
        } else {
            (
                effective_take_profit_percent,
                "strategy take-profit target",
                configured_taker_fee_rate() * 10_000.0,
            )
        };
    let incremental_cost_percent = (spread_bps + 2.0 * assessment.impact_bps + fee_bps) / 100.0;
    let execution_cost_cushion =
        time_adjusted_edge_cost_cushion(now, MIN_EDGE_TO_INCREMENTAL_COST_MULTIPLIER);
    if !edge_percent.is_finite()
        || !incremental_cost_percent.is_finite()
        || edge_percent < incremental_cost_percent * execution_cost_cushion
    {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "{edge_source} ({edge_percent:.3}%) is below the {:.1}x estimated spread, sweep, and fee cost ({:.3}%)",
                execution_cost_cushion,
                incremental_cost_percent * execution_cost_cushion
            ),
        ));
    }
    let deep_book_note = if let Some(assessment) = deep_book {
        if !assessment.impact_bps.is_finite() || assessment.impact_bps > DEEP_BOOK_BAND_BPS {
            return Ok(skipped(&format!(
                "Deep book cannot support a {} {side} order: estimated impact {:.2} bps exceeds {:.0} bps",
                signal.symbol, assessment.impact_bps, DEEP_BOOK_BAND_BPS
            )));
        }
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
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if timestamp.saturating_sub(signal.timestamp) > MAX_SIGNAL_AGE_SECONDS * 1_000 {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Signal became stale before order creation (age {}ms; max {}ms)",
                timestamp.saturating_sub(signal.timestamp),
                MAX_SIGNAL_AGE_SECONDS * 1_000
            ),
        ));
    }
    let mut stage_timings_ms = BTreeMap::new();
    stage_timings_ms.insert("settings_lock_wait".to_owned(), settings_lock_wait_ms);
    stage_timings_ms.insert(
        "execution_plan_local".to_owned(),
        (execution_plan_ms - deep_book_ms).max(0.0),
    );
    stage_timings_ms.insert("signal_selection".to_owned(), signal_selection_ms);
    stage_timings_ms.insert("market_cache_lookup".to_owned(), market_cache_lookup_ms);
    stage_timings_ms.insert("market_http_fetch".to_owned(), market_http_fetch_ms);
    stage_timings_ms.insert("deep_book_local".to_owned(), deep_book_ms);
    stage_timings_ms.insert(
        "signal_queue_wait".to_owned(),
        dashboard.queue_wait_ms.unwrap_or_default(),
    );
    stage_timings_ms.insert(
        "cycle_to_order_record".to_owned(),
        cycle_started.elapsed().as_secs_f64() * 1000.0,
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
    let expires_at = is_post_only.then_some(timestamp.saturating_add(300_000));
    let estimated_ioc_all_in_bps =
        deep_book.map(|assessment| assessment.impact_bps + configured_taker_fee_rate() * 10_000.0);
    let reservation_lock_started = Instant::now();
    let quantity = {
        let mut settings = state.settings.lock().await;
        stage_timings_ms.insert(
            "reservation_lock_wait".to_owned(),
            reservation_lock_started.elapsed().as_secs_f64() * 1_000.0,
        );
        let order_reservation_started = Instant::now();
        reset_daily_budget(&mut settings);
        let allocated = allocated_notional(&settings);
        let category = strategy_category(signal.strategy.as_deref());
        let category_allocated = allocated_notional_for_category(&settings, Some(category));
        let strategy_remaining_budget = Strategy::from_name(strategy_key)
            .map(|kind| match strategy_category(Some(kind.as_str())) {
                StrategyCategory::Daily if !kind.is_live_insight() && !kind.is_dated_carry() => {
                    daily_strategy_effective_budget(&settings, strategy_key)
                        - allocated_notional_for_strategy(&settings, strategy_key)
                }
                StrategyCategory::Intraday if kind.is_live_insight() => {
                    intraday_strategy_effective_budget(&settings, strategy_key)
                        - allocated_notional_for_strategy(&settings, strategy_key)
                }
                _ => f64::INFINITY,
            })
            .unwrap_or(f64::INFINITY);
        let remaining_budget = (settings.daily_notional_limit - allocated)
            .min(category_notional_limit(&settings, category) - category_allocated)
            .min(strategy_remaining_budget)
            .max(0.0);
        notional = notional.min(remaining_budget / stop_headroom_multiplier);
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
            purpose: if scout { "scout" } else { "signal" }.into(),
            strategy: Some(
                signal
                    .strategy
                    .clone()
                    .unwrap_or_else(|| "live-insights".into()),
            ),
            decision_id: Some(decision_id.clone()),
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
    // The pending order and notional reservation are durable-in-memory now.
    // Other cycles can proceed while Position Manager handles the request.
    drop(cycle_guard);
    let submission = SelectedOrderExecution {
        state: state.clone(),
        client: client.clone(),
        position_url: position_url.to_owned(),
        signal: signal.clone(),
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        evidence: evidence.cloned(),
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
