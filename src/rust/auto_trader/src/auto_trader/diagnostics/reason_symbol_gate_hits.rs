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
    if let Some(details) = reason.split("; candidate checks: ").nth(1) {
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
    hits.into_iter()
        .map(|((symbol, direction, gate), detail)| {
            let distance = gate_distance_to_pass(&gate, &detail);
            (symbol, direction, gate, detail, distance)
        })
        .collect()
}

