fn scout_shadow_audit_payload(
    signal: &Signal,
    timestamp: u64,
    notional: f64,
    learning_context: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    evidence: Option<&WalkForward>,
    level_percent: f64,
) -> serde_json::Value {
    let mut payload = signal_audit_payload(
        signal,
        "decision",
        timestamp,
        "shadowed",
        Some("Scout shadow sample recorded; no order submitted"),
        Some(true),
        stop_loss_percent,
        take_profit_percent,
        evidence,
    );
    payload["scout"] = serde_json::json!(true);
    payload["shadow_learning_context"] = serde_json::json!(learning_context);
    payload["shadow_notional_usd"] = serde_json::json!(notional);
    payload["scout_level_percent"] = serde_json::json!(level_percent);
    if let Some(entry_price) = signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("observed_close"))
        .and_then(serde_json::Value::as_f64)
    {
        payload["shadow_entry_price"] = serde_json::json!(entry_price);
    }
    let strategy = signal.strategy.as_deref().unwrap_or("unknown-strategy");
    let level_id = format!(
        "{}|{}|{}|{}|scout-level:{level_percent:.2}",
        signal.timestamp, signal.symbol, signal.direction, strategy
    );
    payload["decision_id"] = serde_json::json!(level_id);
    payload["trade_id"] = serde_json::json!(level_id);
    payload
}

const DAILY_SCOUT_SHADOW_LEVELS_PERCENT: [f64; 4] = [0.25, 0.5, 1.0, 2.0];
const DAILY_SCOUT_SHADOW_COOLDOWN_MS: u64 = 86_400_000;

async fn record_daily_scout_shadow_ladder(
    state: &AutoTraderState,
    candidates: &[Signal],
    now: u64,
) -> Result<(), (StatusCode, String)> {
    if candidates.is_empty() {
        return Ok(());
    }
    let timestamp_ms = now.saturating_mul(1_000);
    let mut settings = state.settings.lock().await;
    settings.shadow_sample_at_ms.retain(|_, timestamp| {
        timestamp_ms.saturating_sub(*timestamp) < DAILY_SCOUT_SHADOW_COOLDOWN_MS
    });
    let mut audits = Vec::new();
    let mut settings_changed = false;
    for signal in candidates {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        let parameters = BTreeMap::new();
        let input = StrategySignal::entry(signal, &parameters);
        if strategy.evaluate(&input) != Ok(StrategyDecision::Scout) {
            continue;
        }
        let Some(distance_percent) = input.scout_breakout_distance_percent() else {
            continue;
        };
        let eligible_levels = DAILY_SCOUT_SHADOW_LEVELS_PERCENT
            .into_iter()
            .filter(|level| distance_percent <= 0.0 && distance_percent >= -*level)
            .filter(|level| {
                let sample_key = format!(
                    "daily-scout-level:{level:.2}|{}|{}|{}",
                    strategy.as_str(), signal.symbol, signal.direction
                );
                !settings.shadow_sample_at_ms.get(&sample_key)
                    .is_some_and(|previous| timestamp_ms.saturating_sub(*previous) < DAILY_SCOUT_SHADOW_COOLDOWN_MS)
            })
            .collect::<Vec<_>>();
        if eligible_levels.is_empty() {
            continue;
        }
        let active_samples = settings.shadow_sample_at_ms.values()
            .filter(|sampled_at| timestamp_ms.saturating_sub(**sampled_at) < 3_600_000)
            .count();
        if active_samples + eligible_levels.len() > settings.max_concurrent_shadow_samples {
            continue;
        }
        for level_percent in eligible_levels {
            let sample_key = format!(
                "daily-scout-level:{level_percent:.2}|{}|{}|{}",
                strategy.as_str(), signal.symbol, signal.direction
            );
            settings.shadow_sample_at_ms.insert(sample_key, timestamp_ms);
            settings_changed = true;
            let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
            let context = market_context(signal);
            audits.push(scout_shadow_audit_payload(
                signal,
                timestamp_ms,
                notional,
                &format!("scout:{context}:level:{level_percent:.2}:trade"),
                settings.stop_loss_percent,
                settings.take_profit_percent,
                None,
                level_percent,
            ));
        }
    }
    drop(settings);
    if settings_changed {
        request_settings_persist(state)?;
    }
    for audit in audits {
        queue_auto_trader_audit(state, audit);
    }
    Ok(())
}

async fn submit_cycle_signal(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    signal: &Signal,
    exploratory: bool,
    scout: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    now: u64,
) -> Result<CycleResult, (StatusCode, String)> {
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
    if scout && daily_signal_candidate(signal) {
        return Ok(CycleResult {
            status: "shadowed",
            reason: Some("Daily scout shadow ladder recorded; no order submitted".into()),
            symbol: Some(signal.symbol.clone()),
            side: Some(if signal.direction == "bullish" { "buy" } else { "sell" }.into()),
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    let capped_probe = scout || exploratory;
    let mut scout_shadow_sample = None;
    if capped_probe {
        let sampled_at = now.saturating_mul(1_000);
        let sample_key = format!(
            "{}|{}|{}",
            signal.strategy.as_deref().unwrap_or("live-insights"),
            signal.symbol,
            signal.direction
        );
        let mut settings = state.settings.lock().await;
        settings
            .shadow_sample_at_ms
            .retain(|_, timestamp| sampled_at.saturating_sub(*timestamp) < 86_400_000);
        if settings
            .shadow_sample_at_ms
            .get(&sample_key)
            .is_some_and(|previous| sampled_at.saturating_sub(*previous) < 300_000)
        {
            return Ok(skipped_for_signal(
                signal,
                "A capped exploration entry for this strategy, symbol, and direction was attempted in the last five minutes",
            ));
        }
        let recent_probe_count = settings
            .shadow_sample_at_ms
            .values()
            .filter(|timestamp| sampled_at.saturating_sub(**timestamp) < 300_000)
            .count();
        if recent_probe_count >= settings.max_concurrent_shadow_samples {
            return Ok(skipped_for_signal(
                signal,
                "The capped exploration-entry limit is already in use",
            ));
        }
        if scout {
            let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
            let context = market_context(signal);
            scout_shadow_sample = Some((notional, format!("scout:{context}:trade")));
            let open_scout_positions = settings
                .positions
                .values()
                .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
                .count();
            let pending_scout_orders = settings
                .orders
                .iter()
                .filter(|order| {
                    order.purpose == "scout"
                        && matches!(
                            order.status.as_str(),
                            "submitting" | "accepted" | "resting" | "partially_filled"
                        )
                        && order.quantity > order.filled_quantity + 1e-12
                })
                .count();
            if open_scout_positions + pending_scout_orders >= SCOUT_MAX_ACTIVE_POSITIONS {
                return Ok(skipped_for_signal(
                    signal,
                    "Maximum number of active and pending scout positions is already in use",
                ));
            }
        }
        settings.shadow_sample_at_ms.insert(sample_key, sampled_at);
        drop(settings);
        request_settings_persist(state)?;
    }
    if let Some((notional, learning_context)) = scout_shadow_sample {
        let shadow_payload = scout_shadow_audit_payload(
            signal,
            now.saturating_mul(1_000),
            notional,
            &learning_context,
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            evidence,
            1.0,
        );
        queue_auto_trader_audit(state, shadow_payload);
        return Ok(CycleResult {
            status: "shadowed",
            reason: Some("Scout shadow sample recorded; no order submitted".into()),
            symbol: Some(signal.symbol.clone()),
            side: Some(if signal.direction == "bullish" { "buy" } else { "sell" }.into()),
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        "selected",
        Some(if scout {
            "scout selected for a capped paper entry"
        } else if exploratory {
            "exploratory candidate selected for a capped paper entry"
        } else {
            "strategy candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    let mut selected_payload = selected_payload;
    selected_payload["scout"] = serde_json::json!(scout);
    queue_auto_trader_audit(state, selected_payload);
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
        market_cache_lookup_ms,
        market_http_fetch_ms,
    } = match load_signal_market_snapshot(client, position_url, dashboard, now, signal).await {
        Ok(snapshot) => snapshot,
        Err(result) => return Ok(result),
    };
    // Market snapshot retrieval above is read-only and can be slow. The selected
    // order path locks through the in-memory reservation, then releases before HTTP.
    submit_selected_signal(SelectedSignalOrderRequest {
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
    })
    .await
}

fn daily_strategy_decision(signal: &Signal) -> Option<StrategyDecision> {
    let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
    let parameters = BTreeMap::new();
    strategy.evaluate(&StrategySignal::entry(signal, &parameters)).ok()
}

fn daily_candidate_priority(settings: &Settings, signal: &Signal) -> (u8, f64, f64) {
    let decision_priority = match daily_strategy_decision(signal) {
        Some(StrategyDecision::Enter) => 0,
        Some(StrategyDecision::Scout) => 1,
        _ => 2,
    };
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
        return (decision_priority, 0.0, 0.0);
    };
    let outcomes = strategy_learning_stats(settings, &signal.symbol, strategy);
    let net_pnl_edge = recent_net_pnl_edge(&outcomes).unwrap_or(0.0);
    let confidence = confident_win_rate_edge(outcomes.positive, outcomes.samples);
    (decision_priority, net_pnl_edge, confidence)
}

fn daily_strategy_candidates<'a>(signals: &[&'a Signal], settings: &Settings) -> Vec<&'a Signal> {
    let mut candidates = signals
        .iter()
        .copied()
        .filter(|signal| daily_signal_candidate(signal))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_priority = daily_candidate_priority(settings, left);
        let right_priority = daily_candidate_priority(settings, right);
        left_priority
            .0
            .cmp(&right_priority.0)
            .then_with(|| right_priority.1.total_cmp(&left_priority.1))
            .then_with(|| right_priority.2.total_cmp(&left_priority.2))
            .then_with(|| left.strategy.cmp(&right.strategy))
    });
    let mut selected_symbols = BTreeSet::new();
    candidates.retain(|signal| selected_symbols.insert(signal.symbol.as_str()));
    candidates
}

async fn prefetch_daily_candidate_market_books(
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    candidates: &[&Signal],
    now: u64,
) -> Dashboard {
    let now_ms = now.saturating_mul(1_000);
    let stale_symbols = candidates
        .iter()
        .map(|signal| signal.symbol.as_str())
        .filter(|symbol| {
            !dashboard.markets.get(*symbol).is_some_and(|market| {
                let updated_at = market
                    .get("updated_at_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                updated_at > 0 && now_ms.saturating_sub(updated_at) <= 5_000
            })
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if stale_symbols.is_empty() {
        return dashboard.clone();
    }

    // Daily strategy orders are still reserved and submitted in candidate
    // priority order below. Only the independent market-book waits are
    // parallelized so a slow symbol cannot add its polling timeout to every
    // candidate behind it.
    let mut tasks = JoinSet::new();
    for symbol in stale_symbols {
        let client = client.clone();
        let position_url = position_url.to_owned();
        tasks.spawn(async move {
            let market = fresh_subscribed_market_book(&client, &position_url, &symbol).await;
            (symbol, market)
        });
    }
    let mut markets = (*dashboard.markets).clone();
    while let Some(result) = tasks.join_next().await {
        if let Ok((symbol, market)) = result {
            if let Some(market) = market {
                markets.insert(symbol, market);
            } else {
                // Record this cycle's failed fetch as fresh-but-empty, so the
                // ordered submission pass skips it instead of retrying the
                // same bounded polling loop serially for each strategy.
                markets.insert(symbol, serde_json::json!({"updated_at_ms": now_ms}));
            }
        }
    }
    let mut prefetched = dashboard.clone();
    prefetched.markets = Arc::new(markets);
    prefetched
}

fn daily_agreement_size_multiplier(signal: &Signal, candidates: &[&Signal]) -> f64 {
    let agreeing_strategies = candidates
        .iter()
        .filter(|candidate| {
            candidate.symbol == signal.symbol && candidate.direction == signal.direction
        })
        .filter_map(|candidate| candidate.strategy.as_deref())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    (1.0 + agreeing_strategies.saturating_sub(1).min(2) as f64 * 0.1).min(1.2)
}

fn rebalance_trend_portfolio_candidate(
    signal: &Signal,
    dashboard: &Dashboard,
    settings: &Settings,
) -> Option<Signal> {
    if signal.strategy.as_deref() != Some("risk-adjusted-trend-portfolio") {
        return Some(signal.clone());
    }
    let weight = signal
        .strategy_decision
        .as_ref()?
        .get("strategy_order_size_multiplier")?
        .as_f64()?;
    let position = settings
        .strategy_positions
        .get(&signal.symbol)
        .and_then(|lots| lots.get("risk-adjusted-trend-portfolio"))
        .or_else(|| {
            settings.positions.get(&signal.symbol).filter(|position| {
                settings.strategy_positions.is_empty()
                    && position.strategy.as_deref() == Some("risk-adjusted-trend-portfolio")
            })
        });
    let usd_rate = quote_usd_rate(settings, &signal.symbol);
    let current_value =
        if let Some(position) = position.filter(|position| position.quantity.abs() > 1e-12) {
            let book = dashboard.markets.get(&signal.symbol)?;
            let bid = book
                .get("bids")?
                .as_array()?
                .first()?
                .get("price")?
                .as_f64()?;
            let ask = book
                .get("asks")?
                .as_array()?
                .first()?
                .get("price")?
                .as_f64()?;
            let mid = (bid + ask) * 0.5;
            if !mid.is_finite() || mid <= 0.0 {
                return None;
            }
            position.quantity.abs() * mid * usd_rate
        } else {
            0.0
        };
    let target_value =
        daily_strategy_effective_budget(settings, "risk-adjusted-trend-portfolio") * weight;
    let delta = target_value - current_value;
    let rebalance_band = (target_value * 0.05).max(MIN_DEEP_BOOK_ORDER_NOTIONAL * usd_rate);
    if !delta.is_finite() || delta.abs() < rebalance_band {
        return None;
    }
    let mut adjusted = signal.clone();
    adjusted.direction = if delta > 0.0 { "bullish" } else { "bearish" }.into();
    let decision = adjusted
        .strategy_decision
        .get_or_insert_with(|| serde_json::json!({}));
    if let Some(object) = decision.as_object_mut() {
        object.insert(
            "portfolio_rebalance_delta_usd".into(),
            serde_json::json!(delta.abs()),
        );
        object.insert(
            "portfolio_rebalance_target_usd".into(),
            serde_json::json!(target_value),
        );
        object.insert(
            "portfolio_rebalance_current_usd".into(),
            serde_json::json!(current_value),
        );
    }
    Some(adjusted)
}

fn daily_strategy_batch_result(
    last_order: Option<CycleResult>,
    evaluated: usize,
    submitted: usize,
    details: &[String],
) -> CycleResult {
    let summary = if details.is_empty() {
        format!(
            "Daily strategy batch: {submitted}/{evaluated} orders queued; shared limits respected"
        )
    } else {
        format!(
            "Daily strategy batch: {submitted}/{evaluated} orders queued; {}",
            details.join("; ")
        )
    };
    if let Some(mut result) = last_order {
        result.reason = Some(summary);
        return result;
    }
    CycleResult {
        status: "skipped",
        reason: Some(summary),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
    }
}

async fn run_cycle_inner(
    state: &AutoTraderState,
    mut dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
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
    if let Some(result) = enforce_daily_realized_loss_limit(state).await? {
        return Ok(result);
    }
    let (live_strategies, strategies_enabled, strategy_settings) = {
        let settings = state.settings.lock().await;
        let legacy_composite = settings
            .strategies
            .iter()
            .any(|strategy| strategy.is_legacy_composite());
        let selected = settings
            .strategies
            .iter()
            .copied()
            .filter(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
            .collect::<Vec<_>>();
        let live =
            if legacy_composite || (settings.signal_strategies_enabled && selected.is_empty()) {
                settings
                    .strategies
                    .iter()
                    .find(|strategy| strategy.is_legacy_composite())
                    .map(|strategy| strategy.expanded_strategies().to_vec())
                    .unwrap_or_default()
            } else {
                selected
            };
        (
            live,
            settings
                .strategies
                .iter()
                .any(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry()),
            settings.clone(),
        )
    };
    let has_live_signal_strategies = !live_strategies.is_empty() && !dashboard.best.is_empty();
    let has_daily_strategy_signals = dashboard.best.iter().any(|signal| {
        signal.strategy_candidate
            && signal
                .strategy
                .as_deref()
                .and_then(Strategy::from_name)
                .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    });
    let mut strategy_filter_details = Vec::new();
    let mut daily_scout_shadow_candidates = Vec::new();
    if has_daily_strategy_signals {
        dashboard.best.retain(|signal| {
            if !signal.strategy_candidate {
                return false;
            }
            let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
                return false;
            };
            if !strategy_settings.strategies.contains(&strategy) {
                if strategy_filter_details.len() < 20 {
                    strategy_filter_details.push(format!(
                        "{} {} [{}] · Strategy disabled in AutoTrader selection",
                        signal.symbol, signal.direction, strategy.as_str()
                    ));
                }
                return false;
            }
            let parameters = BTreeMap::new();
            match strategy.evaluate(&StrategySignal::entry(signal, &parameters)) {
                Ok(StrategyDecision::Enter) => true,
                Ok(StrategyDecision::Scout) => {
                    daily_scout_shadow_candidates.push(signal.clone());
                    true
                }
                Ok(decision) => {
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · Strategy entry rule returned {:?}",
                            signal.symbol, signal.direction, strategy.as_str(), decision
                        ));
                    }
                    false
                }
                Err(error) => {
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · Strategy evaluation error: {}",
                            signal.symbol, signal.direction, strategy.as_str(), error
                        ));
                    }
                    false
                }
            }
        });
    } else if has_live_signal_strategies {
        dashboard.best = dashboard
            .best
            .iter()
            .flat_map(|source_signal| {
                let strategy_settings = &strategy_settings;
                live_strategies.iter().copied().filter_map(move |strategy| {
                    let parameters = strategy_parameter_values(
                        &strategy_settings,
                        &source_signal.symbol,
                        strategy,
                    );
                    let input = StrategySignal::entry(source_signal, &parameters);
                    if !matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter)) {
                        return None;
                    }
                    let mut signal = source_signal.clone();
                    signal.strategy = Some(strategy.as_str().to_owned());
                    signal.strategy_decision = Some(serde_json::json!({
                        "strategy": strategy.as_str(),
                        "contract": strategy.contract(),
                        "strategy_parameters": parameters,
                        "observed_insight": signal.components,
                        "direction": signal.direction,
                        "strength": signal.strength,
                        "score": signal.score,
                    }));
                    Some(signal)
                })
            })
            .collect();
    } else if strategies_enabled {
        return Ok(CycleResult {
            status: "skipped",
            reason: Some("Waiting for Insights daily strategy signals".into()),
            symbol: None,
            side: None,
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    record_daily_scout_shadow_ladder(state, &daily_scout_shadow_candidates, now).await?;
    if dashboard.best.is_empty() {
        let mut reason = "No eligible signal is currently available from enabled live or daily strategies".to_owned();
        if !strategy_filter_details.is_empty() {
            reason.push_str("; strategy filters: ");
            reason.push_str(&strategy_filter_details.join(" | "));
        }
        return Ok(CycleResult {
            status: "skipped",
            reason: Some(reason),
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
    let (open_positions, active_scout_count, strategy_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        let (open_positions, active_scout_count) = settings.positions.iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .fold((BTreeMap::new(), 0usize), |(mut positions, scouts), (symbol, position)| {
                positions.insert(symbol.clone(), position.clone());
                (positions, scouts + usize::from(position.scout_entry))
            });
        (
            open_positions,
            active_scout_count,
            settings.strategy_positions.clone(),
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
        position_block_details,
    } = collect_signal_candidates(
        &signals,
        now,
        risk_appetite,
        &open_positions,
        &strategy_positions,
        max_entries_per_auto_position,
    );
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut daily_batch_result = None;
    if has_daily_strategy_signals {
        let evaluated = fresh_directional
            .iter()
            .filter(|signal| daily_signal_candidate(signal))
            .count();
        let candidates = daily_strategy_candidates(&fresh_directional, &strategy_settings);
        if candidates.is_empty() {
            let reason = format!(
                "No daily strategy candidate passed entry checks; {} blocked by existing positions; {} stale",
                position_blocked_count, stale_signal_count,
            );
            daily_batch_result = Some(skipped(&reason));
        } else {
            let candidate_dashboard = prefetch_daily_candidate_market_books(
                &client,
                &position_url,
                &dashboard,
                &candidates,
                now,
            )
            .await;
            let mut submitted = 0;
            let mut last_order = None;
            let mut details = Vec::new();
            let rebalance_candidates = {
                let settings = state.settings.lock().await;
                candidates
                    .iter()
                    .filter_map(|signal| {
                        rebalance_trend_portfolio_candidate(signal, &candidate_dashboard, &settings)
                    })
                    .collect::<Vec<_>>()
            };
            for signal in &rebalance_candidates {
                let strategy_name = signal.strategy.as_deref().unwrap_or("unknown-strategy");
                let mut sized_signal = signal.clone();
                let decision = sized_signal
                    .strategy_decision
                    .get_or_insert_with(|| serde_json::json!({}));
                if let Some(decision) = decision.as_object_mut() {
                    decision.insert(
                        "strategy_agreement_size_multiplier".into(),
                        serde_json::json!(if signal.strategy.as_deref()
                            == Some("risk-adjusted-trend-portfolio")
                            && decision.get("portfolio_rebalance_delta_usd").is_some()
                        {
                            1.0
                        } else {
                            daily_agreement_size_multiplier(signal, &fresh_directional)
                        }),
                    );
                }
                sized_signal.entry_gate = Some(serde_json::json!({
                    "selection_mode": "daily_strategy",
                    "strategy": strategy_name,
                    "strategy_decision": sized_signal.strategy_decision.clone(),
                }));
                let is_scout = daily_strategy_decision(&sized_signal) == Some(StrategyDecision::Scout);
                let result = match submit_cycle_signal(
                    state,
                    &client,
                    &position_url,
                    &candidate_dashboard,
                    &sized_signal,
                    false,
                    is_scout,
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    signal_selection_ms,
                    now,
                )
                .await
                {
                    Ok(result) => result,
                    Err((status, error)) => {
                        details.push(format!("{strategy_name}: request error {status}: {error}"));
                        continue;
                    }
                };
                if result.status == "order_submitting" {
                    submitted += 1;
                    last_order = Some(result);
                } else {
                    let reason = result.reason.as_deref().unwrap_or(result.status);
                    details.push(format!("{strategy_name}: {reason}"));
                }
            }
            daily_batch_result = Some(daily_strategy_batch_result(
                last_order, evaluated, submitted, &details,
            ));
        }
    }
    let live_signals = if has_daily_strategy_signals {
        signals
            .iter()
            .filter(|signal| !daily_signal_candidate(signal))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let live_fresh_directional = if has_daily_strategy_signals {
        fresh_directional
            .iter()
            .copied()
            .filter(|signal| !daily_signal_candidate(signal))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let live_neutral_scout_candidates = if has_daily_strategy_signals {
        neutral_scout_candidates
            .iter()
            .filter(|signal| !daily_signal_candidate(signal))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let selection_signals = if has_daily_strategy_signals { &live_signals } else { &signals };
    let selection_fresh_directional = if has_daily_strategy_signals {
        &live_fresh_directional
    } else {
        &fresh_directional
    };
    let selection_neutral_scout_candidates = if has_daily_strategy_signals {
        &live_neutral_scout_candidates
    } else {
        &neutral_scout_candidates
    };
    let (signal, exploratory, scout) = match select_cycle_signal(SignalSelectionInput {
        state,
        dashboard: &dashboard,
        signals: selection_signals,
        fresh_directional: selection_fresh_directional,
        neutral_scout_candidates: selection_neutral_scout_candidates,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols: &stale_symbols,
        non_directional_count,
        position_blocked_count,
        position_block_details: &position_block_details,
        learning: &learning,
        risk_appetite,
        active_scout_count,
        open_positions: &open_positions,
        max_entries_per_auto_position,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    }) {
        Ok(selected) => selected,
        Err(result) => return Ok(daily_batch_result.unwrap_or(result)),
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        signal,
        &dashboard,
        &learning,
        risk_appetite,
        active_scout_count,
        if scout {
            "scout"
        } else if exploratory {
            "exploratory"
        } else {
            "strict"
        },
    ));
    let signal_result = submit_cycle_signal(
        state,
        &client,
        &position_url,
        &dashboard,
        &audited_signal,
        exploratory,
        scout,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        now,
    )
    .await?;
    if signal_result.status == "order_submitting" || daily_batch_result.is_none() {
        Ok(signal_result)
    } else {
        Ok(daily_batch_result.unwrap())
    }
}

#[cfg(test)]
mod parallel_daily_cycle_tests {
    use super::*;

    fn signal(strategy: &str) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 10,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn scout_shadow_sample_has_the_contract_required_for_learning_outcomes() {
        let payload = scout_shadow_audit_payload(
            &signal("live-momentum"),
            20_000,
            5.0,
            "scout:long:trend:ETH-USDT:live-momentum:trade",
            0.5,
            0.8,
            None,
            1.0,
        );

        assert_eq!(payload["stage"], "decision");
        assert_eq!(payload["status"], "shadowed");
        assert_eq!(payload["scout"], true);
        assert_eq!(payload["shadow_learning_context"], "scout:long:trend:ETH-USDT:live-momentum:trade");
        assert_eq!(payload["shadow_notional_usd"], 5.0);
        assert_eq!(payload["scout_level_percent"], 1.0);
        assert_eq!(payload["reason"], "Scout shadow sample recorded; no order submitted");
    }

    #[test]
    fn batch_selects_one_highest_priority_strategy_candidate_per_symbol() {
        let signals = vec![
            signal("trend-50-200"),
            signal("donchian-55-btc200"),
            signal("live-momentum"),
        ];
        let candidates =
            daily_strategy_candidates(&signals.iter().collect::<Vec<_>>(), &Settings::default());
        let names = candidates
            .iter()
            .map(|item| item.strategy.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["donchian-55-btc200"]);
    }
}
