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
