fn longer_horizon_score_multiplier(score: f64, bands: &[MissedOpportunityGateBand]) -> f64 {
    if !score.is_finite() {
        return 1.0;
    }
    let absolute_score = score.abs();
    let short_horizon = bands
        .iter()
        .filter(|band| {
            band.gate == MissedOpportunityGate::Score
                && band.horizon_seconds == 300
                && band.threshold <= absolute_score
        })
        .max_by(|left, right| left.threshold.total_cmp(&right.threshold));
    let Some(short) = short_horizon else {
        return 1.0;
    };
    let Some(short_average) = short.average_net_return_percent else {
        return 1.0;
    };
    let Some(short_positive_rate) = short.positive_net_rate else {
        return 1.0;
    };
    let supported_longer_horizon = bands.iter().any(|long| {
        long.gate == MissedOpportunityGate::Score
            && (long.threshold - short.threshold).abs() < 1e-9
            && matches!(long.horizon_seconds, 900 | 3_600)
            && long.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.average_net_return_percent.is_some_and(|average| {
                average >= short_average + 0.10 && average > 0.0
            })
            && long.positive_net_rate.is_some_and(|rate| {
                rate >= 0.70 && rate + 0.05 >= short_positive_rate
            })
            // The stop-target path model treats a stop breach as the exit,
            // even if the terminal mark later recovers above entry.
            && long.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
            && long.stop_target_positive_net_rate.is_some_and(|rate| rate >= 0.70)
            && long.time_to_profit_rate.is_some_and(|rate| rate >= 0.70)
            && long.average_time_to_profit_seconds.is_some_and(|seconds| {
                seconds > 300.0 && seconds <= long.horizon_seconds as f64
            })
    });
    if supported_longer_horizon { 1.5 } else { 1.0 }
}

fn adaptive_missed_opportunity_floor(
    learned_floor: f64,
    gate: MissedOpportunityGate,
    bands: &[MissedOpportunityGateBand],
) -> f64 {
    bands
        .iter()
        .filter(|band| {
            band.gate == gate
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD
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
        .unwrap_or(learned_floor)
}
fn order_type_learning_stats(
    context: &str,
    kind: AutoOrderType,
    learning: &BTreeMap<String, ExecutionLearningStats>,
) -> ExecutionLearningStats {
    aggregate_execution_quality(
        learning,
        &format!("{context}:P"),
        &format!(":exec:{kind:?}"),
    )
}

fn adapt_order_type(
    baseline: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, ExecutionLearningStats>,
    _risk_appetite: f64,
) -> Option<AutoOrderType> {
    let baseline_stats = order_type_learning_stats(context, baseline, learning);
    if baseline_stats.samples < MIN_EXECUTION_QUALITY_SAMPLES
        || baseline_stats.filled_samples < MIN_EXECUTION_QUALITY_FILLS
        || baseline_stats.cost_notional <= 0.0
    {
        return Some(baseline);
    }
    let baseline_cost =
        baseline_stats.all_in_cost_notional_bps / baseline_stats.cost_notional;
    let baseline_fill_rate = baseline_stats.fill_rate_total / baseline_stats.samples as f64;
    ALL_ORDER_TYPES
        .iter()
        .copied()
        .filter(|kind| *kind != baseline)
        .filter_map(|kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            if stats.samples < MIN_EXECUTION_QUALITY_SAMPLES
                || stats.filled_samples < MIN_EXECUTION_QUALITY_FILLS
                || stats.cost_notional <= 0.0
            {
                return None;
            }
            let cost = stats.all_in_cost_notional_bps / stats.cost_notional;
            let fill_rate = stats.fill_rate_total / stats.samples as f64;
            (fill_rate + 0.05 >= baseline_fill_rate && cost + 0.5 < baseline_cost)
                .then_some((kind, cost))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(kind, _)| kind)
        .or(Some(baseline))
}
