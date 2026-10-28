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

