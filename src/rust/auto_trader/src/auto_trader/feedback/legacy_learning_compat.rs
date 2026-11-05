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

#[cfg(test)]
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

#[cfg(test)]
fn strategy_scout_learning_key(symbol: &str, strategy: &str, scenario: &str) -> String {
    format!("scout:{symbol}:{strategy}:{scenario}")
}

#[cfg(test)]
fn strategy_parameter_probe_learning_key(symbol: &str, strategy: &str, parameter: &str, value: f64) -> String {
    format!("probe:{symbol}:{strategy}:{parameter}:{value:.8}")
}

#[cfg(test)]
fn strategy_parameter_scout_combination_learning_key(
    symbol: &str, strategy: &str, combination: &str,
) -> String {
    format!("probe_combo_scout:{symbol}:{strategy}:{combination}")
}

#[cfg(test)]
fn strategy_parameter_scout_probe_learning_key(
    symbol: &str,
    strategy: &str,
    parameter: &str,
    value: f64,
) -> String {
    format!("probe_scout:{symbol}:{strategy}:{parameter}:{value:.8}")
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
                parameters = learned_strategy_parameter_profile(settings, profile_symbol, kind)
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
        let profile_id = strategies::profile_id(profile_symbol, strategy, &parameters);
        let threshold_observations = entry_decision
            .and_then(|decision| threshold_observation(strategy, decision))
            .map(|(parameter_key, threshold, feature_value)| {
                vec![strategies::StrategyThresholdObservation {
                    parameter_key: parameter_key.as_str().to_owned(),
                    threshold,
                    feature_value,
                }]
            })
            .unwrap_or_default();
        let event_id = format!("trade:{}", trade_id.filter(|value| !value.is_empty()).unwrap_or(learning_context.unwrap_or(symbol)));
        if !settings.feedback_outbox.iter().any(|event| matches!(event,
            strategies::FeedbackEvent::TradeOutcome { event_id: pending, .. } if pending == &event_id
        )) {
            settings.feedback_outbox.push(strategies::FeedbackEvent::TradeOutcome {
                event_id,
                observed_at_ms: current_time_ms(),
                learning_context: learning_context.map(str::to_owned),
                symbol: profile_symbol.to_owned(),
                strategy: strategy.to_owned(),
                profile_id,
                parameters,
                // Real trade outcomes update profile statistics, while
                // parameter changes require a successful parameter-specific
                // scout comparison.
                specs: Vec::new(),
                scout: _scout,
                realized_pnl_usd,
                net_return_percent: entry_decision
                    .and_then(|decision| decision.get("initial_entry_notional_usd"))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|notional| *notional > 0.0)
                    .map(|notional| realized_pnl_usd / notional * 100.0)
                    .unwrap_or(0.0),
                threshold_observations,
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
        assert_eq!(profile_id, &strategies::profile_id("BTC-USDT", "live-momentum", parameters));
        assert_eq!(*realized_pnl_usd, 1.25);
    }

    #[test]
    fn completed_live_trade_includes_the_observed_threshold_feature() {
        let mut settings = Settings::default();
        let decision = serde_json::json!({
            "strategy_parameters": { "minimum_momentum": 0.003 },
            "direction": "bullish",
            "observed_insight": {
                "momentum_10s_percent": 0.01,
                "trend_5m_volatility_units": 0.02
            },
            "initial_entry_notional_usd": 100.0
        });
        record_strategy_trade_outcome(
            &mut settings,
            "BTC-USDT",
            Some("live-momentum"),
            Some("btc:momentum"),
            false,
            1.25,
            Some(&decision),
            Some("trade-threshold-1"),
        );
        let Some(FeedbackEvent::TradeOutcome {
            threshold_observations,
            ..
        }) = settings.feedback_outbox.first()
        else {
            panic!("trade outcome was not queued")
        };
        assert_eq!(threshold_observations.len(), 1);
        assert_eq!(threshold_observations[0].parameter_key, "minimum_momentum");
        assert_eq!(threshold_observations[0].threshold, 0.003);
        assert_eq!(threshold_observations[0].feature_value, 0.02);
    }
}

fn realized_usd(settings: &Settings, symbol: &str, quote_amount: f64) -> f64 {
    quote_amount * quote_usd_rate(settings, symbol)
}

#[cfg(test)]
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

