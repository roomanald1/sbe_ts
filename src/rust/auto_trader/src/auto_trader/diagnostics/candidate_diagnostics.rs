fn candidate_entry_diagnostic(
    candidate: &Signal,
    dashboard: &Dashboard,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
    scout_max_active_positions: usize,
) -> String {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning,
        risk_appetite,
        active_scout_count,
        scout_max_active_positions,
    );
    let failures = candidate_entry_failures(candidate, &review, active_scout_count);
    let (walk_forward, research) = format_candidate_evidence(&review);
    let outcome = if failures.is_empty() {
        "did not pass combined entry checks".to_owned()
    } else {
        failures.join(", ")
    };
    format!(
        "{} {} · score {:.2} · strength {:.2} · context {} · {} · {} · {}",
        candidate.symbol,
        candidate.direction,
        candidate.score,
        candidate.strength,
        review.context,
        walk_forward,
        research,
        outcome,
    )
}

fn candidate_entry_audit_value(
    candidate: &Signal,
    dashboard: &Dashboard,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
    scout_max_active_positions: usize,
    selection_mode: &str,
) -> serde_json::Value {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning,
        risk_appetite,
        active_scout_count,
        scout_max_active_positions,
    );
    let failures = if selection_mode == "rejected" {
        candidate_entry_failures(candidate, &review, active_scout_count)
    } else {
        Vec::new()
    };
    serde_json::json!({
        "selection_mode": selection_mode,
        "market_context": review.context,
        "observed": {
            "score": candidate.score,
            "absolute_score": candidate.score.abs(),
            "strength": candidate.strength,
        },
        "thresholds": {
            "strength_floor": review.strength_floor,
            "exploration_score_absolute_floor": review.score_floor,
            "scout_strength_floor": review.scout_strength_floor,
            "scout_score_absolute_floor": review.scout_score_floor,
            "minimum_side_walk_forward_samples": MIN_SIDE_WALK_FORWARD_SAMPLES,
            "minimum_exploration_evidence_samples": MIN_EXPLORATION_EVIDENCE_SAMPLES,
            "scout_minimum_walk_forward_samples": review.scout_minimum_samples,
            "scout_minimum_positive_net_rate": review.scout_minimum_positive_rate,
            "scout_outcomes": {
                "samples": review.scout_outcome_samples,
                "positive_rate": review.scout_outcome_positive_rate,
                "realized_pnl_usd": review.scout_outcome_realized_pnl,
            },
            "minimum_side_direction_match_rate": MIN_SIDE_DIRECTION_MATCH_RATE,
            "minimum_side_positive_net_rate": MIN_SIDE_POSITIVE_NET_RATE,
            "minimum_side_positive_net_lower_95": MIN_SIDE_POSITIVE_NET_LOWER_95,
        },
        "checks": {
            "strength_pass": !review.strength_weak,
            "exploration_score_pass": !review.score_weak,
            "exploration_context_pass": !review.mixed_context,
            "strict_walk_forward_pass": !review.strict_evidence_bad && !review.evidence_missing && !review.research_bad,
            "scout_strength_pass": review.scout_strength_pass,
            "scout_score_pass": review.scout_score_pass,
            "scout_inventory_available": active_scout_count < scout_max_active_positions,
            "scout_walk_forward_thresholds_valid": review.scout_thresholds_valid,
        },
        "rejection_reasons": failures,
    })
}

fn candidate_entry_failures(
    candidate: &Signal,
    review: &CandidateEntryReview<'_>,
    active_scout_count: usize,
) -> Vec<String> {
    let mut failures = Vec::new();
    add_strength_failure(candidate, review, &mut failures);
    add_score_failure(candidate, review, &mut failures);
    add_evidence_failures(review, &mut failures);
    add_scout_failures(candidate, review, active_scout_count, &mut failures);
    failures
}

fn add_strength_failure(
    candidate: &Signal,
    review: &CandidateEntryReview<'_>,
    failures: &mut Vec<String>,
) {
    if review.strength_weak {
        failures.push(format!(
            "strength {:.2} < {:.2}",
            candidate.strength, review.strength_floor
        ));
    }
}

fn add_score_failure(
    candidate: &Signal,
    review: &CandidateEntryReview<'_>,
    failures: &mut Vec<String>,
) {
    if review.mixed_context {
        failures.push("normal exploration blocked for mixed context".into());
    } else if review.score_weak {
        failures.push(format!(
            "score {:.2} < {:.2}",
            candidate.score.abs(), review.score_floor
        ));
    }
}

fn add_evidence_failures(
    review: &CandidateEntryReview<'_>,
    failures: &mut Vec<String>,
) {
    if review.evidence_bad {
        failures.push(format!(
            "exploration needs net return > 0 and positive rate ≥ {:.0}%",
            NEGATIVE_EVIDENCE_FLOOR * 100.0
        ));
    }
    if review.evidence_missing {
        failures.push(format!(
            "normal entry needs {} side samples",
            MIN_SIDE_WALK_FORWARD_SAMPLES
        ));
    }
    if review.strict_evidence_bad {
        failures.push(format!(
            "normal entry needs hit ≥ {:.0}%, net > 0, positive rate ≥ {:.0}%, lower-95 ≥ {:.0}%",
            MIN_SIDE_DIRECTION_MATCH_RATE * 100.0,
            MIN_SIDE_POSITIVE_NET_RATE * 100.0,
            MIN_SIDE_POSITIVE_NET_LOWER_95 * 100.0
        ));
    }
    if review.research_bad {
        failures.push("research-model side return is not positive".into());
    }
}

fn add_scout_failures(
    candidate: &Signal,
    review: &CandidateEntryReview<'_>,
    active_scout_count: usize,
    failures: &mut Vec<String>,
) {
    if !review.scout_thresholds_valid {
        failures.push("required learned Scout walk-forward thresholds are missing or invalid".into());
    } else if active_scout_count >= review.scout_max_active_positions {
        failures.push(format!(
            "scout inventory cap reached ({active_scout_count}/{} open scout positions)",
            review.scout_max_active_positions
        ));
    } else if !review.scout_strength_pass {
        failures.push(format!(
            "scout strength {:.2} < learned {:.2}",
            candidate.strength, review.scout_strength_floor
        ));
    } else if !review.scout_score_pass {
        failures.push(format!(
            "scout |score| {:.2} < learned {:.2}",
            candidate.score.abs(), review.scout_score_floor
        ));
    }
}

#[cfg(test)]
mod candidate_diagnostic_tests {
    use super::*;

    fn candidate(strength: f64, score: f64) -> Signal {
        serde_json::from_value(serde_json::json!({
            "symbol": "BTC-USDT", "direction": "bullish", "strength": strength,
            "score": score, "timestamp": 1, "stale": false, "strategy_candidate": true
        })).unwrap()
    }

    fn dashboard_with_evidence() -> Dashboard {
        serde_json::from_value(serde_json::json!({
            "best": [],
            "walkForward": [{
                "symbol": "BTC-USDT", "horizon_seconds": 300,
                "all_signal_long": {
                    "sample_count": 40, "direction_match_rate": 0.8,
                    "average_net_return_percent": 0.4, "positive_net_rate": 0.7,
                    "positive_net_rate_lower_95": 0.6
                },
                "research_logistic_long": {
                    "sample_count": 40, "average_net_return_percent": 0.3
                }
            }]
        })).unwrap()
    }

    #[test]
    fn rejected_candidate_diagnostic_and_audit_explain_strength_score_and_scout_inventory() {
        let candidate = candidate(0.0, 0.0);
        let dashboard = Dashboard {
            best: Vec::new(), walk_forward: Arc::new(Vec::new()),
            missed_opportunity_bands: Vec::new(), markets: Arc::new(BTreeMap::new()),
            queued_at: None, queue_wait_ms: None,
        };
        let learning = BTreeMap::new();
        let diagnostic = candidate_entry_diagnostic(&candidate, &dashboard, &learning, 50.0, 0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS);
        assert!(diagnostic.contains("BTC-USDT bullish"));
        assert!(diagnostic.contains("strength 0.00 <"));
        assert!(diagnostic.contains("score 0.00 <"));
        let audit = candidate_entry_audit_value(&candidate, &dashboard, &learning, 50.0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS, "rejected");
        assert_eq!(audit["selection_mode"], "rejected");
        assert_eq!(audit["checks"]["scout_inventory_available"], false);
        assert!(audit["rejection_reasons"].as_array().unwrap().iter().any(|reason| reason.as_str().unwrap().contains("scout inventory cap")));
    }

    #[test]
    fn supported_candidate_passes_strict_and_scout_checks_and_audit_omits_rejections() {
        let candidate = candidate(0.9, 0.9);
        let dashboard = dashboard_with_evidence();
        let learning = BTreeMap::new();
        let failures = candidate_entry_failures(&candidate, &review_candidate_entry(&candidate, &dashboard, &learning, 50.0, 0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS), 0);
        assert!(failures.is_empty(), "{failures:?}");
        let audit = candidate_entry_audit_value(&candidate, &dashboard, &learning, 50.0, 0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS, "strict");
        assert_eq!(audit["checks"]["strict_walk_forward_pass"], true);
        assert_eq!(audit["checks"]["scout_strength_pass"], true);
        assert!(audit["rejection_reasons"].as_array().unwrap().is_empty());
        assert!(candidate_entry_diagnostic(&candidate, &dashboard, &learning, 50.0, 0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS).contains("did not pass combined entry checks"));
    }
}
