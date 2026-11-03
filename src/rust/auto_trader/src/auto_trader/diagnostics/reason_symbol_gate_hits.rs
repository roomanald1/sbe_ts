fn symbol_gate_hits_for_reason(reason: &str) -> Vec<(String, String, String, String, Option<f64>)> {
    let mut hits = BTreeMap::<(String, String, String), String>::new();
    if let Some(details) = reason
        .split("; early filters: ")
        .nth(1)
        .and_then(|part| part.split("; candidate checks: ").next())
    {
        for filter in details.split(" | ") {
            let Some((identity, gate_details)) = filter.split_once(" · ") else {
                continue;
            };
            let mut identity = identity.split_whitespace();
            let (Some(symbol), Some(direction)) = (identity.next(), identity.next()) else {
                continue;
            };
            let gate = if gate_details.starts_with("Signal freshness") {
                Some("Signal freshness")
            } else if gate_details.starts_with("Directional signal") {
                Some("Directional signal")
            } else if gate_details.starts_with("Existing position") {
                Some("Existing position / re-entry limit")
            } else {
                None
            };
            if let Some(gate) = gate {
                hits.insert(
                    (symbol.to_owned(), direction.to_owned(), gate.to_owned()),
                    gate_details.to_owned(),
                );
            }
        }
    }
    if let Some(details) = reason
        .split("; candidate checks: ")
        .nth(1)
        .and_then(|part| part.split("; strategy filters: ").next())
    {
        for candidate in details.split(" | ") {
            let Some((identity, checks)) = candidate.rsplit_once(" · ") else {
                continue;
            };
            let mut identity = identity.split_whitespace();
            let (Some(symbol), Some(direction)) = (identity.next(), identity.next()) else {
                continue;
            };
            let mut push = |gate: &str, detail: String| {
                hits.insert(
                    (symbol.to_owned(), direction.to_owned(), gate.to_owned()),
                    detail,
                );
            };
            if checks.contains("strength ") && checks.contains(" < ") {
                push(
                    "Learned strength floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("strength "))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("score ") && checks.contains(" < ") {
                push(
                    "Exploration score floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("score "))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("exploration blocked for mixed context") {
                push(
                    "Exploration disabled for mixed context",
                    format!("{candidate}; failed: mixed market context disables exploration"),
                );
            }
            if checks.contains("exploration needs") {
                push(
                    "Exploration walk-forward return and positive-rate floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("exploration needs"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("normal entry needs") && checks.contains("side samples") {
                push(
                    "Strict walk-forward sample minimum",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("normal entry needs")
                                && check.contains("side samples"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("normal entry needs") && !checks.contains("side samples") {
                push(
                    "Strict walk-forward quality thresholds",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("normal entry needs")
                                && !check.contains("side samples"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("research-model side return") {
                push(
                    "Research-model side evidence",
                    format!("{candidate}; failed: research-model side return is not positive"),
                );
            }
        }
    }
    if let Some(details) = reason.split("; strategy filters: ").nth(1) {
        for filter in details.split(" | ") {
            let Some((identity, detail)) = filter.split_once(" · ") else {
                continue;
            };
            let mut identity = identity.split_whitespace();
            let (Some(symbol), Some(direction)) = (identity.next(), identity.next()) else {
                continue;
            };
            let gate = if detail.starts_with("Strategy disabled") {
                "Strategy enablement"
            } else if detail.starts_with("Strategy evaluation error") {
                "Strategy evaluation"
            } else {
                "Strategy entry rule"
            };
            hits.insert(
                (symbol.to_owned(), direction.to_owned(), gate.to_owned()),
                detail.to_owned(),
            );
        }
    }
    hits.into_iter()
        .map(|((symbol, direction, gate), detail)| {
            let distance = gate_distance_to_pass(&gate, &detail);
            (symbol, direction, gate, detail, distance)
        })
        .collect()
}


#[cfg(test)]
mod reason_symbol_gate_hits_tests {
    use super::symbol_gate_hits_for_reason;

    #[test]
    fn extracts_early_candidate_and_strategy_gate_details_with_distances() {
        let reason = "No tradeable bias; early filters: ETH-USDT bullish · Signal freshness age 12s > max age 10s | ETH-USDT bullish · Directional signal score 0.2 vs floor 0.4; candidate checks: ETH-USDT bullish · strength 0.2 < 0.4, score 0.1 < 0.3, exploration blocked for mixed context, exploration needs positive history, normal entry needs 30 side samples; strategy filters: BTC-USDT bearish · Strategy disabled for this strategy";
        let hits = symbol_gate_hits_for_reason(reason);
        assert!(hits.iter().any(|(symbol, direction, gate, _, distance)|
            symbol == "ETH-USDT" && direction == "bullish" && gate == "Signal freshness" && *distance == Some(0.2)));
        assert!(hits.iter().any(|(_, _, gate, _, _)| gate == "Directional signal"));
        for gate in ["Learned strength floor", "Exploration score floor", "Exploration disabled for mixed context", "Exploration walk-forward return and positive-rate floor", "Strict walk-forward sample minimum", "Strategy enablement"] {
            assert!(hits.iter().any(|hit| hit.2 == gate), "missing {gate}: {hits:?}");
        }
        assert_eq!(symbol_gate_hits_for_reason("reason without structured filters"), Vec::new());
    }
}
