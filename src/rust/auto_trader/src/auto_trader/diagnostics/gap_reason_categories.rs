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

