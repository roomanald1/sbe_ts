struct CandidateEntryReview {
    context: String,
    strength_floor: f64,
    score_floor: f64,
    strategy_evidence_present: bool,
    strength_weak: bool,
    score_weak: bool,
    mixed_context: bool,
}

fn review_candidate_entry(
    candidate: &Signal,
    dashboard: &Dashboard,
    learning_thresholds: &BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
) -> CandidateEntryReview {
    let context = market_context(candidate);
    let risk_scale = risk_floor_scale(risk_appetite);
    let learned = learning_thresholds.get(&context);
    let learned_strength = learned
        .map(|thresholds| thresholds.strength_floor)
        .unwrap_or(MIN_STRENGTH);
    let learned_score = learned
        .map(|thresholds| thresholds.exploration_score_floor)
        .unwrap_or(MIN_EXPLORATION_SCORE);
    let strength_floor = (adaptive_missed_opportunity_floor_for_signal(
        learned_strength,
        MissedOpportunityGate::Strength,
        &dashboard.missed_opportunity_bands,
        &candidate.symbol,
        &candidate.direction,
        candidate.strategy.as_deref(),
        context.split(':').nth(1).unwrap_or(&context),
    ) * risk_scale)
        .clamp(0.0, 1.0);
    let score_floor = (adaptive_missed_opportunity_floor_for_signal(
        learned_score,
        MissedOpportunityGate::Score,
        &dashboard.missed_opportunity_bands,
        &candidate.symbol,
        &candidate.direction,
        candidate.strategy.as_deref(),
        context.split(':').nth(1).unwrap_or(&context),
    ) * risk_scale)
        .max(0.0);
    let strength_weak = candidate.strength < strength_floor;
    let mixed_context = is_mixed_market_context(&context);
    let score_weak = candidate.score.abs() < score_floor;
    CandidateEntryReview {
        context,
        strength_floor,
        score_floor,
        strategy_evidence_present: candidate
            .strategy_decision
            .as_ref()
            .is_some_and(|decision| decision.get("strategy_walk_forward").is_some()),
        strength_weak,
        score_weak,
        mixed_context,
    }
}
