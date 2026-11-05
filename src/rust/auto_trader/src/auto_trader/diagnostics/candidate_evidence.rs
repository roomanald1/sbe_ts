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

fn is_live_scout_only_signal(candidate: &Signal) -> bool {
    candidate
        .strategy_decision
        .as_ref()
        .and_then(|value| value.get("scout_only"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && candidate.strategy.as_deref().and_then(Strategy::from_name)
            .is_some_and(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
}

#[cfg(test)]
mod candidate_evidence_tests {
    use super::*;

    fn candidate(direction: &str) -> Signal {
        serde_json::from_value(serde_json::json!({
            "symbol": "BTC-USDT",
            "direction": direction,
            "strength": 1.0,
            "timestamp": 1,
            "stale": false
        }))
        .unwrap()
    }

    #[test]
    fn selects_matching_entry_horizon_and_checks_directional_evidence() {
        let dashboard: Dashboard = serde_json::from_value(serde_json::json!({
            "best": [],
            "walkForward": [{
                "symbol": "BTC-USDT",
                "horizon_seconds": 300,
                "all_signal_long": {
                    "sample_count": MIN_EXPLORATION_EVIDENCE_SAMPLES,
                    "average_net_return_percent": 0.0,
                    "positive_net_rate": 0.7
                },
                "research_logistic_long": {
                    "sample_count": 10,
                    "average_net_return_percent": 0.2
                }
            }]
        }))
        .unwrap();
        let bullish = candidate("bullish");
        let evidence = candidate_evidence(&bullish, &dashboard).unwrap();
        assert_eq!(evidence.horizon_seconds, 300);
        assert_eq!(side_metrics(&bullish, evidence).sample_count, MIN_EXPLORATION_EVIDENCE_SAMPLES);
        assert_eq!(research_metrics(&bullish, evidence).sample_count, 10);
        assert!(has_negative_exploration_evidence(side_metrics(&bullish, evidence)));
        assert_eq!(count_negative_evidence_candidates(&[&bullish], &dashboard), 1);

        let bearish = candidate("bearish");
        assert_eq!(side_metrics(&bearish, evidence).sample_count, 0);
        assert_eq!(count_negative_evidence_candidates(&[&bearish], &dashboard), 0);
        let unrelated = candidate("bullish");
        assert_eq!(count_negative_evidence_candidates(&[&unrelated], &Dashboard {
            best: Vec::new(), walk_forward: Arc::new(Vec::new()),
            missed_opportunity_bands: Vec::new(), markets: Arc::new(BTreeMap::new()),
            queued_at: None, queue_wait_ms: None,
        }), 0);
    }
}
