fn scout_parameter_probe_suffix(signal: &Signal) -> String {
    let Some(decision) = signal.strategy_decision.as_ref() else { return String::new(); };
    let (Some(key), Some(value)) = (
        decision.get("scout_parameter_key").and_then(serde_json::Value::as_str),
        decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64),
    ) else { return String::new(); };
    compact_scout_probe_suffix(key, value)
}

fn compact_scout_probe_suffix(key: &str, value: f64) -> String {
    if key.starts_with("combo:") {
        format!("|probe:combo:{}", scouts::probe_id(key))
    } else {
        format!("|probe:{key}:{value:.8}")
    }
}

fn scout_audit_payload(
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
        "scouted",
        Some("Scout scout sample recorded; no order submitted"),
        Some(true),
        stop_loss_percent,
        take_profit_percent,
        evidence,
    );
    // Daily signal timestamps are in seconds; scout audit matching uses the
    // millisecond timestamp shared with outcome evaluations.
    payload["signal_timestamp"] = serde_json::json!(timestamp);
    payload["scout"] = serde_json::json!(true);
    payload["scout_learning_context"] = serde_json::json!(learning_context);
    payload["scout_notional_usd"] = serde_json::json!(notional);
    let profile_parameters = signal_strategy_parameters(signal);
    if let Some(strategy) = signal.strategy.as_deref() {
        payload["learning_profile_id"] = serde_json::json!(strategies::profile_id(
            &signal.symbol, strategy, &profile_parameters,
        ));
    }
    payload["strategy_parameters"] = serde_json::json!(profile_parameters);
    if let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) {
        payload["strategy_parameter_specs"] = serde_json::to_value(scout_parameter_specs(strategy))
            .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    }
    if signal.strategy.as_deref().is_some_and(|strategy| strategy.starts_with("donchian-")) {
        payload["scout_level_percent"] = serde_json::json!(level_percent);
    }
    payload["scout_exit_scenario"] = serde_json::json!(exit_scenario);
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
    let probe_suffix = parameter_probe.map_or_else(String::new, |(key, value)| compact_scout_probe_suffix(key, value));
    let sample_group_id = if sample_group_id.starts_with("live-") {
        format!("live-scout:{sample_group_id}|{}|{}{probe_suffix}", signal.symbol, signal.direction)
    } else {
        format!("daily-scout-level:{level_percent:.2}|{sample_group_id}|{}|{}{probe_suffix}", signal.symbol, signal.direction)
    };
    payload["scout_sample_group_id"] = serde_json::json!(sample_group_id);
    let cooldown_probe = parameter_probe.map_or_else(String::new, |(key, value)| compact_scout_probe_suffix(key, value));
    payload["scout_cooldown_key"] = if signal.strategy.as_deref().is_some_and(|strategy| strategy.starts_with("live-")) {
        serde_json::json!(format!("live|{}|{}|{}{cooldown_probe}", signal.strategy.as_deref().unwrap_or("unknown"), signal.symbol, signal.direction))
    } else {
        serde_json::json!(format!("daily|{}|{}|{}|level:{level_percent:.2}{cooldown_probe}",
            signal.strategy.as_deref().unwrap_or("unknown"), signal.symbol, signal.direction))
    };
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
        payload["scout_entry_price"] = serde_json::json!(entry_price);
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

const DAILY_SCOUT_SCOUT_COOLDOWN_MS: u64 = 300_000;
const MAX_CONCURRENT_SCOUT_VARIANTS_PER_PROFILE: usize = 3;

async fn submit_scout_plans(
    _state: &AutoTraderState,
    source: &str,
    now_ms: u64,
    cooldown_ms: u64,
    max_active_batches: usize,
    audits: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, (StatusCode, String)> {
    if audits.is_empty() { return Ok(Vec::new()); }
    let mut grouped = Vec::<(String, Vec<serde_json::Value>)>::new();
    for audit in audits {
        let Some(group) = audit.get("scout_sample_group_id").and_then(serde_json::Value::as_str) else { continue; };
        if let Some((_, plans)) = grouped.iter_mut().find(|(key, _)| key == group) {
            plans.push(audit);
        } else {
            grouped.push((group.to_owned(), vec![audit]));
        }
    }
    let candidate_fingerprint = grouped.iter().map(|(group, _)| group.as_str()).collect::<Vec<_>>().join("|");
    let candidates = grouped.into_iter().map(|(_group, audit_plans)| {
        let first = audit_plans.first().expect("group has an audit");
        let strategy = first.get("strategy").and_then(serde_json::Value::as_str).unwrap_or("unknown");
        let symbol = first.get("scout_parameter_profile_symbol").and_then(serde_json::Value::as_str)
            .or_else(|| first.get("symbol").and_then(serde_json::Value::as_str)).unwrap_or("unknown");
        let parameter_variant = audit_plans.iter().any(|audit| audit.get("scout_parameter_key").is_some());
        let template_sources = if first.get("scout_comparison_id").is_some() {
            audit_plans.iter().collect::<Vec<_>>()
        } else {
            vec![first]
        };
        let audit_templates = template_sources.into_iter().map(|audit| {
            let parameter_specs = audit.get("strategy_parameter_specs")
                .and_then(|value| serde_json::from_value::<Vec<scouts::ParameterSpec>>(value.clone()).ok())
                .unwrap_or_default();
            scouts::ScoutAuditTemplate {
                notional_usd: audit.get("scout_notional_usd").and_then(serde_json::Value::as_f64).unwrap_or_default(),
                learning_context: audit.get("scout_learning_context").and_then(serde_json::Value::as_str).unwrap_or("scout:unknown:trade").to_owned(),
                stop_loss_percent: audit.get("stop_loss_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.5),
                take_profit_percent: audit.get("take_profit_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.8),
                level_percent: audit.get("scout_level_percent").and_then(serde_json::Value::as_f64).unwrap_or_default(),
                strategy_only: audit.get("scout_comparison_id").is_some(),
                parameter_entry_eligible: audit.get("scout_parameter_entry_eligible").and_then(serde_json::Value::as_bool),
                parameter_specs,
                base_audit: audit.clone(),
            }
        }).collect();
        scouts::ScoutPlanCandidate {
            generate_group_id: true,
            sample_group_id: String::new(),
            cooldown_key: String::new(),
            profile_key: format!("{strategy}|{symbol}"),
            eligible: true,
            parameter_variant,
            cooldown_ms: if parameter_variant { 86_400_000 } else { cooldown_ms },
            audit_plans: Vec::new(),
            audit_templates,
            sample_payload: None,
            daily_policy: None,
            live_policy: None,
        }
    }).collect::<Vec<_>>();
    let request = scouts::ScoutPlanRequest {
        request_id: format!("{source}:{now_ms}:{}", scouts::probe_id(&candidate_fingerprint)),
        now_ms,
        max_active_batches,
        max_active_variants_per_profile: MAX_CONCURRENT_SCOUT_VARIANTS_PER_PROFILE,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates,
    };
    let base = std::env::var("SCOUTS_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    scouts::request_scout_plan(&base, &request).await
        .map(|response| response.audits)
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Scout scout plan request failed: {error}")))
}

async fn record_daily_scout_ladder(
    state: &AutoTraderState,
    candidates: &[Signal],
    now: u64,
) -> Result<(), (StatusCode, String)> {
    if candidates.is_empty() {
        return Ok(());
    }
    let timestamp_ms = now.saturating_mul(1_000);
    let settings = state.settings.lock().await.clone();
    let scout_configuration = state.scout_configuration();
    let mut plans = Vec::new();
    for signal in candidates {
        let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
            continue;
        };
        let parameters = signal_strategy_parameters(signal);
        let daily_features_present = StrategySignal::entry(signal, &parameters).daily_features.is_some();
        let decision = evaluate_daily_strategy_signal(signal);
        let selected_scout = scout_configuration.strategies.iter().any(|name| name == strategy.as_str());
        let parameter_probe = signal.strategy_decision.as_ref().and_then(|decision| {
            Some((decision.get("scout_parameter_key")?.as_str()?, decision.get("scout_parameter_value")?.as_f64()?))
        });
        if let (Some((parameter_key, parameter_value)), Some(baseline_decision)) = (
            parameter_probe,
            signal.strategy_decision.as_ref().and_then(|decision| decision.get("scout_parameter_baseline_decision")),
        ) {
            let mut baseline_signal = signal.clone();
            baseline_signal.strategy_decision = Some(baseline_decision.clone());
            let baseline_enters = matches!(evaluate_daily_strategy_signal(&baseline_signal), Ok(StrategyDecision::Enter | StrategyDecision::Scout));
            let variant_enters = matches!(decision, Ok(StrategyDecision::Enter | StrategyDecision::Scout));
            if !baseline_enters && !variant_enters { continue; }
            let comparison_id = format!("daily-parameter:{}:{}:{}:{}", strategy.as_str(), signal.symbol, signal.timestamp, scouts::probe_id(&format!("{parameter_key}:{parameter_value:.8}")));
            let mut templates = Vec::new();
            for (selected, role, entry_eligible) in [
                (baseline_enters.then_some(&baseline_signal), "baseline", baseline_enters),
                (variant_enters.then_some(signal), "variant", variant_enters),
            ] {
                let Some(selected) = selected else { continue; };
                let selected_parameters = signal_strategy_parameters(selected);
                let selected_decision = selected.strategy_decision.as_ref();
                let stop = selected_parameters.get("stop_loss_percent").copied().unwrap_or(settings.stop_loss_percent);
                let target = selected_parameters.get("take_profit_percent").copied().unwrap_or(settings.take_profit_percent);
                let mut decision = selected_decision.cloned().unwrap_or_default();
                decision["scout_comparison_id"] = serde_json::json!(comparison_id);
                decision["scout_comparison_role"] = serde_json::json!(role);
                let mut compared = selected.clone();
                compared.strategy_decision = Some(decision);
                let mut base_audit = signal_audit_payload(&compared, "decision", timestamp_ms, "scouted", Some("Scout parameter comparison sample recorded"), Some(true), stop, target, None);
                base_audit["scout_comparison_id"] = serde_json::json!(comparison_id);
                base_audit["scout_comparison_role"] = serde_json::json!(role);
                base_audit["scout_comparison_parameter_key"] = serde_json::json!(parameter_key);
                base_audit["scout_comparison_parameter_value"] = serde_json::json!(parameter_value);
                base_audit["scout_comparison_parameter_profile_symbol"] = serde_json::json!(signal.symbol);
                if (!baseline_enters || !variant_enters) && role == if !baseline_enters { "variant" } else { "baseline" } {
                    base_audit["scout_comparison_zero_role"] = serde_json::json!(if !baseline_enters { "baseline" } else { "variant" });
                }
                let context = market_context(&compared);
                templates.push(scouts::ScoutAuditTemplate {
                    base_audit,
                    notional_usd: settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION,
                    learning_context: format!("scout:{context}:level:0.00:scenario:strategy:{role}"),
                    stop_loss_percent: stop,
                    take_profit_percent: target,
                    level_percent: selected_parameters.get("breakout_entry_distance_percent").copied().unwrap_or_default(),
                    strategy_only: true,
                    parameter_entry_eligible: Some(entry_eligible),
                    parameter_specs: scout_parameter_specs(strategy),
                });
            }
            plans.push(scouts::ScoutPlanCandidate {
                generate_group_id: true,
                sample_group_id: String::new(), cooldown_key: String::new(),
                profile_key: format!("{}|{}", strategy.as_str(), signal.symbol), eligible: true,
                parameter_variant: true, cooldown_ms: 86_400_000, audit_plans: Vec::new(),
                audit_templates: templates, sample_payload: None, daily_policy: None, live_policy: None,
            });
            continue;
        }

        let strategy_size_multiplier = signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("strategy_order_size_multiplier"))
            .and_then(serde_json::Value::as_f64).filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0).clamp(0.05, 1.0);
        let strategy_parameters = signal.strategy_decision.as_ref().and_then(|decision| decision.get("strategy_parameters"));
        let stop = strategy_parameters.and_then(|parameters| parameters.get("stop_loss_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.stop_loss_percent);
        let target = strategy_parameters.and_then(|parameters| parameters.get("take_profit_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.take_profit_percent);
        let base_audit = signal_audit_payload(signal, "decision", timestamp_ms, "scouted", Some("Scout scout sample recorded; no order submitted"), Some(true), stop, target, None);
        let parameter_entry_eligible = parameter_probe.map(|_| matches!(decision, Ok(StrategyDecision::Enter | StrategyDecision::Scout)));
        let template = scouts::ScoutAuditTemplate {
            base_audit,
            notional_usd: settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION * strategy_size_multiplier,
            learning_context: format!("scout:{}:level:0.00:scenario", market_context(signal)),
            stop_loss_percent: stop,
            take_profit_percent: target,
            level_percent: 0.0,
            strategy_only: false,
            parameter_entry_eligible,
            parameter_specs: scout_parameter_specs(strategy),
        };
        plans.push(scouts::ScoutPlanCandidate {
            generate_group_id: true,
            sample_group_id: String::new(), cooldown_key: String::new(),
            profile_key: format!("{}|{}", strategy.as_str(), signal.symbol), eligible: true,
            parameter_variant: parameter_probe.is_some(), cooldown_ms: if parameter_probe.is_some() { 86_400_000 } else { DAILY_SCOUT_SCOUT_COOLDOWN_MS },
            audit_plans: Vec::new(), audit_templates: vec![template], sample_payload: None,
            daily_policy: Some(scouts::DailyScoutPolicy {
                evaluator_decision: match decision {
                    Ok(StrategyDecision::Enter) => "enter",
                    Ok(StrategyDecision::Scout) => "scout",
                    _ => "reject",
                }.to_owned(),
                selected_scout,
                strategy_candidate: signal.strategy_candidate,
                daily_features_present,
                independent_non_donchian_strategy: is_independent_daily_strategy(strategy.as_str())
                    && !strategy.as_str().starts_with("donchian-"),
                parameter_key: parameter_probe.map(|(key, _)| key.to_owned()),
                parameter_value: parameter_probe.map(|(_, value)| value),
            }),
            live_policy: None,
        });
    }
    let max_active_batches = scout_configuration.max_active_samples;
    drop(settings);
    if plans.is_empty() { return Ok(()); }
    let request_key = plans.iter().flat_map(|candidate| candidate.audit_templates.iter())
        .filter_map(|template| template.base_audit.get("decision_id").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>().join("|");
    let request = scouts::ScoutPlanRequest {
        request_id: format!("daily:{timestamp_ms}:{}", scouts::probe_id(&request_key)),
        now_ms: timestamp_ms,
        max_active_batches,
        max_active_variants_per_profile: MAX_CONCURRENT_SCOUT_VARIANTS_PER_PROFILE,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates: plans,
    };
    let scouts_url = std::env::var("SCOUTS_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    let response = scouts::request_scout_plan(&scouts_url, &request).await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Scout daily scout plan failed: {error}")))?;
    for audit in response.audits { queue_auto_trader_audit(state, audit); }
    Ok(())
}

async fn record_live_strategy_scouts(
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
    let settings = state.settings.lock().await.clone();
    let scout_configuration = state.scout_configuration();
    let mut plans = Vec::new();
    for candidate in candidates {
        let Some(strategy) = candidate.strategy.as_deref().and_then(Strategy::from_name) else { continue; };
        let params = candidate.strategy_decision.as_ref().and_then(|decision| decision.get("strategy_parameters"));
        let stop_loss = params.and_then(|values| values.get("stop_loss_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.stop_loss_percent);
        let take_profit = params.and_then(|values| values.get("take_profit_percent"))
            .and_then(serde_json::Value::as_f64).unwrap_or(settings.take_profit_percent);
        let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
        let base_audit = signal_audit_payload(
            candidate, "decision", timestamp_ms, "scouted",
            Some("Scout scout sample recorded; no order submitted"), Some(true), stop_loss, take_profit, None,
        );
        let market = fresh_markets.get(&candidate.symbol);
        let book_updated_at_ms = market.and_then(|value| value.get("updated_at_ms")).and_then(serde_json::Value::as_u64);
        let best_bid = market.and_then(|value| value.get("bids")).and_then(|value| value.get(0))
            .and_then(|level| level.get("price")).and_then(serde_json::Value::as_f64);
        let best_ask = market.and_then(|value| value.get("asks")).and_then(|value| value.get(0))
            .and_then(|level| level.get("price")).and_then(serde_json::Value::as_f64);
        let context = market_context(candidate);
        plans.push(scouts::ScoutPlanCandidate {
            generate_group_id: true,
            sample_group_id: String::new(), cooldown_key: String::new(),
            profile_key: format!("{}|{}", strategy.as_str(), candidate.symbol),
            eligible: true,
            parameter_variant: false,
            cooldown_ms: 300_000,
            audit_plans: Vec::new(),
            audit_templates: vec![scouts::ScoutAuditTemplate {
                base_audit,
                notional_usd: notional,
                learning_context: format!("scout:{context}:trade:scenario"),
                stop_loss_percent: stop_loss,
                take_profit_percent: take_profit,
                level_percent: 0.0,
                strategy_only: false,
                parameter_entry_eligible: None,
                parameter_specs: scout_parameter_specs(strategy),
            }],
            sample_payload: None,
            daily_policy: None,
            live_policy: Some(scouts::LiveScoutPolicy {
                now_ms: timestamp_ms,
                strategy_enabled: scout_configuration.strategies.iter().any(|name| name == strategy.as_str()) && settings.strategies.contains(&strategy),
                book_updated_at_ms,
                best_bid,
                best_ask,
            }),
        });
    }
    let max_active_batches = scout_configuration.max_active_samples;
    let fingerprint = plans.iter().flat_map(|plan| plan.audit_templates.iter())
        .filter_map(|template| template.base_audit.get("decision_id").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>().join("|");
    let request = scouts::ScoutPlanRequest {
        request_id: format!("live:{timestamp_ms}:{}", scouts::probe_id(&fingerprint)),
        now_ms: timestamp_ms,
        max_active_batches,
        max_active_variants_per_profile: MAX_CONCURRENT_SCOUT_VARIANTS_PER_PROFILE,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates: plans,
    };
    let base = std::env::var("SCOUTS_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    let audits = scouts::request_scout_plan(&base, &request).await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Scout live scout plan failed: {error}")))?.audits;
    for audit in audits {
        queue_auto_trader_audit(state, audit);
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
    if !strategy.is_live_insight() || strategy == Strategy::LiveInsights {
        return Ok(());
    }
    let scouts_url = std::env::var("SCOUTS_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    let variants = match scouts::request_parameter_variants(
        &scouts_url,
        &scouts::ParameterVariantsRequest {
            parameters: baseline_parameters.clone(),
            specs: scout_parameter_specs(strategy),
            day: now / 86_400,
        },
    )
    .await
    {
        Ok(Some(variants)) => variants,
        Ok(None) => Vec::new(),
        Err(error) => {
            crate::service_log!("WARN", "Scout service unavailable for live parameter sweep: {error}");
            Vec::new()
        }
    };
    if variants.is_empty() {
        return Ok(());
    }
    let sweep_key = format!(
        "live-scout-parameter:{}:{}:{}:{}:{}",
        source_signal.symbol,
        strategy.as_str(),
        direction,
        now / 86_400,
        strategies::profile_id(source_signal.symbol.as_str(), strategy.as_str(), baseline_parameters),
    );
    let sampled_at = now.saturating_mul(1_000);
    let mut audits = Vec::new();
    for variant in variants {
        let parameter_key = variant.key;
        let parameter_value = variant.value;
        let changes = variant.changes;
        let variant_parameters = baseline_parameters
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .chain(changes.iter().map(|(key, value)| (key.clone(), *value)))
            .collect::<BTreeMap<_, _>>();
        match record_live_parameter_variant(
            state,
            source_signal,
            strategy,
            direction,
            baseline_parameters,
            &variant_parameters,
            &parameter_key,
            parameter_value,
            entry_price,
            now,
            &sweep_key,
        )
        .await {
            Ok(mut variant_audits) => audits.append(&mut variant_audits),
            Err(error) => crate::service_log!("WARN", "Could not record live Scout parameter variant: {}", error.1),
        }
    }
    let max_active_batches = state.scout_configuration().max_active_samples;
    let audits = submit_scout_plans(state, "live-parameter", sampled_at, 86_400_000, max_active_batches, audits).await?;
    for audit in audits {
        queue_auto_trader_audit(state, audit);
    }
    Ok(())
}

async fn record_live_parameter_variant(
    state: &AutoTraderState,
    source_signal: &Signal,
    strategy: Strategy,
    direction: &str,
    baseline_parameters: &BTreeMap<String, f64>,
    variant_parameters: &BTreeMap<String, f64>,
    parameter_key: &str,
    parameter_value: f64,
    entry_price: f64,
    now: u64,
    sweep_key: &str,
) -> Result<Vec<serde_json::Value>, (StatusCode, String)> {
    if !strategy.is_live_insight() || strategy == Strategy::LiveInsights { return Ok(Vec::new()); }
    let sample_group_id = format!("{sweep_key}|probe:{}", scouts::probe_id(parameter_key));
    let sampled_at = now.saturating_mul(1_000);
    let Some(mut baseline_signal) = parameter_probe_signal(
        source_signal, strategy, direction, baseline_parameters, entry_price,
    ) else {
        return Ok(Vec::new());
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
        return Ok(Vec::new());
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
    let mut audits = Vec::new();
    if baseline_entry_eligible {
        let mut decision = baseline_signal.strategy_decision.clone().unwrap_or_else(|| serde_json::json!({}));
        decision["scout_comparison_id"] = serde_json::json!(comparison_id);
        decision["scout_comparison_role"] = serde_json::json!("baseline");
        baseline_signal.strategy_decision = Some(decision);
        let mut audit = scout_audit_payload(
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
        audit["scout_sample_group_id"] = serde_json::json!(sample_group_id);
        audit["scout_cooldown_key"] = serde_json::json!(format!("live-parameter|{}|{}|{}|{}",
            source_signal.symbol, strategy.as_str(), direction, compact_scout_probe_suffix(&parameter_key, parameter_value)));
        audit["scout_comparison_id"] = serde_json::json!(comparison_id);
        audit["scout_comparison_role"] = serde_json::json!("baseline");
        audit["scout_parameter_entry_eligible"] = serde_json::json!(baseline_entry_eligible);
        audit["scout_comparison_parameter_key"] = serde_json::json!(parameter_key);
        audit["scout_comparison_parameter_value"] = serde_json::json!(parameter_value);
        audit["scout_comparison_parameter_profile_symbol"] = serde_json::json!(source_signal.symbol);
        if !variant_entry_eligible {
            audit["scout_comparison_zero_role"] = serde_json::json!("variant");
        }
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
        let mut audit = scout_audit_payload(
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
        audit["scout_sample_group_id"] = serde_json::json!(sample_group_id);
        audit["scout_cooldown_key"] = serde_json::json!(format!("live-parameter|{}|{}|{}|{}",
            source_signal.symbol, strategy.as_str(), direction, compact_scout_probe_suffix(&parameter_key, parameter_value)));
        audit["scout_comparison_id"] = serde_json::json!(comparison_id);
        audit["scout_comparison_role"] = serde_json::json!("variant");
        audit["scout_parameter_entry_eligible"] = serde_json::json!(variant_entry_eligible);
        audit["scout_comparison_parameter_key"] = serde_json::json!(parameter_key);
        audit["scout_comparison_parameter_value"] = serde_json::json!(parameter_value);
        audit["scout_comparison_parameter_profile_symbol"] = serde_json::json!(source_signal.symbol);
        if !baseline_entry_eligible {
            audit["scout_comparison_zero_role"] = serde_json::json!("baseline");
        }
        let variant_decision_id = audit["decision_id"].as_str().unwrap_or("scout").to_owned();
        audit["decision_id"] = serde_json::json!(format!("{variant_decision_id}|comparison:variant"));
        audit["trade_id"] = audit["decision_id"].clone();
        audits.push(audit);
    }
    Ok(audits)
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
    if should_scout_daily_scout(scout, signal) {
        return Ok(CycleResult {
            status: "scouted",
            reason: Some("Daily scout candidate is scout-only; no order submitted".into()),
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
    let mut scout_sample = None;
    if capped_probe {
        let settings = state.settings.lock().await;
        let scout_configuration = state.scout_configuration();
        if scout {
            let notional = settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION;
            let context = market_context(signal);
            let sample_group_id = format!("live-scout:{}|{}|{}", signal.strategy.as_deref().unwrap_or("live-insights"), signal.symbol, signal.direction);
            scout_sample = Some((notional, format!("scout:{context}:trade"), sample_group_id));
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
            if open_scout_positions + pending_scout_orders >= scout_configuration.max_active_positions {
                return Ok(skipped_for_signal(
                    signal,
                    "Maximum number of active and pending scout positions is already in use",
                ));
            }
        }
        drop(settings);
    }
    if let Some((notional, learning_context, sample_group_id)) = scout_sample {
        let sampled_at = now.saturating_mul(1_000);
        let mut plans = Vec::new();
        let mut scout_payload = scout_audit_payload(
            signal, sampled_at, notional, &format!("{learning_context}:scenario"),
            counterfactual_stop_loss_percent, counterfactual_take_profit_percent, evidence, 0.0, "strategy",
        );
        scout_payload["scout_sample_group_id"] = serde_json::json!(sample_group_id);
        scout_payload["scout_cooldown_key"] = serde_json::json!(format!("live|{}|{}|{}",
            signal.strategy.as_deref().unwrap_or("live-insights"), signal.symbol, signal.direction));
        plans.push(scout_payload);
        let max_active_batches = state.scout_configuration().max_active_samples;
        let planned = submit_scout_plans(state, "live-entry", sampled_at, 300_000, max_active_batches, plans).await?;
        if planned.is_empty() {
            return Ok(skipped_for_signal(signal, "The capped exploration-entry limit is already in use"));
        }
        for audit in planned { queue_auto_trader_audit(state, audit); }
        return Ok(CycleResult {
            status: "scouted",
            reason: Some("Scout scout sample recorded; no order submitted".into()),
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
    let parameters = signal_strategy_parameters(signal);
    let input = StrategySignal::entry(signal, &parameters);
    strategy.evaluate(&input)
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
    let multiplier = settings.learned_strategy_size_multipliers
        .get(strategy.as_str()).copied().unwrap_or(1.0);
    (decision_priority, multiplier - 1.0, 0.0)
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

async fn request_scout_entry_selection(
    candidates: &[&Signal],
    dashboard: &Dashboard,
    active_scout_count: usize,
    scout_max_active_positions: usize,
    risk_appetite: f64,
) -> Result<Option<usize>, String> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut strategy_parameter_specs = BTreeMap::new();
    for strategy in candidates.iter().filter_map(|signal| {
        signal.strategy.as_deref().and_then(Strategy::from_name)
    }) {
        strategy_parameter_specs.entry(strategy.as_str().to_owned())
            .or_insert_with(|| scout_parameter_specs(strategy));
    }
    let request = scouts::ScoutEntrySelectionRequest {
        candidates: candidates.iter().filter_map(|signal| serde_json::to_value(signal).ok()).collect(),
        walk_forward: serde_json::to_value(dashboard.walk_forward.as_ref())
            .ok().and_then(|value| value.as_array().cloned()).unwrap_or_default(),
        strategy_parameter_specs,
        active_scout_count,
        scout_max_active_positions,
        risk_appetite,
    };
    let base = std::env::var("SCOUTS_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3011".into());
    scouts::request_scout_entry_selection(&base, &request).await
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
    let scout_configuration = state.scout_configuration();
    let mut live_source_signals = live_strategy_source_signals(&dashboard.best);
    for signal in &mut live_source_signals {
        inject_learned_parameters(&strategy_settings, signal);
    }
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
    let mut daily_scout_candidates = Vec::new();
    if has_daily_strategy_signals {
        dashboard.best.retain(|signal| {
            if !signal.strategy_candidate {
                return false;
            }
            let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
                return false;
            };
            if !strategy_settings.strategies.contains(&strategy) {
                if scout_configuration.strategies.iter().any(|name| name == strategy.as_str()) {
                    let is_parameter_probe = signal.strategy_decision.as_ref()
                        .and_then(|decision| decision.get("scout_parameter_probe"))
                        .and_then(serde_json::Value::as_bool) == Some(true);
                    match evaluate_daily_strategy_signal(signal) {
                        Ok(_) => {
                            daily_scout_candidates.push(signal.clone());
                        }
                        Err(_) if is_parameter_probe => daily_scout_candidates.push(signal.clone()),
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
                    daily_scout_candidates.push(signal.clone());
                    true
                }
                Ok(StrategyDecision::Scout) => {
                    daily_scout_candidates.push(signal.clone());
                    true
                }
                Ok(_) if is_parameter_probe => {
                    daily_scout_candidates.push(signal.clone());
                    false
                }
                Ok(decision) => {
                    daily_scout_candidates.push(signal.clone());
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
                    if is_parameter_probe {
                        daily_scout_candidates.push(signal.clone());
                    }
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
        live_source_signals
            .iter()
            .flat_map(|source_signal| {
                let strategy_settings = &strategy_settings;
                live_strategies.iter().copied().filter_map(move |strategy| {
                    let parameters = strategy_parameter_values(
                        strategy_settings,
                        &source_signal.symbol,
                        strategy,
                    );
                    let mut signal = live_strategy_feature_signal(source_signal, strategy, &parameters)?;
                    let input = StrategySignal::entry(&signal, &parameters);
                    if !matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter)) {
                        return None;
                    }
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
                    Some(signal)
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    // Scout-only live strategy candidates are generated and evaluated by the
    // Scout process. AutoTrader sends its signal/profile snapshot and continues
    // immediately with real strategies and order decisions.
    let scout_source_signals = live_source_signals.clone();
    let scout_symbols = scout_source_signals.iter()
        .map(|signal| signal.symbol.clone())
        .collect::<BTreeSet<_>>();
    let mut scout_profiles = Vec::new();
    for strategy in scout_configuration.strategies.iter().filter_map(|name| Strategy::from_name(name)).filter(|strategy| {
        strategy.is_live_insight()
            && *strategy != Strategy::LiveInsights
            && !strategy_settings.strategies.contains(strategy)
    }) {
        for symbol in &scout_symbols {
            scout_profiles.push(scouts::LiveScoutProfile {
                symbol: symbol.clone(),
                strategy: strategy.as_str().to_owned(),
                parameters: strategy_parameter_values(&strategy_settings, symbol, strategy),
            });
        }
    }
    if !scout_profiles.is_empty() && !scout_source_signals.is_empty() {
        let scout_state = state.clone();
        let scout_position_url = position_url.clone();
        let markets = dashboard.markets.clone();
        tokio::spawn(async move {
            let scouts_url = std::env::var("SCOUTS_SERVICE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3011".into());
            let request = scouts::LiveScoutCandidatesRequest {
                signals: scout_source_signals.iter()
                    .filter_map(|signal| serde_json::to_value(signal).ok())
                    .collect(),
                profiles: scout_profiles.clone(),
            };
            let values = match scouts::request_live_candidates(&scouts_url, &request).await {
                Ok(values) => values,
                Err(error) => {
                    crate::service_log!("WARN", "Scout service unavailable for live candidate generation: {error}");
                    return;
                }
            };
            let candidates = values.into_iter()
                .filter_map(|value| serde_json::from_value::<Signal>(value).ok())
                .collect::<Vec<_>>();
            if candidates.is_empty() { return; }
            let profiles_by_strategy = scout_profiles.iter()
                .map(|profile| ((profile.strategy.clone(), profile.symbol.clone()), profile.parameters.clone()))
                .collect::<BTreeMap<_, _>>();
            let probes = candidates.iter().filter_map(|candidate| {
                let strategy = Strategy::from_name(candidate.strategy.as_deref()?)?;
                Some(LiveScoutParameterProbe {
                    source_signal: scout_source_signals.iter()
                        .find(|signal| signal.symbol == candidate.symbol)?.clone(),
                    strategy,
                    direction: candidate.direction.clone(),
                    baseline_parameters: profiles_by_strategy
                        .get(&(strategy.as_str().to_owned(), candidate.symbol.clone()))?.clone(),
                })
            }).collect::<Vec<_>>();
            if !probes.is_empty() {
                if let Err((status, error)) = record_live_strategy_parameter_probes(
                    &scout_state,
                    &scout_state.client,
                    &scout_position_url,
                    &probes,
                    &markets,
                    now,
                ).await {
                    crate::service_log!("WARN", "Could not record Scout parameter comparisons ({status}): {error}");
                }
            }
            if let Err((status, error)) = record_live_strategy_scouts(
                &scout_state,
                &scout_state.client,
                &scout_position_url,
                &candidates,
                &markets,
                now,
            ).await {
                crate::service_log!("WARN", "Could not record live Scout scouts ({status}): {error}");
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
    if !daily_scout_candidates.is_empty() {
        let scout_state = state.clone();
        tokio::spawn(async move {
            if let Err((status, error)) = record_daily_scout_ladder(
                &scout_state,
                &daily_scout_candidates,
                now,
            )
            .await
            {
                crate::service_log!(
                    "WARN",
                    "Could not record daily Scout scout ladder ({status}): {error}"
                );
            }
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
    let learning_thresholds = request_learning_context_thresholds(
        &client,
        state,
        &signals,
        &dashboard.missed_opportunity_bands,
    )
    .await;
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
            state.scout_configuration().max_active_positions,
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
    let scout_candidates = selection_fresh_directional.iter().copied()
        .chain(selection_neutral_scout_candidates.iter())
        .collect::<Vec<_>>();
    let selection_result = select_cycle_signal(SignalSelectionInput {
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
        learning_thresholds: &learning_thresholds,
        delegate_scout_fallback: true,
        risk_appetite,
        active_scout_count,
        scout_max_active_positions,
        open_positions: &open_positions,
        max_entries_per_auto_position,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    });
    let (signal, exploratory, mut scout) = match selection_result {
        Ok((signal, exploratory, scout)) => (signal.clone(), exploratory, scout),
        Err(result) => match request_scout_entry_selection(
            &scout_candidates,
            &dashboard,
            active_scout_count,
            scout_max_active_positions,
            risk_appetite,
        ).await {
            Ok(Some(index)) => match scout_candidates.get(index) {
                Some(signal) => ((*signal).clone(), false, true),
                None => return Ok(daily_batch_result.unwrap_or(result)),
            },
            Ok(None) => return Ok(daily_batch_result.unwrap_or(result)),
            Err(error) => {
                crate::service_log!("WARN", "Scout service unavailable for entry selection: {error}");
                return Ok(daily_batch_result.unwrap_or(result));
            }
        },
    };
    scout |= signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("scout_only"))
        .and_then(serde_json::Value::as_bool) == Some(true);
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let mut audited_signal = signal.clone();
    audited_signal.entry_gate = Some(candidate_entry_audit_value(
        &signal,
        &dashboard,
        &learning_thresholds,
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
        let payload = scout_audit_payload(
            &probe, 20_000, 5.0, "scout:probe", 0.5, 0.8, None, 0.0, "strategy",
        );
        assert_eq!(payload["scout_parameter_key"], "minimum_momentum");
        assert_eq!(payload["scout_parameter_value"], 0.05);
        assert_eq!(payload["scout_parameter_profile_symbol"], "ETH-USDT");
        assert_eq!(payload["scout_parameter_entry_eligible"], true);
        assert!(payload["decision_id"].as_str().unwrap().contains("probe:minimum_momentum:0.05000000"));
    }

    #[test]
    fn scout_sample_has_the_contract_required_for_learning_outcomes() {
        let mut live_signal = signal("live-momentum");
        live_signal.strategy_decision = Some(serde_json::json!({ "observed_close": 123.45 }));
        let payload = scout_audit_payload(
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
        assert_eq!(payload["status"], "scouted");
        assert_eq!(payload["scout"], true);
        assert_eq!(payload["signal_timestamp"], 20_000);
        assert_eq!(
            payload["scout_learning_context"],
            "scout:long:trend:ETH-USDT:live-momentum:trade"
        );
        assert_eq!(payload["scout_notional_usd"], 5.0);
        assert!(payload.get("scout_level_percent").is_none());
        assert_eq!(payload["scout_entry_price"], 123.45);
        assert_eq!(payload["scout_exit_scenario"], "strategy");
        let tight_payload = scout_audit_payload(
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
        assert_eq!(tight_payload["scout_exit_scenario"], "tight");
        assert_eq!(tight_payload["stop_loss_percent"], 0.25);
        assert_eq!(tight_payload["take_profit_percent"], 0.4);
        let wide_payload = scout_audit_payload(
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
        assert_eq!(wide_payload["scout_exit_scenario"], "wide");
        assert_eq!(wide_payload["stop_loss_percent"], 0.75);
        assert_eq!(wide_payload["take_profit_percent"], 1.2);
        assert_eq!(
            payload["reason"],
            "Scout scout sample recorded; no order submitted"
        );
    }

    #[test]
    fn daily_strategy_reject_remains_a_reject_for_trade_decisions() {
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
            Some(StrategyDecision::Reject)
        );
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
            Ok(StrategyDecision::Reject)
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
