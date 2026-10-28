struct CandidateEntryReview<'a> {
    context: String,
    strength_floor: f64,
    score_floor: f64,
    scout_strength_floor: f64,
    scout_score_floor: f64,
    evidence: Option<&'a WalkForward>,
    side: Option<&'a SideWalkForwardMetrics>,
    research: Option<&'a SideWalkForwardMetrics>,
    strength_weak: bool,
    score_weak: bool,
    mixed_context: bool,
    evidence_missing: bool,
    evidence_bad: bool,
    research_bad: bool,
    strict_evidence_bad: bool,
    scout_negative: bool,
    scout_strength_pass: bool,
    scout_score_pass: bool,
}

fn review_candidate_entry<'a>(
    candidate: &Signal,
    dashboard: &'a Dashboard,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
) -> CandidateEntryReview<'a> {
    let context = market_context(candidate);
    let risk_scale = risk_floor_scale(risk_appetite);
    let strength_floor = (adaptive_missed_opportunity_floor(
        adaptive_strength_floor(learning, &context),
        "strength",
        &dashboard.missed_opportunity_bands,
    ) * risk_scale)
        .clamp(0.0, 1.0);
    let score_floor = (adaptive_exploration_score_floor(
        learning,
        &context,
        &dashboard.missed_opportunity_bands,
    ) * risk_scale)
        .max(0.0);
    let evidence = candidate_evidence(candidate, dashboard);
    let side = evidence.map(|item| side_metrics(candidate, item));
    let research = evidence.map(|item| research_metrics(candidate, item));
    let strength_weak = candidate.strength < strength_floor;
    let mixed_context = is_mixed_market_context(&context);
    let score_weak = candidate.score.abs() < score_floor;
    let exploration_qualified = !mixed_context
        && candidate.strength.is_finite()
        && candidate.strength >= MIN_EXPLORATION_STRENGTH * risk_scale
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
    let (scout_strength, scout_score) =
        adaptive_scout_thresholds(learning, &context, active_scout_count);
    let scout_strength_floor = (scout_strength * risk_scale).clamp(0.0, 1.0);
    let scout_score_floor = (scout_score * risk_scale).max(0.0);
    let scout_strength_pass = candidate.strength.is_finite()
        && candidate.strength >= scout_strength_floor;
    let scout_score_pass = candidate.score.is_finite()
        && candidate.score.abs() >= scout_score_floor;
    let scout_negative = side.is_some_and(has_negative_exploration_evidence)
        || research.is_some_and(|metrics| {
            metrics.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                && metrics.average_net_return_percent.is_none_or(|net| net <= 0.0)
        });

    CandidateEntryReview {
        context,
        strength_floor,
        score_floor,
        scout_strength_floor,
        scout_score_floor,
        evidence,
        side,
        research,
        strength_weak,
        score_weak,
        mixed_context,
        evidence_missing,
        evidence_bad,
        research_bad,
        strict_evidence_bad,
        scout_negative,
        scout_strength_pass,
        scout_score_pass,
    }
}

