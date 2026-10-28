#[derive(Default)]
struct WalkForwardMetrics {
    max_training_samples: usize,
    evaluated_signals: usize,
    selected_signals: usize,
    correct: usize,
    net_total: f64,
    positive_net: usize,
    signal_long: SideWalkForwardAccumulator,
    signal_short: SideWalkForwardAccumulator,
    all_signal_long: SideWalkForwardAccumulator,
    all_signal_short: SideWalkForwardAccumulator,
    logistic_long: SideWalkForwardAccumulator,
    logistic_short: SideWalkForwardAccumulator,
    research_logistic_long: SideWalkForwardAccumulator,
    research_logistic_short: SideWalkForwardAccumulator,
    baseline_correct: usize,
    baseline_net_total: f64,
    five_minute_trend_samples: usize,
    five_minute_trend_correct: usize,
    five_minute_trend_net_total: f64,
    last_threshold: Option<f64>,
    logistic_selected: usize,
    logistic_correct: usize,
    logistic_positive: usize,
    logistic_net_total: f64,
    logistic_weights: Option<[f64; 5]>,
    trend_logistic_selected: usize,
    trend_logistic_correct: usize,
    trend_logistic_positive: usize,
    trend_logistic_net_total: f64,
    trend_logistic_confident_selected: [usize; 3],
    trend_logistic_confident_correct: [usize; 3],
    trend_logistic_confident_positive: [usize; 3],
    trend_logistic_confident_net: [f64; 3],
    contrarian_selected: usize,
    contrarian_correct: usize,
    contrarian_positive: usize,
    contrarian_net_total: f64,
    contrarian_confident_selected: [usize; 3],
    contrarian_confident_correct: [usize; 3],
    contrarian_confident_positive: [usize; 3],
    contrarian_confident_net: [f64; 3],
    exploration_selected: [usize; 5],
    exploration_correct: [usize; 5],
    exploration_positive: [usize; 5],
    exploration_net: [f64; 5],
    trend_logistic_weights: Option<[f64; 6]>,
    research_logistic_weights: Option<[f64; 9]>,
}

impl WalkForwardMetrics {
    fn new() -> Self {
        Self {
            max_training_samples: 0,
            evaluated_signals: 0,
            selected_signals: 0,
            correct: 0,
            net_total: 0.0,
            positive_net: 0,
            signal_long: SideWalkForwardAccumulator::default(),
            signal_short: SideWalkForwardAccumulator::default(),
            all_signal_long: SideWalkForwardAccumulator::default(),
            all_signal_short: SideWalkForwardAccumulator::default(),
            logistic_long: SideWalkForwardAccumulator::default(),
            logistic_short: SideWalkForwardAccumulator::default(),
            research_logistic_long: SideWalkForwardAccumulator::default(),
            research_logistic_short: SideWalkForwardAccumulator::default(),
            baseline_correct: 0,
            baseline_net_total: 0.0,
            five_minute_trend_samples: 0,
            five_minute_trend_correct: 0,
            five_minute_trend_net_total: 0.0,
            last_threshold: None,
            logistic_selected: 0,
            logistic_correct: 0,
            logistic_positive: 0,
            logistic_net_total: 0.0,
            logistic_weights: None,
            trend_logistic_selected: 0,
            trend_logistic_correct: 0,
            trend_logistic_positive: 0,
            trend_logistic_net_total: 0.0,
            trend_logistic_confident_selected: [0usize; 3],
            trend_logistic_confident_correct: [0usize; 3],
            trend_logistic_confident_positive: [0usize; 3],
            trend_logistic_confident_net: [0.0f64; 3],
            contrarian_selected: 0,
            contrarian_correct: 0,
            contrarian_positive: 0,
            contrarian_net_total: 0.0,
            contrarian_confident_selected: [0usize; 3],
            contrarian_confident_correct: [0usize; 3],
            contrarian_confident_positive: [0usize; 3],
            contrarian_confident_net: [0.0f64; 3],
            exploration_selected: [0usize; 5],
            exploration_correct: [0usize; 5],
            exploration_positive: [0usize; 5],
            exploration_net: [0.0f64; 5],
            trend_logistic_weights: None,
            research_logistic_weights: None,
        }
    }
}

fn build_walk_forward_result(
    symbol: String,
    horizon_seconds: u64,
    metrics: WalkForwardMetrics,
    samples: &[SignalOutcome],
    evaluations: &HashMap<(String, u64, u64), SignalEvaluation>,
) -> SignalWalkForward {
    let WalkForwardMetrics {
        max_training_samples,
        evaluated_signals,
        selected_signals,
        correct,
        net_total,
        positive_net,
        signal_long,
        signal_short,
        all_signal_long,
        all_signal_short,
        logistic_long,
        logistic_short,
        research_logistic_long,
        research_logistic_short,
        baseline_correct,
        baseline_net_total,
        five_minute_trend_samples,
        five_minute_trend_correct,
        five_minute_trend_net_total,
        last_threshold,
        logistic_selected,
        logistic_correct,
        logistic_positive,
        logistic_net_total,
        trend_logistic_selected,
        trend_logistic_correct,
        trend_logistic_positive,
        trend_logistic_net_total,
        trend_logistic_confident_selected,
        trend_logistic_confident_correct,
        trend_logistic_confident_positive,
        trend_logistic_confident_net,
        contrarian_selected,
        contrarian_correct,
        contrarian_positive,
        contrarian_net_total,
        contrarian_confident_selected,
        contrarian_confident_correct,
        contrarian_confident_positive,
        contrarian_confident_net,
        exploration_selected,
        exploration_correct,
        exploration_positive,
        exploration_net,
        ..
    } = metrics;
    let non_overlapping = samples;
    SignalWalkForward {
                symbol,
                horizon_seconds,
                training_samples: max_training_samples,
                evaluated_signals,
                selected_signals,
                selected_strength_threshold: last_threshold,
                direction_match_rate: (selected_signals > 0)
                    .then_some(correct as f64 / selected_signals as f64),
                average_net_return_percent: (selected_signals > 0)
                    .then_some(net_total / selected_signals as f64),
                positive_net_rate: (selected_signals > 0)
                    .then_some(positive_net as f64 / selected_signals as f64),
                baseline_direction_match_rate: (evaluated_signals > 0)
                    .then_some(baseline_correct as f64 / evaluated_signals as f64),
                baseline_average_net_return_percent: (evaluated_signals > 0)
                    .then_some(baseline_net_total / evaluated_signals as f64),
                five_minute_trend_sample_count: five_minute_trend_samples,
                five_minute_trend_match_rate: (five_minute_trend_samples > 0)
                    .then_some(five_minute_trend_correct as f64 / five_minute_trend_samples as f64),
                five_minute_trend_average_net_return_percent: (five_minute_trend_samples > 0)
                    .then_some(five_minute_trend_net_total / five_minute_trend_samples as f64),
                logistic_direction_match_rate: (logistic_selected > 0)
                    .then_some(logistic_correct as f64 / logistic_selected as f64),
                logistic_average_net_return_percent: (logistic_selected > 0)
                    .then_some(logistic_net_total / logistic_selected as f64),
                logistic_positive_net_rate: (logistic_selected > 0)
                    .then_some(logistic_positive as f64 / logistic_selected as f64),
                logistic_selected_signals: logistic_selected,
                trend_logistic_direction_match_rate: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_correct as f64 / trend_logistic_selected as f64),
                trend_logistic_average_net_return_percent: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_net_total / trend_logistic_selected as f64),
                trend_logistic_positive_net_rate: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_positive as f64 / trend_logistic_selected as f64),
                trend_logistic_selected_signals: trend_logistic_selected,
                contrarian_trend_logistic_direction_match_rate: (contrarian_selected > 0)
                    .then_some(contrarian_correct as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_average_net_return_percent: (contrarian_selected > 0)
                    .then_some(contrarian_net_total / contrarian_selected as f64),
                contrarian_trend_logistic_positive_net_rate: (contrarian_selected > 0)
                    .then_some(contrarian_positive as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_selected_signals: contrarian_selected,
                contrarian_trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = contrarian_confident_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0).then_some(
                                contrarian_confident_correct[index] as f64 / selected as f64,
                            ),
                            average_net_return_percent: (selected > 0)
                                .then_some(contrarian_confident_net[index] / selected as f64),
                            positive_net_rate: (selected > 0).then_some(
                                contrarian_confident_positive[index] as f64 / selected as f64,
                            ),
                        }
                    })
                    .collect(),
                exploration_score_bands: EXPLORATION_SCORE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = exploration_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0)
                                .then_some(exploration_correct[index] as f64 / selected as f64),
                            average_net_return_percent: (selected > 0)
                                .then_some(exploration_net[index] / selected as f64),
                            positive_net_rate: (selected > 0)
                                .then_some(exploration_positive[index] as f64 / selected as f64),
                        }
                    })
                    .collect(),
                trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = trend_logistic_confident_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0).then_some(
                                trend_logistic_confident_correct[index] as f64 / selected as f64,
                            ),
                            average_net_return_percent: (selected > 0)
                                .then_some(trend_logistic_confident_net[index] / selected as f64),
                            positive_net_rate: (selected > 0).then_some(
                                trend_logistic_confident_positive[index] as f64 / selected as f64,
                            ),
                        }
                    })
                    .collect(),
                signal_long: signal_long.metrics(),
                signal_short: signal_short.metrics(),
                all_signal_long: all_signal_long.metrics(),
                all_signal_short: all_signal_short.metrics(),
                logistic_long: logistic_long.metrics(),
                logistic_short: logistic_short.metrics(),
                research_logistic_long: research_logistic_long.metrics(),
                research_logistic_short: research_logistic_short.metrics(),
                learned_long_exit_policy: fit_learned_exit_policy(
                    &non_overlapping,
                    &evaluations,
                    "bullish",
                ),
                learned_short_exit_policy: fit_learned_exit_policy(
                    &non_overlapping,
                    &evaluations,
                    "bearish",
                ),
    }
}
