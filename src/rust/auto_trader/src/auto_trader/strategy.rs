async fn run_cycle_inner(
    state: &AutoTraderState,
    mut dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let cycle_started = Instant::now();
    let _cycle = acquire_cycle_lock(state, "signal").await;
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note.clear();
        settings.risk_limited_sizing = false;
    }
    *state.latest_missed_opportunity_bands.lock().await =
        dashboard.missed_opportunity_bands.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "System clock is before Unix epoch".into(),
            )
        })?
        .as_secs();
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    if let Some(result) = enforce_daily_realized_loss_limit(state, &client, &position_url).await? {
        return Ok(result);
    }
    let dashboard_ms = 0.0;
    let regime_now_ms = now.saturating_mul(1000);
    let has_generic_signals = !dashboard.best.is_empty();
    let (generic_signals_enabled, research_strategies_enabled) = {
        let settings = state.settings.lock().await;
        (
            settings.generic_signals_enabled,
            !settings.research_strategies.is_empty(),
        )
    };
    if has_generic_signals {
        if !generic_signals_enabled {
            return Ok(CycleResult {
                status: "skipped",
                reason: Some("Generic insight strategy is disabled".into()),
                symbol: None,
                side: None,
                quantity: None,
                order_type: None,
                order_response: None,
            });
        }
    } else if research_strategies_enabled {
        let research_regime = match load_research_regime_snapshot(state, regime_now_ms).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Ok(CycleResult {
                    status: "skipped",
                    reason: Some(format!(
                        "Research daily market data unavailable; no entry submitted: {error}"
                    )),
                    symbol: None,
                    side: None,
                    quantity: None,
                    order_type: None,
                    order_response: None,
                })
            }
        };
        dashboard.best = research_regime.generated_signals.clone();
        if dashboard.best.is_empty() {
            return Ok(CycleResult {
                status: "skipped",
                reason: Some(research_regime.empty_signal_reason().into()),
                symbol: None,
                side: None,
                quantity: None,
                order_type: None,
                order_response: None,
            });
        }
    }
    if dashboard.best.is_empty() {
        return Ok(CycleResult {
            status: "skipped",
            reason: Some("No generic or daily strategy signals are enabled".into()),
            symbol: None,
            side: None,
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    let selection_started = Instant::now();
    let (
        learning,
        risk_appetite,
        active_scout_count,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    ) = {
        let settings = state.settings.lock().await;
        (
            {
                let mut learning = settings.learning.clone();
                decay_learning_map(&mut learning, current_time_ms());
                learning
            },
            settings.risk_appetite,
            settings
                .positions
                .values()
                .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
                .count(),
            settings.stop_loss_percent,
            settings.take_profit_percent,
        )
    };
    let PreparedSignals {
        signals,
        inferred_direction_count,
        neutral_direction_floor,
    } = prepare_directional_signal_candidates(
        state,
        &dashboard,
        now,
        risk_appetite,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    );
    let (open_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .positions
                .iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| (symbol.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>(),
            settings.max_entries_per_auto_position,
        )
    };
    let FreshSignalCandidates {
        fresh_directional,
        neutral_scout_candidates,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
    } = collect_signal_candidates(
        &signals,
        now,
        risk_appetite,
        &open_positions,
        max_entries_per_auto_position,
    );
    let (signal, exploratory, scout) = match select_cycle_signal(SignalSelectionInput {
        state,
        dashboard: &dashboard,
        signals: &signals,
        fresh_directional: &fresh_directional,
        neutral_scout_candidates: &neutral_scout_candidates,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols: &stale_symbols,
        non_directional_count,
        position_blocked_count,
        learning: &learning,
        risk_appetite,
        active_scout_count,
        open_positions: &open_positions,
        max_entries_per_auto_position,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    }) {
        Ok(selected) => selected,
        Err(result) => return Ok(result),
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let evidence = dashboard
        .walk_forward
        .iter()
        .find(|item| item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds));
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        "selected",
        Some(if scout {
            "scout candidate selected; capped paper exploration"
        } else if exploratory {
            "exploratory candidate selected"
        } else {
            "strict walk-forward candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    let mut selected_payload = selected_payload;
    selected_payload["scout"] = serde_json::json!(scout);
    queue_auto_trader_audit(state, selected_payload);
    let decision_id = signal_decision_id(signal);
    if !signal
        .symbol
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Ok(skipped_for_signal(
            signal,
            "Selected insight has an invalid instrument symbol",
        ));
    }
    let SignalMarketSnapshot {
        market,
        bid,
        ask,
        mid,
        market_inputs_ms,
    } = match load_signal_market_snapshot(&client, &position_url, &dashboard, now, signal).await {
        Ok(snapshot) => snapshot,
        Err(result) => return Ok(result),
    };
    let signal_key = signal.symbol.clone();
    let now_ms = now.saturating_mul(1000);
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let quote_rate = quote_usd_rate(&settings, &signal.symbol);
    if let Some(stopped_at) = settings
        .orders
        .iter()
        .filter(|order| {
            order.symbol == signal_key
                && order.purpose == "stop-loss"
                && order.status == "filled"
                && order.realized_pnl < 0.0
        })
        .filter_map(|order| order.triggered_at)
        .max()
        .filter(|stopped_at| now_ms.saturating_sub(*stopped_at) < POST_STOP_REENTRY_COOLDOWN_MS)
    {
        let seconds_remaining =
            POST_STOP_REENTRY_COOLDOWN_MS.saturating_sub(now_ms.saturating_sub(stopped_at)) / 1000;
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Post-stop cooldown: {signal_key} paused for {seconds_remaining}s after a losing protective stop"
            ),
        ));
    }
    let open_position_count = settings
        .positions
        .values()
        .filter(|position| position.quantity.abs() > 1e-12)
        .count();
    let already_open = settings
        .positions
        .get(&signal_key)
        .is_some_and(|position| position.quantity.abs() > 1e-12);
    if !already_open && open_position_count >= settings.max_open_positions {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Open-position cap reached ({}/{})",
                open_position_count, settings.max_open_positions
            ),
        ));
    }
    if let Some(position) = settings
        .positions
        .get(&signal_key)
        .filter(|position| position.quantity.abs() > 1e-12)
    {
        let signal_quantity = if signal.direction == "bullish" {
            1.0
        } else {
            -1.0
        };
        if position.quantity.signum() != signal_quantity {
            return Ok(skipped_for_signal(
                signal,
                &format!("Opposite signal for {signal_key}; waiting for its stop or target exit"),
            ));
        }
        if position.entries >= settings.max_entries_per_auto_position {
            return Ok(skipped_for_signal(
                signal,
                &format!(
                    "Add-on limit reached for {signal_key} ({} entries per position)",
                    settings.max_entries_per_auto_position
                ),
            ));
        }
    } else if !signal.research_strategy_candidate
        && settings
            .last_signal_direction
            .get(&signal_key)
            .is_some_and(|direction| direction == &signal.direction)
        && settings
            .last_signal_strength
            .get(&signal_key)
            .is_some_and(|strength| signal.strength < strength + MATERIAL_STRENGTH_CHANGE)
    {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Re-entry blocked: {signal_key} signal strength has not improved by {:.2}",
                MATERIAL_STRENGTH_CHANGE
            ),
        ));
    }
    if !signal.research_strategy_candidate
        && settings
            .last_signal_at
            .get(&signal_key)
            .is_some_and(|last| now_ms.saturating_sub(*last) < SIGNAL_COOLDOWN_MS)
    {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Re-entry cooldown: {signal_key} needs a materially new signal after {} seconds",
                SIGNAL_COOLDOWN_MS / 1000
            ),
        ));
    }
    let baseline_type = choose_order_type(&signal, settings.risk_appetite);
    let risk_fraction = if settings.risk_limited_sizing {
        0.25
    } else {
        1.0
    };
    let risk_size_scale = risk_size_scale(settings.risk_appetite);
    let exploration_cap = if scout {
        settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION * risk_size_scale
    } else if exploratory {
        settings.max_order_notional * (settings.exploration_percent / 100.0) * risk_size_scale
    } else {
        settings.max_order_notional
    };
    let risk_order_cap = exploration_cap
        .min(settings.max_order_notional)
        .min(settings.max_order_notional * risk_fraction);
    let stop_headroom_multiplier =
        if matches!(baseline_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
            2.0
        } else {
            1.0
        };
    let mut notional = risk_order_cap.min(
        (settings.daily_notional_limit - settings.used_notional_today) / stop_headroom_multiplier,
    );
    if notional <= 0.0 {
        return Ok(skipped_for_signal(
            signal,
            "Total notional capacity is fully allocated",
        ));
    }
    if exploratory {
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings
            .last_risk_note
            .push_str(if scout {
                "scout order capped at 5% of max order; no sufficiently negative same-side walk-forward evidence"
            } else {
                "small paper-exploration size; no reliable same-side walk-forward edge yet"
            });
    }
    let baseline_params = ExecutionParameters {
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_rate: settings.vwap_participation_percent / 100.0,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
    };
    let minimum_book_order = if scout {
        MIN_SCOUT_DEEP_BOOK_ORDER_NOTIONAL
    } else {
        MIN_DEEP_BOOK_ORDER_NOTIONAL
    };
    let deep_book_started = Instant::now();
    let deep_book = assess_deep_book(
        &market,
        &signal.symbol,
        signal.direction == "bullish",
        notional * quote_rate,
        minimum_book_order * quote_rate,
    );
    let deep_book_ms = deep_book_started.elapsed().as_secs_f64() * 1000.0;
    let learned_exit_policy = learned_exit_policy(&signal, &dashboard.walk_forward);
    let effective_stop_loss_percent = learned_exit_policy
        .map(|(stop, _)| stop)
        .unwrap_or(counterfactual_stop_loss_percent);
    let horizon_take_profit_percent = learned_exit_policy.map(|(_, target)| target).or_else(|| {
        horizon_adjusted_take_profit_percent(
            &signal,
            &dashboard.walk_forward,
            settings.take_profit_percent,
        )
    });
    let effective_take_profit_percent =
        horizon_take_profit_percent.unwrap_or(counterfactual_take_profit_percent);
    let context = market_context(&signal);
    decay_learning_map(&mut settings.learning, now_ms);
    let learned_type = if scout {
        baseline_type
    } else if let Some(order_type) = adapt_order_type(
        baseline_type,
        &context,
        &settings.learning,
        settings.risk_appetite,
    ) {
        order_type
    } else {
        return Ok(skipped_for_signal(
            signal,
            "Recent closed auto-trades show no profitable order type with enough samples",
        ));
    };
    let side_depth = deep_book
        .map(|assessment| {
            if signal.direction == "bullish" {
                assessment.ask_depth_10bps
            } else {
                assessment.bid_depth_10bps
            }
        })
        .unwrap_or(0.0);
    let depth_ratio = if side_depth > 0.0 {
        notional * quote_rate / side_depth
    } else {
        0.0
    };
    let (schedule_depth_threshold, schedule_threshold_samples) =
        learned_schedule_depth_threshold(&settings.learning, &context);
    let depth_ratio_bucket = schedule_depth_ratio_bucket(depth_ratio);
    let order_type = score_execution_order_type(
        signal,
        settings.risk_appetite,
        notional * quote_rate,
        baseline_type,
        learned_type,
        deep_book,
        schedule_depth_threshold,
    );
    let profile = choose_parameter_profile(order_type, &context, &settings.learning, exploratory);
    let execution_params = parameter_profile(baseline_params, profile);
    *state.latest_execution_parameters.lock().await = Some(LatestExecutionParameters {
        timestamp: now_ms,
        symbol: signal.symbol.clone(),
        context: context.clone(),
        baseline_order_type: format!("{baseline_type:?}"),
        selected_order_type: format!("{order_type:?}"),
        profile,
        vwap_duration_minutes: execution_params.vwap_duration_minutes,
        vwap_participation_percent: execution_params.vwap_participation_rate * 100.0,
        schedule_depth_threshold,
        proposed_depth_ratio: depth_ratio,
        schedule_threshold_samples,
        limit_offset_bps: execution_params.limit_offset_bps,
        trigger_offset_bps: execution_params.trigger_offset_bps,
        trailing_distance_percent: execution_params.trailing_distance_percent,
    });
    let learning_context = if scout {
        format!("scout:{context}:P{profile}:depth:{depth_ratio_bucket}")
    } else {
        format!("{context}:P{profile}:depth:{depth_ratio_bucket}")
    };
    drop(settings);
    let side = if signal.direction == "bullish" {
        "buy"
    } else {
        "sell"
    };
    // Walk-forward net returns include configured taker fees, but not the live
    // spread/sweep cost of this particular order. Avoid trading when that
    // incremental round-trip cost would consume the measured edge.
    if let Some(evidence) = evidence {
        let side_edge = if signal.direction == "bullish" {
            evidence.all_signal_long.average_net_return_percent
        } else {
            evidence.all_signal_short.average_net_return_percent
        };
        if let (Some(edge_percent), Some(assessment)) = (side_edge, deep_book) {
            let spread_bps = (ask - bid) / mid * 10_000.0;
            let incremental_cost_percent = (spread_bps + 2.0 * assessment.impact_bps) / 100.0;
            if edge_percent.is_finite()
                && incremental_cost_percent.is_finite()
                && incremental_cost_percent >= edge_percent
            {
                return Ok(skipped_for_signal(
                    signal,
                    &format!(
                        "Estimated spread and sweep cost ({incremental_cost_percent:.3}%) consumes the measured walk-forward edge ({edge_percent:.3}%)"
                    ),
                ));
            }
        }
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
    let mut stage_timings_ms = BTreeMap::new();
    stage_timings_ms.insert("dashboard_fetch".to_owned(), dashboard_ms);
    stage_timings_ms.insert("signal_selection".to_owned(), signal_selection_ms);
    stage_timings_ms.insert("local_market_book".to_owned(), market_inputs_ms);
    stage_timings_ms.insert("deep_book_local".to_owned(), deep_book_ms);
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
    let quantity = {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        let allocated = allocated_notional(&settings);
        let remaining_budget = (settings.daily_notional_limit - allocated).max(0.0);
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
                    .research_strategy
                    .clone()
                    .unwrap_or_else(|| "generic-insight-signals".into()),
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
        });
        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
            settings.orders.drain(..excess);
        }
        persist_settings(&state.settings_path, &settings)?;
        quantity
    };
    submit_and_record_selected_order(SelectedOrderExecution {
        state,
        client: &client,
        position_url: &position_url,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        evidence,
        side,
        quantity,
        notional,
        bid,
        ask,
        mid,
        order_type,
        execution_params,
        timestamp,
        decision_id: &decision_id,
        learning_context: &learning_context,
        horizon_take_profit_percent,
        stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps,
        submitted_limit_price,
        limit_to_mid_bps,
        limit_to_crossing_quote_bps,
        expires_at,
    })
    .await
}

struct SelectedOrderExecution<'a> {
    state: &'a AutoTraderState,
    client: &'a reqwest::Client,
    position_url: &'a str,
    signal: &'a Signal,
    exploratory: bool,
    scout: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    effective_stop_loss_percent: f64,
    effective_take_profit_percent: f64,
    evidence: Option<&'a WalkForward>,
    side: &'a str,
    quantity: f64,
    notional: f64,
    bid: f64,
    ask: f64,
    mid: f64,
    order_type: AutoOrderType,
    execution_params: ExecutionParameters,
    timestamp: u64,
    decision_id: &'a str,
    learning_context: &'a str,
    horizon_take_profit_percent: Option<f64>,
    stage_timings_ms: BTreeMap<String, f64>,
    deep_book: Option<DeepBookAssessment>,
    estimated_ioc_all_in_bps: Option<f64>,
    submitted_limit_price: Option<f64>,
    limit_to_mid_bps: Option<f64>,
    limit_to_crossing_quote_bps: Option<f64>,
    expires_at: Option<u64>,
}

async fn submit_and_record_selected_order(
    input: SelectedOrderExecution<'_>,
) -> Result<CycleResult, (StatusCode, String)> {
    let SelectedOrderExecution {
        state,
        client,
        position_url,
        signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        evidence,
        side,
        quantity,
        notional,
        bid,
        ask,
        mid,
        order_type,
        execution_params,
        timestamp,
        decision_id,
        learning_context,
        horizon_take_profit_percent,
        mut stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps,
        submitted_limit_price,
        limit_to_mid_bps,
        limit_to_crossing_quote_bps,
        expires_at,
    } = input;
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(skipped_for_signal(
            signal,
            "Auto Trader was disabled before order submission",
        ));
    }
    let submit_started = Instant::now();
    let response = match submit_configured_order(
        &client,
        &position_url,
        &signal.symbol,
        side,
        quantity,
        notional,
        bid,
        ask,
        order_type,
        execution_params,
        timestamp.to_string(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".into();
                order.failure_reason = Some(error.1.clone());
            }
            persist_settings(&state.settings_path, &settings)?;
            return Err(error);
        }
    };
    let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
    stage_timings_ms.insert("order_submit_response".to_owned(), submit_response_ms);
    if let Some(handler_ms) = response
        .get("position_manager_handler_ms")
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        stage_timings_ms.insert("position_manager_handler".to_owned(), handler_ms);
        stage_timings_ms.insert(
            "client_transport_and_decode".to_owned(),
            (submit_response_ms - handler_ms).max(0.0),
        );
        if let Some(phases) = response
            .get("processing_timings_ms")
            .and_then(serde_json::Value::as_object)
        {
            for (phase, value) in phases {
                if let Some(milliseconds) = value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                {
                    stage_timings_ms.insert(format!("position_manager_{phase}"), milliseconds);
                }
            }
        }
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let filled = response
        .get("filled_quantity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let average_price = response
        .get("average_execution_price")
        .and_then(serde_json::Value::as_f64);
    let fee = response
        .get("estimated_fee")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        .abs();
    let (realized_delta, learned_type) = if filled > 0.0 {
        let prior_quantity = settings
            .positions
            .get(&signal.symbol)
            .map_or(0.0, |position| position.quantity);
        let incoming_sign = if side == "buy" { 1.0_f64 } else { -1.0_f64 };
        let is_new_position_direction =
            prior_quantity == 0.0 || incoming_sign != prior_quantity.signum();
        let result = average_price
            .map(|price| {
                apply_auto_fill(
                    &mut settings.positions,
                    &signal.symbol,
                    side,
                    filled,
                    price,
                    fee,
                    &format!("{order_type:?}"),
                    &learning_context,
                    true,
                    horizon_take_profit_percent,
                )
            })
            .unwrap_or_default();
        if let Some(position) = settings.positions.get_mut(&signal.symbol) {
            position.entry_decision_id = Some(decision_id.to_owned());
            if signal.research_strategy.is_some() {
                position.research_strategy = signal.research_strategy.clone();
            }
            if position.quantity.abs() > 1e-12 && is_new_position_direction {
                position.stop_loss_percent = Some(effective_stop_loss_percent);
                position.take_profit_percent = Some(effective_take_profit_percent);
            }
        }
        result
    } else {
        (0.0, None)
    };
    let realized_delta_usd = realized_usd(&settings, &signal.symbol, realized_delta);
    settings.realized_pnl += realized_delta_usd;
    settings.daily_realized_pnl += realized_delta_usd;
    if let Some(kind) = learned_type {
        let stats = settings.learning.entry(kind).or_default();
        record_learning_outcome(stats, realized_delta_usd);
    }
    let status = if response
        .get("barrier_order")
        .is_some_and(|value| !value.is_null())
    {
        "armed"
    } else if response.get("order").is_some_and(|value| !value.is_null()) {
        "resting"
    } else if filled > 0.0 {
        "filled"
    } else {
        "accepted"
    };
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.fill_rate = (filled / quantity).clamp(0.0, 1.0);
        order.average_price = average_price;
        order.fees_paid = fee;
        order.execution_order_id = response
            .get("order")
            .and_then(|order| order.get("id"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                (matches!(&order_type, AutoOrderType::Twap | AutoOrderType::Vwap))
                    .then(|| response.get("id").and_then(serde_json::Value::as_str))
                    .flatten()
            })
            .map(str::to_owned);
        order.status = status.to_owned();
        order.realized_pnl = realized_delta;
        order.stage_timings_ms = stage_timings_ms;
        order.failure_reason = None;
    }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    let mut audit_payload = signal_audit_payload(
        signal,
        "entry",
        timestamp,
        status,
        None,
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    audit_payload["scout"] = serde_json::json!(scout);
    audit_payload["effective_stop_loss_percent"] = serde_json::json!(effective_stop_loss_percent);
    audit_payload["effective_take_profit_percent"] =
        serde_json::json!(effective_take_profit_percent);
    audit_payload["order_type"] = serde_json::json!(format!("{order_type:?}"));
    audit_payload["order_quantity"] = serde_json::json!(quantity);
    audit_payload["filled_quantity"] = serde_json::json!(filled);
    audit_payload["average_price"] = serde_json::json!(average_price);
    audit_payload["fees"] = serde_json::json!(fee);
    audit_payload["realized_pnl"] = serde_json::json!(realized_delta);
    let response_received_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let implementation_shortfall_bps = average_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let fee_bps = (filled > 0.0 && mid > 0.0).then(|| fee / (filled * mid) * 10_000.0);
    audit_payload["side"] = serde_json::json!(side);
    audit_payload["decision_mid_price"] = serde_json::json!(mid);
    audit_payload["submitted_at"] = serde_json::json!(timestamp);
    audit_payload["response_received_at"] = serde_json::json!(response_received_at);
    audit_payload["submit_response_ms"] = serde_json::json!(submit_response_ms);
    audit_payload["fill_rate"] = serde_json::json!((filled / quantity).clamp(0.0, 1.0));
    audit_payload["implementation_shortfall_bps"] = serde_json::json!(implementation_shortfall_bps);
    audit_payload["fee_bps"] = serde_json::json!(fee_bps);
    audit_payload["all_in_cost_bps"] = serde_json::json!(implementation_shortfall_bps
        .zip(fee_bps)
        .map(|(shortfall, fees)| shortfall + fees));
    audit_payload["bid"] = serde_json::json!(bid);
    audit_payload["ask"] = serde_json::json!(ask);
    audit_payload["spread_bps"] = serde_json::json!((ask - bid) / mid * 10_000.0);
    audit_payload["book_bid_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.bid_depth_10bps));
    audit_payload["book_ask_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.ask_depth_10bps));
    audit_payload["book_depth_imbalance_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.depth_imbalance_10bps));
    audit_payload["estimated_sweep_bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.impact_bps));
    audit_payload["estimated_ioc_all_in_bps"] = serde_json::json!(estimated_ioc_all_in_bps);
    audit_payload["submitted_limit_price"] = serde_json::json!(submitted_limit_price);
    audit_payload["limit_to_mid_bps"] = serde_json::json!(limit_to_mid_bps);
    audit_payload["limit_to_crossing_quote_bps"] = serde_json::json!(limit_to_crossing_quote_bps);
    audit_payload["expires_at"] = serde_json::json!(expires_at);
    audit_payload["execution_response"] = response.clone();
    queue_auto_trader_audit(state, audit_payload);
    let stop_armed = if filled > 0.0 {
        // VWAP/TWAP can add to the position through several child fills. Rebuild
        // protection from the resulting aggregate position so all stop quantity
        // uses the same weighted entry price and fee basis as take-profit logic.
        let old_stops_cancelled =
            cancel_protective_stops(&client, &position_url, state, &signal.symbol)
                .await
                .unwrap_or(false);
        if !old_stops_cancelled {
            let mut settings = state.settings.lock().await;
            settings.last_risk_note = format!(
                "Existing protective stop could not be replaced for {}; newly filled quantity is covered at the aggregate position basis where possible",
                signal.symbol
            );
            let _ = persist_settings(&state.settings_path, &settings);
        }
        match ensure_protective_stops(&client, &position_url, state).await {
            Ok(()) => {
                let settings = state.settings.lock().await;
                settings
                    .positions
                    .get(&signal.symbol)
                    .is_some_and(|position| !position.protective_stops.is_empty())
            }
            Err((_, error)) => {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!(
                    "Protective stop could not be refreshed: {error}; five-second exit monitor remains as fallback"
                );
                let _ = persist_settings(&state.settings_path, &settings);
                false
            }
        }
    } else {
        false
    };
    Ok(CycleResult {
        status: "order_submitted",
        reason: stop_armed.then(|| "Protective stop-market armed".into()),
        symbol: Some(signal.symbol.clone()),
        side: Some(side.into()),
        quantity: Some(quantity),
        order_type: Some(format!("{order_type:?}")),
        order_response: Some(response),
    })
}

struct SignalMarketSnapshot {
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_inputs_ms: f64,
}

async fn load_signal_market_snapshot(
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    now: u64,
    signal: &Signal,
) -> Result<SignalMarketSnapshot, CycleResult> {
    let market_inputs_started = Instant::now();
    let cached_market = dashboard.markets.get(&signal.symbol).cloned();
    let cached_market_is_fresh = cached_market.as_ref().is_some_and(|market| {
        let updated_at = market
            .get("updated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        updated_at > 0 && now.saturating_mul(1000).saturating_sub(updated_at) <= 5_000
    });
    let market = if cached_market_is_fresh {
        cached_market
    } else {
        fresh_subscribed_market_book(client, position_url, &signal.symbol).await
    };
    let Some(market) = market else {
        return Err(skipped_for_signal(
            signal,
            &format!(
                "Waiting for a fresh subscribed local order book for {}",
                signal.symbol
            ),
        ));
    };
    let market_inputs_ms = market_inputs_started.elapsed().as_secs_f64() * 1000.0;
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid
        .zip(ask)
        .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
    else {
        return Err(skipped_for_signal(
            signal,
            &format!(
                "No current order book for {}; subscribed and waiting for a fresh book",
                signal.symbol
            ),
        ));
    };
    let mid = (bid + ask) / 2.0;
    // Warm the paper manager's book subscription while the remaining order
    // checks run, so the submit path doesn't pay the feed's first-book delay.
    let warm_client = client.clone();
    let warm_url = format!(
        "{position_url}/internal/market-book/{}/subscribe",
        signal.symbol
    );
    tokio::spawn(async move {
        let _ = warm_client.post(warm_url).send().await;
    });
    Ok(SignalMarketSnapshot {
        market,
        bid,
        ask,
        mid,
        market_inputs_ms,
    })
}

struct FreshSignalCandidates<'a> {
    fresh_directional: Vec<&'a Signal>,
    neutral_scout_candidates: Vec<Signal>,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: Vec<String>,
    non_directional_count: usize,
    position_blocked_count: usize,
}

fn collect_signal_candidates<'a>(
    signals: &'a [Signal],
    now: u64,
    risk_appetite: f64,
    open_positions: &BTreeMap<String, TraderPosition>,
    max_entries_per_auto_position: u32,
) -> FreshSignalCandidates<'a> {
    let mut stale_signal_count = 0;
    let mut stalest_signal_age_seconds = 0u64;
    let mut stale_symbols = Vec::new();
    let mut non_directional_count = 0;
    let mut position_blocked_count = 0;
    let fresh_directional = signals
        .iter()
        .filter(|signal| {
            if signal.stale || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
            {
                stale_signal_count += 1;
                let age = now.saturating_sub(signal.timestamp / 1000);
                stalest_signal_age_seconds = stalest_signal_age_seconds.max(age);
                if !stale_symbols.iter().any(|symbol| symbol == &signal.symbol) {
                    stale_symbols.push(signal.symbol.clone());
                }
                return false;
            }
            if !matches!(signal.direction.as_str(), "bullish" | "bearish") {
                non_directional_count += 1;
                return false;
            }
            let allowed_by_position = open_positions.get(&signal.symbol).is_none_or(|position| {
                let signal_quantity = if signal.direction == "bullish" {
                    1.0
                } else {
                    -1.0
                };
                position.quantity.signum() == signal_quantity
                    && position.entries < max_entries_per_auto_position
            });
            if !allowed_by_position {
                position_blocked_count += 1;
            }
            allowed_by_position
        })
        .collect::<Vec<_>>();
    let neutral_scout_candidates = signals
        .iter()
        .filter_map(|signal| {
            if signal.direction != "neutral"
                || signal.stale
                || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
                || !signal.score.is_finite()
                || signal.score.abs() < SCOUT_NEUTRAL_SCORE_FLOOR * risk_floor_scale(risk_appetite)
                || signal.ranging_probability.unwrap_or(0.0)
                    >= NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            {
                return None;
            }
            let mut candidate = signal.clone();
            candidate.direction = if candidate.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            let allowed_by_position =
                open_positions
                    .get(&candidate.symbol)
                    .is_none_or(|position| {
                        let direction = if candidate.direction == "bullish" {
                            1.0
                        } else {
                            -1.0
                        };
                        position.quantity.signum() == direction
                            && position.entries < max_entries_per_auto_position
                    });
            allowed_by_position.then_some(candidate)
        })
        .collect::<Vec<_>>();
    FreshSignalCandidates {
        fresh_directional,
        neutral_scout_candidates,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
    }
}

#[derive(Clone, Copy)]
struct SignalSelectionInput<'a> {
    state: &'a AutoTraderState,
    dashboard: &'a Dashboard,
    signals: &'a [Signal],
    fresh_directional: &'a [&'a Signal],
    neutral_scout_candidates: &'a [Signal],
    now: u64,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: &'a [String],
    non_directional_count: usize,
    position_blocked_count: usize,
    learning: &'a BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
    open_positions: &'a BTreeMap<String, TraderPosition>,
    max_entries_per_auto_position: u32,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
}

fn format_no_tradeable_bias(
    input: &SignalSelectionInput<'_>,
    exploration_qualified: &[&Signal],
    strength_qualified: &[&Signal],
) -> String {
    let SignalSelectionInput {
        dashboard,
        signals,
        fresh_directional,
        neutral_scout_candidates,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
        learning,
        risk_appetite,
        active_scout_count,
        open_positions,
        max_entries_per_auto_position,
        ..
    } = *input;
    let blocked_by_negative = exploration_qualified
        .iter()
        .filter(|signal| {
            dashboard
                .walk_forward
                .iter()
                .find(|item| {
                    item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
                })
                .map(|item| {
                    if signal.direction == "bullish" {
                        &item.all_signal_long
                    } else {
                        &item.all_signal_short
                    }
                })
                .is_some_and(|evidence| {
                    evidence.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                        && (evidence
                            .average_net_return_percent
                            .is_none_or(|net| net <= 0.0)
                            || evidence
                                .positive_net_rate
                                .is_none_or(|rate| rate < NEGATIVE_EVIDENCE_FLOOR))
                })
        })
        .count();
    let mut reason = format!(
        "No tradeable bias: fresh directional {}/{} ({} inferred from neutral score; {} neutral scout probes; filtered: {} stale, {} non-directional, {} blocked by existing positions); contextual learned strength floors passed {}; exploration strength floor {:.2}; contextual raw-score floors passed {}; {} candidate(s) blocked by negative walk-forward history",
        fresh_directional.len(),
        signals.len(),
        neutral_scout_candidates.len(),
        inferred_direction_count,
        stale_signal_count,
        non_directional_count,
        position_blocked_count,
        strength_qualified.len(),
        MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite),
        exploration_qualified.len(),
        blocked_by_negative
    );
    if stale_signal_count > 0 {
        reason.push_str(&format!(
            "; oldest stale signal {}s; symbols {}",
            stalest_signal_age_seconds,
            stale_symbols.join(", ")
        ));
    }
    let early_filter_details = signals.iter().filter_map(|candidate| {
                let age = now.saturating_sub(candidate.timestamp / 1000);
                if candidate.stale {
                    Some(format!("{} {} · Signal freshness (stale flag true; age {}s; max age {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if age > MAX_SIGNAL_AGE_SECONDS {
                    Some(format!("{} {} · Signal freshness (age {}s > max {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if !matches!(candidate.direction.as_str(), "bullish" | "bearish") {
                    let ranging = candidate.ranging_probability.map(|value| format!("{:.1}%", value * 100.0)).unwrap_or_else(|| "n/a (defaults to 0%)".into());
                    Some(format!("{} {} · Directional signal (|score| {:.3} vs floor {:.2}; range probability {} must be below {:.0}% to infer direction)", candidate.symbol, candidate.direction, candidate.score.abs(), neutral_direction_floor, ranging, NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY * 100.0))
                } else {
                    open_positions.get(&candidate.symbol).and_then(|position| {
                        let signal_quantity = if candidate.direction == "bullish" { 1.0 } else { -1.0 };
                        let matching_side = position.quantity.signum() == signal_quantity;
                        if !matching_side {
                            Some(format!("{} {} · Existing position / re-entry limit (opposite position qty {:.8}; signal side {})", candidate.symbol, candidate.direction, position.quantity, candidate.direction))
                        } else if position.entries >= max_entries_per_auto_position {
                            Some(format!("{} {} · Existing position / re-entry limit (entries {} >= max {})", candidate.symbol, candidate.direction, position.entries, max_entries_per_auto_position))
                        } else { None }
                    })
                }
            }).collect::<Vec<_>>();
    if !early_filter_details.is_empty() {
        reason.push_str("; early filters: ");
        reason.push_str(&early_filter_details.join(" | "));
    }
    let candidate_details = fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .map(|candidate| {
                let context = market_context(candidate);
                let learned_floor = adaptive_strength_floor(&learning, &context);
                let floor_scale = risk_floor_scale(risk_appetite);
                let strength_floor = (adaptive_missed_opportunity_floor(
                    learned_floor, "strength", &dashboard.missed_opportunity_bands,
                ) * floor_scale).clamp(0.0, 1.0);
                let score_floor = (adaptive_exploration_score_floor(
                    &learning, &context, &dashboard.missed_opportunity_bands,
                ) * floor_scale).max(0.0);
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let side = evidence.map(|item| if candidate.direction == "bullish" {
                    &item.all_signal_long
                } else {
                    &item.all_signal_short
                });
                let pct = |value: Option<f64>| value.map(|v| format!("{:.1}%", v * 100.0)).unwrap_or_else(|| "n/a".into());
                let metrics = side.map(|m| format!(
                    "walk-forward {}s n={} hit={} net={} positive={} lower95={}",
                    evidence.map(|item| item.horizon_seconds).unwrap_or_default(), m.sample_count,
                    pct(m.direction_match_rate),
                    m.average_net_return_percent.map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()),
                    pct(m.positive_net_rate), pct(m.positive_net_rate_lower_95)
                )).unwrap_or_else(|| "walk-forward evidence unavailable".into());
                let research_metrics = evidence.map(|item| {
                    let research = if candidate.direction == "bullish" { &item.research_logistic_long } else { &item.research_logistic_short };
                    format!("research-model n={} net={}", research.sample_count,
                        research.average_net_return_percent.map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()))
                }).unwrap_or_else(|| "research-model evidence unavailable".into());
                let weak = candidate.strength < strength_floor;
                let mixed_context = context.ends_with(":mixed");
                let score_weak = candidate.score.abs() < score_floor;
                let evidence_missing = !weak && side.is_none_or(|m| m.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES);
                let exploration_path_qualified = !mixed_context
                    && candidate.strength.is_finite()
                    && candidate.strength >= MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite)
                    && candidate.score.is_finite()
                    && candidate.score.abs() >= score_floor;
                let evidence_bad = exploration_path_qualified && side.is_some_and(|m| m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                    && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                        || m.positive_net_rate.is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR)));
                let research = evidence.map(|item| if candidate.direction == "bullish" {
                    &item.research_logistic_long
                } else {
                    &item.research_logistic_short
                });
                let research_bad = (!weak || exploration_path_qualified) && research.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                    && m.average_net_return_percent.is_none_or(|v| v <= 0.0));
                let strict_evidence_bad = !weak && side.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES && (
                    m.direction_match_rate.is_none_or(|v| v < MIN_SIDE_DIRECTION_MATCH_RATE)
                    || m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                    || m.positive_net_rate.is_none_or(|v| v < MIN_SIDE_POSITIVE_NET_RATE)
                    || m.positive_net_rate_lower_95.is_none_or(|v| v < MIN_SIDE_POSITIVE_NET_LOWER_95)
                ));
                let (scout_strength_floor, scout_score_floor) =
                    adaptive_scout_thresholds(&learning, &context, active_scout_count);
                let scout_strength_floor =
                    (scout_strength_floor * risk_floor_scale(risk_appetite)).clamp(0.0, 1.0);
                let scout_score_floor =
                    (scout_score_floor * risk_floor_scale(risk_appetite)).max(0.0);
                let scout_strength_pass = candidate.strength.is_finite() && candidate.strength >= scout_strength_floor;
                let scout_score_pass = candidate.score.is_finite() && candidate.score.abs() >= scout_score_floor;
                let scout_negative = side.is_some_and(|m| m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                    && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                        || m.positive_net_rate.is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR)))
                    || research.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                        && m.average_net_return_percent.is_none_or(|v| v <= 0.0));
                let mut failed = Vec::new();
                if weak { failed.push(format!("strength {:.2} < {:.2}", candidate.strength, strength_floor)); }
                if mixed_context { failed.push("normal exploration blocked for mixed context".into()); }
                else if score_weak { failed.push(format!("score {:.2} < {:.2}", candidate.score.abs(), score_floor)); }
                if evidence_bad { failed.push(format!("exploration needs net return > 0 and positive rate ≥ {:.0}%", NEGATIVE_EVIDENCE_FLOOR * 100.0)); }
                if evidence_missing { failed.push(format!("normal entry needs {} side samples", MIN_SIDE_WALK_FORWARD_SAMPLES)); }
                if strict_evidence_bad { failed.push(format!("normal entry needs hit ≥ {:.0}%, net > 0, positive rate ≥ {:.0}%, lower-95 ≥ {:.0}%", MIN_SIDE_DIRECTION_MATCH_RATE * 100.0, MIN_SIDE_POSITIVE_NET_RATE * 100.0, MIN_SIDE_POSITIVE_NET_LOWER_95 * 100.0)); }
                if research_bad { failed.push("research-model side return is not positive".into()); }
                if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS { failed.push(format!("scout inventory cap reached ({active_scout_count}/{SCOUT_MAX_ACTIVE_POSITIONS} open scout positions)")); }
                else if !scout_strength_pass { failed.push(format!("scout strength {:.2} < learned {:.2}", candidate.strength, scout_strength_floor)); }
                else if !scout_score_pass { failed.push(format!("scout |score| {:.2} < learned {:.2}", candidate.score.abs(), scout_score_floor)); }
                else if scout_negative && active_scout_count > 0 { failed.push(format!("measured negative-history scout probe waits for zero open scouts ({}; net {}; positive {})", side.map(|m| m.sample_count).unwrap_or_default(), side.and_then(|m| m.average_net_return_percent).map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()), side.and_then(|m| m.positive_net_rate).map(|v| format!("{:.1}%", v * 100.0)).unwrap_or_else(|| "n/a".into()))); }
                format!("{} {} · score {:.2} · strength {:.2} · context {} · {} · {} · {}",
                    candidate.symbol, candidate.direction, candidate.score, candidate.strength, context,
                    metrics, research_metrics, if failed.is_empty() { "did not pass combined entry checks".into() } else { failed.join(", ") })
            }).collect::<Vec<_>>();
    if !candidate_details.is_empty() {
        reason.push_str("; candidate checks: ");
        reason.push_str(&candidate_details.join(" | "));
    }
    reason
}

fn select_cycle_signal<'a>(
    input: SignalSelectionInput<'a>,
) -> Result<(&'a Signal, bool, bool), CycleResult> {
    let SignalSelectionInput {
        state,
        dashboard,
        fresh_directional,
        neutral_scout_candidates,
        now,
        learning,
        risk_appetite,
        active_scout_count,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        ..
    } = input;
    if let Some(signal) = fresh_directional
        .iter()
        .copied()
        .filter(|signal| signal.research_strategy_candidate)
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
    {
        return Ok((signal, false, false));
    }
    let strength_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            let context = market_context(signal);
            let learned_floor = adaptive_strength_floor(&learning, &context);
            let floor = (adaptive_missed_opportunity_floor(
                learned_floor,
                "strength",
                &dashboard.missed_opportunity_bands,
            ) * risk_floor_scale(risk_appetite))
            .clamp(0.0, 1.0);
            signal.strength.is_finite() && signal.strength >= floor
        })
        .copied()
        .collect::<Vec<_>>();
    let exploration_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            let context = market_context(signal);
            !context.ends_with(":mixed")
                && signal.strength.is_finite()
                && signal.strength >= MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite)
                && signal.score.is_finite()
                && signal.score.abs()
                    >= (adaptive_exploration_score_floor(
                        &learning,
                        &context,
                        &dashboard.missed_opportunity_bands,
                    ) * risk_floor_scale(risk_appetite))
                    .max(0.0)
        })
        .copied()
        .collect::<Vec<_>>();
    let strict_candidate = strength_qualified
        .iter()
        .copied()
        .filter(|signal| {
            let walk_forward = dashboard.walk_forward.iter().find(|item| {
                item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
            });
            let side_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.all_signal_long
                } else {
                    &item.all_signal_short
                }
            });
            let research_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.research_logistic_long
                } else {
                    &item.research_logistic_short
                }
            });
            side_evidence.is_some_and(|evidence| {
                let research_supports = research_evidence.is_none_or(|research| {
                    research.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || research
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                evidence.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                    && evidence
                        .direction_match_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                    && evidence
                        .average_net_return_percent
                        .is_some_and(|net| net > 0.0)
                    && evidence
                        .positive_net_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_RATE)
                    && evidence
                        .positive_net_rate_lower_95
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
                    && research_supports
            })
        })
        .max_by(|a, b| a.strength.total_cmp(&b.strength));
    let (signal, exploratory, scout) = if let Some(signal) = strict_candidate {
        (signal, false, false)
    } else {
        let exploratory = exploration_qualified
            .iter()
            .copied()
            .filter(|signal| {
                let walk_forward = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let side_evidence = walk_forward.map(|item| {
                    if signal.direction == "bullish" {
                        &item.all_signal_long
                    } else {
                        &item.all_signal_short
                    }
                });
                let research_evidence = walk_forward.map(|item| {
                    if signal.direction == "bullish" {
                        &item.research_logistic_long
                    } else {
                        &item.research_logistic_short
                    }
                });
                let research_supports = research_evidence.is_none_or(|evidence| {
                    evidence.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || evidence
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                research_supports
                    && side_evidence.is_none_or(|evidence| {
                        evidence.sample_count < MIN_EXPLORATION_EVIDENCE_SAMPLES
                            || (evidence
                                .average_net_return_percent
                                .is_some_and(|net| net > 0.0)
                                && evidence
                                    .positive_net_rate
                                    .is_some_and(|rate| rate >= NEGATIVE_EVIDENCE_FLOOR))
                    })
            })
            .max_by(|a, b| a.strength.total_cmp(&b.strength));
        let scout_candidate = if exploratory.is_none() {
            fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .filter(|signal| {
                    let context = market_context(signal);
                    let (strength_floor, score_floor) =
                        adaptive_scout_thresholds(&learning, &context, active_scout_count);
                    let strength_floor =
                        (strength_floor * risk_floor_scale(risk_appetite)).clamp(0.0, 1.0);
                    let score_floor = (score_floor * risk_floor_scale(risk_appetite)).max(0.0);
                    if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS
                        || !signal.strength.is_finite()
                        || signal.strength < strength_floor
                        || !signal.score.is_finite()
                        || signal.score.abs() < score_floor
                    {
                        return false;
                    }
                    let evidence = dashboard.walk_forward.iter().find(|item| {
                        item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
                    });
                    let side = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.all_signal_long
                        } else {
                            &item.all_signal_short
                        }
                    });
                    let research = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.research_logistic_long
                        } else {
                            &item.research_logistic_short
                        }
                    });
                    let measured_negative = side.is_some_and(|m| {
                        m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                            && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                                || m.positive_net_rate
                                    .is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR))
                    }) || research.is_some_and(|m| {
                        m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                            && m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                    });
                    !measured_negative || active_scout_count == 0
                })
                .max_by(|a, b| a.strength.total_cmp(&b.strength))
        } else {
            None
        };
        let Some(signal) = exploratory.or(scout_candidate) else {
            let reason =
                format_no_tradeable_bias(&input, &exploration_qualified, &strength_qualified);
            for candidate in fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
            {
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let payload = signal_audit_payload(
                    candidate,
                    "decision",
                    now.saturating_mul(1000),
                    "skipped",
                    Some(&reason),
                    Some(true),
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    evidence,
                );
                queue_auto_trader_audit(state, payload);
            }
            return Err(skipped(&reason));
        };
        (signal, true, exploratory.is_none())
    };
    Ok((signal, exploratory, scout))
}

struct PreparedSignals {
    signals: Vec<Signal>,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
}

fn prepare_directional_signal_candidates(
    state: &AutoTraderState,
    dashboard: &Dashboard,
    now: u64,
    risk_appetite: f64,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
) -> PreparedSignals {
    let mut signals = dashboard.best.clone();
    for signal in &mut signals {
        let multiplier =
            longer_horizon_score_multiplier(signal.score, &dashboard.missed_opportunity_bands);
        if multiplier > 1.0 {
            signal.score *= multiplier;
        }
    }
    let neutral_direction_floor = NEUTRAL_SCORE_DIRECTION_FLOOR * risk_floor_scale(risk_appetite);
    let mut inferred_direction_count = 0;
    for signal in &mut signals {
        if signal.direction == "neutral"
            && signal.ranging_probability.unwrap_or(0.0) < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            && signal.score.is_finite()
            && signal.score.abs() >= neutral_direction_floor
        {
            signal.direction = if signal.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            inferred_direction_count += 1;
        }
    }
    // Record borderline neutral scores as directional shadow probes only. These
    // are evaluated on every scan and never feed the order-selection path.
    for candidate in signals.iter().filter(|candidate| {
        !candidate.stale
            && candidate.direction == "neutral"
            && now.saturating_sub(candidate.timestamp / 1000) <= MAX_SIGNAL_AGE_SECONDS
            && candidate.score.is_finite()
            && candidate.score.abs() >= SHADOW_MIN_ABS_SCORE
            && candidate.score.abs() < neutral_direction_floor
            && candidate.ranging_probability.unwrap_or(0.0)
                < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
    }) {
        let mut shadow = candidate.clone();
        shadow.direction = if shadow.score > 0.0 {
            "bullish"
        } else {
            "bearish"
        }
        .into();
        let evidence = dashboard.walk_forward.iter().find(|item| {
            item.symbol == shadow.symbol && (60..=300).contains(&item.horizon_seconds)
        });
        let payload = signal_audit_payload(
            &shadow,
            "decision",
            now.saturating_mul(1000),
            "skipped",
            Some(
                "No tradeable bias: shadow-only neutral score probe below live direction floor; no order submitted",
            ),
            Some(true),
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            evidence,
        );
        queue_auto_trader_audit(state, payload);
    }
    PreparedSignals {
        signals,
        inferred_direction_count,
        neutral_direction_floor,
    }
}

async fn enforce_daily_realized_loss_limit(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
    }
    let ledger = fetch_ledger_realized_pnl(client, position_url).await?;
    let realized_today_usd = ledger.realized_pnl_usd.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Daily realized P&L is unavailable from the position ledger because an FX rate is missing"
            .into(),
    ))?;
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    settings.daily_realized_pnl = realized_today_usd;
    if settings.daily_realized_pnl > -settings.daily_realized_loss_limit {
        return Ok(None);
    }
    if !settings.daily_loss_limit_reduced {
        settings.daily_realized_loss_limit = (settings.daily_realized_loss_limit * 0.9)
            .max(MIN_DAILY_REALIZED_LOSS_LIMIT.min(settings.daily_realized_loss_limit));
        settings.daily_loss_limit_reduced = true;
        let _ = persist_settings(&state.settings_path, &settings);
    }
    Ok(Some(skipped(&format!(
        "Daily realized-loss limit reached at USD {:.2}; learned daily cap is now USD {:.2}",
        settings.daily_realized_pnl, settings.daily_realized_loss_limit
    ))))
}

fn profitable_close_probability_5m(
    market: &serde_json::Value,
    position: &TraderPosition,
) -> Option<f64> {
    let closing_long = position.quantity > 0.0;
    let levels = if closing_long {
        market.get("bids")?.as_array()?
    } else {
        market.get("asks")?.as_array()?
    };
    let mut remaining = position.quantity.abs();
    let mut notional = 0.0;
    for level in levels {
        let price = level.get("price")?.as_f64()?;
        let size = level.get("size")?.as_f64()?;
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size < 0.0 {
            return None;
        }
        let quantity = remaining.min(size);
        notional += quantity * price;
        remaining -= quantity;
        if remaining <= 1e-12 {
            break;
        }
    }
    if remaining > 1e-12 || position.quantity.abs() <= 1e-12 {
        return None;
    }
    let close_price = notional / position.quantity.abs();
    if !close_price.is_finite() || close_price <= 0.0 {
        return None;
    }
    let samples = market.get("price_samples")?.as_array()?;
    if samples.len() < 31 {
        return None;
    }
    let parsed = samples
        .iter()
        .map(|sample| {
            let pair = sample.as_array()?;
            Some((pair.first()?.as_u64()?, pair.get(1)?.as_f64()?))
        })
        .collect::<Option<Vec<_>>>()?;
    let returns = parsed
        .windows(2)
        .filter_map(|pair| {
            let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
            (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0)
                .then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
        })
        .collect::<Vec<_>>();
    if returns.len() < 30 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let projected_mean = close_price.ln() + mean * 30.0;
    let projected_sd = (variance * 30.0).sqrt();
    if !projected_sd.is_finite() || projected_sd <= 0.0 {
        return None;
    }
    let below = normal_cdf((position.average_entry_price.ln() - projected_mean) / projected_sd);
    Some(if closing_long { 1.0 - below } else { below })
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0
        - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t)
            * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}
