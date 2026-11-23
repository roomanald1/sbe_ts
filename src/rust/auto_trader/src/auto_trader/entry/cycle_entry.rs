static NEXT_ENTRY_CYCLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn strategy_entry_rejection_reason(
    signal: &Signal,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
) -> String {
    if strategy.is_live_insight() {
        let input = StrategySignal::entry(signal, parameters);
        if let Some(reason) = strategy.entry_rejection_reason(&input) {
            return reason;
        }
        return "Live strategy entry conditions were not met; the evaluator supplied no detail"
            .to_owned();
    }
    let Some(details) = signal.strategy_decision.as_ref() else {
        return format!(
            "{} rejected {}: no strategy rule metrics were attached to the signal",
            strategy.as_str(),
            signal.symbol
        );
    };
    if !signal.strategy_candidate {
        return format!(
            "{} rejected {}: the signal was not marked as a strategy candidate",
            strategy.as_str(),
            signal.symbol
        );
    }
    let btc_gate_failed = details
        .get("btc_gate_passed")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
        || matches!(
            (
                details.get("btc_close").and_then(serde_json::Value::as_f64),
                details.get("btc_sma").and_then(serde_json::Value::as_f64),
            ),
            (Some(close), Some(sma)) if close.is_finite() && sma.is_finite() && close <= sma
        );
    if details
        .get("btc_gate_required")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && btc_gate_failed
    {
        let window = details
            .get("btc_window_days")
            .and_then(serde_json::Value::as_u64)
            .map(|days| format!("SMA{days}"))
            .unwrap_or_else(|| "trend average".to_owned());
        return match (
            details
                .get("btc_close")
                .and_then(serde_json::Value::as_f64),
            details.get("btc_sma").and_then(serde_json::Value::as_f64),
        ) {
            (Some(close), Some(sma)) => {
                format!("BTC trend gate not passed: close {close:.2} vs {window} {sma:.2}")
            }
            _ => format!("BTC trend gate not passed ({window})"),
        };
    }
    if details
        .get("primary_passed")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        let rule = details
            .get("primary_rule")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("primary strategy rule");
        let unit = details
            .get("primary_metric_unit")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let observed = details
            .get("primary_metric_value")
            .and_then(serde_json::Value::as_f64)
            .map(|value| format!("{value:.2} {unit}"))
            .unwrap_or_else(|| "unavailable".to_owned());
        let threshold = details
            .get("primary_level")
            .and_then(serde_json::Value::as_f64)
            .map(|value| format!("{value:.2} {unit}"))
            .unwrap_or_else(|| "unavailable".to_owned());
        return format!("{rule} not met: observed {observed}, threshold {threshold}");
    }
    if details
        .get("secondary_passed")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        let rule = details
            .get("secondary_rule")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("secondary strategy rule");
        let distance = details
            .get("secondary_distance_percent")
            .and_then(serde_json::Value::as_f64)
            .map(|value| format!(" (distance {value:.2}%)"))
            .unwrap_or_default();
        return format!("{rule} not met{distance}");
    }
    if matches!(
        strategy,
        Strategy::Donchian10Btc200
            | Strategy::Donchian20Btc200
            | Strategy::Donchian20Btc200Hold3
            | Strategy::Donchian20Sma200
            | Strategy::Donchian55Btc200
    ) {
        if let (Some(close), Some(level)) = (
            details.get("observed_close").and_then(serde_json::Value::as_f64),
            details.get("primary_level").and_then(serde_json::Value::as_f64),
        ) {
            if close.is_finite() && close > 0.0 && level.is_finite() && level > 0.0 {
                let distance_percent = (close / level - 1.0) * 100.0;
                let allowed_distance_percent = parameters
                    .get("breakout_entry_distance_percent")
                    .copied()
                    .unwrap_or_default();
                if distance_percent < -allowed_distance_percent {
                    return format!(
                        "{} Donchian breakout entry rule not met: close {close:.2} is {distance_percent:.2}% from breakout level {level:.2}; allowed distance is {allowed_distance_percent:.2}%",
                        strategy.as_str()
                    );
                }
            }
        }
    }
    if let Some(reason) = strategy.entry_rejection_reason(&StrategySignal::entry(signal, parameters)) {
        return reason;
    }
    format!(
        "{} rejected {}: its entry conditions were not met",
        strategy.as_str(),
        signal.symbol
    )
}

fn update_latest_strategy_blocker(
    state: &AutoTraderState,
    strategy: &str,
    timestamp_ms: u64,
    blocker: Option<StrategyBlocker>,
) {
    let mut blockers = state
        .latest_strategy_blockers
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if blockers
        .get(strategy)
        .is_some_and(|existing| existing.timestamp_ms > timestamp_ms)
    {
        return;
    }
    if let Some(blocker) = blocker {
        blockers.insert(strategy.to_owned(), blocker);
    } else {
        blockers.remove(strategy);
    }
}

fn candidate_selection_blocker_reason(
    signal: &Signal,
    dashboard: &Dashboard,
    learning_thresholds: &BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
    now: u64,
    fallback_reason: &str,
) -> String {
    if !signal_is_fresh(signal, now) {
        return "Signal is stale".to_owned();
    }
    if !matches!(signal.direction.as_str(), "bullish" | "bearish") {
        return format!("Signal direction is {}", signal.direction);
    }
    if !passes_pullback_confirmation(signal) {
        return "Awaiting 10s pullback confirmation".to_owned();
    }
    let audit = candidate_entry_audit_value(
        signal,
        dashboard,
        learning_thresholds,
        risk_appetite,
        "rejected",
        now.saturating_mul(1_000),
    );
    let failures = audit
        .get("rejection_reasons")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    if failures.is_empty() {
        format!("Not selected this cycle: {fallback_reason}")
    } else {
        format!("Entry gates blocked this signal: {}", failures.join(", "))
    }
}

fn record_cycle_candidate_blockers(
    state: &AutoTraderState,
    signals: &[Signal],
    active_live_strategies: &[Strategy],
    selected: Option<&Signal>,
    dashboard: &Dashboard,
    learning_thresholds: &BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
    now: u64,
    fallback_reason: &str,
) {
    let timestamp_ms = now.saturating_mul(1_000);
    let mut candidates_by_strategy = BTreeMap::<String, Vec<&Signal>>::new();
    for signal in signals {
        let Some(strategy) = signal.strategy.as_deref() else {
            continue;
        };
        candidates_by_strategy
            .entry(strategy.to_owned())
            .or_default()
            .push(signal);
    }
    for (strategy, candidates) in candidates_by_strategy {
        if selected
            .and_then(|signal| signal.strategy.as_deref())
            .is_some_and(|selected_strategy| selected_strategy == strategy)
        {
            update_latest_strategy_blocker(state, &strategy, timestamp_ms, None);
            continue;
        }
        let mut candidates = candidates;
        candidates.sort_by(|left, right| right.strength.total_cmp(&left.strength));
        candidates.dedup_by(|left, right| left.symbol == right.symbol
            && left.direction == right.direction
            && left.strategy_decision == right.strategy_decision);
        let reasons = candidates.iter().map(|candidate| {
            let detail = candidate_selection_blocker_reason(
                candidate, dashboard, learning_thresholds, risk_appetite, now, fallback_reason,
            );
            format!("{} {}: {}", candidate.symbol, candidate.direction, detail)
        }).collect::<Vec<_>>();
        let reason = format!("{} candidate(s) evaluated: {}", reasons.len(), reasons.join("; "));
        let symbol = if candidates.len() == 1 {
            candidates[0].symbol.clone()
        } else {
            "MULTIPLE".to_owned()
        };
        update_latest_strategy_blocker(
            state,
            &strategy,
            timestamp_ms,
            Some(StrategyBlocker {
                strategy: strategy.clone(),
                symbol,
                reason,
                timestamp_ms,
            }),
        );
    }
    for strategy in active_live_strategies {
        let strategy_name = strategy.as_str();
        if selected
            .and_then(|signal| signal.strategy.as_deref())
            .is_some_and(|selected_strategy| selected_strategy == strategy_name)
        {
            update_latest_strategy_blocker(state, strategy_name, timestamp_ms, None);
            continue;
        }
        let has_current_blocker = state
            .latest_strategy_blockers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(strategy_name)
            .is_some_and(|blocker| blocker.timestamp_ms >= timestamp_ms);
        if has_current_blocker {
            continue;
        }
        let reason = if selected.is_some() {
            "No candidate from this strategy reached selection; another strategy was selected"
                .to_owned()
        } else {
            format!("No candidate from this strategy reached selection: {fallback_reason}")
        };
        update_latest_strategy_blocker(
            state,
            strategy_name,
            timestamp_ms,
            Some(StrategyBlocker {
                strategy: strategy_name.to_owned(),
                symbol: "MARKET".to_owned(),
                reason,
                timestamp_ms,
            }),
        );
    }
}

async fn submit_cycle_signal(
    state: &AutoTraderState,
    dashboard: &Dashboard,
    signal: &Signal,
    exploratory: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    context_threshold_refresh_ms: f64,
    cycle_started_at: Instant,
    now: u64,
) -> Result<CycleResult, (StatusCode, String)> {
    let daily_rule_checks = daily_cold_start_exploration_checks(signal);
    let daily_strategy_classification = signal
        .strategy
        .as_deref()
        .and_then(Strategy::from_name)
        .is_some_and(|kind| !kind.is_live_insight() && !kind.is_dated_carry());
    // Daily candidates with an eligible strategy rule but no walk-forward
    // sample are the bounded paper cold-start path. Keep this contract explicit
    // at the execution handoff, even if the signal entered the cycle through a
    // different upstream selection path.
    let daily_cold_start = daily_strategy_classification
        && daily_rule_checks.0
        && daily_rule_checks.1
        && !daily_rule_checks.2
        || is_daily_cold_start_exploration_candidate(signal);
    let daily_cold_start = !exploratory && daily_cold_start;
    eprintln!(
        "COLD_START_TRACE strategy={:?} candidate={} exploratory_input={} daily_classification={} parameter_probe={} rule_checks={:?} cold_start={}",
        signal.strategy,
        signal.strategy_candidate,
        exploratory,
        signal
            .strategy
            .as_deref()
            .and_then(Strategy::from_name)
            .is_some_and(|kind| !kind.is_live_insight() && !kind.is_dated_carry()),
        is_daily_parameter_exploration_probe(signal),
        daily_rule_checks,
        daily_cold_start,
    );
    if signal
        .strategy
        .as_deref()
        .is_some_and(|strategy| strategy.contains("monthly"))
    {
        let (rule_evidence_available, rule_evidence_eligible, walk_forward_present) =
            daily_cold_start_exploration_checks(signal);
        let decision = signal.strategy_decision.as_ref();
        let walk_forward_samples = decision
            .and_then(|decision| decision.get("strategy_walk_forward"))
            .and_then(|evidence| evidence.get("sampleCount"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let required_samples = decision
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(|parameters| parameters.get("scout_minimum_walk_forward_samples"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        crate::service_log!(
            "INFO",
            "Daily cold-start trace: strategy_candidate={} parameter_probe={} rule_evidence_available={} rule_evidence_eligible={} walk_forward_present={} walk_forward_samples={} required_samples={} exploratory_input={} cold_start={}",
            signal.strategy_candidate,
            is_daily_parameter_exploration_probe(signal),
            rule_evidence_available,
            rule_evidence_eligible,
            walk_forward_present,
            walk_forward_samples,
            required_samples,
            exploratory,
            daily_cold_start,
        );
    }
    let mut selected_signal = signal.clone();
    if daily_cold_start {
        let gate = selected_signal
            .entry_gate
            .get_or_insert_with(|| serde_json::json!({}));
        if let Some(gate) = gate.as_object_mut() {
            gate.insert(
                "selection_mode".into(),
                serde_json::json!("daily_strategy_cold_start_exploration"),
            );
            gate.insert("exploratory".into(), serde_json::json!(true));
            let checks = gate
                .entry("checks")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(checks) = checks.as_object_mut() {
                checks.insert("cold_start_exploration_pass".into(), serde_json::json!(true));
            }
        }
    }
    if daily_strategy_classification {
        let (rule_evidence_available, rule_evidence_eligible, walk_forward_present) =
            daily_cold_start_exploration_checks(signal);
        let gate = selected_signal
            .entry_gate
            .get_or_insert_with(|| serde_json::json!({}));
        if let Some(gate) = gate.as_object_mut() {
            let checks = gate
                .entry("checks")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(checks) = checks.as_object_mut() {
                checks.insert(
                    "daily_rule_evidence_available".into(),
                    serde_json::json!(rule_evidence_available),
                );
                checks.insert(
                    "daily_rule_evidence_eligible".into(),
                    serde_json::json!(rule_evidence_eligible),
                );
                checks.insert(
                    "strategy_walk_forward_evidence_present".into(),
                    serde_json::json!(walk_forward_present),
                );
                checks.insert(
                    "cold_start_exploration_pass".into(),
                    serde_json::json!(daily_cold_start),
                );
                checks.insert(
                    "daily_strategy_classification_pass".into(),
                    serde_json::json!(daily_strategy_classification),
                );
                checks.insert(
                    "strategy_candidate_flag".into(),
                    serde_json::json!(signal.strategy_candidate),
                );
                checks.insert(
                    "daily_parameter_probe".into(),
                    serde_json::json!(is_daily_parameter_exploration_probe(signal)),
                );
            }
        }
    }
    let signal = &selected_signal;
    let exploratory = exploratory || daily_cold_start;
    if state.audit_health.load(Ordering::Relaxed) != 3
        || state.audit_failure_latched.load(Ordering::Relaxed)
    {
        return Ok(skipped_for_signal(
            signal,
            "Audit persistence unavailable; new entries paused until AutoTrader restart",
        ));
    }
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
    } = match load_signal_market_snapshot(dashboard, now, signal).await {
        Ok(snapshot) => snapshot,
        Err(result) => return Ok(result),
    };
    // Market snapshot retrieval above is read-only and can be slow. The selected
    // order path locks through the in-memory reservation, then releases before HTTP.
    submit_selected_signal(SelectedSignalOrderRequest {
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
    })
    .await
}

fn selected_signal_outcome_audit_payload(
    signal: &Signal,
    result: &CycleResult,
    exploratory: bool,
    timestamp_ms: u64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> serde_json::Value {
    let reason = result
        .reason
        .as_deref()
        .unwrap_or("No order submitted; no reason was provided");
    let strategy = signal.strategy.as_deref().unwrap_or("generic_signal");
    let gate_id = final_entry_gate_id(reason);
    let mut payload = strategy_rejection_audit_payload(
        signal,
        strategy,
        reason,
        timestamp_ms,
        stop_loss_percent,
        take_profit_percent,
        "final_entry",
    );
    payload["exploratory"] = serde_json::json!(exploratory);
    payload["status"] = serde_json::json!(result.status);
    if let Some(selection_gate) = signal.entry_gate.clone() {
        payload["entry_gate"] = selection_gate.clone();
        payload["gate_path"] = serde_json::json!([
            {
                "stage": "candidate_selection",
                "status": "selected",
                "selection_mode": selection_gate.get("selection_mode"),
                "gate": selection_gate,
            },
            {
                "stage": "execution",
                "status": "blocked",
                "gate_id": gate_id,
                "reason": reason,
                "evaluated": true,
                "short_circuited": true,
                "subsequent_checks": "not_evaluated_after_block",
            }
        ]);
    }
    payload["execution_gate"] = serde_json::json!({
        "gate_id": gate_id,
        "stage": "post_selection_execution",
        "status": "blocked",
        "result_status": result.status,
        "reason": reason,
        "evaluated": true,
        "order_submitted": false,
        "short_circuited": true,
        "subsequent_checks": "not_evaluated_after_block",
    });
    payload
}

fn final_entry_gate_id(reason: &str) -> &'static str {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("estimated spread, sweep, and fee cost") {
        "execution_cost_vs_edge"
    } else if reason.contains("walk_forward") {
        "walk_forward_evidence"
    } else if reason.contains("cannot estimate execution slippage") {
        "order_book_unavailable"
    } else if reason.contains("deep book") || reason.contains("deep-book") {
        "deep_book_liquidity"
    } else if reason.contains("strategy entry paused") {
        "learned_strategy_pause"
    } else if reason.contains("daily") || reason.contains("notional") || reason.contains("budget") {
        "notional_budget"
    } else if reason.contains("position") || reason.contains("open order") {
        "position_or_order_limit"
    } else if reason.contains("stop") || reason.contains("loss") {
        "protective_risk_limit"
    } else {
        "execution_or_risk_gate"
    }
}

async fn evaluate_daily_strategy_signals(
    signals: &[Signal],
    enabled_strategies: &[Strategy],
) -> Vec<Result<StrategyDecision, String>> {
    signals.iter().map(|signal| {
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name);
        if !strategy.is_some_and(|strategy| enabled_strategies.contains(&strategy)) {
            return Err("Daily strategy is not enabled".to_owned());
        }
        if !live_worker_pre_evaluated(signal) {
            return Err("Daily strategy decision is missing its worker result".to_owned());
        }
        Ok(if signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("live_worker_decision"))
            .and_then(serde_json::Value::as_str) == Some("enter")
        {
            StrategyDecision::Enter
        } else {
            StrategyDecision::Reject
        })
    }).collect()
}

async fn attach_strategy_walk_forward_evidence(
    state: &AutoTraderState,
    signals: &mut [Signal],
    now: u64,
) {
    let evidence_by_profile = state
        .settings
        .lock()
        .await
        .learned_walk_forward_evidence
        .clone();
    for signal in signals {
        if !signal_is_fresh(signal, now) || live_worker_pre_evaluated(signal) {
            continue;
        }
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        let parameters = signal_strategy_parameters(signal);
        let profile_id = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| strategy::profile_id(&signal.symbol, strategy.as_str(), &parameters));
        let profile_symbol = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_symbol"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&signal.symbol);
        let evidence = evidence_by_profile
            .get(&format!("{profile_id}|regime:{}", strategy::market_regime(signal).unwrap_or("mixed")))
            .or_else(|| {
                evidence_by_profile.get(&format!("{profile_symbol}|{}|regime:{}", strategy.as_str(), strategy::market_regime(signal).unwrap_or("mixed")))
            })
            .or_else(|| evidence_by_profile.get(&profile_id))
            .or_else(|| {
                evidence_by_profile.get(&format!("{profile_symbol}|{}", strategy.as_str()))
            });
        let Some(evidence) = evidence else {
            continue;
        };
        let Some(decision) = signal
            .strategy_decision
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        decision.insert(
            "strategy_walk_forward".into(),
            serde_json::to_value(evidence).unwrap_or_default(),
        );
    }
}

fn daily_candidate_priority(settings: &Settings, signal: &Signal) -> (u8, f64, f64) {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
        return (0, 0.0, 0.0);
    };
    let multiplier = settings
        .learned_strategy_size_multipliers
        .get(strategy.as_str())
        .copied()
        .unwrap_or(1.0);
    let is_probe = is_daily_parameter_exploration_probe(signal);
    let probe_value = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("scout_parameter_value"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default();
    // Prefer a normal rule pass; when only probe candidates qualify, choose
    // the least permissive threshold that reaches this observation.
    (u8::from(is_probe), multiplier - 1.0, if is_probe { -probe_value } else { 0.0 })
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
        tasks.spawn(async move {
            let market = fresh_subscribed_market_book(&symbol).await;
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
        filled_quantity: None,
        entry_diagnostic: None,
    }
}

async fn request_learning_context_thresholds(
    state: &AutoTraderState,
    signals: &[Signal],
) -> BTreeMap<String, strategy::ContextThresholds> {
    let contexts = signals.iter().map(market_context).collect::<BTreeSet<_>>();
    let settings = state.settings.lock().await;
    contexts
        .into_iter()
        .filter_map(|context| {
            settings
                .learned_context_thresholds
                .get(&context)
                .cloned()
                .map(|thresholds| (context, thresholds))
        })
        .collect()
}

fn candidate_cycle_blocker(
    raw_signal_count: usize,
    candidate_count: usize,
    fresh_candidate_count: usize,
    strategy_rejected_count: usize,
) -> (EntryCycleOutcome, &'static str) {
    if raw_signal_count == 0 {
        (EntryCycleOutcome::NoSignals, "signal_source")
    } else if strategy_rejected_count > 0 {
        (EntryCycleOutcome::CandidatesRejected, "strategy_entry_gate")
    } else if candidate_count == 0 {
        (EntryCycleOutcome::NoEligibleCandidates, "candidate_filter")
    } else if fresh_candidate_count == 0 {
        (EntryCycleOutcome::NoFreshCandidates, "signal_freshness_or_direction")
    } else {
        (EntryCycleOutcome::CandidatesRejected, "candidate_selection_gates")
    }
}

fn update_entry_candidate_diagnostic(
    diagnostic: &mut LatestEntryDiagnostic,
    signal: &Signal,
    stage: &str,
    reason: Option<&str>,
) {
    let decision_id = signal_decision_id(signal);
    let candidate_index = diagnostic
        .candidate_details
        .iter()
        .position(|candidate| candidate.decision_id == decision_id)
        .unwrap_or_else(|| {
            diagnostic.candidate_details.push(EntryCandidateDiagnostic {
                decision_id: decision_id.clone(),
                worker_event_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("live_worker_event_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                profile_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| {
                        decision
                            .get("learning_profile_id")
                            .or_else(|| decision.get("scout_parameter_profile_id"))
                    })
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                symbol: signal.symbol.clone(),
                strategy: signal.strategy.clone(),
                stage: stage.to_owned(),
                observed: serde_json::json!({
                    "score": signal.score,
                    "strength": signal.strength,
                    "direction": signal.direction,
                }),
                required: serde_json::json!({}),
                checks: serde_json::json!({}),
                evidence_contract: serde_json::Value::Null,
                measurements: Vec::new(),
                reasons: Vec::new(),
            });
            diagnostic.candidate_details.len() - 1
        });
    let candidate = &mut diagnostic.candidate_details[candidate_index];
    candidate.stage = stage.to_owned();
    if let Some(reason) = reason.filter(|reason| !reason.trim().is_empty()) {
        if !candidate.reasons.iter().any(|existing| existing == reason) {
            candidate.reasons.push(reason.to_owned());
        }
        candidate
            .measurements
            .extend(parse_reported_gate_measurements(stage, reason));
    }
    recompute_entry_candidate_gate_counts(diagnostic);
}

fn parse_reported_gate_measurements(
    gate_id: &str,
    detail: &str,
) -> Vec<EntryGateMeasurement> {
    fn parse_number(token: &str) -> Option<f64> {
        let numeric = token.trim_matches(|character: char| {
            !character.is_ascii_digit() && character != '.' && character != '-'
        });
        let value = numeric.parse::<f64>().ok()?;
        value.is_finite().then_some(value)
    }

    detail
        .split([';', '|', '·', ','])
        .filter_map(|segment| {
            [">=", "<=", ">", "<"]
                .into_iter()
                .find_map(|operator| {
                    let (left, right) = segment.split_once(operator)?;
                    let observed = left.split_whitespace().rev().find_map(parse_number)?;
                    let required_token = right.split_whitespace().find(|token| {
                        token.chars().any(|character| character.is_ascii_digit())
                    })?;
                    let required = parse_number(required_token)?;
                    let numeric_end = required_token
                        .find(|character: char| {
                            !character.is_ascii_digit() && character != '.' && character != '-'
                        })
                        .unwrap_or(required_token.len());
                    let unit = if numeric_end < required_token.len() {
                        Some(required_token[numeric_end..].to_owned())
                    } else {
                        right
                            .split_whitespace()
                            .skip_while(|token| *token != required_token)
                            .nth(1)
                            .filter(|unit| {
                                unit.chars().any(|character| character.is_ascii_alphabetic() || character == '%')
                            })
                            .map(|unit| unit.trim_matches(|character: char| !character.is_ascii_alphabetic() && character != '%').to_owned())
                    };
                    Some(EntryGateMeasurement {
                        gate_id: gate_id.to_owned(),
                        observed: Some(observed),
                        operator: Some(operator.to_owned()),
                        required: Some(required),
                        unit,
                        status: "reported_comparison".into(),
                        detail: segment.trim().to_owned(),
                    })
                })
        })
        .collect()
}

fn candidate_threshold_measurements(audit: &serde_json::Value) -> Vec<EntryGateMeasurement> {
    [
        (
            "strength_floor",
            "strength",
            "strength_floor",
            "strength_pass",
        ),
        (
            "raw_score_floor",
            "absolute_score",
            "exploration_score_absolute_floor",
            "exploration_score_pass",
        ),
    ]
    .into_iter()
    .filter_map(|(gate_id, observed_key, required_key, check_key)| {
        let observed = audit["observed"][observed_key].as_f64()?;
        let required = audit["thresholds"][required_key].as_f64()?;
        let passed = audit["checks"][check_key].as_bool()?;
        Some(EntryGateMeasurement {
            gate_id: gate_id.into(),
            observed: Some(observed),
            operator: Some(">=".into()),
            required: Some(required),
            unit: None,
            status: if passed { "passed" } else { "failed" }.into(),
            detail: format!("observed {observed:.6}; required at least {required:.6}"),
        })
    })
    .collect()
}

fn recompute_entry_candidate_gate_counts(diagnostic: &mut LatestEntryDiagnostic) {
    let mut counts = BTreeMap::<(String, String, Option<String>), u64>::new();
    for candidate in &diagnostic.candidate_details {
        let mut gates = BTreeSet::new();
        if candidate.checks["strength_pass"] == false {
            gates.insert("strength_floor");
        }
        if candidate.checks["exploration_score_pass"] == false {
            gates.insert("raw_score_floor");
        }
        if candidate.checks["exploration_context_pass"] == false {
            gates.insert("exploration_context");
        }
        match candidate.stage.as_str() {
            "selection_rejected" => {
                gates.insert("candidate_selection_gates");
            }
            "execution_failed" | "execution_blocked" => {
                gates.insert("execution_or_risk_gate");
            }
            _ => {}
        }
        for gate_id in gates {
            *counts
                .entry((
                    gate_id.to_owned(),
                    candidate.symbol.clone(),
                    candidate.strategy.clone(),
                ))
                .or_default() += 1;
        }
    }
    diagnostic.candidate_gate_counts = counts
        .into_iter()
        .map(|((gate_id, symbol, strategy), count)| EntryGateCandidateCount {
            gate_id,
            symbol,
            strategy,
            count,
        })
        .collect();
}

async fn run_cycle_inner(
    state: &AutoTraderState,
    mut dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let raw_signal_count = dashboard.best.len();
    dashboard.best.retain(|signal| strategy::is_autotrader_symbol_allowed(&signal.symbol));
    let allowed_signal_count = dashboard.best.len();
    let strategy_candidate_count = dashboard
        .best
        .iter()
        .filter(|signal| signal.strategy_candidate || signal.strategy.is_some())
        .count();
    let mut missed_bands = (*dashboard.missed_opportunity_bands).clone();
    missed_bands.retain(|band| band.symbol.as_deref().is_none_or(strategy::is_autotrader_symbol_allowed));
    dashboard.missed_opportunity_bands = Arc::new(missed_bands);
    let cycle_started_at = Instant::now();
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note.clear();
        settings.risk_limited_sizing = false;
    }
    *state.latest_missed_opportunity_bands.lock().await =
        Arc::clone(&dashboard.missed_opportunity_bands);
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "System clock is before Unix epoch".into(),
            )
        })?
        .as_millis() as u64;
    let now = timestamp_ms / 1_000;
    let cycle_id = format!(
        "autotrader-{timestamp_ms}-{}",
        NEXT_ENTRY_CYCLE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut entry_diagnostic = LatestEntryDiagnostic {
        cycle_id,
        timestamp_ms,
        worker_event_ids: dashboard
            .best
            .iter()
            .filter_map(|signal| {
                signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("live_worker_event_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .collect(),
        candidate_decision_ids: Vec::new(),
        gate_counts: Vec::new(),
        candidate_gate_counts: Vec::new(),
        candidate_details: Vec::new(),
        outcome: if raw_signal_count == 0 {
            EntryCycleOutcome::NoSignals
        } else {
            EntryCycleOutcome::NoEligibleCandidates
        },
        raw_signal_count,
        allowed_signal_count,
        strategy_candidate_count,
        fresh_directional_candidate_count: 0,
        stale_candidate_count: 0,
        non_directional_candidate_count: 0,
        position_blocked_candidate_count: 0,
        strategy_rejected_candidate_count: 0,
        evidence_present_candidate_count: 0,
        eligible_candidate_count: 0,
        rejected_candidate_count: 0,
        selected_candidate_count: 0,
        order_attempt_count: 0,
        submitted_order_count: 0,
        fill_confirmed: false,
        primary_gate_id: None,
        primary_blocker: None,
        detail: String::new(),
    };
    for signal in &mut dashboard.best {
        let decision = signal
            .strategy_decision
            .get_or_insert_with(|| serde_json::json!({}));
        if let Some(decision) = decision.as_object_mut() {
            decision.insert(
                "entry_cycle_id".into(),
                serde_json::json!(entry_diagnostic.cycle_id.clone()),
            );
        }
    }
    entry_diagnostic.candidate_decision_ids = dashboard
        .best
        .iter()
        .map(signal_decision_id)
        .collect();
    if let Some(result) = enforce_daily_realized_loss_limit(state).await? {
        entry_diagnostic.outcome = EntryCycleOutcome::GlobalRiskBlock;
        entry_diagnostic.primary_gate_id = Some("daily_realized_loss_limit".to_owned());
        entry_diagnostic.primary_blocker = result.reason.clone();
        entry_diagnostic.detail = result.reason.clone().unwrap_or_else(|| result.status.into());
        return Ok(with_entry_diagnostic(result, entry_diagnostic));
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
    let (risk_appetite, counterfactual_stop_loss_percent, counterfactual_take_profit_percent) = {
        let settings = state.settings.lock().await;
        (
            settings.risk_appetite,
            settings.stop_loss_percent,
            settings.take_profit_percent,
        )
    };
    let daily_signal_strategies = dashboard
        .best
        .iter()
        .filter(|signal| daily_signal_candidate(signal))
        .filter_map(|signal| signal.strategy.as_deref())
        .collect::<BTreeSet<_>>();
    for strategy in strategy_settings
        .strategies
        .iter()
        .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    {
        if !daily_signal_strategies.contains(strategy.as_str()) {
            let strategy_name = strategy.as_str();
            let timestamp_ms = now.saturating_mul(1_000);
            update_latest_strategy_blocker(
                state,
                strategy_name,
                timestamp_ms,
                Some(StrategyBlocker {
                    strategy: strategy_name.to_owned(),
                    symbol: "MARKET".to_owned(),
                    reason: "No current daily strategy candidate received from Insights".to_owned(),
                    timestamp_ms,
                }),
            );
        }
    }
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
            normalize_rsi_entry_parameter_probe(signal);
            normalize_donchian_breakout_parameter_probe(signal);
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
    if !live_strategies.is_empty() && live_source_signals.is_empty() {
        let symbol = all_live_source_signals
            .first()
            .map(|signal| signal.symbol.as_str())
            .unwrap_or("MARKET");
        let reason = if stale_live_source_signals.is_empty() {
            "No live insight signal is available"
        } else {
            "No fresh live insight signal is available"
        };
        for strategy in &live_strategies {
            let strategy_name = strategy.as_str();
            let timestamp_ms = now.saturating_mul(1000);
            update_latest_strategy_blocker(
                state,
                strategy_name,
                timestamp_ms,
                Some(StrategyBlocker {
                    strategy: strategy_name.to_owned(),
                    symbol: symbol.to_owned(),
                    reason: reason.to_owned(),
                    timestamp_ms,
                }),
            );
        }
    }
    let mut strategy_filter_details = Vec::new();
    let mut strategy_rejected_count = 0_usize;
    if has_daily_strategy_signals {
        let daily_decisions = evaluate_daily_strategy_signals(
            &dashboard.best,
            &strategy_settings.strategies,
        )
        .await;
        let mut retained = Vec::with_capacity(dashboard.best.len());
        for (index, signal) in std::mem::take(&mut dashboard.best).into_iter().enumerate() {
            if !signal.strategy_candidate {
                continue;
            }
            let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
                continue;
            };
            if !strategy_settings.strategies.contains(&strategy) {
                continue;
            }
            let is_scout_probe = signal
                .strategy_decision
                .as_ref()
                .and_then(|decision| decision.get("scout_parameter_probe"))
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            if is_scout_probe {
                continue;
            }
            match daily_decisions.get(index).cloned().unwrap_or_else(|| {
                Err("Daily strategy evaluation response was missing".to_owned())
            }) {
                Ok(StrategyDecision::Enter) => retained.push(signal),
                Ok(_) => {
                    strategy_rejected_count = strategy_rejected_count.saturating_add(1);
                    let timestamp_ms = now.saturating_mul(1000);
                    let reason = strategy_entry_rejection_reason(
                        &signal,
                        strategy,
                        &signal_strategy_parameters(&signal),
                    );
                    let blocker = StrategyBlocker {
                        strategy: strategy.as_str().to_owned(),
                        symbol: signal.symbol.clone(),
                        reason: reason.clone(),
                        timestamp_ms,
                    };
                    update_latest_strategy_blocker(
                        state,
                        strategy.as_str(),
                        timestamp_ms,
                        Some(blocker),
                    );
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · {}",
                            signal.symbol,
                            signal.direction,
                            strategy.as_str(),
                            reason
                        ));
                    }
                    queue_strategy_rejection_audit(
                        state,
                        &signal,
                        &strategy.as_str(),
                        &reason,
                        timestamp_ms,
                        counterfactual_stop_loss_percent,
                        counterfactual_take_profit_percent,
                        "daily_evaluator",
                    );
                }
                Err(error) => {
                    strategy_rejected_count = strategy_rejected_count.saturating_add(1);
                    let timestamp_ms = now.saturating_mul(1000);
                    let reason = format!("Strategy evaluation error: {error}");
                    let blocker = StrategyBlocker {
                        strategy: strategy.as_str().to_owned(),
                        symbol: signal.symbol.clone(),
                        reason: reason.clone(),
                        timestamp_ms,
                    };
                    update_latest_strategy_blocker(
                        state,
                        strategy.as_str(),
                        timestamp_ms,
                        Some(blocker),
                    );
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · Strategy evaluation error: {}",
                            signal.symbol,
                            signal.direction,
                            strategy.as_str(),
                            error
                        ));
                    }
                    queue_strategy_rejection_audit(
                        state,
                        &signal,
                        &strategy.as_str(),
                        &reason,
                        timestamp_ms,
                        counterfactual_stop_loss_percent,
                        counterfactual_take_profit_percent,
                        "daily_evaluator_error",
                    );
                }
            }
        }
        dashboard.best = retained;
    }
    let mut live_strategy_candidates = Vec::new();
    if has_live_signal_strategies {
        for source_signal in &live_source_signals {
            if !live_worker_pre_evaluated(source_signal) {
                continue;
            }
            let Some(strategy) = source_signal.strategy.as_deref().and_then(Strategy::from_name) else {
                continue;
            };
            if live_strategies.contains(&strategy) {
                live_strategy_candidates.push(source_signal.clone());
            }
        }
        attach_strategy_walk_forward_evidence(state, &mut live_strategy_candidates, now).await;
    }
    let mut live_strategy_signals = Vec::new();
    for signal in &live_strategy_candidates {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        if signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("live_worker_decision"))
            .and_then(serde_json::Value::as_str) == Some("enter")
        {
            update_latest_strategy_blocker(state, strategy.as_str(), now.saturating_mul(1000), None);
            live_strategy_signals.push(signal.clone());
        } else {
            strategy_rejected_count = strategy_rejected_count.saturating_add(1);
            let timestamp_ms = now.saturating_mul(1000);
            let parameters = signal_strategy_parameters(signal);
            let reason = strategy_entry_rejection_reason(signal, strategy, &parameters);
            update_latest_strategy_blocker(
                state,
                strategy.as_str(),
                timestamp_ms,
                Some(StrategyBlocker {
                    strategy: strategy.as_str().to_owned(),
                    symbol: signal.symbol.clone(),
                    reason: reason.clone(),
                    timestamp_ms,
                }),
            );
            if strategy_filter_details.len() < 20 {
                strategy_filter_details.push(format!(
                    "{} {} [{}] · {}",
                    signal.symbol, signal.direction, strategy.as_str(), reason
                ));
            }
            queue_strategy_rejection_audit(
                state,
                signal,
                strategy.as_str(),
                &reason,
                timestamp_ms,
                counterfactual_stop_loss_percent,
                counterfactual_take_profit_percent,
                "live_worker",
            );
        }
    }
    let active_position_keys = {
        let settings = state.settings.lock().await;
        settings.strategy_positions.iter().flat_map(|(symbol, lots)| {
            lots.iter().filter(|(_, position)| position.quantity.abs() > 1e-12 && !position.scout_entry)
                .map(|(strategy, _)| (symbol.clone(), strategy.clone()))
        }).collect::<BTreeSet<_>>()
    };
    {
        let mut latest = state.latest_live_strategy_signals.lock().await;
        let refreshed = live_source_signals
            .iter()
            .flat_map(|signal| {
                live_strategies
                    .iter()
                    .map(move |strategy| (signal.symbol.clone(), strategy.as_str().to_owned()))
            })
            .collect::<BTreeSet<_>>();
        for key in refreshed {
            latest.remove(&key);
        }
        for signal in &live_strategy_signals {
            let Some(strategy) = signal.strategy.as_deref() else {
                continue;
            };
            let key = (signal.symbol.clone(), strategy.to_owned());
            if !active_position_keys.contains(&key) {
                continue;
            }
            latest
                .entry(key)
                .or_default()
                .push(signal.clone());
        }
        bound_latest_strategy_signals(&mut latest, &active_position_keys, current_time_ms());
    }

    if has_daily_strategy_signals {
        dashboard.best.extend(live_strategy_signals);
        dashboard.best.append(&mut stale_live_source_signals);
    } else if has_live_signal_strategies {
        // Once live strategies are available, do not fall back to the raw
        // generic directional signal when every strategy evaluator rejects
        // the cycle. A generic signal has no strategy/profile ownership and
        // creates an unattributed position that cannot enter the learning
        // feedback loop.
        dashboard.best = live_strategy_signals;
        dashboard.best.append(&mut stale_live_source_signals);
    } else if strategies_enabled {
        entry_diagnostic.outcome = EntryCycleOutcome::WaitingForStrategySignals;
        entry_diagnostic.primary_gate_id = Some("insights_strategy_feed".to_owned());
        entry_diagnostic.primary_blocker = Some("Waiting for Insights daily strategy signals".into());
        entry_diagnostic.detail = "No daily strategy signal arrived in the latest cycle".into();
        return Ok(with_entry_diagnostic(CycleResult {
            status: "skipped",
            reason: Some("Waiting for Insights daily strategy signals".into()),
            symbol: None,
            side: None,
            quantity: None,
            order_type: None,
            order_response: None,
            filled_quantity: None,
            entry_diagnostic: None,
        }, entry_diagnostic));
    }
    let selection_started = Instant::now();
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
    let threshold_refresh_started = Instant::now();
    let learning_thresholds = request_learning_context_thresholds(
        state,
        &learning_signals,
    )
    .await;
    entry_diagnostic.candidate_details = learning_signals
        .iter()
        .map(|signal| {
            let audit = candidate_entry_audit_value(
                signal,
                &dashboard,
                &learning_thresholds,
                risk_appetite,
                "audit",
                timestamp_ms,
            );
            EntryCandidateDiagnostic {
                decision_id: signal_decision_id(signal),
                worker_event_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("live_worker_event_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                profile_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| {
                        decision
                            .get("learning_profile_id")
                            .or_else(|| decision.get("scout_parameter_profile_id"))
                    })
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                symbol: signal.symbol.clone(),
                strategy: signal.strategy.clone(),
                stage: "prepared_candidate".into(),
                observed: audit["observed"].clone(),
                required: audit["thresholds"].clone(),
                checks: audit["checks"].clone(),
                evidence_contract: audit["evidence_contract"].clone(),
                measurements: candidate_threshold_measurements(&audit),
                reasons: audit["rejection_reasons"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            }
        })
        .collect();
    recompute_entry_candidate_gate_counts(&mut entry_diagnostic);
    let context_threshold_refresh_ms = threshold_refresh_started.elapsed().as_secs_f64() * 1_000.0;
    let (open_positions, strategy_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        let open_positions = settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12 && !position.scout_entry)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<BTreeMap<_, _>>();
        let strategy_positions = settings
            .strategy_positions
            .iter()
            .filter_map(|(symbol, lots)| {
                let real_lots = lots
                    .iter()
                    .filter(|(_, position)| {
                        position.quantity.abs() > 1e-12 && !position.scout_entry
                    })
                    .map(|(strategy, position)| (strategy.clone(), position.clone()))
                    .collect::<BTreeMap<_, _>>();
                (!real_lots.is_empty()).then(|| (symbol.clone(), real_lots))
            })
            .collect();
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
        fresh_directional_count,
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
    entry_diagnostic.fresh_directional_candidate_count = fresh_directional_count;
    entry_diagnostic.stale_candidate_count = stale_signal_count;
    entry_diagnostic.non_directional_candidate_count = non_directional_count;
    entry_diagnostic.position_blocked_candidate_count = position_blocked_count;
    entry_diagnostic.strategy_rejected_candidate_count = strategy_rejected_count;
    entry_diagnostic.evidence_present_candidate_count = entry_diagnostic
        .candidate_details
        .iter()
        .filter(|candidate| candidate.checks["strategy_walk_forward_evidence_present"] == true)
        .count();
    entry_diagnostic.eligible_candidate_count = fresh_directional.len();
    entry_diagnostic.rejected_candidate_count = strategy_rejected_count;
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
            entry_diagnostic.outcome = if evaluated == 0 {
                EntryCycleOutcome::NoEligibleCandidates
            } else {
                EntryCycleOutcome::CandidatesRejected
            };
            entry_diagnostic.rejected_candidate_count = entry_diagnostic
                .rejected_candidate_count
                .saturating_add(evaluated);
            entry_diagnostic.primary_gate_id = Some("daily_strategy_entry_gates".into());
            entry_diagnostic.primary_blocker = Some(reason.clone());
            entry_diagnostic.detail = reason.clone();
            daily_batch_result = Some(skipped(&reason));
        } else {
            let candidate_dashboard = prefetch_daily_candidate_market_books(
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
                update_entry_candidate_diagnostic(
                    &mut entry_diagnostic,
                    signal,
                    "selected_for_rebalance",
                    None,
                );
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
                let daily_parameter_probe = is_daily_parameter_exploration_probe(signal);
                let daily_cold_start = is_daily_cold_start_exploration_candidate(signal);
                let (daily_rule_evidence_available, daily_rule_evidence_eligible, daily_walk_forward_present) =
                    daily_cold_start_exploration_checks(signal);
                let exploratory = daily_parameter_probe || daily_cold_start;
                sized_signal.entry_gate = Some(serde_json::json!({
                    "selection_mode": if daily_parameter_probe {
                        "daily_strategy_parameter_exploration"
                    } else if daily_cold_start {
                        "daily_strategy_cold_start_exploration"
                    } else {
                        "daily_strategy"
                    },
                    "exploratory": exploratory,
                    "checks": {
                        "daily_rule_evidence_available": daily_rule_evidence_available,
                        "daily_rule_evidence_eligible": daily_rule_evidence_eligible,
                        "strategy_walk_forward_evidence_present": daily_walk_forward_present,
                        "daily_parameter_probe": daily_parameter_probe,
                        "cold_start_exploration_pass": daily_cold_start,
                    },
                    "strategy": strategy_name,
                    "strategy_decision": sized_signal.strategy_decision.clone(),
                }));
                let result = match submit_cycle_signal(
                    state,
                    &candidate_dashboard,
                    &sized_signal,
                    exploratory,
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    signal_selection_ms,
                    context_threshold_refresh_ms,
                    cycle_started_at,
                    now,
                )
                .await
                {
                    Ok(result) => result,
                    Err((status, error)) => {
                        let reason = format!("Execution request failed ({status}): {error}");
                        update_entry_candidate_diagnostic(
                            &mut entry_diagnostic,
                            signal,
                            "execution_failed",
                            Some(&reason),
                        );
                        entry_diagnostic.order_attempt_count = entry_diagnostic
                            .order_attempt_count
                            .saturating_add(1);
                        entry_diagnostic.rejected_candidate_count = entry_diagnostic
                            .rejected_candidate_count
                            .saturating_add(1);
                        details.push(format!("{strategy_name}: request error {status}: {error}"));
                        queue_strategy_rejection_audit(
                            state,
                            &sized_signal,
                            strategy_name,
                            &reason,
                            now.saturating_mul(1_000),
                            counterfactual_stop_loss_percent,
                            counterfactual_take_profit_percent,
                            "daily_execution",
                        );
                        continue;
                    }
                };
                entry_diagnostic.order_attempt_count = entry_diagnostic
                    .order_attempt_count
                    .saturating_add(1);
                if result.status == "order_submitting" {
                    update_entry_candidate_diagnostic(
                        &mut entry_diagnostic,
                        signal,
                        "order_queued",
                        None,
                    );
                    submitted += 1;
                    entry_diagnostic.submitted_order_count = entry_diagnostic
                        .submitted_order_count
                        .saturating_add(1);
                    last_order = Some(result);
                } else {
                    update_entry_candidate_diagnostic(
                        &mut entry_diagnostic,
                        signal,
                        "execution_blocked",
                        result.reason.as_deref(),
                    );
                    entry_diagnostic.rejected_candidate_count = entry_diagnostic
                        .rejected_candidate_count
                        .saturating_add(1);
                    let reason = result.reason.as_deref().unwrap_or(result.status);
                    details.push(format!("{strategy_name}: {reason}"));
                    queue_strategy_rejection_audit(
                        state,
                        &sized_signal,
                        strategy_name,
                        reason,
                        now.saturating_mul(1_000),
                        counterfactual_stop_loss_percent,
                        counterfactual_take_profit_percent,
                        "daily_execution",
                    );
                }
            }
            daily_batch_result = Some(daily_strategy_batch_result(
                last_order, evaluated, submitted, &details,
            ));
            if submitted > 0 {
                entry_diagnostic.outcome = EntryCycleOutcome::OrderQueued;
                entry_diagnostic.selected_candidate_count = submitted;
            } else if !details.is_empty() {
                entry_diagnostic.outcome = EntryCycleOutcome::ExecutionBlocked;
                entry_diagnostic.primary_gate_id = Some("daily_strategy_execution".into());
                entry_diagnostic.primary_blocker = details.first().cloned();
                entry_diagnostic.detail = details.join("; ");
            } else if rebalance_candidates.is_empty() {
                entry_diagnostic.outcome = EntryCycleOutcome::ExecutionBlocked;
                entry_diagnostic.primary_gate_id =
                    Some("portfolio_rebalance_threshold".into());
                entry_diagnostic.primary_blocker = Some(
                    "Portfolio candidate did not cross its rebalance band".into(),
                );
                entry_diagnostic.detail = entry_diagnostic.primary_blocker.clone().unwrap();
            }
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
    let (mut signal, mut exploratory) = match selection_result {
        Ok((signal, exploratory)) => (signal.clone(), exploratory),
        Err(result) => {
            let audit_timestamp = now.saturating_mul(1000);
            let cycle_reason = result
                .reason
                .clone()
                .unwrap_or_else(|| result.status.to_owned());
            for candidate in selection_signals
                .iter()
                .filter(|candidate| candidate.strategy_candidate || candidate.strategy.is_some())
            {
                let mut audited_candidate = candidate.clone();
                audited_candidate.entry_gate = Some(candidate_entry_audit_value(
                    candidate,
                    &dashboard,
                    &learning_thresholds,
                    risk_appetite,
                    "rejected",
                    audit_timestamp,
                ));
                let rejection_details = audited_candidate
                    .entry_gate
                    .as_ref()
                    .and_then(|audit| audit.get("rejection_reasons"))
                    .and_then(serde_json::Value::as_array)
                    .map(|reasons| {
                        reasons
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|details| !details.is_empty());
                let reason = rejection_details.map_or_else(
                    || cycle_reason.clone(),
                    |details| format!("{cycle_reason}; candidate checks: {details}"),
                );
                update_entry_candidate_diagnostic(
                    &mut entry_diagnostic,
                    &audited_candidate,
                    "selection_rejected",
                    Some(&reason),
                );
                let strategy = audited_candidate
                    .strategy
                    .as_deref()
                    .unwrap_or("generic_signal");
                let payload = strategy_rejection_audit_payload(
                    &audited_candidate,
                    strategy,
                    &reason,
                    audit_timestamp,
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    "cycle_selection",
                );
                queue_auto_trader_audit(state, payload);
            }
            // Learning receives at most one actual Strategy candidate per
            // cycle. Generic feed signals and Scout probes are not attributed
            // missed opportunities.
            if let Some(candidate) = selection_signals.iter()
                .filter(|signal| signal.strategy.as_deref().and_then(Strategy::from_name)
                    .is_some_and(|strategy| strategy.is_live_insight()))
                .filter(|signal| {
                    signal.strategy_candidate
                        && strategy::is_autotrader_symbol_allowed(&signal.symbol)
                        && matches!(signal.direction.as_str(), "bullish" | "bearish")
                        && signal.strength.is_finite()
                        && signal.score.is_finite()
                })
                .max_by(|left, right| left.strength.total_cmp(&right.strength))
            {
                let strategy = candidate.strategy.as_deref().unwrap_or_default();
                let parameters = signal_strategy_parameters(candidate);
                let profile_id = strategy::profile_id(&candidate.symbol, strategy, &parameters);
                let signal_timestamp_ms = if candidate.timestamp < 1_000_000_000_000 {
                    candidate.timestamp.saturating_mul(1_000)
                } else { candidate.timestamp };
                let event_id = format!("missed:{}:{}:{}", candidate.symbol, strategy, signal_timestamp_ms);
                let event = FeedbackEvent::MissedOpportunityCandidate {
                    event_id: event_id.clone(), observed_at_ms: signal_timestamp_ms,
                    symbol: candidate.symbol.clone(), strategy: strategy.to_owned(), profile_id,
                    parameters, direction: candidate.direction.clone(), strength: candidate.strength,
                    score: candidate.score, learning_context: market_context(candidate),
                };
                let mut settings = state.settings.lock().await;
                match enqueue_scheduled_missed_opportunity_feedback(
                    &mut settings,
                    event,
                    current_time_ms(),
                ) {
                    MissedFeedbackEnqueueResult::Queued
                    | MissedFeedbackEnqueueResult::CoalescedExistingCase => {
                        drop(settings);
                        let _ = request_settings_persist(state);
                    }
                    MissedFeedbackEnqueueResult::EvictedOldestMissed => {
                        drop(settings);
                        record_feedback_terminal_drop(
                            state,
                            "missed_opportunity",
                            &format!("Evicted oldest missed-opportunity candidate for {event_id}"),
                            FeedbackTerminalDropKind::EvictedOldest,
                        )
                        .await;
                    }
                    MissedFeedbackEnqueueResult::DroppedOutcomeCapacity => {
                        drop(settings);
                        record_feedback_terminal_drop(
                            state,
                            "missed_opportunity",
                            &format!("Dropped missed-opportunity candidate {event_id}: feedback outbox is full of trade/scout outcomes"),
                            FeedbackTerminalDropKind::OutcomeCapacity,
                        )
                        .await;
                    }
                    MissedFeedbackEnqueueResult::Duplicate
                    | MissedFeedbackEnqueueResult::Throttled => {}
                }
            }
            record_cycle_candidate_blockers(
                state,
                selection_signals,
                &live_strategies,
                None,
                &dashboard,
                &learning_thresholds,
                risk_appetite,
                now,
                &cycle_reason,
            );
            if entry_diagnostic.primary_gate_id.is_none()
                && entry_diagnostic.submitted_order_count == 0
            {
                let candidate_count = selection_signals
                    .iter()
                    .filter(|candidate| {
                        candidate.strategy_candidate || candidate.strategy.is_some()
                    })
                    .count();
                let (outcome, gate_id) = candidate_cycle_blocker(
                    entry_diagnostic.raw_signal_count,
                    candidate_count,
                    selection_fresh_directional.len(),
                    strategy_rejected_count,
                );
                entry_diagnostic.outcome = outcome;
                entry_diagnostic.primary_gate_id = Some(gate_id.into());
                entry_diagnostic.primary_blocker = strategy_filter_details
                    .first()
                    .cloned()
                    .or_else(|| Some(cycle_reason.clone()));
                entry_diagnostic.detail = if strategy_filter_details.is_empty() {
                    cycle_reason
                } else {
                    format!(
                        "{} · {}",
                        strategy_filter_details.join("; "),
                        cycle_reason
                    )
                };
                entry_diagnostic.rejected_candidate_count = entry_diagnostic
                    .rejected_candidate_count
                    .saturating_add(
                        selection_signals
                            .iter()
                            .filter(|candidate| {
                                candidate.strategy_candidate || candidate.strategy.is_some()
                            })
                            .count(),
                    );
            }
            let result = daily_batch_result.unwrap_or(result);
            return Ok(with_entry_diagnostic(result, entry_diagnostic));
        }
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    update_entry_candidate_diagnostic(&mut entry_diagnostic, &signal, "selected", None);
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        &signal,
        &dashboard,
        &learning_thresholds,
        risk_appetite,
        if exploratory { "exploratory" } else { "strict" },
        now.saturating_mul(1000),
    ));
    let mut submission_error = false;
    let mut signal_result = match submit_cycle_signal(
        state,
        &dashboard,
        &audited_signal,
        exploratory,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        context_threshold_refresh_ms,
        cycle_started_at,
        now,
    )
    .await
    {
        Ok(result) => result,
        Err((status, error)) => {
            submission_error = true;
            let reason = format!("Execution request failed ({status}): {error}");
            queue_strategy_rejection_audit(
                state,
                &audited_signal,
                audited_signal
                    .strategy
                    .as_deref()
                    .unwrap_or("unknown-strategy"),
                &reason,
                now.saturating_mul(1_000),
                counterfactual_stop_loss_percent,
                counterfactual_take_profit_percent,
                "final_entry",
            );
            skipped_for_signal(&audited_signal, &reason)
        }
    };
    let mut attempted_ids = BTreeSet::from([signal_decision_id(&signal)]);
    // A deterministic rejection before reservation should not consume the
    // opportunity when another eligible candidate is already available.
    while !submission_error && signal_result.status == "skipped" && attempted_ids.len() < 3 {
        let remaining_fresh = selection_fresh_directional
            .iter()
            .copied()
            .filter(|candidate| !attempted_ids.contains(&signal_decision_id(candidate)))
            .collect::<Vec<_>>();
        let Ok((next, next_exploratory)) = select_cycle_signal(SignalSelectionInput {
            dashboard: &dashboard,
            signals: selection_signals,
            fresh_directional: &remaining_fresh,
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
        }) else {
            break;
        };
        let next_signal = next.clone();
        update_entry_candidate_diagnostic(
            &mut entry_diagnostic,
            &signal,
            "execution_blocked",
            signal_result.reason.as_deref(),
        );
        entry_diagnostic.selected_candidate_count += 1;
        entry_diagnostic.order_attempt_count += 1;
        entry_diagnostic.rejected_candidate_count += 1;
        queue_auto_trader_audit(
            state,
            selected_signal_outcome_audit_payload(
                &audited_signal,
                &signal_result,
                exploratory,
                now.saturating_mul(1_000),
                counterfactual_stop_loss_percent,
                counterfactual_take_profit_percent,
            ),
        );
        if let Some(strategy) = audited_signal.strategy.as_deref() {
            update_latest_strategy_blocker(
                state,
                strategy,
                now.saturating_mul(1_000),
                Some(StrategyBlocker {
                    strategy: strategy.to_owned(),
                    symbol: audited_signal.symbol.clone(),
                    reason: signal_result.reason.clone().unwrap_or_else(|| signal_result.status.to_owned()),
                    timestamp_ms: now.saturating_mul(1_000),
                }),
            );
        }
        signal = next_signal;
        exploratory = next_exploratory;
        attempted_ids.insert(signal_decision_id(&signal));
        update_entry_candidate_diagnostic(&mut entry_diagnostic, &signal, "selected", None);
        audited_signal = signal.clone();
        audited_signal.entry_gate = Some(candidate_entry_audit_value(
            &signal,
            &dashboard,
            &learning_thresholds,
            risk_appetite,
            if exploratory { "exploratory" } else { "strict" },
            now.saturating_mul(1_000),
        ));
        signal_result = match submit_cycle_signal(
            state,
            &dashboard,
            &audited_signal,
            exploratory,
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            signal_selection_ms,
            context_threshold_refresh_ms,
            cycle_started_at,
            now,
        )
        .await
        {
            Ok(result) => result,
            Err((status, error)) => {
                submission_error = true;
                skipped_for_signal(
                    &audited_signal,
                    &format!("Execution request failed ({status}): {error}"),
                )
            }
        };
    }
    let remaining_for_audit = selection_signals
        .iter()
        .filter(|candidate| {
            let id = signal_decision_id(candidate);
            !attempted_ids.contains(&id) || id == signal_decision_id(&signal)
        })
        .cloned()
        .collect::<Vec<_>>();
    record_cycle_candidate_blockers(
        state,
        &remaining_for_audit,
        &live_strategies,
        Some(&signal),
        &dashboard,
        &learning_thresholds,
        risk_appetite,
        now,
        "another candidate ranked higher",
    );
    for candidate in remaining_for_audit.iter().filter(|candidate| {
        candidate
            .strategy
            .as_deref()
            .and_then(Strategy::from_name)
            .is_some_and(|kind| kind.is_live_insight())
            && signal_decision_id(candidate) != signal_decision_id(&signal)
    }) {
        let strategy = candidate.strategy.as_deref().unwrap_or("unknown-strategy");
        let reason = "Not selected this cycle; another candidate ranked higher";
        update_entry_candidate_diagnostic(
            &mut entry_diagnostic,
            candidate,
            "not_selected",
            Some(reason),
        );
        queue_strategy_rejection_audit(
            state,
            candidate,
            strategy,
            reason,
            now.saturating_mul(1_000),
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            "auto_trader_selection",
        );
    }
    update_entry_candidate_diagnostic(
        &mut entry_diagnostic,
        &signal,
        if signal_result.status == "order_submitting" {
            "order_queued"
        } else {
            "execution_blocked"
        },
        signal_result.reason.as_deref(),
    );
    entry_diagnostic.selected_candidate_count = entry_diagnostic
        .selected_candidate_count
        .saturating_add(1);
    entry_diagnostic.order_attempt_count = entry_diagnostic.order_attempt_count.saturating_add(1);
    if signal_result.status == "order_submitting" {
        entry_diagnostic.submitted_order_count = entry_diagnostic
            .submitted_order_count
            .saturating_add(1);
        entry_diagnostic.outcome = EntryCycleOutcome::OrderQueued;
        entry_diagnostic.primary_gate_id = None;
        entry_diagnostic.primary_blocker = None;
        entry_diagnostic.detail = "Candidate passed selection and was queued for execution".into();
    } else if entry_diagnostic.submitted_order_count == 0 {
        entry_diagnostic.outcome = EntryCycleOutcome::ExecutionBlocked;
        entry_diagnostic.primary_gate_id = Some("execution_or_risk_gate".into());
        entry_diagnostic.primary_blocker = signal_result.reason.clone();
        entry_diagnostic.detail = signal_result
            .reason
            .clone()
            .unwrap_or_else(|| signal_result.status.to_owned());
        entry_diagnostic.rejected_candidate_count = entry_diagnostic
            .rejected_candidate_count
            .saturating_add(1);
    }
    if signal_result.status != "order_submitting" {
        queue_auto_trader_audit(
            state,
            selected_signal_outcome_audit_payload(
                &audited_signal,
                &signal_result,
                exploratory,
                now.saturating_mul(1_000),
                counterfactual_stop_loss_percent,
                counterfactual_take_profit_percent,
            ),
        );
        if let Some(strategy) = audited_signal.strategy.as_deref() {
            let timestamp_ms = now.saturating_mul(1_000);
            let reason = signal_result
                .reason
                .clone()
                .unwrap_or_else(|| format!("No order submitted (status {})", signal_result.status));
            update_latest_strategy_blocker(
                state,
                strategy,
                timestamp_ms,
                Some(StrategyBlocker {
                    strategy: strategy.to_owned(),
                    symbol: audited_signal.symbol.clone(),
                    reason,
                    timestamp_ms,
                }),
            );
        }
    }
    let result = if signal_result.status == "order_submitting" || daily_batch_result.is_none() {
        signal_result
    } else {
        daily_batch_result.unwrap()
    };
    Ok(with_entry_diagnostic(result, entry_diagnostic))
}

#[cfg(test)]
mod parallel_daily_cycle_tests {
    use super::*;

    #[test]
    fn entry_diagnostic_distinguishes_missing_candidates_from_gate_rejections() {
        assert!(matches!(
            candidate_cycle_blocker(0, 0, 0, 0),
            (EntryCycleOutcome::NoSignals, "signal_source")
        ));
        assert!(matches!(
            candidate_cycle_blocker(8, 0, 0, 0),
            (EntryCycleOutcome::NoEligibleCandidates, "candidate_filter")
        ));
        assert!(matches!(
            candidate_cycle_blocker(8, 2, 0, 0),
            (
                EntryCycleOutcome::NoFreshCandidates,
                "signal_freshness_or_direction"
            )
        ));
        assert!(matches!(
            candidate_cycle_blocker(8, 2, 2, 0),
            (
                EntryCycleOutcome::CandidatesRejected,
                "candidate_selection_gates"
            )
        ));
        assert!(matches!(
            candidate_cycle_blocker(8, 2, 0, 2),
            (
                EntryCycleOutcome::CandidatesRejected,
                "strategy_entry_gate"
            )
        ));
    }

    #[test]
    fn gate_measurement_parser_preserves_observed_required_values_and_units() {
        let measurements = parse_reported_gate_measurements(
            "execution_blocked",
            "estimated impact 20 bps > 10 bps; lower 95% bound -0.120% requires > 0.000%",
        );
        assert_eq!(measurements.len(), 2);
        assert_eq!(measurements[0].observed, Some(20.0));
        assert_eq!(measurements[0].required, Some(10.0));
        assert_eq!(measurements[0].unit.as_deref(), Some("bps"));
        assert_eq!(measurements[1].observed, Some(-0.12));
        assert_eq!(measurements[1].required, Some(0.0));
        assert_eq!(measurements[1].unit.as_deref(), Some("%"));
    }

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
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn exploratory_selection_records_the_downstream_skip_with_gate_context() {
        let mut candidate = signal("live-order-book");
        candidate.entry_gate = Some(serde_json::json!({
            "selection_mode": "exploratory",
            "checks": { "strength_pass": false }
        }));
        let result = skipped_for_signal(&candidate, "Waiting for a fresh subscribed local order book");
        let audit = selected_signal_outcome_audit_payload(&candidate, &result, true, 11_000, 0.5, 0.8);

        assert_eq!(audit["stage"], "strategy_rejection");
        assert_eq!(audit["status"], "skipped");
        assert_eq!(audit["reason"], "Waiting for a fresh subscribed local order book");
        assert_eq!(audit["exploratory"], true);
        assert_eq!(audit["entry_gate"]["checks"]["strength_pass"], false);
        assert_eq!(
            audit["decision_id"],
            "10|ETH-USDT|bullish|strategy:live-order-book|phase:final_entry|audit:11000"
        );
    }

    #[test]
    fn strategy_rejection_audit_preserves_the_strategy_and_gate_reason() {
        let first = signal("live-order-book");
        let second = signal("live-trade-flow");
        let first_audit = strategy_rejection_audit_payload(
            &first,
            "live-order-book",
            "Strength 0.12 < 0.30",
            11_000,
            0.5,
            0.8,
            "live_worker",
        );
        let second_audit = strategy_rejection_audit_payload(
            &second,
            "live-trade-flow",
            "Directional flow imbalance was below threshold",
            11_000,
            0.5,
            0.8,
            "live_worker",
        );

        assert_eq!(first_audit["stage"], "strategy_rejection");
        assert_eq!(first_audit["status"], "rejected");
        assert_eq!(
            first_audit["reason"],
            "Strength 0.12 < 0.30"
        );
        assert_eq!(
            first_audit["entry_gate"]["selection_mode"],
            "strategy_rejection"
        );
        assert_ne!(first_audit["decision_id"], second_audit["decision_id"]);
    }

    #[tokio::test]
    async fn rejected_strategy_candidate_reaches_the_persistence_queue() {
        let candidate = signal("live-order-book");
        let queue = AuditQueue::new(8, 64 * 1024);
        let result = enqueue_strategy_rejection_audit(
            &queue,
            &candidate,
            "live-order-book",
            "Directional order-book imbalance 0.12; requires at least 0.16",
            11_000,
            0.5,
            0.8,
            "live_worker",
        );

        assert_eq!(result, AuditEnqueueResult::Queued);
        let persisted = queue.recv().await;
        assert_eq!(persisted["stage"], "strategy_rejection");
        assert_eq!(persisted["status"], "rejected");
        assert_eq!(
            persisted["reason"],
            "Directional order-book imbalance 0.12; requires at least 0.16"
        );
        assert_eq!(queue.depth(), 0);
    }

    #[test]
    fn candidate_selection_blocker_reports_downstream_entry_gates() {
        let mut candidate = signal("trend-50-200");
        candidate.score = 0.28;
        candidate.strength = 0.16;
        let dashboard: Dashboard = serde_json::from_value(serde_json::json!({ "best": [] }))
            .unwrap();
        let reason = candidate_selection_blocker_reason(
            &candidate,
            &dashboard,
            &BTreeMap::new(),
            0.5,
            10,
            "No tradeable bias",
        );
        assert!(reason.contains("strength"), "{reason}");
        assert!(reason.contains("score"), "{reason}");
    }

    #[test]
    fn daily_strategy_blocker_reports_the_failed_primary_rule() {
        let mut candidate = signal("trend-50-200");
        candidate.strategy_decision = Some(serde_json::json!({
            "observed_close": 90.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "primary_passed": false,
            "btc_gate_required": false,
            "btc_gate_passed": true
        }));
        let reason = strategy_entry_rejection_reason(
            &candidate,
            Strategy::Trend50200,
            &BTreeMap::new(),
        );
        assert!(reason.contains("primary strategy rule not met"), "{reason}");
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
