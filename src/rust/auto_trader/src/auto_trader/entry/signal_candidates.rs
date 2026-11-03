fn is_daily_strategy_signal(signal: &Signal) -> bool {
    signal.strategy_candidate
        && signal.strategy.as_deref().is_some_and(is_independent_daily_strategy)
}

fn daily_signal_candidate(signal: &Signal) -> bool {
    let is_daily_candidate = is_daily_strategy_signal(signal);
    // Defense in depth: RSI entries must still satisfy the primary rule when queued.
    let failed_rsi_entry_gate = signal.strategy.as_deref() == Some("rsi2-oversold-rebound")
        && signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("primary_passed"))
            .and_then(serde_json::Value::as_bool) == Some(false);
    is_daily_candidate && !failed_rsi_entry_gate
}

fn is_fallback_signal_candidate(signal: &Signal) -> bool {
    !is_daily_strategy_signal(signal)
}

fn should_shadow_daily_scout(scout: bool, signal: &Signal) -> bool {
    scout && is_daily_strategy_signal(signal)
}

fn signal_max_age_seconds(signal: &Signal) -> u64 {
    if signal.strategy.as_deref().and_then(Strategy::from_name)
        .is_some_and(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
    {
        MAX_DAILY_SIGNAL_AGE_SECONDS
    } else {
        MAX_SIGNAL_AGE_SECONDS
    }
}

fn signal_allowed_by_positions(
    signal: &Signal,
    open_positions: &BTreeMap<String, TraderPosition>,
    strategy_positions: &BTreeMap<String, BTreeMap<String, TraderPosition>>,
    max_entries_per_auto_position: u32,
) -> bool {
    let direction = if signal.direction == "bullish" { 1.0 } else { -1.0 };
    let Some(strategy_key) = signal.strategy.as_deref() else {
        return open_positions.get(&signal.symbol).is_none_or(|position| {
            position.quantity.signum() == direction
                && position_entries_for_category(position, StrategyCategory::Intraday)
                    < max_entries_per_auto_position
        });
    };
    let category = strategy_category(Some(strategy_key));
    let has_portfolio_weight = signal.strategy.as_deref() == Some("risk-adjusted-trend-portfolio")
        && signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("strategy_order_size_multiplier"))
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|weight| weight.is_finite() && (0.0..=0.25).contains(&weight));
    let aggregate_position = open_positions.get(&signal.symbol);
    if aggregate_position.is_some_and(|position| position.quantity.signum() != direction) {
        return false;
    }
    if daily_signal_candidate(signal) {
        let strategy_position = strategy_positions.get(&signal.symbol)
            .and_then(|lots| lots.get(strategy_key))
            .filter(|position| position.quantity.abs() > 1e-12)
            .or_else(|| aggregate_position.filter(|position| {
                strategy_positions.is_empty()
                    && position.strategy.as_deref() == Some(strategy_key)
            }));
        return strategy_position.is_none_or(|position| {
            position.quantity.signum() == direction
                && (has_portfolio_weight || position_entries_for_category(position, category) < max_entries_per_auto_position)
        });
    }
    aggregate_position.is_none_or(|position| {
        position.quantity.signum() == direction
            && position_entries_for_category(position, category) < max_entries_per_auto_position
    })
}

fn signal_candidate_position_block_reason(
    signal: &Signal,
    open_positions: &BTreeMap<String, TraderPosition>,
    strategy_positions: &BTreeMap<String, BTreeMap<String, TraderPosition>>,
    max_entries_per_auto_position: u32,
) -> Option<String> {
    let direction = if signal.direction == "bullish" { 1.0 } else { -1.0 };
    let strategy = signal.strategy.as_deref();
    let aggregate = open_positions.get(&signal.symbol);
    if let Some(position) = aggregate.filter(|position| position.quantity.signum() != direction) {
        return Some(format!("opposite aggregate position ({:.8})", position.quantity));
    }
    if daily_signal_candidate(signal) {
        if let Some(strategy) = strategy {
            let existing = strategy_positions.get(&signal.symbol).and_then(|lots| lots.get(strategy))
                .filter(|position| position.quantity.abs() > 1e-12)
                .or_else(|| aggregate.filter(|position| {
                    strategy_positions.is_empty() && position.strategy.as_deref() == Some(strategy)
                }));
            if let Some(position) = existing {
                if position.quantity.signum() != direction {
                    return Some(format!("opposite {strategy} position ({:.8})", position.quantity));
                }
                let portfolio_weight = strategy == "risk-adjusted-trend-portfolio"
                    && signal.strategy_decision.as_ref()
                        .and_then(|decision| decision.get("strategy_order_size_multiplier"))
                        .and_then(serde_json::Value::as_f64)
                        .is_some_and(|weight| weight.is_finite() && (0.0..=0.25).contains(&weight));
                let entries = position_entries_for_category(position, strategy_category(Some(strategy)));
                if !portfolio_weight && entries >= max_entries_per_auto_position {
                    return Some(format!("{strategy} entries {entries} reached per-position maximum {max_entries_per_auto_position}"));
                }
            }
        }
    } else if let Some(position) = aggregate {
        let category = strategy_category(strategy);
        let entries = position_entries_for_category(position, category);
        if entries >= max_entries_per_auto_position {
            return Some(format!("aggregate {} entries {entries} reached per-position maximum {max_entries_per_auto_position}", strategy_category_key(category)));
        }
    }
    None
}

fn collect_signal_candidates<'a>(
    signals: &'a [Signal],
    now: u64,
    risk_appetite: f64,
    open_positions: &BTreeMap<String, TraderPosition>,
    strategy_positions: &BTreeMap<String, BTreeMap<String, TraderPosition>>,
    max_entries_per_auto_position: u32,
) -> FreshSignalCandidates<'a> {
    let mut stale_signal_count = 0;
    let mut stalest_signal_age_seconds = 0u64;
    let mut stale_symbols = Vec::new();
    let mut non_directional_count = 0;
    let mut position_blocked_count = 0;
    let mut position_block_details = Vec::new();
    let fresh_directional = signals
        .iter()
        .filter(|signal| {
            if is_stablecoin_pair(&signal.symbol) {
                return false;
            }
            let max_age = signal_max_age_seconds(signal);
            if signal.stale || now.saturating_sub(signal.timestamp / 1000) > max_age
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
            let block_reason = signal_candidate_position_block_reason(
                signal, open_positions, strategy_positions, max_entries_per_auto_position,
            );
            let allowed_by_position = block_reason.is_none();
            if !allowed_by_position {
                position_blocked_count += 1;
                if position_block_details.len() < 20 {
                    position_block_details.push(format!(
                        "{} {} [{}] · Existing position / re-entry limit: {}",
                        signal.symbol,
                        signal.direction,
                        signal.strategy.as_deref().unwrap_or("unattributed"),
                        block_reason.unwrap_or_default()
                    ));
                }
            }
            allowed_by_position
        })
        .collect::<Vec<_>>();
    let neutral_scout_candidates = signals
        .iter()
        .filter_map(|signal| {
            if is_stablecoin_pair(&signal.symbol)
                || signal.direction != "neutral"
                || signal.stale
                || now.saturating_sub(signal.timestamp / 1000) > signal_max_age_seconds(signal)
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
            let allowed_by_position = signal_allowed_by_positions(
                &candidate,
                open_positions,
                strategy_positions,
                max_entries_per_auto_position,
            );
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
        position_block_details,
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
    position_block_details: &'a [String],
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
        position_block_details,
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
    let mut early_filter_details = signals.iter().filter_map(|candidate| {
                let age = now.saturating_sub(candidate.timestamp / 1000);
                if is_stablecoin_pair(&candidate.symbol) {
                    Some(format!(
                        "{} {} · Stablecoin pair excluded from directional and scout entries",
                        candidate.symbol, candidate.direction
                    ))
                } else if candidate.stale {
                    Some(format!("{} {} · Signal freshness (stale flag true; age {}s; max age {}s)", candidate.symbol, candidate.direction, age, signal_max_age_seconds(candidate)))
                } else if age > signal_max_age_seconds(candidate) {
                    Some(format!("{} {} · Signal freshness (age {}s > max {}s)", candidate.symbol, candidate.direction, age, signal_max_age_seconds(candidate)))
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
        early_filter_details.extend(position_block_details.iter().cloned());
        reason.push_str("; early filters: ");
        reason.push_str(&early_filter_details.join(" | "));
    } else if !position_block_details.is_empty() {
        reason.push_str("; early filters: ");
        reason.push_str(&position_block_details.join(" | "));
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

#[cfg(test)]
mod parallel_daily_candidate_tests {
    use super::*;

    fn signal(strategy: &str, direction: &str) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: direction.into(),
            strength: 1.0,
            timestamp: 100_000,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }


    #[test]
    fn failed_rsi_daily_candidate_cannot_fall_back_to_strict_selection() {
        let mut candidate = signal("rsi2-oversold-rebound", "bullish");
        candidate.strategy_decision = Some(serde_json::json!({
            "primary_passed": false,
            "primary_metric_value": 100.0,
            "primary_level": 10.0,
            "scout_parameter_probe": true,
            "scout_parameter_key": "maximum_hold_days",
            "scout_parameter_value": 4.0
        }));

        assert!(is_daily_strategy_signal(&candidate));
        assert!(!daily_signal_candidate(&candidate));
        assert!(!is_fallback_signal_candidate(&candidate));
    }

    #[test]
    fn daily_scouts_are_shadow_only() {
        let candidate = signal("trend-50-200", "bullish");

        assert!(should_shadow_daily_scout(true, &candidate));
        assert!(!should_shadow_daily_scout(false, &candidate));
    }

    #[test]
    fn same_side_daily_strategy_is_not_blocked_by_another_strategy_lot() {
        let mut aggregate = TraderPosition::default();
        aggregate.quantity = 1.0;
        aggregate.entries_by_category.insert("daily".into(), 1);
        let open = BTreeMap::from([("ETH-USDT".into(), aggregate)]);
        let mut existing_lot = TraderPosition::default();
        existing_lot.quantity = 1.0;
        existing_lot.entries_by_category.insert("daily".into(), 1);
        let lots = BTreeMap::from([(
            "ETH-USDT".into(),
            BTreeMap::from([("donchian-55-btc200".into(), existing_lot)]),
        )]);

        assert!(signal_allowed_by_positions(
            &signal("trend-50-200", "bullish"), &open, &lots, 1,
        ));
        assert!(!signal_allowed_by_positions(
            &signal("donchian-55-btc200", "bullish"), &open, &lots, 1,
        ));
        assert!(!signal_allowed_by_positions(
            &signal("trend-50-200", "bearish"), &open, &lots, 1,
        ));
    }
}
