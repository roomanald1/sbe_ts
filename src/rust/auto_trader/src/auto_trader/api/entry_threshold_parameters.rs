fn signal_qualification_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Signal qualification",
            "Maximum signal age",
            MAX_SIGNAL_AGE_SECONDS as f64,
            "seconds",
        ),
        parameter(
            "Signal qualification",
            "Neutral score direction floor",
            NEUTRAL_SCORE_DIRECTION_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Neutral inference maximum range probability",
            NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY,
            "probability",
        ),
        parameter(
            "Signal qualification",
            "Base strict strength floor",
            MIN_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Strict strength floor after poor learning",
            LEARNED_STRENGTH_FLOOR * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Samples before strong negative-evidence adjustment",
            STRONG_NEGATIVE_LEARNING_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Positive rate boundary for strong negative evidence",
            STRONG_NEGATIVE_LEARNING_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Strict strength floor after strong negative evidence",
            STRONG_NEGATIVE_STRENGTH_FLOOR * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Samples before strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Positive rate triggering strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Minimum exploration strength",
            MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Base exploration score floor",
            MIN_EXPLORATION_SCORE * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 10 weak samples",
            EXPLORATION_SCORE_MID_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 20 weak samples",
            EXPLORATION_SCORE_HIGH_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after strong negative evidence",
            STRONG_NEGATIVE_SCORE_FLOOR * risk_floor_scale(risk_appetite),
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 10-sample adjustment",
            EXPLORATION_SCORE_MID_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 20-sample adjustment",
            EXPLORATION_SCORE_HIGH_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration adjustment positive-rate boundaries",
            EXPLORATION_SCORE_MID_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Exploration stronger adjustment boundary",
            EXPLORATION_SCORE_HIGH_WIN_RATE,
            "rate",
        ),
    ]
}

fn scout_qualification_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![parameter(
        "Scout qualification",
        "Neutral scout score floor",
        SCOUT_NEUTRAL_SCORE_FLOOR * risk_floor_scale(risk_appetite),
        "score",
    )]
}

fn historical_evidence_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Historical evidence",
            "Minimum strict walk-forward samples",
            MIN_SIDE_WALK_FORWARD_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum direction match rate",
            MIN_SIDE_DIRECTION_MATCH_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum positive net rate",
            MIN_SIDE_POSITIVE_NET_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum 95% positive-rate lower bound",
            MIN_SIDE_POSITIVE_NET_LOWER_95,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration evidence samples",
            MIN_EXPLORATION_EVIDENCE_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration positive rate",
            NEGATIVE_EVIDENCE_FLOOR,
            "rate",
        ),
    ]
}

fn counterfactual_relaxation_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Counterfactual relaxation",
            "Minimum resolved stop/target samples",
            COUNTERFACTUAL_RELAXATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum positive stop/target rate",
            COUNTERFACTUAL_RELAXATION_POSITIVE_RATE,
            "rate",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed strength threshold",
            COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD,
            "strength",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed score threshold",
            COUNTERFACTUAL_SCORE_MIN_THRESHOLD,
            "score",
        ),
    ]
}

