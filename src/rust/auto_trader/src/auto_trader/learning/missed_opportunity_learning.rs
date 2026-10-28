fn longer_horizon_score_multiplier(score: f64, bands: &[MissedOpportunityGateBand]) -> f64 {
    if !score.is_finite() {
        return 1.0;
    }
    let absolute_score = score.abs();
    let short_horizon = bands
        .iter()
        .filter(|band| {
            band.gate == "score" && band.horizon_seconds == 300 && band.threshold <= absolute_score
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
        long.gate == "score"
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
    gate: &str,
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
    learning: &BTreeMap<String, LearningStats>,
) -> LearningStats {
    // Paper Market and IOC both execute immediately against the same book
    // levels with the same taker fee and no resting remainder. Treat their
    // historical cohorts as one execution behavior so evidence is not split
    // across two labels for identical fills.
    let equivalent_kinds: &[AutoOrderType] = match kind {
        AutoOrderType::Market | AutoOrderType::Ioc => &[AutoOrderType::Market, AutoOrderType::Ioc],
        _ => std::slice::from_ref(&kind),
    };
    let mut total = LearningStats::default();
    for equivalent_kind in equivalent_kinds {
        let prefix = format!("{context}:P");
        let suffix = format!(":{equivalent_kind:?}");
        for stats in learning
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(&suffix))
            .map(|(_, stats)| stats)
        {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
        }
        if let Some(stats) = learning.get(&format!("{context}:{equivalent_kind:?}")) {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
        }
    }
    total
}

fn adapt_order_type(
    baseline: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
) -> Option<AutoOrderType> {
    let favor_non_post_only = risk_appetite >= AGGRESSIVE_ORDER_PRIORITY_APPETITE;
    // A post-only entry is intentionally allowed as a cold-start experiment in
    // calm, costly-to-aggress conditions. Do not let the default IOC fallback
    // erase that signal before the maker cohort has enough outcomes to learn.
    if matches!(baseline, AutoOrderType::PostOnlyLimit) && !favor_non_post_only {
        let maker_stats =
            order_type_learning_stats(context, AutoOrderType::PostOnlyLimit, learning);
        if maker_stats.samples < MIN_EXECUTION_LEARNING_SAMPLES {
            return Some(AutoOrderType::PostOnlyLimit);
        }
    }
    let candidates = ALL_ORDER_TYPES
        .iter()
        .copied()
        .filter_map(|kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            let average_pnl = stats.realized_pnl / stats.samples.max(1) as f64;
            let positive_rate = stats.positive as f64 / stats.samples.max(1) as f64;
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && !(favor_non_post_only && matches!(kind, AutoOrderType::PostOnlyLimit))
                && average_pnl > 0.0
                && positive_rate >= 0.55)
                .then_some((kind, average_pnl))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        // A losing record rules out that order type, not the signal. Keep
        // trying the alternate execution type while its own evidence is
        // immature; only stop when both choices have mature losing records.
        let is_mature_unprofitable = |kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && (stats.realized_pnl / stats.samples as f64 <= 0.0
                    || (stats.positive as f64 / stats.samples as f64) < 0.55)
        };
        let baseline_unprofitable = is_mature_unprofitable(baseline);
        let ioc_unprofitable = is_mature_unprofitable(AutoOrderType::Ioc);
        if baseline_unprofitable && ioc_unprofitable {
            return None;
        }
        return Some(if baseline_unprofitable {
            AutoOrderType::Ioc
        } else if ioc_unprofitable {
            baseline
        } else {
            AutoOrderType::Ioc
        });
    }
    candidates
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind)
}

