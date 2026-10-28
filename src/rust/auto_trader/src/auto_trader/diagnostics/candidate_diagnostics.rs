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

