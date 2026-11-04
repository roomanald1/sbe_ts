fn scout_parameter_probe_suffix(signal: &Signal) -> String {
    let Some(decision) = signal.strategy_decision.as_ref() else { return String::new(); };
    let (Some(key), Some(value)) = (
        decision.get("scout_parameter_key").and_then(serde_json::Value::as_str),
        decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64),
    ) else { return String::new(); };
    format!("|probe:{key}:{value:.8}")
}

fn scout_shadow_audit_payload(
    signal: &Signal,
    timestamp: u64,
    notional: f64,
    learning_context: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    evidence: Option<&WalkForward>,
    level_percent: f64,
    exit_scenario: &str,
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
    // Daily signal timestamps are in seconds; shadow audit matching uses the
    // millisecond timestamp shared with outcome evaluations.
    payload["signal_timestamp"] = serde_json::json!(timestamp);
    payload["scout"] = serde_json::json!(true);
    payload["shadow_learning_context"] = serde_json::json!(learning_context);
    payload["shadow_notional_usd"] = serde_json::json!(notional);
    if signal.strategy.as_deref().is_some_and(|strategy| strategy.starts_with("donchian-")) {
        payload["scout_level_percent"] = serde_json::json!(level_percent);
    }
    payload["shadow_exit_scenario"] = serde_json::json!(exit_scenario);
    if let Some(decision) = signal.strategy_decision.as_ref() {
        if let Some(id) = decision.get("scout_comparison_id") { payload["scout_comparison_id"] = id.clone(); }
        if let Some(role) = decision.get("scout_comparison_role") { payload["scout_comparison_role"] = role.clone(); }
    }
    let parameter_probe = signal.strategy_decision.as_ref().and_then(|decision| {
        Some((decision.get("scout_parameter_key")?.as_str()?, decision.get("scout_parameter_value")?.as_f64()?))
    });
    if let Some((key, value)) = parameter_probe {
        payload["scout_parameter_key"] = serde_json::json!(key);
        payload["scout_parameter_value"] = serde_json::json!(value);
        payload["scout_parameter_profile_symbol"] = signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("scout_parameter_profile_symbol"))
            .cloned().unwrap_or_else(|| serde_json::json!(signal.symbol));
    }
    if parameter_probe.is_some() {
        let parameter_values = signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(|parameters| serde_json::from_value::<BTreeMap<String, f64>>(parameters.clone()).ok())
            .unwrap_or_default();
        let entry_eligible = signal.strategy.as_deref().and_then(Strategy::from_name)
            .is_some_and(|strategy| {
                matches!(strategy.evaluate(&StrategySignal::entry(signal, &parameter_values)), Ok(StrategyDecision::Enter))
            });
        payload["scout_parameter_entry_eligible"] = serde_json::json!(entry_eligible);
    }
    let sample_group_id = signal.strategy.as_deref().unwrap_or("unknown-strategy");
    let probe_suffix = parameter_probe.map_or_else(String::new, |(key, value)| format!("|probe:{key}:{value:.8}"));
    let sample_group_id = if sample_group_id.starts_with("live-") {
        format!("live-scout:{sample_group_id}|{}|{}{probe_suffix}", signal.symbol, signal.direction)
    } else {
        format!("daily-scout-level:{level_percent:.2}|{sample_group_id}|{}|{}{probe_suffix}", signal.symbol, signal.direction)
    };
    payload["shadow_sample_group_id"] = serde_json::json!(sample_group_id);
    let is_breakout = signal.strategy.as_deref().is_some_and(|name| name.starts_with("donchian-"));
    let entry_price = if is_breakout {
        signal.strategy_decision.as_ref().and_then(|decision| {
            let breakout = decision.get("primary_level")?.as_f64()?;
            (breakout.is_finite() && breakout > 0.0)
                .then_some(breakout * (1.0 - level_percent / 100.0))
        })
    } else {
        signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("observed_close"))
            .and_then(serde_json::Value::as_f64)
    };
    if let Some(entry_price) = entry_price {
        payload["shadow_entry_price"] = serde_json::json!(entry_price);
    }
    let strategy = signal.strategy.as_deref().unwrap_or("unknown-strategy");
    let probe_suffix = scout_parameter_probe_suffix(signal);
    let level_id = if strategy.starts_with("donchian-") {
        format!(
            "{}|{}|{}|{}|scout-level:{level_percent:.2}|exit:{exit_scenario}{probe_suffix}",
            signal.timestamp, signal.symbol, signal.direction, strategy
        )
    } else {
        format!(
            "{}|{}|{}|{}|scout|exit:{exit_scenario}{probe_suffix}",
            signal.timestamp, signal.symbol, signal.direction, strategy
        )
    };
    payload["decision_id"] = serde_json::json!(level_id);
    payload["trade_id"] = serde_json::json!(level_id);
    payload
}

const DAILY_SCOUT_SHADOW_COOLDOWN_MS: u64 = 300_000;
const LIVE_SCOUT_PARAMETER_COOLDOWN_MS: u64 = 3_600_000;
const SCOUT_TIGHT_EXIT_DISTANCE_MULTIPLIER: f64 = 0.5;
const SCOUT_WIDE_EXIT_DISTANCE_MULTIPLIER: f64 = 1.5;

fn shadow_entry_limit_blocks(scout: bool, active_group_exists: bool, previous_attempt_ms: Option<u64>, sampled_at_ms: u64, active_sample_count: usize, sample_limit: usize) -> bool {
    scout && (active_group_exists
        || previous_attempt_ms.is_some_and(|previous| sampled_at_ms.saturating_sub(previous) < 300_000)
        || active_sample_count >= sample_limit)
}

async fn replay_recent_scout_shadows(state: &AutoTraderState) -> bool {
    let base = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let active_groups = state.settings.lock().await.shadow_sample_active_scenarios.clone();
    if active_groups.is_empty() { return true; }
    let since = current_time_ms().saturating_sub(7 * 86_400_000);
    let response = match state.client
        .get(format!("{base}/signals/auto-trader-audit?since={since}&limit=10000"))
        .timeout(std::time::Duration::from_secs(5)).send().await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(response) => response,
        Err(error) => {
            crate::service_log!("WARN", "Could not replay recent Scout shadows for learning: {error}");
            return false;
        }
    };
    let audits = match response.json::<Vec<serde_json::Value>>().await {
        Ok(audits) => audits,
        Err(error) => {
            crate::service_log!("WARN", "Could not decode recent Scout shadow audits: {error}");
            return false;
        }
    };
    for audit in audits.into_iter().filter(|audit| {
        audit.get("stage").and_then(serde_json::Value::as_str) == Some("decision")
            && audit.get("status").and_then(serde_json::Value::as_str) == Some("shadowed")
            && audit.get("scout").and_then(serde_json::Value::as_bool) == Some(true)
            && audit.get("shadow_sample_group_id").and_then(serde_json::Value::as_str)
                .zip(audit.get("shadow_exit_scenario").and_then(serde_json::Value::as_str))
                .is_some_and(|(group, scenario)| active_groups.get(group).is_some_and(|pending| pending.contains(scenario)))
    }) {
        register_scout_shadow_for_learning(state, &audit);
    }
    true
}

fn register_scout_shadow_for_learning(state: &AutoTraderState, payload: &serde_json::Value) {
    let Some(decision_id) = payload.get("decision_id").and_then(serde_json::Value::as_str) else { return; };
    let Some(symbol) = payload.get("symbol").and_then(serde_json::Value::as_str) else { return; };
    let Some(direction) = payload.get("direction").and_then(serde_json::Value::as_str) else { return; };
    let Some(timestamp) = payload.get("timestamp").and_then(serde_json::Value::as_u64) else { return; };
    let Some(entry_price) = payload.get("shadow_entry_price").and_then(serde_json::Value::as_f64) else { return; };
    let Some(strategy) = payload.get("strategy").and_then(serde_json::Value::as_str) else { return; };
    let exit_scenario = payload.get("shadow_exit_scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("strategy");
    let sample_group_id = payload.get("shadow_sample_group_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let stop_loss_percent = payload.get("stop_loss_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.5);
    let take_profit_percent = payload.get("take_profit_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.8);
    let strategy_parameters = payload.get("strategy_decision").and_then(|decision| decision.get("strategy_parameters"));
    let strategy_parameter = |name: &str, default: f64| strategy_parameters
        .and_then(|parameters| parameters.get(name))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(default);
    let profit_protection_activation_percent = strategy_parameter("profit_protection_activation_percent", DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT);
    let profit_protection_giveback_percent = strategy_parameter("profit_protection_giveback_percent", DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT);
    let profit_protection_minimum_lock_percent = strategy_parameter("profit_protection_minimum_lock_percent", DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT);
    let base = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let client = state.client.clone();
    let request = serde_json::json!({
        "decision_id": decision_id,
        "symbol": symbol,
        "timestamp": timestamp,
        "direction": direction,
        "entry_price": entry_price,
        "strategy": strategy,
        "exit_scenario": exit_scenario,
        "sample_group_id": sample_group_id,
        "strategy_exit_enabled": Strategy::from_name(strategy)
            .is_some_and(|kind| !kind.is_live_insight() && !kind.is_dated_carry()),
        "scout_parameter_key": payload.get("strategy_decision")
            .and_then(|decision| decision.get("scout_parameter_key")),
        "scout_parameter_value": payload.get("strategy_decision")
            .and_then(|decision| decision.get("scout_parameter_value")),
        "scout_parameter_profile_symbol": payload.get("strategy_decision")
            .and_then(|decision| decision.get("scout_parameter_profile_symbol")),
        "scout_comparison_id": payload.get("scout_comparison_id"),
        "scout_comparison_role": payload.get("scout_comparison_role"),
        "scout_minimum_hold_days": strategy_parameter("minimum_hold_days", f64::NAN).is_finite()
            .then(|| strategy_parameter("minimum_hold_days", f64::NAN)),
        "scout_maximum_hold_days": strategy_parameter("maximum_hold_days", f64::NAN).is_finite()
            .then(|| strategy_parameter("maximum_hold_days", f64::NAN)),
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "profit_protection_activation_percent": profit_protection_activation_percent,
        "profit_protection_giveback_percent": profit_protection_giveback_percent,
        "profit_protection_minimum_lock_percent": profit_protection_minimum_lock_percent,
    });
    let symbol = symbol.to_owned();
    tokio::spawn(async move {
        for attempt in 0..3 {
            match client.post(format!("{base}/internal/signals/scout-shadow"))
                .timeout(std::time::Duration::from_secs(2)).json(&request).send().await
                .and_then(reqwest::Response::error_for_status)
            {
                Ok(_) => return,
                Err(error) if attempt == 2 => crate::service_log!("WARN", "Could not register Scout shadow for outcome learning {}: {error}", symbol),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt + 1))).await,
            }
        }
    });
}

fn daily_scout_sample_cooldown_elapsed(now_ms: u64, previous_ms: u64) -> bool {
    now_ms.saturating_sub(previous_ms) >= DAILY_SCOUT_SHADOW_COOLDOWN_MS
}

fn live_parameter_probe_cooldown_elapsed(now_ms: u64, previous_ms: u64) -> bool {
    now_ms.saturating_sub(previous_ms) >= LIVE_SCOUT_PARAMETER_COOLDOWN_MS
}

fn live_scout_strategies_enabled_for_trading(settings: &Settings) -> Vec<Strategy> {
    settings.scout_strategies.iter().copied()
        .filter(|strategy| strategy.is_live_insight() && settings.strategies.contains(strategy))
        .collect()
}

fn should_record_daily_scout(decision: Result<StrategyDecision, String>, selected_scout: bool) -> bool {
    decision == Ok(StrategyDecision::Scout)
        || (selected_scout && decision == Ok(StrategyDecision::Enter))
}

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
        let parameters = signal_strategy_parameters(signal);
        let input = StrategySignal::entry(signal, &parameters);
        let decision = evaluate_daily_strategy_signal(signal);
        let selected_scout = settings.scout_strategies.contains(&strategy);
        let direct_entry_scout = selected_scout && decision == Ok(StrategyDecision::Enter);
        let parameter_probe = signal.strategy_decision.as_ref().and_then(|decision| {
            Some((decision.get("scout_parameter_key")?.as_str()?, decision.get("scout_parameter_value")?.as_f64()?))
        });
        if !should_record_daily_scout(decision.clone(), selected_scout) && parameter_probe.is_none() {
            continue;
        }
        // Keep rejected parameter variants in the shadow batch too. Their
        // forward markout measures whether moving the entry boundary would
        // have helped, instead of counting a synthetic zero as an outcome.
        // Breakout-distance ladders apply to Donchian. Other daily strategies
        // use a neutral shadow level and learn from the resulting simulated
        // trade outcome through their own strategy profile.
        let distance_percent = if strategy.as_str().starts_with("donchian-") {
            let Some(distance) = input.scout_breakout_distance_percent() else {
                continue;
            };
            distance
        } else {
            0.0
        };
        let level_candidates = if direct_entry_scout {
            vec![0.0]
        } else if let Some(("breakout_entry_distance_percent", value)) = parameter_probe {
            vec![value]
        } else if parameter_probe.is_some() {
            vec![0.0]
        } else {
            DAILY_SCOUT_SHADOW_LEVELS_PERCENT.to_vec()
        };
        let eligible_levels = level_candidates
            .into_iter()
            .filter(|level| direct_entry_scout
                || parameter_probe.is_some()
                || (distance_percent <= 0.0 && distance_percent >= -*level))
            .filter(|level| {
        let probe_suffix = parameter_probe.map_or_else(String::new, |(key, value)| format!("|probe:{key}:{value:.8}"));
        let sample_key = format!(
                    "daily-scout-level:{level:.2}|{}|{}|{}{probe_suffix}",
                    strategy.as_str(),
                    signal.symbol,
                    signal.direction
                );
                !settings.shadow_sample_active_scenarios.contains_key(&sample_key)
                    && settings
                        .shadow_sample_at_ms
                        .get(&sample_key)
                        .is_none_or(|previous| {
                            daily_scout_sample_cooldown_elapsed(timestamp_ms, *previous)
                        })
            })
            .collect::<Vec<_>>();
        if eligible_levels.is_empty() {
            continue;
        }
        // All Scout and exploratory groups share this global sample cap.
        let active_samples = settings.shadow_sample_active_scenarios.len();
        let available_slots = settings
            .max_concurrent_shadow_samples
            .saturating_sub(active_samples);
        if available_slots == 0 {
            continue;
        }
        // Keep the nearest breakout levels first when the configured sample
        // cap cannot fit the full ladder. Dropping the whole batch meant the
        // default cap of five could suppress a fully qualified eight-level
        // scout entirely.
        for level_percent in eligible_levels.into_iter().take(available_slots) {
            let parameter_probe = signal.strategy_decision.as_ref().and_then(|decision| {
                Some((decision.get("scout_parameter_key")?.as_str()?, decision.get("scout_parameter_value")?.as_f64()?))
            });
            let probe_suffix = parameter_probe.map_or_else(String::new, |(key, value)| format!("|probe:{key}:{value:.8}"));
            let sample_key = format!(
                "daily-scout-level:{level_percent:.2}|{}|{}|{}{probe_suffix}",
                strategy.as_str(), signal.symbol, signal.direction
            );
            let context = market_context(signal);
            if let (Some((parameter_key, parameter_value)), Some(base_decision)) = (
                parameter_probe,
                signal.strategy_decision.as_ref()
                    .and_then(|decision| decision.get("scout_parameter_baseline_decision"))
                    .and_then(serde_json::Value::as_object),
            ) {
                let comparison_id = format!("{sample_key}|{}|baseline-vs-variant", signal.timestamp);
                let mut baseline_signal = signal.clone();
                baseline_signal.strategy_decision = Some(serde_json::Value::Object(base_decision.clone()));
                let baseline_result = evaluate_daily_strategy_signal(&baseline_signal);
                let baseline_enters = matches!(baseline_result, Ok(StrategyDecision::Enter | StrategyDecision::Scout));
                let variant_enters = matches!(decision, Ok(StrategyDecision::Enter | StrategyDecision::Scout));
                if !baseline_enters && !variant_enters { continue; }

                settings.shadow_sample_at_ms.insert(sample_key.clone(), timestamp_ms);
                settings.shadow_sample_active_scenarios.insert(
                    sample_key.clone(), BTreeSet::from(["strategy".to_owned()]),
                );
                settings_changed = true;
                let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
                let paired_key = scout_parameter_learning_key(&signal.symbol, strategy, parameter_key, parameter_value);
                let pair = settings.scout_comparisons.entry(comparison_id.clone()).or_default();
                pair.updated_at_ms = timestamp_ms;
                pair.learning_key = paired_key;
                if !baseline_enters { pair.baseline_return_percent = Some(0.0); }
                if !variant_enters { pair.variant_return_percent = Some(0.0); }
                let mut variant_signal = signal.clone();
                if baseline_enters {
                    let baseline_decision = baseline_signal.strategy_decision.as_mut().unwrap();
                    baseline_decision["scout_comparison_id"] = serde_json::json!(comparison_id);
                    baseline_decision["scout_comparison_role"] = serde_json::json!("baseline");
                }
                if variant_enters {
                    let variant_decision = variant_signal.strategy_decision.as_mut().unwrap();
                    variant_decision["scout_comparison_id"] = serde_json::json!(comparison_id);
                    variant_decision["scout_comparison_role"] = serde_json::json!("variant");
                }
                for (selected, role) in [
                    (baseline_enters.then_some(&baseline_signal), "baseline"),
                    (variant_enters.then_some(&variant_signal), "variant"),
                ] {
                    let Some(selected) = selected else { continue; };
                    let selected_parameters = signal_strategy_parameters(selected);
                    let selected_stop = selected_parameters.get("stop_loss_percent").copied().unwrap_or(settings.stop_loss_percent);
                    let selected_target = selected_parameters.get("take_profit_percent").copied().unwrap_or(settings.take_profit_percent);
                    let selected_level_percent = if role == "baseline"
                        && parameter_key == "breakout_entry_distance_percent"
                    {
                        signal.strategy_decision.as_ref()
                            .and_then(|decision| decision.get("scout_parameter_baseline_decision"))
                            .and_then(|decision| decision.get("strategy_parameters"))
                            .and_then(|parameters| parameters.get("breakout_entry_distance_percent"))
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(level_percent)
                    } else {
                        level_percent
                    };
                    let mut audit = scout_shadow_audit_payload(
                        selected, timestamp_ms, notional,
                        &format!("scout:{context}:level:{level_percent:.2}:scenario:strategy:{role}"),
                        selected_stop, selected_target, None, selected_level_percent, "strategy",
                    );
                    audit["shadow_sample_group_id"] = serde_json::json!(sample_key);
                    audit["decision_id"] = serde_json::json!(format!("{}|{role}", audit["decision_id"].as_str().unwrap_or("scout")));
                    audit["scout_comparison_id"] = serde_json::json!(comparison_id);
                    audit["scout_comparison_role"] = serde_json::json!(role);
                    audits.push(audit);
                }
                continue;
            }
            settings.shadow_sample_at_ms.insert(sample_key.clone(), timestamp_ms);
            settings.shadow_sample_active_scenarios.insert(
                sample_key.clone(),
                BTreeSet::from(["strategy".to_owned(), "tight".to_owned(), "wide".to_owned()]),
            );
            settings_changed = true;
            let strategy_size_multiplier = signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_order_size_multiplier"))
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(1.0)
                .clamp(0.05, 1.0);
            let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION * strategy_size_multiplier;
            let strategy_parameters = signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("strategy_parameters"));
            let stop_loss_percent = strategy_parameters
                .and_then(|parameters| parameters.get("stop_loss_percent"))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(settings.stop_loss_percent);
            let take_profit_percent = strategy_parameters
                .and_then(|parameters| parameters.get("take_profit_percent"))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(settings.take_profit_percent);
            for (scenario, multiplier) in [
                ("strategy", 1.0),
                ("tight", SCOUT_TIGHT_EXIT_DISTANCE_MULTIPLIER),
                ("wide", SCOUT_WIDE_EXIT_DISTANCE_MULTIPLIER),
            ] {
                let mut audit = scout_shadow_audit_payload(
                    signal,
                    timestamp_ms.saturating_add(audits.len() as u64),
                    notional,
                    &format!("scout:{context}:level:{level_percent:.2}:scenario:{scenario}:trade"),
                    stop_loss_percent * multiplier,
                    take_profit_percent * multiplier,
                    None,
                    level_percent,
                    scenario,
                );
                audit["shadow_sample_group_id"] = serde_json::json!(sample_key.clone());
                if direct_entry_scout {
                    if let Some(entry_price) = signal.strategy_decision.as_ref()
                        .and_then(|decision| decision.get("observed_close"))
                        .and_then(serde_json::Value::as_f64)
                    {
                        audit["shadow_entry_price"] = serde_json::json!(entry_price);
                    }
                }
                audits.push(audit);
            }
        }
    }
    drop(settings);
    if settings_changed {
        request_settings_persist(state)?;
    }
    for audit in audits {
        register_scout_shadow_for_learning(state, &audit);
        queue_auto_trader_audit(state, audit);
    }
    Ok(())
}

async fn record_live_strategy_scout_shadows(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
    candidates: &[Signal],
    markets: &BTreeMap<String, serde_json::Value>,
    now: u64,
) -> Result<(), (StatusCode, String)> {
    if candidates.is_empty() { return Ok(()); }
    let timestamp_ms = now.saturating_mul(1_000);
    let mut fresh_markets = markets.clone();
    let symbols = candidates.iter().map(|candidate| candidate.symbol.as_str()).collect::<BTreeSet<_>>();
    for symbol in symbols {
        let cached_is_fresh = fresh_markets.get(symbol).is_some_and(|market| {
            let updated_at = market.get("updated_at_ms").and_then(serde_json::Value::as_u64).unwrap_or_default();
            updated_at > 0 && timestamp_ms.saturating_sub(updated_at) <= 5_000
        });
        if !cached_is_fresh {
            if let Some(market) = fresh_subscribed_market_book(client, position_url, symbol).await {
                fresh_markets.insert(symbol.to_owned(), market);
            }
        }
    }
    let mut settings = state.settings.lock().await;
    settings.shadow_sample_at_ms.retain(|_, timestamp| timestamp_ms.saturating_sub(*timestamp) < 86_400_000);
    let mut audits = Vec::new();
    let mut settings_changed = false;
    let (mut unselected, mut missing_market, mut invalid_market) = (0usize, 0usize, 0usize);
    let (mut active_group_blocked, mut cooldown_blocked, mut capacity_blocked) = (0usize, 0usize, 0usize);
    for candidate in candidates {
        let Some(strategy) = candidate.strategy.as_deref().and_then(Strategy::from_name) else { unselected += 1; continue; };
        if !settings.scout_strategies.contains(&strategy) || !settings.strategies.contains(&strategy) { unselected += 1; continue; }
        let Some(market) = fresh_markets.get(&candidate.symbol) else { missing_market += 1; continue; };
        let updated_at = market.get("updated_at_ms").and_then(serde_json::Value::as_u64).unwrap_or_default();
        if updated_at == 0 || timestamp_ms.saturating_sub(updated_at) > 5_000 { continue; }
        let Some((bid, ask)) = market["bids"][0]["price"].as_f64()
            .zip(market["asks"][0]["price"].as_f64())
            .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
        else { invalid_market += 1; continue; };
        let sample_key = format!("{}|{}|{}", strategy.as_str(), candidate.symbol, candidate.direction);
        let active_group_key = format!("live-scout:{sample_key}");
        let active_group_exists = settings.shadow_sample_active_scenarios.contains_key(&active_group_key);
        let previous_attempt_ms = settings.shadow_sample_at_ms.get(&sample_key).copied();
        let active_sample_count = settings.shadow_sample_active_scenarios.len();
        if active_group_exists {
            active_group_blocked += 1;
            continue;
        }
        if previous_attempt_ms.is_some_and(|previous| timestamp_ms.saturating_sub(previous) < 300_000) {
            cooldown_blocked += 1;
            continue;
        }
        if active_sample_count >= settings.max_concurrent_shadow_samples {
            capacity_blocked += 1;
            continue;
        }

        let mut signal = candidate.clone();
        let decision = signal.strategy_decision.get_or_insert_with(|| serde_json::json!({}));
        decision["observed_close"] = serde_json::json!(bid + (ask - bid) * 0.5);
        let params = decision.get("strategy_parameters");
        let stop_loss = params.and_then(|values| values.get("stop_loss_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.stop_loss_percent);
        let take_profit = params.and_then(|values| values.get("take_profit_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.take_profit_percent);
        let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
        let context = market_context(&signal);
        settings.shadow_sample_at_ms.insert(sample_key, timestamp_ms);
        settings.shadow_sample_active_scenarios.insert(
            active_group_key.clone(),
            BTreeSet::from(["strategy".to_owned(), "tight".to_owned(), "wide".to_owned()]),
        );
        settings_changed = true;
        for (index, (scenario, multiplier)) in [
            ("strategy", 1.0),
            ("tight", SCOUT_TIGHT_EXIT_DISTANCE_MULTIPLIER),
            ("wide", SCOUT_WIDE_EXIT_DISTANCE_MULTIPLIER),
        ].into_iter().enumerate() {
            let mut audit = scout_shadow_audit_payload(
                &signal,
                timestamp_ms.saturating_add(index as u64),
                notional,
                &format!("scout:{context}:trade:scenario:{scenario}"),
                stop_loss * multiplier,
                take_profit * multiplier,
                None,
                0.0,
                scenario,
            );
            audit["shadow_sample_group_id"] = serde_json::json!(active_group_key);
            audits.push(audit);
        }
    }
    drop(settings);
    if settings_changed { request_settings_persist(state)?; }
    let registered = audits.len() / 3;
    for audit in audits {
        register_scout_shadow_for_learning(state, &audit);
        queue_auto_trader_audit(state, audit);
    }
    static LAST_SCOUT_BATCH_DIAGNOSTIC_MS: AtomicU64 = AtomicU64::new(0);
    if timestamp_ms.saturating_sub(LAST_SCOUT_BATCH_DIAGNOSTIC_MS.load(Ordering::Relaxed)) >= 30_000
        && LAST_SCOUT_BATCH_DIAGNOSTIC_MS.compare_exchange(
            LAST_SCOUT_BATCH_DIAGNOSTIC_MS.load(Ordering::Relaxed), timestamp_ms, Ordering::Relaxed, Ordering::Relaxed,
        ).is_ok()
    {
        crate::service_log!("INFO", "Live Scout shadow batch: candidates={} registered_groups={} unselected={} missing_market={} invalid_market={} active_group_blocked={} cooldown_blocked={} capacity_blocked={}", candidates.len(), registered, unselected, missing_market, invalid_market, active_group_blocked, cooldown_blocked, capacity_blocked);
    }
    Ok(())
}

struct LiveScoutParameterProbe {
    source_signal: Signal,
    strategy: Strategy,
    direction: String,
    baseline_parameters: BTreeMap<String, f64>,
}

fn parameter_probe_signal(
    source_signal: &Signal,
    strategy: Strategy,
    direction: &str,
    parameters: &BTreeMap<String, f64>,
    entry_price: f64,
) -> Option<Signal> {
    let mut signal = live_strategy_feature_signal(source_signal, strategy, parameters)?;
    signal.direction = direction.to_owned();
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
        "direction_source": "strategy_insight",
        "observed_close": entry_price,
    }));
    Some(signal)
}

fn live_parameter_probe_entry_eligible(
    source_signal: &Signal,
    strategy: Strategy,
    direction: &str,
    parameters: &BTreeMap<String, f64>,
    entry_price: f64,
) -> bool {
    parameter_probe_signal(source_signal, strategy, direction, parameters, entry_price)
        .is_some_and(|signal| {
            matches!(
                strategy.evaluate(&StrategySignal::entry(&signal, parameters)),
                Ok(StrategyDecision::Enter),
            ) && passes_pullback_confirmation(&signal)
        })
}

fn parameter_probe_changes_exit_behavior(parameter_key: &str, strategy: Strategy) -> bool {
    let keys = if parameter_key.starts_with("combo:") {
        let Some(values) = parse_scout_combination(parameter_key, strategy) else { return false; };
        values.keys().cloned().collect::<Vec<_>>()
    } else {
        vec![parameter_key.to_owned()]
    };
    keys.iter().any(|key| matches!(key.as_str(),
        "stop_loss_percent"
            | "take_profit_percent"
            | "profit_protection_activation_percent"
            | "profit_protection_giveback_percent"
            | "profit_protection_minimum_lock_percent"
            | "minimum_hold_days"
            | "maximum_hold_days"
    ))
}

async fn record_live_strategy_parameter_probes(
    state: &AutoTraderState,
    client: &reqwest::Client,
    position_url: &str,
    probes: &[LiveScoutParameterProbe],
    markets: &BTreeMap<String, serde_json::Value>,
    now: u64,
) -> Result<(), (StatusCode, String)> {
    if probes.is_empty() { return Ok(()); }
    let timestamp_ms = now.saturating_mul(1_000);
    let mut fresh_markets = markets.clone();
    let symbols = probes.iter().map(|probe| probe.source_signal.symbol.as_str()).collect::<BTreeSet<_>>();
    for symbol in symbols {
        let cached_is_fresh = fresh_markets.get(symbol).is_some_and(|market| {
            let updated_at = market.get("updated_at_ms").and_then(serde_json::Value::as_u64).unwrap_or_default();
            updated_at > 0 && timestamp_ms.saturating_sub(updated_at) <= 5_000
        });
        if !cached_is_fresh {
            if let Some(market) = fresh_subscribed_market_book(client, position_url, symbol).await {
                fresh_markets.insert(symbol.to_owned(), market);
            }
        }
    }
    for probe in probes {
        let Some(market) = fresh_markets.get(&probe.source_signal.symbol) else { continue; };
        let updated_at = market.get("updated_at_ms").and_then(serde_json::Value::as_u64).unwrap_or_default();
        if updated_at == 0 || timestamp_ms.saturating_sub(updated_at) > 5_000 { continue; }
        let Some((bid, ask)) = market["bids"][0]["price"].as_f64()
            .zip(market["asks"][0]["price"].as_f64())
            .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
        else { continue; };
        let entry_price = bid + (ask - bid) * 0.5;
        if let Err((status, error)) = record_live_parameter_probe(
            state,
            &probe.source_signal,
            probe.strategy,
            &probe.direction,
            &probe.baseline_parameters,
            entry_price,
            now,
        ).await {
            crate::service_log!("WARN", "Could not record live Scout parameter comparison ({status}): {error}", status=status);
        }
    }
    Ok(())
}

async fn record_live_parameter_probe(
    state: &AutoTraderState,
    source_signal: &Signal,
    strategy: Strategy,
    direction: &str,
    baseline_parameters: &BTreeMap<String, f64>,
    entry_price: f64,
    now: u64,
) -> Result<(), (StatusCode, String)> {
    if !strategy.is_live_insight() || strategy == Strategy::LiveInsights { return Ok(()); }
    let mut variant_parameters = baseline_parameters.clone();
    let pair = next_scout_parameter_combination(
        &source_signal.symbol, strategy, &variant_parameters, now / 86_400,
    );
    let (parameter_key, parameter_value) = if let Some(combination) = pair.as_ref() {
        for (key, value) in combination { variant_parameters.insert(key.clone(), *value); }
        let Some(key) = scout_combination_key(combination, &variant_parameters) else { return Ok(()); };
        (key, 0.0)
    } else {
        let Some((key, value)) = next_scout_parameter_probe(
            &source_signal.symbol, strategy, &variant_parameters, now / 86_400,
        ) else { return Ok(()); };
        variant_parameters.insert(key.clone(), value);
        (key, value)
    };
    let sample_group_id = format!(
        "live-scout-parameter:{}:{}:{}:{}",
        source_signal.symbol, strategy.as_str(), direction, parameter_key,
    );
    let sampled_at = now.saturating_mul(1_000);
    {
        let mut settings = state.settings.lock().await;
        settings.shadow_sample_at_ms.retain(|_, timestamp| sampled_at.saturating_sub(*timestamp) < 86_400_000);
        if settings.shadow_sample_active_scenarios.contains_key(&sample_group_id)
            || settings.shadow_sample_at_ms.get(&sample_group_id)
                .is_some_and(|previous| !live_parameter_probe_cooldown_elapsed(sampled_at, *previous))
            || settings.shadow_sample_active_scenarios.len() >= settings.max_concurrent_shadow_samples
        {
            return Ok(());
        }
        settings.shadow_sample_at_ms.insert(sample_group_id.clone(), sampled_at);
        settings.shadow_sample_active_scenarios.insert(
            sample_group_id.clone(), BTreeSet::from(["preparing".to_owned()]),
        );
        request_settings_persist(state)?;
    }

    let Some(mut baseline_signal) = parameter_probe_signal(
        source_signal, strategy, direction, baseline_parameters, entry_price,
    ) else {
        let mut settings = state.settings.lock().await;
        settings.shadow_sample_active_scenarios.remove(&sample_group_id);
        request_settings_persist(state)?;
        return Ok(());
    };
    let mut variant_signal = parameter_probe_signal(
        source_signal, strategy, direction, &variant_parameters, entry_price,
    );
    let baseline_entry_eligible = live_parameter_probe_entry_eligible(
        source_signal, strategy, direction, baseline_parameters, entry_price,
    );
    let variant_entry_eligible = live_parameter_probe_entry_eligible(
        source_signal, strategy, direction, &variant_parameters, entry_price,
    );
    if !baseline_entry_eligible && !variant_entry_eligible {
        let mut settings = state.settings.lock().await;
        settings.shadow_sample_active_scenarios.remove(&sample_group_id);
        request_settings_persist(state)?;
        return Ok(());
    }
    if baseline_entry_eligible && variant_entry_eligible
        && !parameter_probe_changes_exit_behavior(&parameter_key, strategy)
    {
        // Both entry rules accepted the same signal and the changed parameters
        // don't affect exits, so the simulated trades would be identical.
        let mut settings = state.settings.lock().await;
        settings.shadow_sample_active_scenarios.remove(&sample_group_id);
        request_settings_persist(state)?;
        return Ok(());
    }

    let baseline_exit_parameters = baseline_parameters;
    let variant_exit_parameters = &variant_parameters;
    let stop_loss = |parameters: &BTreeMap<String, f64>| {
        parameters.get("stop_loss_percent").copied().unwrap_or(DEFAULT_STOP_LOSS_PERCENT)
    };
    let take_profit = |parameters: &BTreeMap<String, f64>| {
        parameters.get("take_profit_percent").copied().unwrap_or(DEFAULT_TAKE_PROFIT_PERCENT)
    };
    let notional = state.settings.lock().await.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
    let comparison_id = format!("{}|{}|baseline-vs-variant", sample_group_id, source_signal.timestamp);
    let baseline_group = format!("{sample_group_id}|baseline");
    let variant_group = format!("{sample_group_id}|variant");
    {
        let mut settings = state.settings.lock().await;
        settings.shadow_sample_active_scenarios.remove(&sample_group_id);
        let needed = usize::from(baseline_entry_eligible) + usize::from(variant_entry_eligible);
        if settings.shadow_sample_active_scenarios.len().saturating_add(needed) > settings.max_concurrent_shadow_samples {
            request_settings_persist(state)?;
            return Ok(());
        }
        if baseline_entry_eligible {
            settings.shadow_sample_active_scenarios.insert(
                baseline_group.clone(), BTreeSet::from(["strategy".to_owned()]),
            );
        }
        if variant_entry_eligible {
            settings.shadow_sample_active_scenarios.insert(
                variant_group.clone(), BTreeSet::from(["strategy".to_owned()]),
            );
        }
        let pair = settings.scout_comparisons.entry(comparison_id.clone()).or_default();
        pair.updated_at_ms = sampled_at;
        pair.learning_key = scout_parameter_learning_key(
            &source_signal.symbol, strategy, &parameter_key, parameter_value,
        );
        if !baseline_entry_eligible { pair.baseline_return_percent = Some(0.0); }
        if !variant_entry_eligible { pair.variant_return_percent = Some(0.0); }
        request_settings_persist(state)?;
    }

    let mut audits = Vec::new();
    if baseline_entry_eligible {
        let mut decision = baseline_signal.strategy_decision.clone().unwrap_or_else(|| serde_json::json!({}));
        decision["scout_comparison_id"] = serde_json::json!(comparison_id);
        decision["scout_comparison_role"] = serde_json::json!("baseline");
        baseline_signal.strategy_decision = Some(decision);
        let mut audit = scout_shadow_audit_payload(
            &baseline_signal,
            sampled_at,
            notional,
            &format!("scout:live-parameter:{}:{}:{}:baseline:trade", source_signal.symbol, strategy.as_str(), parameter_key),
            stop_loss(baseline_exit_parameters),
            take_profit(baseline_exit_parameters),
            None,
            0.0,
            "strategy",
        );
        audit["shadow_sample_group_id"] = serde_json::json!(baseline_group);
        audit["scout_comparison_id"] = serde_json::json!(comparison_id);
        audit["scout_comparison_role"] = serde_json::json!("baseline");
        let baseline_decision_id = audit["decision_id"].as_str().unwrap_or("scout").to_owned();
        audit["decision_id"] = serde_json::json!(format!("{baseline_decision_id}|comparison:baseline"));
        audit["trade_id"] = audit["decision_id"].clone();
        audits.push(audit);
    }
    if variant_entry_eligible {
        let mut variant_signal = variant_signal.take().expect("eligible variant signal exists");
        let mut decision = variant_signal.strategy_decision.clone().unwrap_or_else(|| serde_json::json!({}));
        decision["scout_parameter_probe"] = serde_json::json!(true);
        decision["scout_parameter_key"] = serde_json::json!(parameter_key);
        decision["scout_parameter_value"] = serde_json::json!(parameter_value);
        decision["scout_parameter_profile_symbol"] = serde_json::json!(source_signal.symbol);
        decision["scout_parameter_entry_eligible"] = serde_json::json!(true);
        decision["scout_comparison_id"] = serde_json::json!(comparison_id);
        decision["scout_comparison_role"] = serde_json::json!("variant");
        variant_signal.strategy_decision = Some(decision);
        let mut audit = scout_shadow_audit_payload(
            &variant_signal,
            sampled_at,
            notional,
            &format!("scout:live-parameter:{}:{}:{}:variant:trade", source_signal.symbol, strategy.as_str(), parameter_key),
            stop_loss(variant_exit_parameters),
            take_profit(variant_exit_parameters),
            None,
            0.0,
            "strategy",
        );
        audit["shadow_sample_group_id"] = serde_json::json!(variant_group);
        audit["scout_comparison_id"] = serde_json::json!(comparison_id);
        audit["scout_comparison_role"] = serde_json::json!("variant");
        let variant_decision_id = audit["decision_id"].as_str().unwrap_or("scout").to_owned();
        audit["decision_id"] = serde_json::json!(format!("{variant_decision_id}|comparison:variant"));
        audit["trade_id"] = audit["decision_id"].clone();
        audits.push(audit);
    }
    for audit in audits {
        register_scout_shadow_for_learning(state, &audit);
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
    if state.audit_health.load(Ordering::Relaxed) != 3
        || state.audit_failure_latched.load(Ordering::Relaxed)
    {
        return Ok(skipped_for_signal(
            signal,
            "Audit persistence unavailable; new entries paused until AutoTrader restart",
        ));
    }
    if should_shadow_daily_scout(scout, signal) {
        return Ok(CycleResult {
            status: "shadowed",
            reason: Some("Daily scout candidate is shadow-only; no order submitted".into()),
            symbol: Some(signal.symbol.clone()),
            side: Some(
                if signal.direction == "bullish" {
                    "buy"
                } else {
                    "sell"
                }
                .into(),
            ),
            quantity: None,
            order_type: None,
            order_response: None,
        });
    }
    let mut priced_live_scout = None;
    if scout && is_live_scout_only_signal(signal) {
        let market = match load_signal_market_snapshot(
            client,
            position_url,
            dashboard,
            now,
            signal,
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(result) => return Ok(result),
        };
        let mut candidate = signal.clone();
        candidate
            .strategy_decision
            .get_or_insert_with(|| serde_json::json!({}))["observed_close"] =
            serde_json::json!(market.mid);
        priced_live_scout = Some(candidate);
    }
    let signal = priced_live_scout.as_ref().unwrap_or(signal);
    let evidence = dashboard.walk_forward.iter().find(|item| {
        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    });
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
        let active_group_key = format!("live-scout:{sample_key}");
        let recent_probe_count = settings.shadow_sample_active_scenarios.len();
        let previous_attempt_ms = settings.shadow_sample_at_ms.get(&sample_key).copied();
        if shadow_entry_limit_blocks(
            scout,
            settings.shadow_sample_active_scenarios.contains_key(&active_group_key),
            previous_attempt_ms,
            sampled_at,
            recent_probe_count,
            settings.max_concurrent_shadow_samples,
        ) {
            let reason = if recent_probe_count >= settings.max_concurrent_shadow_samples {
                "The capped exploration-entry limit is already in use"
            } else {
                "A capped exploration entry for this strategy, symbol, and direction was attempted in the last five minutes"
            };
            return Ok(skipped_for_signal(signal, reason));
        }
        if scout {
            let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
            let context = market_context(signal);
            let sample_group_id = active_group_key;
            scout_shadow_sample = Some((notional, format!("scout:{context}:trade"), sample_group_id));
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
            if open_scout_positions + pending_scout_orders >= settings.scout_max_active_positions {
                return Ok(skipped_for_signal(
                    signal,
                    "Maximum number of active and pending scout positions is already in use",
                ));
            }
        }
        if let Some((_, _, sample_group_id)) = scout_shadow_sample.as_ref() {
            settings.shadow_sample_active_scenarios.insert(
                sample_group_id.clone(),
                BTreeSet::from(["strategy".to_owned(), "tight".to_owned(), "wide".to_owned()]),
            );
        }
        settings.shadow_sample_at_ms.insert(sample_key, sampled_at);
        drop(settings);
        request_settings_persist(state)?;
    }
    if let Some((notional, learning_context, sample_group_id)) = scout_shadow_sample {
        for (scenario_index, (scenario, multiplier)) in [
            ("strategy", 1.0),
            ("tight", SCOUT_TIGHT_EXIT_DISTANCE_MULTIPLIER),
                ("wide", SCOUT_WIDE_EXIT_DISTANCE_MULTIPLIER),
        ].into_iter().enumerate() {
            let mut shadow_payload = scout_shadow_audit_payload(
                signal,
                now.saturating_mul(1_000).saturating_add(scenario_index as u64),
                notional,
                &format!("{learning_context}:scenario:{scenario}"),
                counterfactual_stop_loss_percent * multiplier,
                counterfactual_take_profit_percent * multiplier,
                evidence,
                0.0,
                scenario,
            );
            shadow_payload["shadow_sample_group_id"] = serde_json::json!(sample_group_id);
            register_scout_shadow_for_learning(state, &shadow_payload);
            queue_auto_trader_audit(state, shadow_payload);
        }
        return Ok(CycleResult {
            status: "shadowed",
            reason: Some("Scout shadow sample recorded; no order submitted".into()),
            symbol: Some(signal.symbol.clone()),
            side: Some(
                if signal.direction == "bullish" {
                    "buy"
                } else {
                    "sell"
                }
                .into(),
            ),
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

fn evaluate_daily_strategy_signal(signal: &Signal) -> Result<StrategyDecision, String> {
    let strategy = signal
        .strategy
        .as_deref()
        .and_then(Strategy::from_name)
        .ok_or_else(|| "Unknown daily strategy".to_string())?;
    if signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("scout_parameter_probe"))
        .and_then(serde_json::Value::as_bool) == Some(true)
    {
        return Ok(StrategyDecision::Scout);
    }
    let parameters = signal_strategy_parameters(signal);
    let input = StrategySignal::entry(signal, &parameters);
    let decision = strategy.evaluate(&input)?;
    // Every published independent daily-strategy candidate with daily feature
    // data can run as a deliberately small scout when it misses full entry.
    if decision == StrategyDecision::Reject
        && input.daily_features.is_some()
        && signal.strategy_candidate
        && is_independent_daily_strategy(strategy.as_str())
        && !strategy.as_str().starts_with("donchian-")
    {
        Ok(StrategyDecision::Scout)
    } else {
        Ok(decision)
    }
}

fn daily_strategy_decision(signal: &Signal) -> Option<StrategyDecision> {
    evaluate_daily_strategy_signal(signal).ok()
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
        let mut live = enabled_live.clone();
        live.extend(settings.scout_strategies.iter().copied().filter(|strategy| {
            strategy.is_live_insight() && !strategy.is_legacy_composite() && !enabled_live.contains(strategy)
        }));
        (
            live,
            settings
                .strategies
                .iter()
                .any(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry()),
            settings.clone(),
        )
    };
    let live_source_signals = live_strategy_source_signals(&dashboard.best);
    let has_live_signal_strategies = !live_strategies.is_empty() && !live_source_signals.is_empty();
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
                if strategy_settings.scout_strategies.contains(&strategy) {
                    let is_parameter_probe = signal.strategy_decision.as_ref()
                        .and_then(|decision| decision.get("scout_parameter_probe"))
                        .and_then(serde_json::Value::as_bool) == Some(true);
                    match evaluate_daily_strategy_signal(signal) {
                        Ok(StrategyDecision::Scout | StrategyDecision::Enter) => {
                            daily_scout_shadow_candidates.push(signal.clone());
                        }
                        Ok(_) if is_parameter_probe => daily_scout_shadow_candidates.push(signal.clone()),
                        Ok(_) => {}
                        Err(error) if strategy_filter_details.len() < 20 => {
                            strategy_filter_details.push(format!(
                                "{} {} [{}] · Scout evaluation error: {}",
                                signal.symbol, signal.direction, strategy.as_str(), error
                            ));
                        }
                        Err(_) => {}
                    }
                    return false;
                }
                if strategy_filter_details.len() < 20 {
                    strategy_filter_details.push(format!(
                        "{} {} [{}] · Strategy disabled in AutoTrader selection",
                        signal.symbol,
                        signal.direction,
                        strategy.as_str()
                    ));
                }
                return false;
            }
            let is_parameter_probe = signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("scout_parameter_probe"))
                .and_then(serde_json::Value::as_bool) == Some(true);
            match evaluate_daily_strategy_signal(signal) {
                Ok(StrategyDecision::Enter) => {
                    if strategy_settings.scout_strategies.contains(&strategy) {
                        daily_scout_shadow_candidates.push(signal.clone());
                    }
                    true
                }
                Ok(StrategyDecision::Scout) => {
                    daily_scout_shadow_candidates.push(signal.clone());
                    true
                }
                Ok(_) if is_parameter_probe => {
                    daily_scout_shadow_candidates.push(signal.clone());
                    false
                }
                Ok(decision) => {
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · Strategy entry rule returned {:?}",
                            signal.symbol,
                            signal.direction,
                            strategy.as_str(),
                            decision
                        ));
                    }
                    false
                }
                Err(error) => {
                    if strategy_filter_details.len() < 20 {
                        strategy_filter_details.push(format!(
                            "{} {} [{}] · Strategy evaluation error: {}",
                            signal.symbol,
                            signal.direction,
                            strategy.as_str(),
                            error
                        ));
                    }
                    false
                }
            }
        });
    }
    let live_strategy_signals = if has_live_signal_strategies {
        live_source_signals.iter().flat_map(|source_signal| {
            let strategy_settings = &strategy_settings;
            live_strategies.iter().copied().flat_map(move |strategy| {
                let parameters = strategy_parameter_values(
                    strategy_settings,
                    &source_signal.symbol,
                    strategy,
                );
                let scout_only = !strategy_settings.strategies.contains(&strategy);
                let candidates = if scout_only {
                    live_scout_direction_candidates(source_signal, strategy, &parameters)
                } else {
                    live_strategy_feature_signal(source_signal, strategy, &parameters)
                        .into_iter()
                        .collect()
                };
                candidates.into_iter().filter_map(move |mut signal| {
                    let input = StrategySignal::entry(&signal, &parameters);
                    if !matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter)) {
                        return None;
                    }
                    if scout_only {
                        signal.score = source_signal.score.abs()
                            * if signal.direction == "bullish" { 1.0 } else { -1.0 };
                    }
                    signal.strategy = Some(strategy.as_str().to_owned());
                    signal.strategy_decision = Some(serde_json::json!({
                        "scout_only": scout_only,
                        "strategy": strategy.as_str(),
                        "contract": strategy.contract(),
                        "strategy_parameters": parameters,
                        "observed_insight": signal.components,
                        "direction": signal.direction,
                        "strength": signal.strength,
                        "score": signal.score,
                        "direction_source": if scout_only { "strategy_insight" } else { "aggregate_signal" },
                    }));
                    Some(signal)
                })
            })
        }).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let selected_live_scout_strategies = strategy_settings.scout_strategies.iter().copied()
        .filter(|strategy| strategy.is_live_insight() && *strategy != Strategy::LiveInsights)
        .collect::<Vec<_>>();
    let live_parameter_probe_candidates = live_source_signals.iter().flat_map(|source_signal| {
        let strategy_settings = &strategy_settings;
        selected_live_scout_strategies.iter().copied().flat_map(move |strategy| {
            let parameters = strategy_parameter_values(strategy_settings, &source_signal.symbol, strategy);
            live_scout_direction_probes(source_signal, strategy, &parameters).into_iter()
                .map(move |candidate| LiveScoutParameterProbe {
                    source_signal: source_signal.clone(),
                    strategy,
                    direction: candidate.direction,
                    baseline_parameters: parameters.clone(),
                })
        })
    }).collect::<Vec<_>>();
    if !live_parameter_probe_candidates.is_empty() {
        let probe_state = state.clone();
        let probe_client = client.clone();
        let probe_position_url = position_url.clone();
        let markets = dashboard.markets.clone();
        tokio::spawn(async move {
            if let Err((status, error)) = record_live_strategy_parameter_probes(
                &probe_state,
                &probe_client,
                &probe_position_url,
                &live_parameter_probe_candidates,
                &markets,
                now,
            ).await {
                crate::service_log!("WARN", "Could not record live Scout parameter comparisons ({status}): {error}");
            }
        });
    }
    let live_scout_strategies = live_scout_strategies_enabled_for_trading(&strategy_settings);
    let mut live_scout_signals = live_strategy_signals.iter()
        .filter(|signal| signal.strategy.as_deref().and_then(Strategy::from_name)
            .is_some_and(|strategy| live_scout_strategies.contains(&strategy)))
        .cloned()
        .collect::<Vec<_>>();
    live_scout_signals.extend(live_source_signals.iter().flat_map(|source_signal| {
        let strategy_settings = &strategy_settings;
        live_scout_strategies.iter().copied().flat_map(move |strategy| {
            let parameters = strategy_parameter_values(strategy_settings, &source_signal.symbol, strategy);
            live_scout_direction_candidates(source_signal, strategy, &parameters)
                .into_iter()
                .map(move |mut signal| {
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
                        "direction_source": "strategy_insight",
                    }));
                    signal
                })
        })
    }));
    static LAST_SCOUT_CANDIDATE_DIAGNOSTIC_MS: AtomicU64 = AtomicU64::new(0);
    let diagnostic_ms = now.saturating_mul(1_000);
    if diagnostic_ms.saturating_sub(LAST_SCOUT_CANDIDATE_DIAGNOSTIC_MS.load(Ordering::Relaxed)) >= 30_000
        && LAST_SCOUT_CANDIDATE_DIAGNOSTIC_MS.compare_exchange(
            LAST_SCOUT_CANDIDATE_DIAGNOSTIC_MS.load(Ordering::Relaxed), diagnostic_ms, Ordering::Relaxed, Ordering::Relaxed,
        ).is_ok()
    {
        crate::service_log!("INFO", "Live Scout candidates prepared: selected_strategies={} shadow_candidates={} trade_candidates={}", live_scout_strategies.len(), live_scout_signals.len(), live_strategy_signals.len());
    }
    if !live_scout_signals.is_empty() {
        let scout_state = state.clone();
        let scout_position_url = position_url.clone();
        let markets = dashboard.markets.clone();
        tokio::spawn(async move {
            if let Err((status, error)) = record_live_strategy_scout_shadows(
                &scout_state,
                &scout_state.client,
                &scout_position_url,
                &live_scout_signals,
                &markets,
                now,
            ).await {
                crate::service_log!("WARN", "Could not record live strategy Scout shadows ({status}): {error}");
            }
        });
    }
    if has_daily_strategy_signals {
        dashboard.best.extend(live_strategy_signals);
    } else if has_live_signal_strategies {
        dashboard.best = live_strategy_signals;
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
    if !daily_scout_shadow_candidates.is_empty() {
        let scout_state = state.clone();
        tokio::spawn(async move {
            if let Err((status, error)) = record_daily_scout_shadow_ladder(
                &scout_state,
                &daily_scout_shadow_candidates,
                now,
            )
            .await
            {
                crate::service_log!(
                    "WARN",
                    "Could not record daily Scout shadow ladder ({status}): {error}"
                );
            }
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
    let (open_positions, active_scout_count, strategy_positions, max_entries_per_auto_position, scout_max_active_positions) = {
        let settings = state.settings.lock().await;
        let (open_positions, active_scout_count) = settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .fold(
                (BTreeMap::new(), 0usize),
                |(mut positions, scouts), (symbol, position)| {
                    if !position.scout_entry {
                        positions.insert(symbol.clone(), position.clone());
                    }
                    (positions, scouts + usize::from(position.scout_entry))
                },
            );
        let strategy_positions = settings.strategy_positions.iter().filter_map(|(symbol, lots)| {
            let real_lots = lots.iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12 && !position.scout_entry)
                .map(|(strategy, position)| (strategy.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>();
            (!real_lots.is_empty()).then(|| (symbol.clone(), real_lots))
        }).collect();
        (
            open_positions,
            active_scout_count,
            strategy_positions,
            settings.max_entries_per_auto_position,
            settings.scout_max_active_positions,
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
                let is_scout =
                    daily_strategy_decision(&sized_signal) == Some(StrategyDecision::Scout);
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
    let live_neutral_scout_candidates = if has_daily_strategy_signals {
        neutral_scout_candidates
            .iter()
            .filter(|signal| is_fallback_signal_candidate(signal))
            .cloned()
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
    let selection_neutral_scout_candidates = if has_daily_strategy_signals {
        &live_neutral_scout_candidates
    } else {
        &neutral_scout_candidates
    };
    let (signal, exploratory, mut scout) = match select_cycle_signal(SignalSelectionInput {
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
        scout_max_active_positions,
        open_positions: &open_positions,
        max_entries_per_auto_position,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    }) {
        Ok(selected) => selected,
        Err(result) => return Ok(daily_batch_result.unwrap_or(result)),
    };
    scout |= signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("scout_only"))
        .and_then(serde_json::Value::as_bool) == Some(true);
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        signal,
        &dashboard,
        &learning,
        risk_appetite,
        active_scout_count,
        scout_max_active_positions,
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
    fn full_shadow_sample_limit_does_not_block_real_exploratory_entries() {
        assert!(!shadow_entry_limit_blocks(false, true, Some(900_000), 1_000_000, 1_000, 1_000));
        assert!(shadow_entry_limit_blocks(true, false, None, 1_000_000, 1_000, 1_000));
    }

    #[test]
    fn live_parameter_probe_records_a_signal_rejected_only_by_the_baseline() {
        let mut source = signal("live-trade-flow");
        source.components.trade_flow_by_window = vec![WindowedSignalValue {
            window_seconds: 60,
            value: Some(0.10),
            ..Default::default()
        }];
        let baseline = BTreeMap::from([
            ("minimum_flow_imbalance".into(), 0.15),
            ("trade_flow_window_seconds".into(), 60.0),
        ]);
        let variant = BTreeMap::from([
            ("minimum_flow_imbalance".into(), 0.05),
            ("trade_flow_window_seconds".into(), 60.0),
        ]);
        assert!(!live_parameter_probe_entry_eligible(
            &source, Strategy::LiveTradeFlow, "bullish", &baseline, 100.0,
        ));
        assert!(live_parameter_probe_entry_eligible(
            &source, Strategy::LiveTradeFlow, "bullish", &variant, 100.0,
        ));
    }

    #[test]
    fn selected_live_strategy_is_shadowed_even_when_enabled_for_trading() {
        let mut settings = Settings::default();
        settings.strategies.push(Strategy::LiveMomentum);
        settings.scout_strategies.push(Strategy::LiveMomentum);
        assert_eq!(live_scout_strategies_enabled_for_trading(&settings), vec![Strategy::LiveMomentum]);
    }

    #[test]
    fn daily_enter_candidates_are_shadowed_when_selected_as_scouts() {
        assert!(should_record_daily_scout(Ok(StrategyDecision::Enter), true));
        assert!(!should_record_daily_scout(Ok(StrategyDecision::Enter), false));
        assert!(should_record_daily_scout(Ok(StrategyDecision::Scout), false));
    }

    #[test]
    fn daily_scout_can_be_sampled_again_after_five_minutes() {
        assert!(!daily_scout_sample_cooldown_elapsed(299_999, 0));
        assert!(daily_scout_sample_cooldown_elapsed(300_000, 0));
    }

    #[test]
    fn live_parameter_probe_cooldown_remains_one_hour() {
        assert!(!live_parameter_probe_cooldown_elapsed(3_599_999, 0));
        assert!(live_parameter_probe_cooldown_elapsed(3_600_000, 0));
    }

    #[test]
    fn parameter_probe_audit_preserves_the_exact_value_and_entry_result() {
        let mut probe = signal("live-momentum");
        probe.components.momentum_10s_percent = Some(0.1);
        probe.strategy_decision = Some(serde_json::json!({
            "observed_close": 100.0,
            "direction": "bullish",
            "strategy_parameters": {"minimum_momentum": 0.05},
            "scout_parameter_probe": true,
            "scout_parameter_key": "minimum_momentum",
            "scout_parameter_value": 0.05,
            "scout_parameter_profile_symbol": "ETH-USDT"
        }));
        let payload = scout_shadow_audit_payload(
            &probe, 20_000, 5.0, "scout:probe", 0.5, 0.8, None, 0.0, "strategy",
        );
        assert_eq!(payload["scout_parameter_key"], "minimum_momentum");
        assert_eq!(payload["scout_parameter_value"], 0.05);
        assert_eq!(payload["scout_parameter_profile_symbol"], "ETH-USDT");
        assert_eq!(payload["scout_parameter_entry_eligible"], true);
        assert!(payload["decision_id"].as_str().unwrap().contains("probe:minimum_momentum:0.05000000"));
    }

    #[test]
    fn scout_shadow_sample_has_the_contract_required_for_learning_outcomes() {
        let mut live_signal = signal("live-momentum");
        live_signal.strategy_decision = Some(serde_json::json!({ "observed_close": 123.45 }));
        let payload = scout_shadow_audit_payload(
            &live_signal,
            20_000,
            5.0,
            "scout:long:trend:ETH-USDT:live-momentum:trade",
            0.5,
            0.8,
            None,
            1.0,
            "strategy",
        );

        assert_eq!(payload["stage"], "decision");
        assert_eq!(payload["status"], "shadowed");
        assert_eq!(payload["scout"], true);
        assert_eq!(payload["signal_timestamp"], 20_000);
        assert_eq!(
            payload["shadow_learning_context"],
            "scout:long:trend:ETH-USDT:live-momentum:trade"
        );
        assert_eq!(payload["shadow_notional_usd"], 5.0);
        assert!(payload.get("scout_level_percent").is_none());
        assert_eq!(payload["shadow_entry_price"], 123.45);
        assert_eq!(payload["shadow_exit_scenario"], "strategy");
        let tight_payload = scout_shadow_audit_payload(
            &live_signal,
            20_001,
            5.0,
            "scout:long:trend:ETH-USDT:live-momentum:scenario:tight:trade",
            0.25,
            0.4,
            None,
            1.0,
            "tight",
        );
        assert_ne!(payload["decision_id"], tight_payload["decision_id"]);
        assert_eq!(tight_payload["shadow_exit_scenario"], "tight");
        assert_eq!(tight_payload["stop_loss_percent"], 0.25);
        assert_eq!(tight_payload["take_profit_percent"], 0.4);
        let wide_payload = scout_shadow_audit_payload(
            &live_signal,
            20_002,
            5.0,
            "scout:long:trend:ETH-USDT:live-momentum:scenario:wide:trade",
            0.75,
            1.2,
            None,
            1.0,
            "wide",
        );
        assert_ne!(payload["decision_id"], wide_payload["decision_id"]);
        assert_eq!(wide_payload["shadow_exit_scenario"], "wide");
        assert_eq!(wide_payload["stop_loss_percent"], 0.75);
        assert_eq!(wide_payload["take_profit_percent"], 1.2);
        assert_eq!(
            payload["reason"],
            "Scout shadow sample recorded; no order submitted"
        );
    }

    #[test]
    fn daily_strategy_scout_is_kept_in_the_candidate_batch() {
        let mut scout = signal("trend-50-200");
        scout.strategy_decision = Some(serde_json::json!({
            "observed_close": 90.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "primary_passed": false,
            "btc_gate_required": false,
            "btc_gate_passed": true
        }));

        assert_eq!(
            daily_strategy_decision(&scout),
            Some(StrategyDecision::Scout)
        );
        let candidates = daily_strategy_candidates(&[&scout], &Settings::default());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].strategy.as_deref(), Some("trend-50-200"));
    }

    #[test]
    fn daily_strategy_candidates_outside_donchian_can_become_scouts() {
        let mut trend = signal("trend-50-200");
        trend.strategy_decision = Some(serde_json::json!({
            "observed_close": 90.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "primary_passed": false,
            "btc_gate_required": false,
            "btc_gate_passed": true
        }));

        assert_eq!(
            evaluate_daily_strategy_signal(&trend),
            Ok(StrategyDecision::Scout)
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
