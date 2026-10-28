fn collect_signal_candidates<'a>(
    signals: &'a [Signal],
    now: u64,
    risk_appetite: f64,
    open_positions: &BTreeMap<String, TraderPosition>,
    max_entries_per_auto_position: u32,
) -> FreshSignalCandidates<'a> {
    let mut stale_signal_count = 0;
    let mut stalest_signal_age_seconds = 0u64;
    let mut stale_symbols = Vec::new();
    let mut non_directional_count = 0;
    let mut position_blocked_count = 0;
    let fresh_directional = signals
        .iter()
        .filter(|signal| {
            if signal.stale || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
            {
                stale_signal_count += 1;
                let age = now.saturating_sub(signal.timestamp / 1000);
                stalest_signal_age_seconds = stalest_signal_age_seconds.max(age);
                if !stale_symbols.iter().any(|symbol| symbol == &signal.symbol) {
                    stale_symbols.push(signal.symbol.clone());
                }
                return false;
            }
            if !matches!(signal.direction.as_str(), "bullish" | "bearish") {
                non_directional_count += 1;
                return false;
            }
            let allowed_by_position = open_positions.get(&signal.symbol).is_none_or(|position| {
                let signal_quantity = if signal.direction == "bullish" {
                    1.0
                } else {
                    -1.0
                };
                let category = strategy_category(signal.strategy.as_deref());
                position.quantity.signum() == signal_quantity
                    && position_entries_for_category(position, category) < max_entries_per_auto_position
            });
            if !allowed_by_position {
                position_blocked_count += 1;
            }
            allowed_by_position
        })
        .collect::<Vec<_>>();
    let neutral_scout_candidates = signals
        .iter()
        .filter_map(|signal| {
            if signal.direction != "neutral"
                || signal.stale
                || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
                || !signal.score.is_finite()
                || signal.score.abs() < SCOUT_NEUTRAL_SCORE_FLOOR * risk_floor_scale(risk_appetite)
                || signal.ranging_probability.unwrap_or(0.0)
                    >= NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            {
                return None;
            }
            let mut candidate = signal.clone();
            candidate.direction = if candidate.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            let allowed_by_position =
                open_positions
                    .get(&candidate.symbol)
                    .is_none_or(|position| {
                        let direction = if candidate.direction == "bullish" {
                            1.0
                        } else {
                            -1.0
                        };
                        let category = strategy_category(candidate.strategy.as_deref());
                        position.quantity.signum() == direction
                            && position_entries_for_category(position, category) < max_entries_per_auto_position
                    });
            allowed_by_position.then_some(candidate)
        })
        .collect::<Vec<_>>();
    FreshSignalCandidates {
        fresh_directional,
        neutral_scout_candidates,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
    }
}

#[derive(Clone, Copy)]
struct SignalSelectionInput<'a> {
    state: &'a AutoTraderState,
    dashboard: &'a Dashboard,
    signals: &'a [Signal],
    fresh_directional: &'a [&'a Signal],
    neutral_scout_candidates: &'a [Signal],
    now: u64,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: &'a [String],
    non_directional_count: usize,
    position_blocked_count: usize,
    learning: &'a BTreeMap<String, LearningStats>,
    risk_appetite: f64,
    active_scout_count: usize,
    open_positions: &'a BTreeMap<String, TraderPosition>,
    max_entries_per_auto_position: u32,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
}

fn format_no_tradeable_bias(
    input: &SignalSelectionInput<'_>,
    exploration_qualified: &[&Signal],
    strength_qualified: &[&Signal],
) -> String {
    let SignalSelectionInput {
        dashboard,
        signals,
        fresh_directional,
        neutral_scout_candidates,
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
        learning,
        risk_appetite,
        active_scout_count,
        open_positions,
        max_entries_per_auto_position,
        ..
    } = *input;
    let blocked_by_negative = count_negative_evidence_candidates(exploration_qualified, dashboard);
    let waiting_for_pullback = fresh_directional
        .iter()
        .filter(|signal| needs_pullback_confirmation(signal) && !passes_pullback_confirmation(signal))
        .count();
    let mut reason = format!(
        "No tradeable bias: fresh directional {}/{} ({} inferred from neutral score; {} neutral scout probes; filtered: {} stale, {} non-directional, {} blocked by existing positions, {} awaiting 10s pullback); contextual learned strength floors passed {}; exploration strength floor {:.2}; contextual raw-score floors passed {}; {} candidate(s) blocked by negative walk-forward history",
        fresh_directional.len(),
        signals.len(),
        neutral_scout_candidates.len(),
        inferred_direction_count,
        stale_signal_count,
        non_directional_count,
        position_blocked_count,
        waiting_for_pullback,
        strength_qualified.len(),
        MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite),
        exploration_qualified.len(),
        blocked_by_negative
    );
    if stale_signal_count > 0 {
        reason.push_str(&format!(
            "; oldest stale signal {}s; symbols {}",
            stalest_signal_age_seconds,
            stale_symbols.join(", ")
        ));
    }
    let early_filter_details = signals.iter().filter_map(|candidate| {
                let age = now.saturating_sub(candidate.timestamp / 1000);
                if candidate.stale {
                    Some(format!("{} {} · Signal freshness (stale flag true; age {}s; max age {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if age > MAX_SIGNAL_AGE_SECONDS {
                    Some(format!("{} {} · Signal freshness (age {}s > max {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if !matches!(candidate.direction.as_str(), "bullish" | "bearish") {
                    let ranging = candidate.ranging_probability.map(|value| format!("{:.1}%", value * 100.0)).unwrap_or_else(|| "n/a (defaults to 0%)".into());
                    Some(format!("{} {} · Directional signal (|score| {:.3} vs floor {:.2}; range probability {} must be below {:.0}% to infer direction)", candidate.symbol, candidate.direction, candidate.score.abs(), neutral_direction_floor, ranging, NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY * 100.0))
                } else {
                    open_positions.get(&candidate.symbol).and_then(|position| {
                        let signal_quantity = if candidate.direction == "bullish" { 1.0 } else { -1.0 };
                        let matching_side = position.quantity.signum() == signal_quantity;
                        if !matching_side {
                            Some(format!("{} {} · Existing position / re-entry limit (opposite position qty {:.8}; signal side {})", candidate.symbol, candidate.direction, position.quantity, candidate.direction))
                        } else if position_entries_for_category(position, strategy_category(candidate.strategy.as_deref())) >= max_entries_per_auto_position {
                            let entries = position_entries_for_category(position, strategy_category(candidate.strategy.as_deref()));
                            Some(format!("{} {} · Existing position / re-entry limit (category entries {} >= max {})", candidate.symbol, candidate.direction, entries, max_entries_per_auto_position))
                        } else { None }
                    })
                }
            }).collect::<Vec<_>>();
    if !early_filter_details.is_empty() {
        reason.push_str("; early filters: ");
        reason.push_str(&early_filter_details.join(" | "));
    }
    let candidate_details = fresh_directional
        .iter()
        .copied()
        .chain(neutral_scout_candidates.iter())
        .map(|candidate| {
            candidate_entry_diagnostic(
                candidate,
                dashboard,
                learning,
                risk_appetite,
                active_scout_count,
            )
        })
        .collect::<Vec<_>>();
    if !candidate_details.is_empty() {
        reason.push_str("; candidate checks: ");
        reason.push_str(&candidate_details.join(" | "));
    }
    reason
}




include!("../diagnostics/candidate_entry_review.rs");
