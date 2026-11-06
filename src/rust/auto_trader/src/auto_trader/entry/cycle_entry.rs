async fn submit_cycle_signal(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
    dashboard: &Dashboard,
    signal: &Signal,
    exploratory: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    now: u64,
) -> Result<CycleResult, (StatusCode, String)> {
    if state.audit_health.load(Ordering::Relaxed) != 3
        || state.audit_failure_latched.load(Ordering::Relaxed)
    {
        return Ok(skipped_for_signal(
            signal,
            "Audit persistence unavailable; new entries paused until AutoTrader restart",
        ));
    }
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        "selected",
        Some(if exploratory {
            "exploratory candidate selected for a capped paper entry"
        } else {
            "strategy candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
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

fn evaluate_daily_strategy_signal(signal: &Signal) -> Result<StrategyDecision, String> {
    let strategy = signal
        .strategy
        .as_deref()
        .and_then(Strategy::from_name)
        .ok_or_else(|| "Unknown daily strategy".to_string())?;
    let parameters = signal_strategy_parameters(signal);
    let input = StrategySignal::entry(signal, &parameters);
    strategy.evaluate(&input)
}

async fn attach_strategy_walk_forward_evidence(
    state: &AutoTraderState,
    signals: &mut [Signal],
    now: u64,
) {
    let candidates = signals.iter().filter_map(|signal| {
        if !signal_is_fresh(signal, now) {
            return None;
        }
        let strategy = signal.strategy.as_ref()?;
        Strategy::from_name(strategy)?;
        Some(scouts::StrategyEvidenceCandidate {
            symbol: signal.symbol.clone(),
            strategy: strategy.clone(),
            signal_timestamp_ms: signal.timestamp,
            parameters: signal_strategy_parameters(signal),
        })
    }).collect::<Vec<_>>();
    if candidates.is_empty() {
        return;
    }
    let base = std::env::var("SCOUTS_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    let request = scouts::StrategyEvidenceRequest { candidates: candidates.clone() };
    let evidence = match scouts::request_strategy_walk_forward(&state.client, &base, &request).await {
        Ok(evidence) if evidence.len() == candidates.len() => evidence,
        Ok(_) => {
            log_strategy_walk_forward_failure(
                "Scouts returned a misaligned strategy walk-forward response",
            );
            return;
        }
        Err(error) => {
            log_strategy_walk_forward_failure(&format!(
                "Could not load strategy walk-forward evidence: {error}"
            ));
            return;
        }
    };
    let evidence_by_key = candidates.into_iter().zip(evidence)
        .filter_map(|(candidate, evidence)| Some((
            (candidate.strategy, candidate.symbol, candidate.signal_timestamp_ms),
            evidence?,
        )))
        .collect::<BTreeMap<_, _>>();
    for signal in signals {
        let Some(strategy) = signal.strategy.as_ref() else { continue; };
        let key = (strategy.clone(), signal.symbol.clone(), signal.timestamp);
        let Some(evidence) = evidence_by_key.get(&key) else { continue; };
        let Some(decision) = signal.strategy_decision.as_mut().and_then(serde_json::Value::as_object_mut) else { continue; };
        let typed_evidence = strategies::StrategyWalkForwardEvidence {
            sample_count: evidence.sample_count,
            positive_net_rate: Some(evidence.positive_net_rate),
            average_net_return_percent: Some(evidence.average_net_return_percent),
            last_sample_timestamp_ms: evidence.last_sample_timestamp_ms,
        };
        decision.insert(
            "strategy_walk_forward".into(),
            serde_json::to_value(typed_evidence).unwrap_or_default(),
        );
    }
}

fn log_strategy_walk_forward_failure(message: &str) {
    static LAST_WARNING_MS: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default();
    let previous = LAST_WARNING_MS.load(std::sync::atomic::Ordering::Relaxed);
    if now_ms.saturating_sub(previous) < 30_000
        || LAST_WARNING_MS
            .compare_exchange(
                previous,
                now_ms,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
    {
        return;
    }
    crate::service_log!("WARN", "{message}");
}

fn daily_candidate_priority(settings: &Settings, signal: &Signal) -> (u8, f64, f64) {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
        return (0, 0.0, 0.0);
    };
    let multiplier = settings.learned_strategy_size_multipliers
        .get(strategy.as_str()).copied().unwrap_or(1.0);
    (0, multiplier - 1.0, 0.0)
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

async fn request_learning_context_thresholds(
    client: &reqwest::Client,
    state: &AutoTraderState,
    signals: &[Signal],
    missed_bands: &[MissedOpportunityGateBand],
) -> BTreeMap<String, strategies::ContextThresholds> {
    if signals.is_empty() {
        return BTreeMap::new();
    }
    let contexts = signals.iter().map(market_context).collect::<BTreeSet<_>>();
    request_learning_context_thresholds_for_contexts(client, state, contexts, missed_bands).await
}

async fn request_learning_context_thresholds_for_contexts(
    client: &reqwest::Client,
    state: &AutoTraderState,
    contexts: BTreeSet<String>,
    missed_bands: &[MissedOpportunityGateBand],
) -> BTreeMap<String, strategies::ContextThresholds> {
    let context_prefixes = contexts.iter().flat_map(|context| {
        [format!("{context}:"), format!("scout:{context}:")]
    }).collect::<Vec<_>>();
    let (relevant_learning, import_legacy_learning) = {
        let settings = state.settings.lock().await;
        let import_legacy_learning = !settings.context_learning_imported;
        let learning = if import_legacy_learning { settings.learning.iter()
            .filter(|(key, _)| key.ends_with(":trade") && (context_prefixes.is_empty()
                || context_prefixes.iter().any(|prefix| key.starts_with(prefix))))
            .map(|(key, stats)| (key.clone(), strategies::ContextLearningStat {
                samples: stats.samples,
                positive: stats.positive,
                realized_pnl: stats.realized_pnl,
                updated_at_ms: stats.updated_at_ms,
            }))
            .collect() } else { BTreeMap::new() };
        (learning, import_legacy_learning)
    };
    let bands = missed_bands.iter().map(|band| strategies::ContextMissedOpportunityBand {
        gate: match band.gate {
            MissedOpportunityGate::Score => "score",
            MissedOpportunityGate::Strength => "strength",
            MissedOpportunityGate::Other => "other",
        }.to_owned(),
        threshold: band.threshold,
        horizon_seconds: band.horizon_seconds,
        sample_count: band.sample_count,
        average_net_return_percent: band.average_net_return_percent,
        positive_net_rate: band.positive_net_rate,
        time_to_profit_sample_count: band.time_to_profit_sample_count,
        time_to_profit_rate: band.time_to_profit_rate,
        average_time_to_profit_seconds: band.average_time_to_profit_seconds,
        stop_target_sample_count: band.stop_target_sample_count,
        stop_target_average_net_return_percent: band.stop_target_average_net_return_percent,
        stop_target_positive_net_rate: band.stop_target_positive_net_rate,
    }).collect::<Vec<_>>();
    let request = strategies::ContextThresholdRequest {
        learning: relevant_learning,
        contexts: contexts.into_iter().map(|context| strategies::ContextThresholdInput {
            context,
            missed_opportunity_bands: bands.clone(),
        }).collect(),
    };
    let base = std::env::var("LEARNING_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3010".into());
    let response = client.post(format!("{}/context-thresholds", base.trim_end_matches('/')))
        .timeout(Duration::from_millis(250))
        .json(&request)
        .send()
        .await
        .map_err(|error| error.to_string())
        .and_then(|response| response.error_for_status().map_err(|error| error.to_string()));
    match response {
        Ok(response) => match response.json::<strategies::ContextThresholdResponse>().await {
            Ok(response) => {
                if import_legacy_learning {
                    let mut settings = state.settings.lock().await;
                    settings.context_learning_imported = true;
                    drop(settings);
                    let _ = request_settings_persist(state);
                }
                response.thresholds
            },
            Err(error) => {
                crate::service_log!("WARN", "Learning service returned invalid entry thresholds; using neutral defaults: {error}");
                BTreeMap::new()
            }
        },
        Err(error) => {
            crate::service_log!("WARN", "Learning service unavailable for entry thresholds; using neutral defaults: {error}");
            BTreeMap::new()
        }
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
        let enabled_live =
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
            enabled_live,
            settings
                .strategies
                .iter()
                .any(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry()),
            settings.clone(),
        )
    };
    let all_live_source_signals = live_strategy_source_signals(&dashboard.best);
    let mut stale_live_source_signals = all_live_source_signals
        .iter()
        .filter(|signal| !signal_is_fresh(signal, now))
        .cloned()
        .collect::<Vec<_>>();
    let has_live_source_signals = !all_live_source_signals.is_empty();
    let mut live_source_signals = all_live_source_signals
        .iter()
        .filter(|signal| signal_is_fresh(signal, now))
        .cloned()
        .collect::<Vec<_>>();
    for signal in &mut live_source_signals {
        inject_learned_parameters(&strategy_settings, signal);
    }
    for signal in &mut dashboard.best {
        if daily_signal_candidate(signal) {
            inject_learned_parameters(&strategy_settings, signal);
        }
    }
    let has_daily_strategy_signals = dashboard.best.iter().any(|signal| {
        signal.strategy_candidate
            && signal
                .strategy
                .as_deref()
                .and_then(Strategy::from_name)
                .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    });
    if has_daily_strategy_signals {
        attach_strategy_walk_forward_evidence(state, &mut dashboard.best, now).await;
    }
    let has_live_signal_strategies = !live_strategies.is_empty() && has_live_source_signals;
    let mut strategy_filter_details = Vec::new();
    if has_daily_strategy_signals {
        dashboard.best.retain(|signal| {
            if !signal.strategy_candidate {
                return false;
            }
            let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
                return false;
            };
            if !strategy_settings.strategies.contains(&strategy) {
                return false;
            }
            let is_scout_probe = signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("scout_parameter_probe"))
                .and_then(serde_json::Value::as_bool) == Some(true);
            if is_scout_probe { return false; }
            match evaluate_daily_strategy_signal(signal) {
                Ok(StrategyDecision::Enter) => true,
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
    }
    let mut live_strategy_candidates = Vec::new();
    if has_live_signal_strategies {
        for source_signal in &live_source_signals {
            for strategy in live_strategies.iter().copied() {
                let parameters = strategy_parameter_values(
                    &strategy_settings,
                    &source_signal.symbol,
                    strategy,
                );
                let Some(mut signal) = live_strategy_feature_signal(source_signal, strategy, &parameters) else {
                    continue;
                };
                signal.strategy = Some(strategy.as_str().to_owned());
                signal.strategy_decision = Some(serde_json::json!({
                    "scout_only": false,
                    "strategy": strategy.as_str(),
                    "contract": strategy.contract(),
                    "strategy_parameters": parameters,
                    "observed_insight": signal.components,
                    "direction": signal.direction,
                    "strength": signal.strength,
                    "score": signal.score,
                    "direction_source": "aggregate_signal",
                }));
                live_strategy_candidates.push(signal);
            }
        }
        attach_strategy_walk_forward_evidence(state, &mut live_strategy_candidates, now).await;
    }
    let live_strategy_signals = live_strategy_candidates
        .into_iter()
        .filter(|signal| {
            let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
                return false;
            };
            let parameters = signal_strategy_parameters(signal);
            matches!(
                strategy.evaluate(&StrategySignal::entry(signal, &parameters)),
                Ok(StrategyDecision::Enter)
            )
        })
        .collect::<Vec<_>>();

    if has_daily_strategy_signals {
        dashboard.best.extend(live_strategy_signals);
        dashboard.best.append(&mut stale_live_source_signals);
    } else if has_live_signal_strategies {
        dashboard.best = if live_strategy_signals.is_empty() {
            live_source_signals
                .into_iter()
                .filter(|signal| signal.strategy.is_none() && !signal.strategy_candidate)
                .collect()
        } else {
            live_strategy_signals
        };
        dashboard.best.append(&mut stale_live_source_signals);
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
    let selection_started = Instant::now();
    let (
        risk_appetite,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    ) = {
        let settings = state.settings.lock().await;
        (
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
    let learning_signals = signals
        .iter()
        .filter(|signal| signal_is_fresh(signal, now))
        .cloned()
        .collect::<Vec<_>>();
    let learning_thresholds = request_learning_context_thresholds(
        &client,
        state,
        &learning_signals,
        &dashboard.missed_opportunity_bands,
    )
    .await;
    let (open_positions, strategy_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        let open_positions = settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12 && !position.scout_entry)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<BTreeMap<_, _>>();
        let strategy_positions = settings.strategy_positions.iter().filter_map(|(symbol, lots)| {
            let real_lots = lots.iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12 && !position.scout_entry)
                .map(|(strategy, position)| (strategy.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>();
            (!real_lots.is_empty()).then(|| (symbol.clone(), real_lots))
        }).collect();
        (
            open_positions,
            strategy_positions,
            settings.max_entries_per_auto_position,
        )
    };
    // Walk-forward and learned-threshold calls can take hundreds of
    // milliseconds. Recheck the input age after those calls before allowing
    // any signal to proceed toward entry.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let FreshSignalCandidates {
        fresh_directional,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
        position_block_details,
    } = collect_signal_candidates(
        &signals,
        now,
        &open_positions,
        &strategy_positions,
        max_entries_per_auto_position,
    );
    if stale_signal_count > 0 {
        state
            .stale_signal_candidates_total
            .fetch_add(stale_signal_count as u64, Ordering::Relaxed);
        state
            .stale_signal_cycles_total
            .fetch_add(1, Ordering::Relaxed);
    }
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
                let result = match submit_cycle_signal(
                    state,
                    &client,
                    &position_url,
                    &candidate_dashboard,
                    &sized_signal,
                    false,
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
            .filter(|signal| is_fallback_signal_candidate(signal))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let live_fresh_directional = if has_daily_strategy_signals {
        fresh_directional
            .iter()
            .copied()
            .filter(|signal| is_fallback_signal_candidate(signal))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let selection_signals = if has_daily_strategy_signals {
        &live_signals
    } else {
        &signals
    };
    let selection_fresh_directional = if has_daily_strategy_signals {
        &live_fresh_directional
    } else {
        &fresh_directional
    };
    let selection_result = select_cycle_signal(SignalSelectionInput {
        dashboard: &dashboard,
        signals: selection_signals,
        fresh_directional: selection_fresh_directional,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols: &stale_symbols,
        non_directional_count,
        position_blocked_count,
        position_block_details: &position_block_details,
        learning_thresholds: &learning_thresholds,
        risk_appetite,
        open_positions: &open_positions,
        max_entries_per_auto_position,
    });
    let (signal, exploratory) = match selection_result {
        Ok((signal, exploratory)) => (signal.clone(), exploratory),
        Err(result) => return Ok(daily_batch_result.unwrap_or(result)),
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        &signal,
        &dashboard,
        &learning_thresholds,
        risk_appetite,
        if exploratory {
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
    fn daily_strategy_reject_remains_a_reject_for_trade_decisions() {
        let mut candidate = signal("trend-50-200");
        candidate.strategy_decision = Some(serde_json::json!({
            "observed_close": 90.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "primary_passed": false,
            "btc_gate_required": false,
            "btc_gate_passed": true
        }));

        assert_eq!(
            evaluate_daily_strategy_signal(&candidate).ok(),
            Some(StrategyDecision::Reject)
        );
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
