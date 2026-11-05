fn should_explore_execution_profile(signal: &Signal, kind: AutoOrderType) -> bool {
    if !matches!(
        kind,
        AutoOrderType::Limit
            | AutoOrderType::Gtd
            | AutoOrderType::StopMarket
            | AutoOrderType::StopLimit
            | AutoOrderType::TrailingStop
            | AutoOrderType::Barrier
            | AutoOrderType::Vwap
            | AutoOrderType::Twap
    ) {
        return false;
    }
    // Reserve one in ten eligible strict entries for bounded, repeatable
    // profile comparisons. Hash the signal identity so retries keep their
    // assignment and the remaining orders stay on the established path.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    signal.symbol.hash(&mut hasher);
    signal.direction.hash(&mut hasher);
    signal.strategy.hash(&mut hasher);
    signal.timestamp.hash(&mut hasher);
    format!("{kind:?}").hash(&mut hasher);
    hasher.finish() % 10 == 0
}

#[cfg(test)]
#[cfg(test)]
fn choose_parameter_profile(
    kind: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, ExecutionLearningStats>,
    explore: bool,
) -> usize {
    // Profiles only change parameters used by these order types. Keeping the
    // others on P0 avoids recording duplicate, indistinguishable executions.
    if !matches!(
        kind,
        AutoOrderType::Limit
            | AutoOrderType::Gtd
            | AutoOrderType::StopMarket
            | AutoOrderType::StopLimit
            | AutoOrderType::TrailingStop
            | AutoOrderType::Barrier
            | AutoOrderType::Vwap
            | AutoOrderType::Twap
    ) {
        return 0;
    }
    let profiles = (0..PARAMETER_PROFILE_COUNT)
        .map(|profile| {
            let prefix = format!("{context}:P{profile}:");
            let suffix = format!(":exec:{kind:?}");
            let stats = aggregate_execution_quality(learning, &prefix, &suffix);
            (profile, stats)
        })
        .collect::<Vec<_>>();
    if explore {
        // Do not let a proven P0 baseline prevent P1/P2 from ever receiving
        // observations. Test the least-observed profile until each has enough
        // submissions and fills to be compared meaningfully.
        if let Some((profile, _)) = profiles.iter().filter(|(_, stats)| {
            stats.samples < MIN_EXECUTION_QUALITY_SAMPLES
                || stats.filled_samples < MIN_EXECUTION_QUALITY_FILLS
        }).min_by_key(|(_, stats)| stats.samples)
        {
            return *profile;
        }
    }
    let proven = profiles
        .iter()
        .filter(|(_, stats)| {
            stats.samples >= MIN_EXECUTION_QUALITY_SAMPLES
                && stats.filled_samples >= MIN_EXECUTION_QUALITY_FILLS
                && stats.cost_notional > 0.0
                && stats.fill_rate_total / stats.samples as f64 >= 0.60
        })
        .min_by(|a, b| {
            (a.1.all_in_cost_notional_bps / a.1.cost_notional)
                .total_cmp(&(b.1.all_in_cost_notional_bps / b.1.cost_notional))
        });
    if let Some((profile, _)) = proven {
        return *profile;
    }
    0
}

fn choose_order_type(signal: &Signal, risk_appetite: f64) -> AutoOrderType {
    let volatility = signal.components.volatility_1m_percent.unwrap_or(0.0).abs();
    let trend_units = signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs();
    let momentum = signal.components.momentum_10s_percent.unwrap_or(0.0).abs();
    let pressure = signal
        .components
        .pressure_without_price_move
        .unwrap_or(0.0)
        .abs();
    let strength_scale = risk_floor_scale(risk_appetite);
    if risk_appetite >= AGGRESSIVE_ORDER_PRIORITY_APPETITE {
        if trend_units >= ORDER_STOP_MARKET_TREND_UNITS
            && signal.strength >= ORDER_STOP_MARKET_STRENGTH * strength_scale
        {
            return AutoOrderType::StopMarket;
        }
        if pressure >= ORDER_MARKET_PRESSURE
            && signal.strength >= ORDER_MARKET_STRENGTH * strength_scale
        {
            return AutoOrderType::Market;
        }
        if momentum >= ORDER_IOC_MOMENTUM_PERCENT
            && signal.strength >= ORDER_IOC_STRENGTH * strength_scale
        {
            return AutoOrderType::Ioc;
        }
    }
    // Use passive maker execution by default to reduce fee drag. The explicit
    // high urgency rules below, learned alternatives, and large-order
    // scheduling still provide routes to faster execution.
    if volatility >= ORDER_STOP_LIMIT_VOLATILITY_PERCENT
        && trend_units >= ORDER_STOP_LIMIT_TREND_UNITS
    {
        AutoOrderType::StopLimit
    } else if trend_units >= ORDER_STOP_MARKET_TREND_UNITS
        && signal.strength >= ORDER_STOP_MARKET_STRENGTH * strength_scale
    {
        AutoOrderType::StopMarket
    } else if pressure >= ORDER_MARKET_PRESSURE
        && signal.strength >= ORDER_MARKET_STRENGTH * strength_scale
    {
        AutoOrderType::Market
    } else if momentum >= ORDER_IOC_MOMENTUM_PERCENT
        && signal.strength >= ORDER_IOC_STRENGTH * strength_scale
    {
        AutoOrderType::Ioc
    } else {
        AutoOrderType::PostOnlyLimit
    }
}

fn schedule_depth_ratio_bucket(ratio: f64) -> &'static str {
    match ratio {
        value if value < 0.05 => "lt05",
        value if value < 0.10 => "05_10",
        value if value < 0.15 => "10_15",
        value if value < 0.25 => "15_25",
        value if value < 0.40 => "25_40",
        _ => "ge40",
    }
}

#[cfg(test)]
#[cfg(test)]
fn learned_schedule_depth_threshold(
    learning: &BTreeMap<String, ExecutionLearningStats>,
    context: &str,
) -> (f64, usize) {
    let mut threshold = DEFAULT_SCHEDULE_DEPTH_RATIO;
    let mut paired_samples = 0;
    let cohorts: [(&str, f64, f64); 5] = [
        ("05_10", 0.05, 0.10),
        ("10_15", 0.10, 0.15),
        ("15_25", 0.15, 0.25),
        ("25_40", 0.25, 0.40),
        ("ge40", 0.40, MAX_SCHEDULE_DEPTH_RATIO),
    ];
    for (bucket, lower, upper) in cohorts {
        let marker = format!(":depth:{bucket}:");
        let mut scheduled = ExecutionLearningStats::default();
        let mut alternatives = ExecutionLearningStats::default();
        for (key, stats) in learning.iter().filter(|(key, _)| {
            key.starts_with(&format!("{context}:P")) && key.contains(&marker)
        }) {
            let mut recent = stats.clone();
            decay_execution_quality(&mut recent, current_time_ms());
            if key.ends_with(":exec:Vwap") || key.ends_with(":exec:Twap") {
                scheduled.samples += recent.samples;
                scheduled.filled_samples += recent.filled_samples;
                scheduled.fill_rate_total += recent.fill_rate_total;
                scheduled.cost_notional += recent.cost_notional;
                scheduled.all_in_cost_notional_bps += recent.all_in_cost_notional_bps;
            } else {
                alternatives.samples += recent.samples;
                alternatives.filled_samples += recent.filled_samples;
                alternatives.fill_rate_total += recent.fill_rate_total;
                alternatives.cost_notional += recent.cost_notional;
                alternatives.all_in_cost_notional_bps += recent.all_in_cost_notional_bps;
            }
        }
        if scheduled.samples < MIN_EXECUTION_QUALITY_SAMPLES
            || scheduled.filled_samples < MIN_EXECUTION_QUALITY_FILLS
            || scheduled.cost_notional <= 0.0
        {
            continue;
        }
        paired_samples += scheduled.samples + alternatives.samples;
        let scheduled_average =
            scheduled.all_in_cost_notional_bps / scheduled.cost_notional;
        let scheduled_fill_rate = scheduled.fill_rate_total / scheduled.samples as f64;
        if alternatives.samples >= MIN_EXECUTION_QUALITY_SAMPLES
            && alternatives.filled_samples >= MIN_EXECUTION_QUALITY_FILLS
            && alternatives.cost_notional > 0.0
        {
            let alternatives_average =
                alternatives.all_in_cost_notional_bps / alternatives.cost_notional;
            let alternatives_fill_rate = alternatives.fill_rate_total / alternatives.samples as f64;
            if scheduled_average + 0.5 < alternatives_average
                && scheduled_fill_rate + 0.05 >= alternatives_fill_rate
            {
                threshold = threshold.min(lower.max(MIN_SCHEDULE_DEPTH_RATIO));
            } else if scheduled_average > alternatives_average + 0.5
                && scheduled_fill_rate + 0.05 < alternatives_fill_rate
            {
                threshold = threshold.max(upper.min(MAX_SCHEDULE_DEPTH_RATIO));
            }
        }
    }
    (threshold.clamp(MIN_SCHEDULE_DEPTH_RATIO, MAX_SCHEDULE_DEPTH_RATIO), paired_samples)
}

#[cfg(test)]
mod order_type_selection_behavior_tests {
    use super::*;

    fn signal() -> Signal {
        Signal {
            symbol: "BTC-USDT".into(), direction: "bullish".into(), strength: 1.0,
            timestamp: 10, stale: false, score: 1.0, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: false,
            strategy: None, strategy_decision: None, market_regime: None,
            news_attribution: None, entry_gate: None,
        }
    }

    fn stats(samples: usize, fills: usize, cost: f64) -> ExecutionLearningStats {
        ExecutionLearningStats {
            samples, filled_samples: fills, fill_rate_total: fills as f64,
            cost_notional: 100.0, all_in_cost_notional_bps: cost * 100.0,
            updated_at_ms: current_time_ms(),
        }
    }

    #[test]
    fn exploration_assignment_is_stable_and_only_uses_parameterized_order_types() {
        let mut candidate = signal();
        assert!(!should_explore_execution_profile(&candidate, AutoOrderType::Ioc));
        assert!(!should_explore_execution_profile(&candidate, AutoOrderType::Market));
        let assignment = should_explore_execution_profile(&candidate, AutoOrderType::Limit);
        assert_eq!(should_explore_execution_profile(&candidate, AutoOrderType::Limit), assignment);
        candidate.timestamp += 1;
        assert_eq!(should_explore_execution_profile(&candidate, AutoOrderType::Limit), should_explore_execution_profile(&candidate, AutoOrderType::Limit));
    }

    #[test]
    fn parameter_profiles_explore_unobserved_cohorts_then_select_lowest_validated_cost() {
        let context = "long:trend:BTC-USDT";
        let mut learning = BTreeMap::new();
        learning.insert(format!("{context}:P0:BTC-USDT:exec:Limit"), stats(MIN_EXECUTION_QUALITY_SAMPLES, MIN_EXECUTION_QUALITY_FILLS, 3.0));
        assert_eq!(choose_parameter_profile(AutoOrderType::Limit, context, &learning, true), 1);
        assert_eq!(choose_parameter_profile(AutoOrderType::Market, context, &learning, true), 0);
        learning.insert(format!("{context}:P1:BTC-USDT:exec:Limit"), stats(12, 10, 1.0));
        learning.insert(format!("{context}:P2:BTC-USDT:exec:Limit"), stats(2, 1, 2.0));
        assert_eq!(choose_parameter_profile(AutoOrderType::Limit, context, &learning, false), 1);
        assert_eq!(choose_parameter_profile(AutoOrderType::Limit, context, &learning, true), 2);
    }

    #[test]
    fn order_selection_uses_aggressive_rules_then_passive_fallback() {
        let mut signal = signal();
        assert_eq!(choose_order_type(&signal, 50.0), AutoOrderType::PostOnlyLimit);
        signal.components.trend_5m_volatility_units = Some(10.0);
        assert_eq!(choose_order_type(&signal, 100.0), AutoOrderType::StopMarket);
        signal.components.trend_5m_volatility_units = None;
        signal.components.pressure_without_price_move = Some(10.0);
        assert_eq!(choose_order_type(&signal, 100.0), AutoOrderType::Market);
        signal.components.pressure_without_price_move = None;
        signal.components.momentum_10s_percent = Some(10.0);
        assert_eq!(choose_order_type(&signal, 100.0), AutoOrderType::Ioc);
        signal.strength = 0.0;
        signal.components.momentum_10s_percent = None;
        signal.components.volatility_1m_percent = Some(10.0);
        signal.components.trend_5m_volatility_units = Some(10.0);
        assert_eq!(choose_order_type(&signal, 10.0), AutoOrderType::StopLimit);
    }

    #[test]
    fn order_type_adaptation_requires_validated_fill_and_cost_advantage() {
        let context = "long:trend:BTC-USDT";
        let mut learning = BTreeMap::new();
        assert_eq!(adapt_order_type(AutoOrderType::Ioc, context, &learning, 50.0), Some(AutoOrderType::Ioc));
        learning.insert(format!("{context}:P0:BTC-USDT:exec:Ioc"), stats(20, 16, 3.0));
        learning.insert(format!("{context}:P0:BTC-USDT:exec:Limit"), stats(20, 16, 1.0));
        learning.insert(format!("{context}:P1:BTC-USDT:exec:Vwap"), stats(20, 18, 0.2));
        assert_eq!(adapt_order_type(AutoOrderType::Ioc, context, &learning, 50.0), Some(AutoOrderType::Vwap));
        assert_eq!(schedule_depth_ratio_bucket(0.049), "lt05");
        assert_eq!(schedule_depth_ratio_bucket(0.05), "05_10");
        assert_eq!(schedule_depth_ratio_bucket(0.10), "10_15");
        assert_eq!(schedule_depth_ratio_bucket(0.15), "15_25");
        assert_eq!(schedule_depth_ratio_bucket(0.25), "25_40");
        assert_eq!(schedule_depth_ratio_bucket(0.40), "ge40");
    }

    #[test]
    fn schedule_threshold_learning_requires_paired_samples_and_moves_by_cost_and_fill_quality() {
        let context = "long:trend:BTC-USDT";
        let baseline = learned_schedule_depth_threshold(&BTreeMap::new(), context);
        assert_eq!(baseline, (DEFAULT_SCHEDULE_DEPTH_RATIO, 0));
        let mut learning = BTreeMap::new();
        learning.insert(format!("{context}:P0:depth:10_15:exec:Vwap"), stats(20, 16, 1.0));
        learning.insert(format!("{context}:P0:depth:10_15:exec:Limit"), stats(20, 16, 3.0));
        assert_eq!(learned_schedule_depth_threshold(&learning, context), (0.10_f64.max(MIN_SCHEDULE_DEPTH_RATIO), 40));
        learning.insert(format!("{context}:P0:depth:25_40:exec:Vwap"), stats(20, 10, 5.0));
        learning.insert(format!("{context}:P0:depth:25_40:exec:Limit"), stats(20, 18, 1.0));
        let (threshold, paired) = learned_schedule_depth_threshold(&learning, context);
        assert_eq!(paired, 80);
        assert!(threshold >= 0.25);
    }
}
