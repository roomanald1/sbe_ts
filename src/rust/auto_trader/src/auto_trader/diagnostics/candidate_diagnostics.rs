fn candidate_entry_diagnostic(
    candidate: &Signal,
    dashboard: &Dashboard,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
) -> String {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning,
        risk_appetite,
        active_scout_count,
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
    selection_mode: &str,
) -> serde_json::Value {
    let review = review_candidate_entry(
        candidate,
        dashboard,
        learning,
        risk_appetite,
        active_scout_count,
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
            "scout_inventory_available": active_scout_count < SCOUT_MAX_ACTIVE_POSITIONS,
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
    if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS {
        failures.push(format!(
            "scout inventory cap reached ({active_scout_count}/{SCOUT_MAX_ACTIVE_POSITIONS} open scout positions)"
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
    } else if review.scout_negative && active_scout_count > 0 {
        failures.push(format_scout_wait_reason(review));
    }
}

fn format_scout_wait_reason(review: &CandidateEntryReview<'_>) -> String {
    let samples = review.side.map_or(0, |metrics| metrics.sample_count);
    let net_return = review
        .side
        .and_then(|metrics| metrics.average_net_return_percent)
        .map_or_else(|| "n/a".into(), |net| format!("{net:+.3}%"));
    let positive_rate = review
        .side
        .and_then(|metrics| metrics.positive_net_rate)
        .map_or_else(|| "n/a".into(), |rate| format!("{:.1}%", rate * 100.0));
    format!(
        "measured negative-history scout probe waits for zero open scouts ({samples}; net {net_return}; positive {positive_rate})"
    )
}
