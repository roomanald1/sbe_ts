fn format_candidate_evidence(review: &CandidateEntryReview) -> (String, String) {
    let strategy = if review.strategy_evidence_present {
        "Learning strategy/profile evidence attached"
    } else {
        "Learning strategy/profile evidence unavailable"
    };
    (strategy.into(), "raw-signal research evidence not used".into())
}
