fn longer_horizon_score_multiplier(score: f64, bands: &[MissedOpportunityGateBand]) -> f64 {
    if !score.is_finite() {
        return 1.0;
    }
    let absolute_score = score.abs();
    bands
        .iter()
        .filter(|band| {
            band.gate == MissedOpportunityGate::Score
                && band.symbol.is_none()
                && matches!(band.horizon_seconds, 900 | 3_600)
                && band.threshold <= absolute_score
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|average| average > 0.0)
                && band.stop_target_positive_net_rate.is_some_and(|rate| {
                    wilson_lower_bound(rate, band.stop_target_sample_count)
                        .is_some_and(|lower| lower > 0.50)
                })
                && (band.horizon_seconds == 300
                    || (band.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                        && band.time_to_profit_rate.is_some_and(|rate| rate >= 0.50)
                        && band.average_time_to_profit_seconds.is_some_and(|seconds| {
                            seconds > 300.0 && seconds <= band.horizon_seconds as f64
                        })))
        })
        .filter_map(|band| {
            let average = band.stop_target_average_net_return_percent?;
            let rate = band.stop_target_positive_net_rate?;
            let sample_confidence = band.stop_target_sample_count as f64
                / (band.stop_target_sample_count as f64 + COUNTERFACTUAL_RELAXATION_SAMPLES as f64);
            let rate_confidence = (wilson_lower_bound(rate, band.stop_target_sample_count)? - 0.50)
                .clamp(0.0, 0.50)
                * 2.0;
            let return_scale = (average / (1.0 + average)).clamp(0.0, 1.0);
            Some(1.0 + 0.5 * sample_confidence * rate_confidence * return_scale)
        })
        .max_by(f64::total_cmp)
        .unwrap_or(1.0)
}

fn adaptive_missed_opportunity_floor(
    learned_floor: f64,
    gate: MissedOpportunityGate,
    bands: &[MissedOpportunityGateBand],
) -> f64 {
    adaptive_missed_opportunity_floor_for_scope(learned_floor, gate, bands, None)
}

fn adaptive_missed_opportunity_floor_for_signal(
    learned_floor: f64,
    gate: MissedOpportunityGate,
    bands: &[MissedOpportunityGateBand],
    symbol: &str,
    direction: &str,
    strategy: Option<&str>,
    market_context: &str,
) -> f64 {
    let scoped_floor = adaptive_missed_opportunity_floor_for_scope(
        learned_floor,
        gate,
        bands,
        Some((symbol, direction, strategy, market_context)),
    );
    if scoped_floor < learned_floor {
        scoped_floor
    } else {
        adaptive_missed_opportunity_floor(learned_floor, gate, bands)
    }
}

fn adaptive_missed_opportunity_floor_for_scope(
    learned_floor: f64,
    gate: MissedOpportunityGate,
    bands: &[MissedOpportunityGateBand],
    scope: Option<(&str, &str, Option<&str>, &str)>,
) -> f64 {
    let eligible = bands
        .iter()
        .filter(|band| {
            let minimum_threshold = match gate {
                MissedOpportunityGate::Score => COUNTERFACTUAL_SCORE_MIN_THRESHOLD,
                MissedOpportunityGate::Strength => COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD,
                MissedOpportunityGate::Other => f64::INFINITY,
            };
            band.gate == gate
                && match scope {
                    Some((symbol, direction, strategy, market_context)) => {
                        band.symbol.as_deref() == Some(symbol)
                            && band.direction.as_deref() == Some(direction)
                            && band.strategy.as_deref() == strategy
                            && band.market_context.as_deref() == Some(market_context)
                    }
                    None => band.symbol.is_none(),
                }
                && matches!(band.horizon_seconds, 300 | 900 | 3_600)
                && band.threshold >= minimum_threshold
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= COUNTERFACTUAL_RELAXATION_POSITIVE_RATE)
                && (band.horizon_seconds == 300
                    || (band.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                        && band
                            .average_net_return_percent
                            .is_some_and(|value| value > 0.0)
                        && band
                            .positive_net_rate
                            .is_some_and(|value| value >= COUNTERFACTUAL_RELAXATION_POSITIVE_RATE)
                        && band.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                        && band.time_to_profit_rate.is_some_and(|value| value >= 0.50)
                        && band.average_time_to_profit_seconds.is_some_and(|seconds| {
                            seconds > 300.0 && seconds <= band.horizon_seconds as f64
                        })))
        })
        .collect::<Vec<_>>();
    let use_short_horizon = eligible.iter().any(|band| band.horizon_seconds == 300);
    eligible
        .into_iter()
        .filter(|band| {
            if use_short_horizon {
                band.horizon_seconds == 300
            } else {
                matches!(band.horizon_seconds, 900 | 3_600)
            }
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor)
}

fn wilson_lower_bound(rate: f64, sample_count: usize) -> Option<f64> {
    if !rate.is_finite() || !(0.0..=1.0).contains(&rate) || sample_count == 0 {
        return None;
    }
    let n = sample_count as f64;
    let z = 1.96;
    let denominator = 1.0 + z * z / n;
    let center = rate + z * z / (2.0 * n);
    let margin = z * ((rate * (1.0 - rate) / n + z * z / (4.0 * n * n)).sqrt());
    Some(((center - margin) / denominator).clamp(0.0, 1.0))
}
#[cfg(test)]
#[cfg(test)]
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

#[cfg(test)]
#[cfg(test)]
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
    let baseline_cost = baseline_stats.all_in_cost_notional_bps / baseline_stats.cost_notional;
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

#[cfg(test)]
mod missed_opportunity_learning_behavior_tests {
    use super::*;

    fn band(
        gate: MissedOpportunityGate,
        threshold: f64,
        horizon_seconds: u64,
    ) -> MissedOpportunityGateBand {
        MissedOpportunityGateBand {
            gate,
            threshold,
            symbol: None,
            direction: None,
            strategy: None,
            market_context: None,
            horizon_seconds,
            sample_count: 100,
            average_net_return_percent: Some(0.2),
            positive_net_rate: Some(0.65),
            time_to_profit_sample_count: 100,
            time_to_profit_rate: Some(0.75),
            average_time_to_profit_seconds: Some(400.0),
            stop_target_sample_count: 100,
            stop_target_average_net_return_percent: Some(0.3),
            stop_target_positive_net_rate: Some(0.7),
        }
    }

    #[test]
    fn longer_horizon_multiplier_requires_positive_stop_target_and_timely_profit_evidence() {
        let short = band(MissedOpportunityGate::Score, 0.4, 300);
        let mut long = band(MissedOpportunityGate::Score, 0.4, 900);
        long.average_net_return_percent = Some(0.4);
        long.positive_net_rate = Some(0.75);
        assert_eq!(longer_horizon_score_multiplier(0.5, &[short.clone()]), 1.0);
        assert_eq!(
            longer_horizon_score_multiplier(f64::NAN, &[short.clone(), long.clone()]),
            1.0
        );
        let multiplier = longer_horizon_score_multiplier(0.5, &[short.clone(), long.clone()]);
        assert!(multiplier > 1.0 && multiplier < 1.5);
        let mut bad_long = long.clone();
        bad_long.stop_target_average_net_return_percent = Some(-0.1);
        assert_eq!(
            longer_horizon_score_multiplier(0.5, &[short.clone(), bad_long]),
            1.0
        );
        let mut too_slow = long;
        too_slow.average_time_to_profit_seconds = Some(901.0);
        assert_eq!(
            longer_horizon_score_multiplier(0.5, &[short, too_slow]),
            1.0
        );
    }

    #[test]
    fn adaptive_floor_uses_longer_horizon_when_short_horizon_is_not_supported() {
        let valid_high = band(MissedOpportunityGate::Strength, 0.5, 300);
        let valid_low = band(MissedOpportunityGate::Strength, 0.35, 300);
        assert_eq!(
            adaptive_missed_opportunity_floor(
                0.8,
                MissedOpportunityGate::Strength,
                &[valid_high.clone(), valid_low.clone()]
            ),
            0.35
        );
        assert_eq!(
            adaptive_missed_opportunity_floor(
                0.8,
                MissedOpportunityGate::Score,
                &[valid_high.clone()]
            ),
            0.8
        );
        let mut invalid = valid_low;
        invalid.stop_target_sample_count = 0;
        assert_eq!(
            adaptive_missed_opportunity_floor(
                0.8,
                MissedOpportunityGate::Strength,
                &[valid_high, invalid]
            ),
            0.5
        );
        let mut longer = band(MissedOpportunityGate::Strength, 0.35, 900);
        longer.average_time_to_profit_seconds = Some(500.0);
        assert_eq!(
            adaptive_missed_opportunity_floor(0.8, MissedOpportunityGate::Strength, &[longer]),
            0.35
        );
    }

    #[test]
    fn live_threshold_override_uses_matching_symbol_direction_and_strategy_evidence() {
        let mut scoped = band(MissedOpportunityGate::Score, 0.3, 300);
        scoped.symbol = Some("BTC-USDT".into());
        scoped.direction = Some("bullish".into());
        scoped.strategy = Some("donchian-20-btc200".into());
        scoped.market_context = Some("trend".into());
        let global = band(MissedOpportunityGate::Score, 0.4, 300);

        assert_eq!(
            adaptive_missed_opportunity_floor_for_signal(
                0.8,
                MissedOpportunityGate::Score,
                &[scoped.clone(), global.clone()],
                "BTC-USDT",
                "bullish",
                Some("donchian-20-btc200"),
                "trend",
            ),
            0.3
        );
        assert_eq!(
            adaptive_missed_opportunity_floor_for_signal(
                0.8,
                MissedOpportunityGate::Score,
                &[scoped, global],
                "BTC-USDT",
                "bearish",
                Some("donchian-20-btc200"),
                "trend",
            ),
            0.4
        );
    }

    #[test]
    fn confidence_multiplier_is_monotonic_and_bounded() {
        let low = band(MissedOpportunityGate::Score, 0.4, 900);
        let mut high = low.clone();
        high.stop_target_average_net_return_percent = Some(1.5);
        high.stop_target_positive_net_rate = Some(0.9);
        assert!(
            longer_horizon_score_multiplier(0.5, &[high])
                > longer_horizon_score_multiplier(0.5, &[low])
        );
        assert!(
            longer_horizon_score_multiplier(0.5, &[band(MissedOpportunityGate::Score, 0.4, 300)])
                <= 1.5
        );
    }
}
