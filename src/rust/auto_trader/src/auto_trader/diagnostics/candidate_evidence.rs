fn count_negative_evidence_candidates(
    candidates: &[&Signal],
    dashboard: &Dashboard,
) -> usize {
    candidates
        .iter()
        .filter(|candidate| {
            candidate_evidence(candidate, dashboard)
                .map(|evidence| side_metrics(candidate, evidence))
                .is_some_and(has_negative_exploration_evidence)
        })
        .count()
}

fn candidate_evidence<'a>(
    candidate: &Signal,
    dashboard: &'a Dashboard,
) -> Option<&'a WalkForward> {
    dashboard.walk_forward.iter().find(|item| {
        item.symbol == candidate.symbol && is_entry_evidence_horizon(item.horizon_seconds)
    })
}

fn side_metrics<'a>(candidate: &Signal, evidence: &'a WalkForward) -> &'a SideWalkForwardMetrics {
    if candidate.direction == "bullish" {
        &evidence.all_signal_long
    } else {
        &evidence.all_signal_short
    }
}

fn research_metrics<'a>(
    candidate: &Signal,
    evidence: &'a WalkForward,
) -> &'a SideWalkForwardMetrics {
    if candidate.direction == "bullish" {
        &evidence.research_logistic_long
    } else {
        &evidence.research_logistic_short
    }
}

fn has_negative_exploration_evidence(metrics: &SideWalkForwardMetrics) -> bool {
    metrics.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
        && (metrics.average_net_return_percent.is_none_or(|net| net <= 0.0)
            || metrics
                .positive_net_rate
                .is_none_or(|rate| rate < NEGATIVE_EVIDENCE_FLOOR))
}

