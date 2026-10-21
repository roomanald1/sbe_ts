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

fn gate_distance_to_pass(gate: &str, detail: &str) -> Option<f64> {
    fn number(token: &str) -> Option<f64> {
        let cleaned = token.trim_matches(|character: char| {
            !character.is_ascii_digit() && character != '.' && character != '-'
        });
        cleaned.parse().ok()
    }
    fn number_at_end(text: &str) -> Option<f64> {
        text.split_whitespace().rev().find_map(number)
    }
    fn number_at_start(text: &str) -> Option<f64> {
        text.split_whitespace().find_map(number)
    }
    fn lower_bound_gap(actual: f64, threshold: f64) -> f64 {
        if threshold == 0.0 {
            actual.abs()
        } else {
            ((threshold - actual) / threshold.abs()).max(0.0)
        }
    }
    fn metric(text: &str, name: &str, section: Option<&str>) -> Option<f64> {
        let scoped = match section {
            Some(section) => text.split_once(section)?.1,
            None => text,
        };
        let value = scoped.split_once(name)?.1.split_whitespace().next()?;
        if value == "n/a" { None } else { number(value) }
    }
    fn positive_rate_gap(detail: &str, threshold: f64) -> Option<f64> {
        let actual_percent = metric(detail, "positive=", None)?;
        Some(lower_bound_gap(actual_percent / 100.0, threshold))
    }
    fn return_to_zero_gap(detail: &str, section: Option<&str>) -> Option<f64> {
        let actual_percent = metric(detail, "net=", section)?;
        // Return values are reported in percent; one percentage point is the normalization unit at a zero threshold.
        Some((-actual_percent).max(0.0))
    }

    let mut gaps = Vec::new();
    if let Some(failed) = detail.split("failed: ").nth(1) {
        if let Some((actual_text, threshold_text)) = failed.split_once(" < ") {
            if let (Some(actual), Some(threshold)) =
                (number_at_end(actual_text), number_at_start(threshold_text))
            {
                gaps.push(lower_bound_gap(actual, threshold));
            }
        }
        let mut parts = failed.split(" > ");
        if let (Some(actual), Some(threshold)) = (
            parts.next().and_then(number_at_end),
            parts.next().and_then(number_at_start),
        ) {
            gaps.push(if threshold == 0.0 {
                (actual - threshold).max(0.0)
            } else {
                ((actual - threshold) / threshold.abs()).max(0.0)
            });
        }
    }
    if gate == "Signal freshness" {
        if let Some((age, max_age)) = detail
            .split_once("age ")
            .and_then(|(_, tail)| tail.split_once("s > max age "))
        {
            if let (Some(actual), Some(threshold)) =
                (number_at_start(age), number_at_start(max_age))
            {
                gaps.push(if threshold == 0.0 {
                    1.0
                } else {
                    ((actual - threshold) / threshold).max(0.0)
                });
            }
        }
    }
    if gate == "Directional signal" {
        if let Some((score, floor)) = detail.split_once("vs floor ") {
            if let (Some(actual), Some(threshold)) = (number_at_end(score), number_at_start(floor))
            {
                gaps.push(lower_bound_gap(actual, threshold));
            }
        }
        if let Some((probability, threshold)) = detail
            .split_once("range probability ")
            .and_then(|(_, tail)| tail.split_once(" must be below "))
        {
            if let (Some(actual), Some(limit)) =
                (number_at_start(probability), number_at_start(threshold))
            {
                if limit != 0.0 {
                    gaps.push(((actual - limit) / limit.abs()).max(0.0));
                }
            }
        }
    }
    if gate == "Strict walk-forward sample minimum" {
        if let (Some(actual), Some(required)) = (
            detail.split("n=").nth(1).and_then(number_at_start),
            detail.split("needs ").nth(1).and_then(number_at_start),
        ) {
            if required != 0.0 {
                gaps.push(((required - actual) / required).max(0.0));
            }
        }
    }
    if gate == "Strict walk-forward quality thresholds" {
        if let Some(hit_percent) = metric(detail, "hit=", None) {
            gaps.push(lower_bound_gap(
                hit_percent / 100.0,
                MIN_SIDE_DIRECTION_MATCH_RATE,
            ));
        }
        if let Some(return_gap) = return_to_zero_gap(detail, None) {
            gaps.push(return_gap);
        }
        if let Some(gap) = positive_rate_gap(detail, MIN_SIDE_POSITIVE_NET_RATE) {
            gaps.push(gap);
        }
        if let Some(lower_percent) = metric(detail, "lower95=", None) {
            gaps.push(lower_bound_gap(
                lower_percent / 100.0,
                MIN_SIDE_POSITIVE_NET_LOWER_95,
            ));
        }
    }
    if gate == "Exploration walk-forward return and positive-rate floor" {
        if let Some(return_gap) = return_to_zero_gap(detail, None) {
            gaps.push(return_gap);
        }
        if let Some(gap) = positive_rate_gap(detail, NEGATIVE_EVIDENCE_FLOOR) {
            gaps.push(gap);
        }
    }
    if gate == "Research-model side evidence" {
        if let Some(return_gap) = return_to_zero_gap(detail, Some("research-model ")) {
            gaps.push(return_gap);
        }
    }
    if gate == "Existing position / re-entry limit" {
        if let Some((entries, max_entries)) = detail
            .split_once("entries ")
            .and_then(|(_, tail)| tail.split_once(" >= max "))
        {
            if let (Some(actual), Some(limit)) =
                (number_at_start(entries), number_at_start(max_entries))
            {
                if limit != 0.0 {
                    gaps.push(((actual - limit) / limit).max(0.0));
                }
            }
        }
    }
    if gate == "Execution cost versus measured edge" {
        if let (Some(cost), Some(edge)) = (
            detail
                .split_once("Estimated spread and sweep cost (")
                .and_then(|(_, tail)| tail.split_once("%) consumes"))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once("measured walk-forward edge (")
                .and_then(|(_, tail)| tail.split_once("%)"))
                .and_then(|(value, _)| number(value)),
        ) {
            let scale = edge.abs().max(0.001);
            gaps.push(((cost - edge) / scale).max(0.0));
        }
    }
    if gate == "Deep-book minimum order capacity" {
        if let (Some(capacity), Some(minimum)) = (
            detail
                .split_once("Deep-book capacity for ")
                .and_then(|(_, tail)| tail.split_once(" ("))
                .and_then(|(_, tail)| tail.split_once(") is below"))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once("minimum paper order (")
                .and_then(|(_, tail)| tail.split_once(')'))
                .and_then(|(value, _)| number(value)),
        ) {
            gaps.push(lower_bound_gap(capacity, minimum));
        }
    }
    if gate == "Deep-book impact limit" {
        if let (Some(impact), Some(limit)) = (
            detail
                .split_once("estimated impact ")
                .and_then(|(_, tail)| tail.split_once(" bps exceeds "))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once(" bps exceeds ")
                .and_then(|(_, tail)| number_at_start(tail)),
        ) {
            gaps.push(if limit == 0.0 {
                impact.max(0.0)
            } else {
                ((impact - limit) / limit.abs()).max(0.0)
            });
        }
    }
    gaps.into_iter()
        .filter(|gap| gap.is_finite())
        .reduce(f64::max)
}

fn gate_hits_for_reason(reason: &str) -> Vec<String> {
    let lower = reason.to_ascii_lowercase();
    let mut gates = Vec::new();
    let count_after = |marker: &str| {
        lower
            .split(marker)
            .nth(1)
            .and_then(|tail| tail.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default()
    };
    let mut add = |condition: bool, label: &str| {
        if condition && !gates.iter().any(|existing| existing == label) {
            gates.push(label.to_owned());
        }
    };
    if lower.starts_with("no tradeable bias") {
        add(count_after("filtered: ") > 0, "Signal freshness");
        add(count_after("stale, ") > 0, "Directional signal");
        add(
            count_after("non-directional, ") > 0,
            "Existing position / re-entry limit",
        );
        add(
            lower.contains("strength ") && lower.contains(" < "),
            "Learned strength floor",
        );
        add(
            lower.contains("score ") && lower.contains(" < "),
            "Exploration score floor",
        );
        add(
            lower.contains("exploration blocked for mixed context"),
            "Exploration disabled for mixed context",
        );
        add(
            lower.contains("exploration needs"),
            "Exploration walk-forward return and positive-rate floor",
        );
        add(
            lower.contains("normal entry needs") && lower.contains("side samples"),
            "Strict walk-forward sample minimum",
        );
        add(
            lower.contains("normal entry needs") && !lower.contains("side samples"),
            "Strict walk-forward quality thresholds",
        );
        add(
            lower.contains("research-model side return"),
            "Research-model side evidence",
        );
        let blocked_by_negative = lower
            .split("candidate(s) blocked by negative walk-forward history")
            .next()
            .and_then(|prefix| prefix.split_whitespace().last())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        add(blocked_by_negative > 0, "Negative walk-forward evidence");
        if gates.is_empty() {
            if count_after("fresh directional ") == 0 {
                gates.push("No fresh directional candidate".into());
            } else {
                gates.push("Candidate selection / combined entry checks".into());
            }
        }
    } else if lower.contains("daily realized-loss limit") {
        gates.push("Daily realized-loss limit".into());
    } else if lower.contains("invalid instrument symbol") {
        gates.push("Instrument symbol validation".into());
    } else if lower.contains("no cached local order book")
        || lower.contains("no current order book")
    {
        gates.push("Local order book availability".into());
    } else if lower.contains("stale local order book") {
        gates.push("Local order book freshness".into());
    } else if lower.contains("post-stop cooldown")
        || lower.contains("re-entry cooldown")
        || lower.contains("re-entry blocked")
        || lower.contains("opposite signal")
        || lower.contains("add-on limit")
        || lower.contains("open-position cap")
    {
        gates.push("Position / re-entry / cooldown".into());
    } else if lower.contains("notional capacity") || lower.contains("capacity is fully allocated") {
        gates.push("Available notional capacity".into());
    } else if lower.contains("daily notional")
        || lower.contains("budget")
        || lower.contains("capacity")
    {
        gates.push("Available budget / position capacity".into());
    } else if lower.contains("no profitable order type") {
        gates.push("Learned order-type profitability".into());
    } else if lower.contains("spread and sweep cost") {
        gates.push("Execution cost versus measured edge".into());
    } else if lower.contains("deep book cannot support") {
        gates.push("Deep-book impact limit".into());
    } else if lower.contains("deep-book capacity") {
        gates.push("Deep-book minimum order capacity".into());
    } else if lower.contains("walk-forward") || lower.contains("negative") {
        gates.push("Walk-forward history".into());
    } else if lower.contains("cycle error") {
        gates.push("Cycle error".into());
    } else {
        gates.push(gap_reason_category(reason));
    }
    gates
}

fn gap_reason_category(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.starts_with("no tradeable bias") {
        let counts = lower
            .split("fresh directional ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or("0/0");
        let mut counts = counts
            .split('/')
            .filter_map(|value| value.parse::<usize>().ok());
        let usable = counts.next().unwrap_or_default();
        let total = counts.next().unwrap_or_default();
        if total == 0 {
            return "No insights available".into();
        }
        if usable == 0 {
            let filtered_count = |marker: &str| {
                lower
                    .split(marker)
                    .nth(1)
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default()
            };
            let stale = filtered_count("filtered: ");
            let no_direction = filtered_count("non-directional, ");
            let position = filtered_count("blocked by existing positions");
            if position > 0 {
                return "Existing position blocked signals".into();
            }
            if no_direction > 0 {
                return "No directional signal".into();
            }
            if stale > 0 {
                return "Signals were stale".into();
            }
            return "No usable directional signal".into();
        }
        let strength = lower
            .split("contextual learned strength floors passed ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let exploration = lower
            .split("contextual raw-score floors passed ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        if strength == 0 && exploration == 0 {
            return "Signal too weak for entry".into();
        }
        let blocked = lower
            .split("candidate(s) blocked by negative walk-forward history")
            .next()
            .and_then(|prefix| prefix.split_whitespace().last())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        if blocked > 0 {
            return "Walk-forward history blocked candidates".into();
        }
        return "No candidate passed all entry checks".into();
    }
    if lower.contains("daily realized-loss limit") {
        return "Daily realized-loss limit reached".into();
    }
    if lower.contains("daily notional")
        || lower.contains("budget")
        || lower.contains("capacity is fully allocated")
    {
        return "Available trading budget or position capacity was insufficient".into();
    }
    if lower.contains("walk-forward") || lower.contains("negative") {
        return "Historical walk-forward evidence did not qualify".into();
    }
    if lower.contains("stale local order book") {
        return "Local order book data was stale".into();
    }
    if lower.contains("no cached local order book") {
        return "No cached local order book was available".into();
    }
    if lower.contains("open position") || lower.contains("already") {
        return "Existing position or cooldown blocked another entry".into();
    }
    if lower.starts_with("cycle error:") {
        return "Cycle error".into();
    }
    reason
        .split(|character| character == ':' || character == ';' || character == '·')
        .next()
        .unwrap_or(reason)
        .trim()
        .to_owned()
}

fn skipped_for_signal(signal: &Signal, reason: &str) -> CycleResult {
    let mut cycle = skipped(reason);
    cycle.symbol = Some(signal.symbol.clone());
    cycle.side = Some(signal.direction.clone());
    cycle
}
fn skipped(reason: &str) -> CycleResult {
    CycleResult {
        status: "skipped",
        reason: Some(reason.into()),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
    }
}
fn upstream_error(error: reqwest::Error) -> (StatusCode, String) {
    (
        StatusCode::BAD_GATEWAY,
        format!("Auto trader upstream request failed: {error}"),
    )
}
