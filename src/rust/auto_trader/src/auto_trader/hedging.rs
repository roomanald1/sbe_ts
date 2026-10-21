#[derive(Clone)]
struct RiskHolding {
    symbol: String,
    signed_usd: f64,
}

fn calculate_risk_holdings(
    positions: &[serde_json::Value],
    rates: &serde_json::Value,
) -> (Vec<RiskHolding>, usize, usize) {
    let mut holdings = Vec::new();
    let mut fallback_marks = 0;
    for position in positions {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let quantity = position["quantity"].as_f64().unwrap_or(0.0);
        let live_price = position["latest_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let entry_price = position["average_entry_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let quote = symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or_default();
        let rate = rates[quote].as_f64().unwrap_or(0.0);
        if quantity.is_finite() && rate.is_finite() && rate > 0.0 {
            if let Some(price) = live_price.or(entry_price) {
                if live_price.is_none() {
                    fallback_marks += 1;
                }
                holdings.push(RiskHolding {
                    symbol: symbol.to_owned(),
                    signed_usd: quantity * price * rate,
                });
            }
        }
    }
    let unknown_marks = positions.len().saturating_sub(holdings.len());
    (holdings, fallback_marks, unknown_marks)
}

async fn update_risk_coverage_note(
    state: &AutoTraderState,
    position_count: usize,
    fallback_marks: usize,
    unknown_marks: usize,
) {
    let mut settings = state.settings.lock().await;
    settings.last_risk_note = match (fallback_marks, unknown_marks) {
        (fallback, unknown) if unknown > 0 => format!(
            "risk covers {position_count} open positions; {fallback} use entry-price marks and {unknown} are unmarked, so hedging is paused"
        ),
        (fallback, _) if fallback > 0 => {
            format!("risk covers {position_count} open positions; {fallback} use entry-price marks")
        }
        _ => format!("risk covers {position_count} open positions"),
    };
    settings.risk_limited_sizing = unknown_marks > 0;
}

fn calculate_portfolio_variance(
    holdings: &[RiskHolding],
    pair_correlations: &serde_json::Value,
) -> f64 {
    let mut portfolio_variance = holdings
        .iter()
        .map(|holding| holding.signed_usd.powi(2))
        .sum::<f64>();
    if let Some(pairs) = pair_correlations.as_array() {
        for i in 0..holdings.len() {
            for j in (i + 1)..holdings.len() {
                if let Some(pair) = pairs.iter().find(|pair| {
                    (pair["symbol_a"].as_str() == Some(&holdings[i].symbol)
                        && pair["symbol_b"].as_str() == Some(&holdings[j].symbol))
                        || (pair["symbol_b"].as_str() == Some(&holdings[i].symbol)
                            && pair["symbol_a"].as_str() == Some(&holdings[j].symbol))
                }) {
                    if let Some(correlation) = pair["correlation"]
                        .as_f64()
                        .filter(|value| value.is_finite())
                    {
                        portfolio_variance += 2.0
                            * holdings[i].signed_usd
                            * holdings[j].signed_usd
                            * correlation.clamp(-1.0, 1.0);
                    }
                }
            }
        }
    }
    portfolio_variance
}

fn select_portfolio_hedge_candidate(
    holdings: &[RiskHolding],
    positions: &[serde_json::Value],
    rates: &serde_json::Value,
    settings: &Settings,
    now_ms: u64,
    portfolio_shock_risk: f64,
) -> Option<(String, String, f64, f64, f64, f64)> {
    let mut best: Option<(String, String, f64, f64, f64, f64)> = None;
    // Reduce the largest direct holding first, before considering a correlated proxy.
    if let Some(holding) = holdings
        .iter()
        .max_by(|a, b| a.signed_usd.abs().total_cmp(&b.signed_usd.abs()))
    {
        if holding.signed_usd.abs() > 0.0 {
            let row = positions
                .iter()
                .find(|row| row["symbol"].as_str() == Some(&holding.symbol));
            let price = row
                .and_then(|row| {
                    row["latest_price"]
                        .as_f64()
                        .or_else(|| row["average_entry_price"].as_f64())
                })
                .unwrap_or(0.0);
            let quote = holding
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            let last = settings
                .last_hedge_at
                .get(&holding.symbol)
                .copied()
                .unwrap_or(0);
            if price > 0.0 && quote_rate > 0.0 && now_ms.saturating_sub(last) >= 300_000 {
                best = Some((
                    holding.symbol.clone(),
                    if holding.signed_usd > 0.0 {
                        "sell"
                    } else {
                        "buy"
                    }
                    .into(),
                    holding.signed_usd.abs().min(settings.max_order_notional),
                    price,
                    quote_rate,
                    holding.signed_usd.abs(),
                ));
            }
        }
    }
    for position in positions {
        if best.is_some() {
            break;
        }
        let candidate_rows = position["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for candidate in candidate_rows {
            let hedge_symbol = candidate["symbol"].as_str().unwrap_or_default();
            let hedge_price = candidate["latest_price"].as_f64().unwrap_or(0.0);
            let quote = hedge_symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            if hedge_price <= 0.0 || quote_rate <= 0.0 {
                continue;
            }
            let mut covariance = 0.0_f64;
            let mut usable_correlations = 0;
            let mut strongest = 0.0_f64;
            let mut enough_samples = false;
            let mut complete_sample = true;
            for holding in holdings {
                let relation = if holding.symbol == hedge_symbol {
                    Some((1.0, usize::MAX))
                } else {
                    positions
                        .iter()
                        .find(|row| row["symbol"].as_str() == Some(&holding.symbol))
                        .and_then(|row| row["candidates"].as_array())
                        .and_then(|rows| {
                            rows.iter()
                                .find(|row| row["symbol"].as_str() == Some(hedge_symbol))
                        })
                        .and_then(|row| {
                            row["correlation"]
                                .as_f64()
                                .zip(row["paired_samples"].as_u64())
                                .map(|(corr, n)| (corr, n as usize))
                        })
                };
                if let Some((correlation, samples)) = relation.filter(|(corr, _)| corr.is_finite())
                {
                    if samples < MIN_HEDGE_CORRELATION_SAMPLES {
                        complete_sample = false;
                    }
                    let correlation = correlation.clamp(-1.0, 1.0);
                    covariance += holding.signed_usd * correlation;
                    usable_correlations += 1;
                    if correlation.abs() > strongest {
                        strongest = correlation.abs();
                        enough_samples = samples >= MIN_HEDGE_CORRELATION_SAMPLES;
                    }
                }
            }
            if usable_correlations != holdings.len()
                || !complete_sample
                || !enough_samples
                || strongest < MIN_HEDGE_CORRELATION
                || !covariance.is_finite()
                || covariance.abs() < settings.max_order_notional * 0.1
            {
                continue;
            }
            let last = settings
                .last_hedge_at
                .get(hedge_symbol)
                .copied()
                .unwrap_or(0);
            if now_ms.saturating_sub(last) < 300_000 {
                continue;
            }
            let hedge_notional = covariance.abs().min(settings.max_order_notional);
            let score = covariance.abs().min(portfolio_shock_risk);
            if best.as_ref().is_none_or(|current| score > current.5) {
                let side = if covariance > 0.0 { "sell" } else { "buy" };
                best = Some((
                    hedge_symbol.to_owned(),
                    side.to_owned(),
                    hedge_notional,
                    hedge_price,
                    quote_rate,
                    score,
                ));
            }
        }
    }
    best
}

async fn maybe_place_portfolio_hedge(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let candidates_request = client
        .get(format!("{base}/positions/hedge-candidates"))
        .send();
    let rates_request = client.get(format!("{base}/positions/usd-rates")).send();
    let (candidates_response, rates_response) =
        tokio::try_join!(candidates_request, rates_request).map_err(upstream_error)?;
    let candidates_response = candidates_response
        .error_for_status()
        .map_err(upstream_error)?;
    let rates_response = rates_response.error_for_status().map_err(upstream_error)?;
    let (candidates, rates_response) = tokio::try_join!(
        candidates_response.json::<serde_json::Value>(),
        rates_response.json::<serde_json::Value>(),
    )
    .map_err(upstream_error)?;
    let rates = &rates_response["rates"];
    let positions = candidates["positions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let (holdings, fallback_marks, unknown_marks) = calculate_risk_holdings(&positions, rates);
    update_risk_coverage_note(state, positions.len(), fallback_marks, unknown_marks).await;
    if holdings.is_empty() || unknown_marks > 0 {
        return Ok(None);
    }
    let portfolio_variance =
        calculate_portfolio_variance(&holdings, &candidates["pair_correlations"]);
    let portfolio_shock_risk = portfolio_variance.max(0.0).sqrt() * PORTFOLIO_SHOCK_PERCENT;
    let settings_snapshot = state.settings.lock().await.clone();
    let hedge_execution = settings_snapshot
        .learning
        .get("portfolio-hedge:execution")
        .cloned()
        .unwrap_or_default();
    let hedge_win_rate = hedge_execution.positive as f64 / hedge_execution.samples.max(1) as f64;
    // A hedge that reliably gets filled can be attempted at lower risk. Poor
    // fill outcomes raise the trigger so the trader does not repeatedly cross
    // into illiquid markets. Keep the adjustment bounded and require evidence.
    let hedge_risk_scale = if hedge_execution.samples >= MIN_HEDGE_EXECUTION_LEARNING_SAMPLES {
        (1.25 - 0.5 * hedge_win_rate).clamp(0.75, 1.25)
    } else {
        1.0
    };
    let risk_trigger = settings_snapshot.daily_realized_loss_limit * hedge_risk_scale;
    if portfolio_shock_risk <= risk_trigger {
        return Ok(None);
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let best = select_portfolio_hedge_candidate(
        &holdings,
        &positions,
        rates,
        &settings_snapshot,
        now_ms,
        portfolio_shock_risk,
    );
    let Some(candidate) = best else {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but no liquid hedge has enough reliable correlation data; new exposure is blocked",
        )));
    };
    execute_portfolio_hedge(client, base, state, candidate, now_ms, portfolio_shock_risk).await
}

async fn execute_portfolio_hedge(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    candidate: (String, String, f64, f64, f64, f64),
    now_ms: u64,
    portfolio_shock_risk: f64,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let (symbol, side, requested_notional, price, quote_rate, _) = candidate;
    // Candidate and rate reads, plus correlation/risk calculations above, are
    // read-only. Keep them off the shared cycle lock so a slow position-manager
    // response cannot delay a signal or position-maintenance decision. Acquire
    // the lock only when we are ready to reserve the hedge and run its
    // state-changing stop/order sequence.
    let _cycle = acquire_cycle_lock(state, "hedge_evaluation").await;
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let notional =
        requested_notional.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but the daily hedge budget is exhausted",
        )));
    }
    let timestamp = now_ms;
    let quantity = notional / (price * quote_rate);
    settings.used_notional_today = allocated_notional(&settings);
    settings.last_hedge_at.insert(symbol.clone(), timestamp);
    settings.orders.push(TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.clone(),
        side: side.clone(),
        order_type: "Ioc".into(),
        purpose: "portfolio-hedge".into(),
        decision_id: None,
        quantity,
        filled_quantity: 0.0,
        fill_rate: 0.0,
        average_price: None,
        fees_paid: 0.0,
        reserved_notional: notional,
        execution_order_id: None,
        processed_fill_trade_ids: Vec::new(),
        learning_context: None,
        book_bid_depth_10bps: None,
        book_ask_depth_10bps: None,
        book_depth_imbalance_10bps: None,
        estimated_sweep_bps: None,
        estimated_ioc_all_in_bps: None,
        decision_bid: None,
        decision_ask: None,
        decision_mid: None,
        submitted_limit_price: None,
        target_price: None,
        horizon_take_profit_percent: None,
        limit_to_mid_bps: None,
        limit_to_crossing_quote_bps: None,
        expires_at: None,
        failure_reason: None,
        stage_timings_ms: BTreeMap::new(),
        status: "submitting".into(),
        realized_pnl: 0.0,
    });
    if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
        let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
        settings.orders.drain(..excess);
    }
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    if !cancel_protective_stops(client, base, state, &symbol).await? {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_stop_active".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(Some(skipped(
            "Could not cancel the existing protective stop; deferred portfolio hedge",
        )));
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        settings.last_hedge_at.remove(&symbol);
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        drop(settings);
        ensure_protective_stops(client, base, state).await?;
        return Ok(Some(skipped(
            "Auto Trader was disabled before hedge submission",
        )));
    }
    let response = match submit_configured_order(
        client,
        base,
        &symbol,
        &side,
        quantity,
        notional,
        price,
        price,
        AutoOrderType::Ioc,
        ExecutionParameters::default(),
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
            }
            persist_settings(&state.settings_path, &settings)?;
            drop(settings);
            ensure_protective_stops(client, base, state).await?;
            return Err(error);
        }
    };
    let mut settings = state.settings.lock().await;
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
    let execution_learning = settings
        .learning
        .entry("portfolio-hedge:execution".into())
        .or_default();
    record_learning_outcome(
        execution_learning,
        if filled >= quantity * 0.8 { 1.0 } else { 0.0 },
    );
    let (realized_pnl, _) = if filled > 0.0 {
        average_price
            .map(|execution| {
                apply_auto_fill(
                    &mut settings.positions,
                    &symbol,
                    &side,
                    filled,
                    execution,
                    fee,
                    "Ioc",
                    "portfolio-hedge",
                    None,
                )
            })
            .unwrap_or_default()
    } else {
        (0.0, None)
    };
    let realized_pnl_usd = realized_usd(&settings, &symbol, realized_pnl);
    settings.realized_pnl += realized_pnl_usd;
    settings.daily_realized_pnl += realized_pnl_usd;
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
        order.average_price = average_price;
        order.realized_pnl = realized_pnl;
        order.status = status.into();
    }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    ensure_protective_stops(client, base, state).await?;
    Ok(Some(CycleResult {
        status: "hedge_submitted",
        reason: Some(format!(
            "Portfolio 1% shock risk was USD {:.2}; submitted a risk-reducing hedge",
            portfolio_shock_risk
        )),
        symbol: Some(symbol),
        side: Some(side),
        quantity: Some(quantity),
        order_type: Some("Ioc hedge".into()),
        order_response: Some(response),
    }))
}
