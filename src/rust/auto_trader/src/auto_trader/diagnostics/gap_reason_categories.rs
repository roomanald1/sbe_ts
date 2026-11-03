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
            .split_once("candidate(s) blocked by negative walk-forward history")
            .and_then(|(prefix, _)| prefix.split_whitespace().last())
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


#[cfg(test)]
mod gap_reason_category_tests {
    use super::gap_reason_category;

    #[test]
    fn summarizes_common_entry_gap_reasons() {
        let cases = [
            ("No tradeable bias: fresh directional 0/8 filtered: 4 non-directional, 0", "Signals were stale"),
            ("No tradeable bias: fresh directional 0/8 non-directional, 4", "No directional signal"),
            ("No tradeable bias: fresh directional 0/8 blocked by existing positions 2", "Existing position blocked signals"),
            ("No tradeable bias: fresh directional 0/0", "No insights available"),
            ("No tradeable bias: fresh directional 2/8 contextual learned strength floors passed 0 contextual raw-score floors passed 0", "Signal too weak for entry"),
            ("No tradeable bias: fresh directional 2/8 contextual learned strength floors passed 1 contextual raw-score floors passed 0 2 candidate(s) blocked by negative walk-forward history", "Walk-forward history blocked candidates"),
            ("No tradeable bias: fresh directional 2/8 contextual learned strength floors passed 1 contextual raw-score floors passed 1", "No candidate passed all entry checks"),
            ("Daily realized-loss limit reached", "Daily realized-loss limit reached"),
            ("daily notional budget reached", "Available trading budget or position capacity was insufficient"),
            ("negative walk-forward history", "Historical walk-forward evidence did not qualify"),
            ("stale local order book", "Local order book data was stale"),
            ("no cached local order book", "No cached local order book was available"),
            ("already open position", "Existing position or cooldown blocked another entry"),
            ("Cycle error: upstream unavailable", "Cycle error"),
            ("Rejected: insufficient data; retry later", "Rejected"),
        ];
        for (reason, expected) in cases {
            assert_eq!(gap_reason_category(reason), expected, "{reason}");
        }
    }
}
