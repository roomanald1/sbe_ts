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
    } else if lower.contains("strategy filters:") {
        if lower.contains("strategy disabled") {
            gates.push("Strategy enablement".into());
        }
        if lower.contains("strategy entry rule returned") {
            gates.push("Strategy entry rule".into());
        }
        if lower.contains("strategy evaluation error") {
            gates.push("Strategy evaluation".into());
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


#[cfg(test)]
mod reason_gate_hits_tests {
    use super::gate_hits_for_reason;

    #[test]
    fn maps_combined_candidate_reasons_to_distinct_gates() {
        let reason = "No tradeable bias: filtered: 2 stale, 1 non-directional, 3 strength 0.2 < 0.4; score 0.1 < 0.3; exploration blocked for mixed context; exploration needs positive history; normal entry needs 30 side samples; research-model side return must be positive; 2 candidate(s) blocked by negative walk-forward history";
        let gates = gate_hits_for_reason(reason);
        for expected in [
            "Signal freshness", "Directional signal", "Existing position / re-entry limit",
            "Learned strength floor", "Exploration score floor", "Exploration disabled for mixed context",
            "Exploration walk-forward return and positive-rate floor", "Strict walk-forward sample minimum",
            "Research-model side evidence", "Negative walk-forward evidence",
        ] { assert!(gates.iter().any(|gate| gate == expected), "missing {expected}: {gates:?}"); }
        assert_eq!(gate_hits_for_reason("No tradeable bias: fresh directional 0/0"), vec!["No fresh directional candidate"]);
        assert_eq!(gate_hits_for_reason("Daily realized-loss limit reached"), vec!["Daily realized-loss limit"]);
        assert_eq!(gate_hits_for_reason("No cached local order book"), vec!["Local order book availability"]);
    }

    #[test]
    fn maps_strategy_and_position_failures_without_duplicate_gates() {
        assert_eq!(gate_hits_for_reason("Strategy filters: strategy disabled; strategy entry rule returned false; strategy evaluation error"), vec!["Strategy enablement", "Strategy entry rule", "Strategy evaluation"]);
        assert_eq!(gate_hits_for_reason("Post-stop cooldown blocked re-entry"), vec!["Position / re-entry / cooldown"]);
        assert_eq!(gate_hits_for_reason("No profitable order type after spread and sweep cost"), vec!["Learned order-type profitability"]);
        assert_eq!(gate_hits_for_reason("Deep book cannot support the estimated impact"), vec!["Deep-book impact limit"]);
        assert_eq!(gate_hits_for_reason("unclassified reason"), vec!["unclassified reason"]);
    }
}
