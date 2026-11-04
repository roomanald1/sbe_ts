const LEARNING_HALF_LIFE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn utc_day_start_ms() -> u64 {
    current_time_ms() / 86_400_000 * 86_400_000
}

fn decay_learning_stats(stats: &mut LearningStats, now_ms: u64) {
    if stats.updated_at_ms == 0 {
        // Legacy persisted profiles have no timestamp; begin their decay clock
        // when first read after this version is deployed.
        stats.updated_at_ms = now_ms;
        return;
    }
    let elapsed_ms = now_ms.saturating_sub(stats.updated_at_ms);
    if elapsed_ms == 0 {
        return;
    }
    let weight = 0.5_f64.powf(elapsed_ms as f64 / LEARNING_HALF_LIFE_MS as f64);
    stats.samples = (stats.samples as f64 * weight).round() as usize;
    stats.positive = ((stats.positive as f64 * weight).round() as usize).min(stats.samples);
    stats.realized_pnl *= weight;
    stats.updated_at_ms = now_ms;
}

fn decay_learning_map(learning: &mut BTreeMap<String, LearningStats>, now_ms: u64) {
    for stats in learning.values_mut() {
        decay_learning_stats(stats, now_ms);
    }
}

fn record_closed_trade_outcome(
    settings: &mut Settings,
    return_percent: f64,
    realized_usd: f64,
    fees_usd: f64,
) {
    // Closed spot trades cannot lose more than their entry notional. Reject
    // malformed returns before they enter the persisted rolling statistics.
    if !return_percent.is_finite()
        || return_percent < -100.0
        || !realized_usd.is_finite()
        || !fees_usd.is_finite()
    {
        return;
    }
    // These parallel arrays are a persisted legacy format. Discard an already
    // misaligned window rather than attaching new money values to old returns.
    let count = settings.closed_trade_returns_percent.len();
    if settings.closed_trade_timestamps_ms.len() != count
        || settings.closed_trade_realized_usd.len() != count
        || settings.closed_trade_fees_usd.len() != count
    {
        settings.closed_trade_returns_percent.clear();
        settings.closed_trade_timestamps_ms.clear();
        settings.closed_trade_realized_usd.clear();
        settings.closed_trade_fees_usd.clear();
    }
    settings.closed_trade_returns_percent.push(return_percent);
    settings.closed_trade_timestamps_ms.push(current_time_ms());
    settings.closed_trade_realized_usd.push(realized_usd);
    settings.closed_trade_fees_usd.push(fees_usd);
    let excess = settings
        .closed_trade_returns_percent
        .len()
        .saturating_sub(settings.closed_trade_window);
    if excess > 0 {
        settings.closed_trade_returns_percent.drain(0..excess);
        settings.closed_trade_timestamps_ms.drain(0..excess);
        settings.closed_trade_realized_usd.drain(0..excess);
        settings.closed_trade_fees_usd.drain(0..excess);
    }
}

fn strategy_trade_learning_key(symbol: &str, strategy: &str) -> String {
    format!("trade:{symbol}:{strategy}")
}

fn scout_context_learning_key(learning_context: &str) -> Option<String> {
    let context = learning_context.strip_prefix("scout:")?;
    let context = context.split(":scenario:").next()?;
    let context = context.strip_suffix(":trade")?;
    (!context.is_empty()).then(|| format!("scout:{context}:trade"))
}

fn strategy_shadow_learning_key(symbol: &str, strategy: &str, scenario: &str) -> String {
    format!("shadow:{symbol}:{strategy}:{scenario}")
}

fn strategy_parameter_probe_learning_key(symbol: &str, strategy: &str, parameter: &str, value: f64) -> String {
    format!("probe:{symbol}:{strategy}:{parameter}:{value:.8}")
}

fn strategy_parameter_shadow_combination_learning_key(
    symbol: &str, strategy: &str, combination: &str,
) -> String {
    format!("probe_combo_shadow:{symbol}:{strategy}:{combination}")
}

fn strategy_parameter_shadow_probe_learning_key(
    symbol: &str,
    strategy: &str,
    parameter: &str,
    value: f64,
) -> String {
    format!("probe_shadow:{symbol}:{strategy}:{parameter}:{value:.8}")
}

fn record_strategy_trade_outcome(
    settings: &mut Settings,
    symbol: &str,
    strategy: Option<&str>,
    learning_context: Option<&str>,
    _scout: bool,
    realized_pnl_usd: f64,
    entry_decision: Option<&serde_json::Value>,
    trade_id: Option<&str>,
) {
    if !realized_pnl_usd.is_finite() {
        return;
    }
    // Scout fills are deliberately small, but still provide strategy-specific
    // evidence for tuning the profile that produced them.
    if let Some(strategy) = strategy {
        let profile_symbol = entry_decision
            .and_then(|decision| decision.get("scout_parameter_profile_symbol"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(symbol);
        let mut parameters = entry_decision
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
            .collect::<BTreeMap<_, _>>();
        if let Some(kind) = Strategy::from_name(strategy) {
            let specs = kind.parameter_specs();
            if specs.iter().any(|spec| !parameters.contains_key(spec.key.as_str())) {
                let signal_parameters = parameters.clone();
                parameters = strategy_parameter_profile(settings, profile_symbol, kind)
                    .parameters.into_iter().map(|parameter| (parameter.key, parameter.value))
                    .collect();
                parameters.extend(signal_parameters);
                if let Some(decision) = entry_decision {
                    if let (Some(key), Some(value)) = (
                        decision.get("scout_parameter_key").and_then(serde_json::Value::as_str),
                        decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64),
                    ) {
                        if !key.starts_with("combo:") { parameters.insert(key.to_owned(), value); }
                        else if let Some(variant) = parse_scout_combination(key, kind) { parameters.extend(variant); }
                    }
                }
            }
        }
        let parameter_specs = Strategy::from_name(strategy)
            .map(Strategy::parameter_specs)
            .unwrap_or_default()
            .into_iter()
            .map(|spec| strategy_core::ParameterSpec {
                key: spec.key.as_str().to_owned(),
                label: spec.label.to_owned(),
                default: spec.default,
                min: spec.min,
                max: spec.max,
                step: spec.step,
                unit: spec.unit.to_owned(),
                learning_direction: spec.learning_direction,
            })
            .filter(|spec| parameters.contains_key(&spec.key))
            .collect::<Vec<_>>();
        let profile_id = strategy_core::profile_id(profile_symbol, strategy, &parameters);
        let event_id = format!("trade:{}", trade_id.filter(|value| !value.is_empty()).unwrap_or(learning_context.unwrap_or(symbol)));
        if !settings.feedback_outbox.iter().any(|event| matches!(event,
            strategy_core::FeedbackEvent::TradeOutcome { event_id: pending, .. } if pending == &event_id
        )) {
            settings.feedback_outbox.push(strategy_core::FeedbackEvent::TradeOutcome {
                event_id,
                observed_at_ms: current_time_ms(),
                symbol: profile_symbol.to_owned(),
                strategy: strategy.to_owned(),
                profile_id,
                parameters,
                specs: parameter_specs,
                scout: _scout,
                realized_pnl_usd,
                net_return_percent: entry_decision
                    .and_then(|decision| decision.get("initial_entry_notional_usd"))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|notional| *notional > 0.0)
                    .map(|notional| realized_pnl_usd / notional * 100.0)
                    .unwrap_or(0.0),
            });
        }
    }
}

#[cfg(test)]
mod feedback_outbox_tests {
    use super::*;

    #[test]
    fn completed_trade_is_queued_for_external_learning_with_its_exact_profile() {
        let mut settings = Settings::default();
        let decision = serde_json::json!({
            "strategy_parameters": { "minimum_momentum": 0.003 },
            "learning_profile_id": "ignored-stale-id"
        });
        record_strategy_trade_outcome(
            &mut settings, "BTC-USDT", Some("live-momentum"), Some("scout:btc"), false,
            1.25, Some(&decision), Some("trade-42"),
        );
        assert!(settings.learning.is_empty(), "the trader must not update learner state inline");
        let Some(FeedbackEvent::TradeOutcome {
            event_id, profile_id, parameters, realized_pnl_usd, ..
        }) = settings.feedback_outbox.first() else {
            panic!("trade outcome was not queued")
        };
        assert_eq!(event_id, "trade:trade-42");
        assert_eq!(parameters["minimum_momentum"], 0.003);
        assert_eq!(profile_id, &strategy_core::profile_id("BTC-USDT", "live-momentum", parameters));
        assert_eq!(*realized_pnl_usd, 1.25);
    }
}

fn realized_usd(settings: &Settings, symbol: &str, quote_amount: f64) -> f64 {
    quote_amount * quote_usd_rate(settings, symbol)
}

fn learning_stats_for_day(stats: &LearningStats, day: &str) -> LearningStats {
    if stats.daily_utc_day == day {
        stats.clone()
    } else {
        LearningStats::default()
    }
}

fn record_learning_outcome(stats: &mut LearningStats, realized_pnl: f64) {
    let now_ms = current_time_ms();
    decay_learning_stats(stats, now_ms);
    let day = utc_day();
    if stats.daily_utc_day != day {
        stats.daily_utc_day = day;
        stats.daily_samples = 0;
        stats.daily_positive = 0;
        stats.daily_realized_pnl = 0.0;
    }
    stats.daily_samples += 1;
    stats.daily_realized_pnl += realized_pnl;
    stats.samples += 1;
    stats.recent_trade_pnls.push((now_ms, realized_pnl));
    if stats.recent_trade_pnls.len() > STRATEGY_SIZING_RECENT_TRADES {
        let excess = stats.recent_trade_pnls.len() - STRATEGY_SIZING_RECENT_TRADES;
        stats.recent_trade_pnls.drain(0..excess);
    }
    if realized_pnl > 0.0 {
        stats.positive += 1;
        stats.daily_positive += 1;
    }
    stats.realized_pnl += realized_pnl;
    stats.updated_at_ms = now_ms;
}

#[cfg(test)]
mod scout_context_learning_tests {
    use super::*;

    #[test]
    fn scout_scenario_context_maps_to_one_baseline_context_key() {
        assert_eq!(
            scout_context_learning_key(
                "scout:long:range:XRP-USDC:live-momentum:trade:scenario:strategy"
            ),
            Some("scout:long:range:XRP-USDC:live-momentum:trade".into())
        );
        assert_eq!(scout_context_learning_key("ordinary-context"), None);
    }
}
