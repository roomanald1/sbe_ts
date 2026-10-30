fn market_context(signal: &Signal) -> String {
    let regime = if signal.strategy_candidate {
        "trend"
    } else if signal.ranging_probability.unwrap_or(0.0) >= CONTEXT_RANGE_MIN_PROBABILITY {
        "range"
    } else if signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs()
        >= CONTEXT_TREND_MIN_VOLATILITY_UNITS
    {
        "trend"
    } else if signal.components.volatility_1m_percent.unwrap_or(0.0).abs()
        >= CONTEXT_VOLATILE_MIN_PERCENT
    {
        "volatile"
    } else {
        "mixed"
    };
    let direction = if signal.direction == "bullish" {
        "long"
    } else {
        "short"
    };
    if let Some(strategy) = signal.strategy.as_deref() {
        format!("{direction}:{regime}:{}:{strategy}", signal.symbol)
    } else {
        format!("{direction}:{regime}:{}", signal.symbol)
    }
}

const ALL_ORDER_TYPES: [AutoOrderType; 12] = [
    AutoOrderType::Market,
    AutoOrderType::Ioc,
    AutoOrderType::Fok,
    AutoOrderType::Limit,
    AutoOrderType::Gtd,
    AutoOrderType::PostOnlyLimit,
    AutoOrderType::Barrier,
    AutoOrderType::StopMarket,
    AutoOrderType::StopLimit,
    AutoOrderType::TrailingStop,
    AutoOrderType::Vwap,
    AutoOrderType::Twap,
];
fn context_learning_stats(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
) -> LearningStats {
    let prefix = format!("{context}:");
    learning
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(":trade"))
        .map(|(_, stats)| stats)
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        })
}

fn smoothed_win_rate(stats: &LearningStats) -> f64 {
    // A light Beta(1, 1) prior prevents tiny cohorts from moving gates sharply.
    (stats.positive as f64 + 1.0) / (stats.samples as f64 + 2.0)
}

fn adaptive_scout_thresholds(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    active_scout_count: usize,
) -> (f64, f64) {
    let stats = context_learning_stats(learning, &format!("scout:{context}"));
    let win_rate = smoothed_win_rate(&stats);
    let losing_cohort = stats.realized_pnl < 0.0;
    let density = active_scout_count.min(SCOUT_MAX_ACTIVE_POSITIONS) as f64;
    let mut strength =
        (SCOUT_MIN_STRENGTH - 0.15 + density * 0.025).clamp(SCOUT_MIN_STRENGTH_FLOOR, 0.25);
    let mut score =
        (SCOUT_MIN_ABS_SCORE - 0.15 + density * 0.05).clamp(SCOUT_MIN_SCORE_FLOOR, 0.40);
    if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES {
        if losing_cohort && win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE {
            let adjustment = if stats.samples >= 20 { 2.0 } else { 1.0 };
            strength = (strength + 0.05 * adjustment).min(0.30);
            score = (score + 0.10 * adjustment).min(0.50);
        } else if stats.realized_pnl > 0.0 && win_rate >= 0.60 {
            strength = (strength - 0.02).max(SCOUT_MIN_STRENGTH_FLOOR);
            score = (score - 0.03).max(SCOUT_MIN_SCORE_FLOOR);
        }
    }
    (strength, score)
}

fn risk_floor_scale(risk_appetite: f64) -> f64 {
    1.5 - risk_appetite.clamp(0.0, 100.0) / 100.0
}

fn risk_size_scale(risk_appetite: f64) -> f64 {
    0.5 + 2.5 * risk_appetite.clamp(0.0, 100.0) / 100.0
}

fn adaptive_strength_floor(learning: &BTreeMap<String, LearningStats>, context: &str) -> f64 {
    let stats = context_learning_stats(learning, context);
    let win_rate = smoothed_win_rate(&stats);
    let losing_cohort = stats.realized_pnl < 0.0;
    if stats.samples >= STRONG_NEGATIVE_LEARNING_SAMPLES
        && losing_cohort
        && win_rate < STRONG_NEGATIVE_LEARNING_WIN_RATE
    {
        STRONG_NEGATIVE_STRENGTH_FLOOR
    } else if stats.samples >= 20
        && losing_cohort
        && win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE
    {
        (LEARNED_STRENGTH_FLOOR + 0.07).min(0.60)
    } else if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES
        && losing_cohort
        && win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE
    {
        LEARNED_STRENGTH_FLOOR
    } else {
        MIN_STRENGTH
    }
}

fn adaptive_exploration_score_floor(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    missed_opportunity_bands: &[MissedOpportunityGateBand],
) -> f64 {
    let stats = context_learning_stats(learning, context);
    let win_rate = smoothed_win_rate(&stats);
    let losing_cohort = stats.realized_pnl < 0.0;
    let learned_floor = if stats.samples >= STRONG_NEGATIVE_LEARNING_SAMPLES
        && losing_cohort
        && win_rate < STRONG_NEGATIVE_LEARNING_WIN_RATE
    {
        STRONG_NEGATIVE_SCORE_FLOOR
    } else if stats.samples >= EXPLORATION_SCORE_HIGH_SAMPLES
        && losing_cohort
        && win_rate < EXPLORATION_SCORE_HIGH_WIN_RATE
    {
        EXPLORATION_SCORE_HIGH_FLOOR
    } else if stats.samples >= EXPLORATION_SCORE_MID_SAMPLES
        && losing_cohort
        && win_rate < EXPLORATION_SCORE_MID_WIN_RATE
    {
        EXPLORATION_SCORE_MID_FLOOR
    } else {
        MIN_EXPLORATION_SCORE
    };
    // Five-minute counterfactual stop/target outcomes may relax gates when they
    // are profitable. Longer terminal outcomes also inform the score floor when
    // mature cohorts show that otherwise-rejected signals need more time to turn
    // net positive. This changes entry selection, not live position exit timing.
    let short_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == MissedOpportunityGate::Score
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    let longer_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == MissedOpportunityGate::Score
                && matches!(band.horizon_seconds, 900 | 3_600)
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.positive_net_rate.is_some_and(|value| value >= 0.55)
                // Reject a later recovery if the path would already have hit
                // the configured stop before the longer horizon elapsed.
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.stop_target_positive_net_rate.is_some_and(|value| value >= 0.55)
                && band.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.time_to_profit_rate.is_some_and(|value| value >= 0.50)
                && band.average_time_to_profit_seconds.is_some_and(|value| {
                    value > 300.0 && value <= band.horizon_seconds as f64
                })
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    short_horizon_floor.min(longer_horizon_floor)
}


fn is_mixed_market_context(context: &str) -> bool {
    context.split(':').nth(1) == Some("mixed")
}
