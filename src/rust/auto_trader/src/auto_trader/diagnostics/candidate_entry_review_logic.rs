struct CandidateEntryReview<'a> {
    context: String,
    strength_floor: f64,
    score_floor: f64,
    evidence: Option<&'a WalkForward>,
    stale_evidence_age_ms: Option<u64>,
    side: Option<&'a SideWalkForwardMetrics>,
    research: Option<&'a SideWalkForwardMetrics>,
    strength_weak: bool,
    score_weak: bool,
    mixed_context: bool,
    evidence_missing: bool,
    evidence_bad: bool,
    research_bad: bool,
    strict_evidence_bad: bool,
}

fn review_candidate_entry<'a>(
    candidate: &Signal,
    dashboard: &'a Dashboard,
    learning_thresholds: &BTreeMap<String, strategies::ContextThresholds>,
    risk_appetite: f64,
    now_ms: u64,
) -> CandidateEntryReview<'a> {
    let context = market_context(candidate);
    let risk_scale = risk_floor_scale(risk_appetite);
    let learned = learning_thresholds.get(&context);
    let learned_strength = learned.map(|thresholds| thresholds.strength_floor).unwrap_or(MIN_STRENGTH);
    let learned_score = learned.map(|thresholds| thresholds.exploration_score_floor).unwrap_or(MIN_EXPLORATION_SCORE);
    let strength_floor = (adaptive_missed_opportunity_floor(
        learned_strength,
        MissedOpportunityGate::Strength,
        &dashboard.missed_opportunity_bands,
    ) * risk_scale)
        .clamp(0.0, 1.0);
    let score_floor = (learned_score * risk_scale)
        .max(0.0);
    let evidence = candidate_evidence(candidate, dashboard, now_ms);
    let stale_evidence_age_ms = dashboard
        .walk_forward
        .iter()
        .filter(|item| {
            item.symbol == candidate.symbol
                && is_entry_evidence_horizon(item.horizon_seconds)
                && item.evidence_generated_at_ms > 0
                && item.evidence_generated_at_ms <= now_ms
        })
        .map(|item| now_ms - item.evidence_generated_at_ms)
        .filter(|age| *age > MAX_WALK_FORWARD_EVIDENCE_AGE_MS)
        .min();
    let side = evidence.map(|item| side_metrics(candidate, item));
    let research = evidence.map(|item| research_metrics(candidate, item));
    let strength_weak = candidate.strength < strength_floor;
    let mixed_context = is_mixed_market_context(&context);
    let score_weak = candidate.score.abs() < score_floor;
    let exploration_qualified = !mixed_context
        && candidate.strength.is_finite()
        && candidate.strength
            >= learned_strength.max(MIN_EXPLORATION_STRENGTH)
                * risk_scale
        && candidate.score.is_finite()
        && candidate.score.abs() >= score_floor;
    let evidence_missing = !strength_weak
        && side.is_none_or(|metrics| metrics.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES);
    let evidence_bad = exploration_qualified && side.is_some_and(has_negative_exploration_evidence);
    let research_bad = (!strength_weak || exploration_qualified)
        && research.is_some_and(|metrics| {
            metrics.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                && metrics.average_net_return_percent.is_none_or(|net| net <= 0.0)
        });
    let strict_evidence_bad = !strength_weak && side.is_some_and(|metrics| {
        metrics.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
            && (metrics
                .direction_match_rate
                .is_none_or(|rate| rate < MIN_SIDE_DIRECTION_MATCH_RATE)
                || metrics.average_net_return_percent.is_none_or(|net| net <= 0.0)
                || metrics
                    .positive_net_rate
                    .is_none_or(|rate| rate < MIN_SIDE_POSITIVE_NET_RATE)
                || metrics
                    .positive_net_rate_lower_95
                    .is_none_or(|rate| rate < MIN_SIDE_POSITIVE_NET_LOWER_95))
    });
    CandidateEntryReview {
        context,
        strength_floor,
        score_floor,
        evidence,
        stale_evidence_age_ms,
        side,
        research,
        strength_weak,
        score_weak,
        mixed_context,
        evidence_missing,
        evidence_bad,
        research_bad,
        strict_evidence_bad,
    }
}

