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

