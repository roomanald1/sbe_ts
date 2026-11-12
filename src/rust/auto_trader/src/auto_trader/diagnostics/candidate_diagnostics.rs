#[cfg(test)]
static EMPTY_CONTEXT_THRESHOLDS: std::sync::LazyLock<BTreeMap<String, strategy::ContextThresholds>> =
    std::sync::LazyLock::new(BTreeMap::new);

fn candidate_entry_diagnostic(
    candidate: &Signal,
    dashboard: &Dashboard,
    learning_thresholds: &BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
    _now_ms: u64,
) -> String {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning_thresholds,
        risk_appetite,
    );
    let failures = candidate_entry_failures(candidate, &review);
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
    learning_thresholds: &BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
    selection_mode: &str,
    _now_ms: u64,
) -> serde_json::Value {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning_thresholds,
        risk_appetite,
    );
    let failures = if selection_mode == "rejected" {
        candidate_entry_failures(candidate, &review)
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
        },
        "checks": {
            "strategy_walk_forward_evidence_present": review.strategy_evidence_present,
            "strength_pass": !review.strength_weak,
            "exploration_score_pass": !review.score_weak,
            "exploration_context_pass": !review.mixed_context,
        },
        "rejection_reasons": failures,
    })
}

fn candidate_entry_failures(
    candidate: &Signal,
    review: &CandidateEntryReview,
) -> Vec<String> {
    let mut failures = Vec::new();
    add_strength_failure(candidate, review, &mut failures);
    add_score_failure(candidate, review, &mut failures);
    failures
}

fn add_strength_failure(
    candidate: &Signal,
    review: &CandidateEntryReview,
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
    review: &CandidateEntryReview,
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
            "best": []
        })).unwrap()
    }

    #[test]
    fn rejected_candidate_diagnostic_and_audit_explain_strength_and_score() {
        let candidate = candidate(0.0, 0.0);
        let dashboard = Dashboard {
            best: Vec::new(),
            missed_opportunity_bands: Arc::new(Vec::new()), markets: Arc::new(BTreeMap::new()),
            queued_at: None, queue_wait_ms: None,
        };
        let diagnostic = candidate_entry_diagnostic(&candidate, &dashboard, &EMPTY_CONTEXT_THRESHOLDS, 50.0, 1_000);
        assert!(diagnostic.contains("BTC-USDT bullish"));
        assert!(diagnostic.contains("strength 0.00 <"));
        assert!(diagnostic.contains("score 0.00 <"));
        let audit = candidate_entry_audit_value(&candidate, &dashboard, &EMPTY_CONTEXT_THRESHOLDS, 50.0, "rejected", 1_000);
        assert_eq!(audit["selection_mode"], "rejected");
        assert!(audit["rejection_reasons"].as_array().unwrap().iter().any(|reason| reason.as_str().unwrap().contains("strength")));
    }

    #[test]
    fn supported_candidate_passes_strict_checks_and_audit_omits_rejections() {
        let candidate = candidate(0.9, 0.9);
        let dashboard = dashboard_with_evidence();
        let failures = candidate_entry_failures(&candidate, &review_candidate_entry(&candidate, &dashboard, &EMPTY_CONTEXT_THRESHOLDS, 50.0));
        assert!(failures.is_empty(), "{failures:?}");
        let audit = candidate_entry_audit_value(&candidate, &dashboard, &EMPTY_CONTEXT_THRESHOLDS, 50.0, "strict", 1_000);
        assert_eq!(audit["checks"]["strategy_walk_forward_evidence_present"], false);
        assert!(audit["rejection_reasons"].as_array().unwrap().is_empty());
        assert!(candidate_entry_diagnostic(&candidate, &dashboard, &EMPTY_CONTEXT_THRESHOLDS, 50.0, 1_000).contains("did not pass combined entry checks"));
    }
}
