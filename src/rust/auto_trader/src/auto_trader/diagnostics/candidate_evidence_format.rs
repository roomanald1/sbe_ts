fn format_candidate_evidence(review: &CandidateEntryReview<'_>) -> (String, String) {
    let Some(evidence) = review.evidence else {
        return (
            "walk-forward evidence unavailable".into(),
            "research-model evidence unavailable".into(),
        );
    };
    let walk_forward = review.side.map_or_else(
        || "walk-forward evidence unavailable".into(),
        |metrics| {
            format!(
                "walk-forward {}s n={} hit={} net={} positive={} lower95={}",
                evidence.horizon_seconds,
                metrics.sample_count,
                format_rate(metrics.direction_match_rate),
                format_net_return(metrics.average_net_return_percent),
                format_rate(metrics.positive_net_rate),
                format_rate(metrics.positive_net_rate_lower_95),
            )
        },
    );
    let research = review.research.map_or_else(
        || "research-model evidence unavailable".into(),
        |metrics| {
            format!(
                "research-model n={} net={}",
                metrics.sample_count,
                format_net_return(metrics.average_net_return_percent),
            )
        },
    );
    (walk_forward, research)
}

fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "n/a".into(), |value| format!("{:.1}%", value * 100.0))
}

fn format_net_return(net_return: Option<f64>) -> String {
    net_return.map_or_else(|| "n/a".into(), |value| format!("{value:+.3}%"))
}

#[cfg(test)]
mod candidate_evidence_format_tests {
    use super::*;

    #[test]
    fn renders_present_and_missing_walk_forward_metrics() {
        let evidence: WalkForward = serde_json::from_value(serde_json::json!({
            "symbol": "BTC-USDT",
            "horizon_seconds": 300
        }))
        .unwrap();
        let side = SideWalkForwardMetrics {
            sample_count: 20,
            direction_match_rate: Some(0.75),
            average_net_return_percent: Some(0.125),
            positive_net_rate: Some(0.6),
            positive_net_rate_lower_95: None,
        };
        let research = SideWalkForwardMetrics {
            sample_count: 8,
            average_net_return_percent: None,
            ..SideWalkForwardMetrics::default()
        };
        let review = CandidateEntryReview {
            context: "test".into(),
            strength_floor: 0.0, score_floor: 0.0,
            evidence: Some(&evidence), side: Some(&side), research: Some(&research),
            strength_weak: false, score_weak: false, mixed_context: false,
            evidence_missing: false, evidence_bad: false, research_bad: false,
            strict_evidence_bad: false,
        };
        let (walk_forward, research_text) = format_candidate_evidence(&review);
        assert_eq!(walk_forward, "walk-forward 300s n=20 hit=75.0% net=+0.125% positive=60.0% lower95=n/a");
        assert_eq!(research_text, "research-model n=8 net=n/a");

        let missing = CandidateEntryReview { evidence: None, ..review };
        assert_eq!(format_candidate_evidence(&missing), (
            "walk-forward evidence unavailable".into(),
            "research-model evidence unavailable".into(),
        ));
    }
}
