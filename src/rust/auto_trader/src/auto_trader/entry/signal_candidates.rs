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

fn live_strategy_source_signals(signals: &[Signal]) -> Vec<Signal> {
    signals.iter().filter(|signal| !is_daily_strategy_signal(signal)).cloned().collect()
}

fn live_strategy_feature_signal(
    signal: &Signal,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
) -> Result<Signal, String> {
    let parameter = |key: StrategyParameterKey| {
        parameters.get(key.as_str()).copied().ok_or_else(|| {
            format!("Configured parameter '{}' is unavailable", key.as_str())
        })
    };
    let mut configured = signal.clone();
    match strategy {
        Strategy::LiveMomentum => {
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let trend_window = parameter(StrategyParameterKey::TrendLookbackSeconds)? as u16;
            configured.components.momentum_10s_percent = signal.components.momentum_by_window
                .iter().find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?.value;
            configured.components.trend_5m_volatility_units = signal.components.trend_volatility_units_by_window
                .iter().find(|feature| feature.window_seconds == trend_window)
                .ok_or_else(|| format!("Trend-volatility feature window {trend_window}s is missing"))?.value;
        }
        Strategy::LiveTradeFlow => {
            let window = parameter(StrategyParameterKey::TradeFlowWindowSeconds)? as u16;
            configured.components.trade_flow_imbalance = signal.components.trade_flow_by_window
                .iter().find(|feature| feature.window_seconds == window)
                .ok_or_else(|| format!("Trade-flow feature window {window}s is missing"))?.value;
        }
        Strategy::LiveOrderBook => {
            let depth = parameter(StrategyParameterKey::BookDepthLevels)? as u8;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            configured.components.book_imbalance = signal.components.book_imbalance_by_depth
                .iter().find(|feature| feature.depth_levels == depth)
                .ok_or_else(|| format!("Order-book feature at depth {depth} is missing"))?.value;
            configured.components.momentum_10s_percent = signal.components.momentum_by_window
                .iter().find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?.value;
        }
        Strategy::LiveVwapDislocation => {
            let window = parameter(StrategyParameterKey::VwapLookbackSeconds)? as u16;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let minimum_count = parameter(StrategyParameterKey::VwapMinimumTradeCount)? as u16;
            let minimum_span_ms = (parameter(StrategyParameterKey::VwapMinimumTradeSpanSeconds)? * 1_000.0) as u32;
            configured.components.momentum_10s_percent = signal.components.momentum_by_window
                .iter().find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?.value;
            let feature = signal.components.vwap_dislocation_by_window
                .iter().find(|feature| feature.window_seconds == window)
                .ok_or_else(|| format!("VWAP-dislocation feature window {window}s is missing"))?;
            configured.components.mispricing_vs_vwap_bps =
                (feature.sample_count >= minimum_count && feature.span_ms >= minimum_span_ms)
                    .then_some(feature.value).flatten();
        }
        Strategy::LiveAbsorptionPressure => {
            let window = parameter(StrategyParameterKey::AbsorptionLookbackSeconds)? as u16;
            let replenishment_ms = (parameter(StrategyParameterKey::AbsorptionReplenishmentSeconds)? * 1_000.0).round() as u32;
            let tolerance = parameter(StrategyParameterKey::AbsorptionTouchToleranceBps)?;
            let maximum_move = parameter(StrategyParameterKey::AbsorptionMaximumPriceMovePercent)?;
            let momentum_window = parameter(StrategyParameterKey::MomentumLookbackSeconds)? as u16;
            let movement = signal.components.momentum_by_window.iter()
                .find(|feature| feature.window_seconds == momentum_window)
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s is missing"))?
                .value
                .ok_or_else(|| format!("Momentum feature window {momentum_window}s has no value yet"))?;
            configured.components.momentum_10s_percent = Some(movement);
            if movement.abs() > maximum_move {
                return Err(format!(
                    "Price movement {movement:.4}% exceeds the configured maximum {maximum_move:.4}%"
                ));
            }
            let pressure = signal.components.absorption_pressure_by_window.iter()
                .find(|feature| feature.window_seconds == window
                    && feature.replenishment_window_ms == replenishment_ms
                    && (feature.touch_tolerance_bps - tolerance).abs() < 1e-6)
                .and_then(|feature| feature.value);
            let flow_window = parameter(StrategyParameterKey::TradeFlowWindowSeconds)? as u16;
            let minimum_flow = parameter(StrategyParameterKey::AbsorptionMinimumFlowImbalance)?;
            let flow_fallback = signal.components.trade_flow_by_window.iter()
                .find(|feature| feature.window_seconds == flow_window)
                .and_then(|feature| feature.value)
                .filter(|flow| flow.abs() >= minimum_flow)
                .map(|flow| -flow);
            configured.components.pressure_without_price_move = pressure.or(flow_fallback);
        }
        _ => return Err(format!("Strategy '{}' has no live-feature adapter", strategy.as_str())),
    }
    // The selected scalar features and applied parameters are retained in the
    // decision audit; raw variants are only needed before strategy evaluation.
    configured.components.momentum_by_window.clear();
    configured.components.trend_volatility_units_by_window.clear();
    configured.components.trade_flow_by_window.clear();
    configured.components.book_imbalance_by_depth.clear();
    configured.components.vwap_dislocation_by_window.clear();
    configured.components.absorption_pressure_by_window.clear();
    Ok(configured)
}

#[cfg(test)]
fn live_scout_direction_probes(
    signal: &Signal,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
) -> Vec<Signal> {
    let Ok(signal) = live_strategy_feature_signal(signal, strategy, parameters) else {
        return Vec::new();
    };
    let source_values: Vec<f64> = match strategy {
        Strategy::LiveMomentum => [
            signal.components.momentum_10s_percent,
            signal.components.trend_5m_volatility_units,
        ]
        .into_iter()
        .flatten()
        .collect(),
        Strategy::LiveTradeFlow => signal.components.trade_flow_imbalance.into_iter().collect(),
        Strategy::LiveOrderBook => signal.components.book_imbalance.into_iter().collect(),
        Strategy::LiveVwapDislocation => signal.components.mispricing_vs_vwap_bps.into_iter().collect(),
        Strategy::LiveAbsorptionPressure => signal.components.pressure_without_price_move
            .map(|pressure| -pressure)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    let mut directions = source_values
        .into_iter()
        .filter(|value| value.is_finite() && *value != 0.0)
        .map(|value| if value > 0.0 { "bullish" } else { "bearish" })
        .collect::<Vec<_>>();
    directions.sort_unstable();
    directions.dedup();
    directions.into_iter().map(|direction| {
        let mut candidate = signal.clone();
        candidate.direction = direction.to_owned();
        candidate
    }).collect()
}

#[cfg(test)]
fn live_scout_direction_candidates(
    signal: &Signal,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
) -> Vec<Signal> {
    let passing = live_scout_direction_probes(signal, strategy, parameters)
        .into_iter()
        .filter_map(|candidate| {
            let input = StrategySignal::entry(&candidate, parameters);
            matches!(strategy.evaluate(&input), Ok(StrategyDecision::Enter)).then_some(candidate)
        })
        .collect::<Vec<_>>();
    let distinct_directions = passing
        .iter()
        .map(|candidate| candidate.direction.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    // Momentum has two inputs. If they independently qualify in opposite
    // directions, the scout has no unambiguous side and must wait.
    if distinct_directions.len() == 1 { passing } else { Vec::new() }
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

fn signal_is_fresh(signal: &Signal, now: u64) -> bool {
    !signal.stale && now.saturating_sub(signal.timestamp / 1_000) <= signal_max_age_seconds(signal)
}

#[cfg(test)]
#[cfg(test)]
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
            if !matches!(signal.direction.as_str(), "bullish" | "bearish") {
                non_directional_count += 1;
                return false;
            }
            // Only count stale directional signals as stale candidates. Neutral
            // and ranging snapshots cannot enter a position, so including them
            // inflated the stale-signal metric with signals that were never
            // actionable.
            if !signal_is_fresh(signal, now) {
                stale_signal_count += 1;
                let age = now.saturating_sub(signal.timestamp / 1000);
                stalest_signal_age_seconds = stalest_signal_age_seconds.max(age);
                if !stale_symbols.iter().any(|symbol| symbol == &signal.symbol) {
                    stale_symbols.push(signal.symbol.clone());
                }
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
    FreshSignalCandidates {
        fresh_directional,
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
    dashboard: &'a Dashboard,
    signals: &'a [Signal],
    fresh_directional: &'a [&'a Signal],
    now: u64,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
    stale_signal_count: usize,
    stalest_signal_age_seconds: u64,
    stale_symbols: &'a [String],
    non_directional_count: usize,
    position_blocked_count: usize,
    position_block_details: &'a [String],
    learning_thresholds: &'a BTreeMap<String, strategy::ContextThresholds>,
    risk_appetite: f64,
    open_positions: &'a BTreeMap<String, TraderPosition>,
    max_entries_per_auto_position: u32,
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
        now,
        inferred_direction_count,
        neutral_direction_floor,
        stale_signal_count,
        stalest_signal_age_seconds,
        stale_symbols,
        non_directional_count,
        position_blocked_count,
        position_block_details,
        learning_thresholds,
        risk_appetite,
        open_positions,
        max_entries_per_auto_position,
        ..
    } = *input;
    let waiting_for_pullback = fresh_directional
        .iter()
        .filter(|signal| needs_pullback_confirmation(signal) && !passes_pullback_confirmation(signal))
        .count();
    let mut reason = format!(
        "No tradeable bias: fresh directional {}/{} ({} inferred from neutral score; filtered: {} stale, {} non-directional, {} blocked by existing positions, {} awaiting 10s pullback); contextual learned strength floors passed {}; exploration strength floor {:.2}; contextual raw-score floors passed {}",
        fresh_directional.len(),
        signals.len(),
        inferred_direction_count,
        stale_signal_count,
        non_directional_count,
        position_blocked_count,
        waiting_for_pullback,
        strength_qualified.len(),
        MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite),
        exploration_qualified.len()
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
        .map(|candidate| {
            candidate_entry_diagnostic(
                candidate,
                dashboard,
                learning_thresholds,
                risk_appetite,
                now.saturating_mul(1000),
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
mod live_scout_direction_tests {
    use super::*;

    fn source() -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "neutral".into(),
            strength: 0.5,
            timestamp: 100_000,
            stale: false,
            score: 0.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: false,
            strategy: None,
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    fn parameters(strategy: Strategy) -> BTreeMap<String, f64> {
        strategy.parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect()
    }

    #[test]
    fn live_feature_blocker_identifies_the_missing_configured_window() {
        let error = live_strategy_feature_signal(
            &source(),
            Strategy::LiveMomentum,
            &parameters(Strategy::LiveMomentum),
        )
        .unwrap_err();
        assert_eq!(error, "Momentum feature window 10s is missing");
    }

    #[test]
    fn absorption_blocker_identifies_a_feature_window_still_warming_up() {
        let mut signal = source();
        signal.components.momentum_by_window = vec![WindowedSignalValue {
            window_seconds: 10,
            value: None,
            ..Default::default()
        }];
        let error = live_strategy_feature_signal(
            &signal,
            Strategy::LiveAbsorptionPressure,
            &parameters(Strategy::LiveAbsorptionPressure),
        )
        .unwrap_err();
        assert_eq!(error, "Momentum feature window 10s has no value yet");
    }

    #[test]
    fn neutral_aggregate_signal_can_produce_independent_live_scout_directions() {
        let cases = [
            (Strategy::LiveMomentum, "bullish", 0),
            (Strategy::LiveTradeFlow, "bearish", 1),
            (Strategy::LiveOrderBook, "bullish", 2),
            (Strategy::LiveVwapDislocation, "bearish", 3),
            (Strategy::LiveAbsorptionPressure, "bearish", 4),
        ];
        for (strategy, expected_direction, component) in cases {
            let mut signal = source();
            match component {
                0 => {
                    signal.components.momentum_by_window = vec![WindowedSignalValue {
                        window_seconds: 10, value: Some(0.05), ..Default::default()
                    }];
                    signal.components.trend_volatility_units_by_window = vec![WindowedSignalValue {
                        window_seconds: 300, value: Some(0.1), ..Default::default()
                    }];
                }
                1 => signal.components.trade_flow_by_window = vec![WindowedSignalValue {
                    window_seconds: 60, value: Some(-0.5), ..Default::default()
                }],
                2 => {
                    signal.components.book_imbalance_by_depth = vec![DepthSignalValue {
                        depth_levels: 5, value: Some(0.5)
                    }];
                    signal.components.momentum_by_window = vec![WindowedSignalValue {
                        window_seconds: 10, value: Some(-0.1), ..Default::default()
                    }];
                }
                3 => {
                    signal.components.vwap_dislocation_by_window = vec![WindowedSignalValue {
                        window_seconds: 60, value: Some(-2.0), sample_count: 5, span_ms: 10_000
                    }];
                    signal.components.momentum_by_window = vec![WindowedSignalValue {
                        window_seconds: 10, value: Some(0.1), ..Default::default()
                    }];
                }
                4 => {
                    signal.components.momentum_by_window = vec![WindowedSignalValue {
                        window_seconds: 10, value: Some(0.0), ..Default::default()
                    }];
                    signal.components.absorption_pressure_by_window = vec![AbsorptionPressureValue {
                        window_seconds: 30, replenishment_window_ms: 2_000,
                        touch_tolerance_bps: 2.0, value: Some(0.5)
                    }];
                }
                _ => unreachable!(),
            }
            let candidates = live_scout_direction_candidates(&signal, strategy, &parameters(strategy));
            assert_eq!(candidates.len(), 1, "{} should pass from its own input", strategy.as_str());
            assert_eq!(candidates[0].direction, expected_direction);
        }
    }

    #[test]
    fn live_parameter_probe_can_sample_a_direction_rejected_by_baseline() {
        let mut signal = source();
        signal.components.trade_flow_by_window = vec![WindowedSignalValue {
            window_seconds: 60, value: Some(0.1), ..Default::default()
        }];
        let parameters = parameters(Strategy::LiveTradeFlow);
        let probes = live_scout_direction_probes(&signal, Strategy::LiveTradeFlow, &parameters);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].direction, "bullish");
        assert!(!matches!(
            Strategy::LiveTradeFlow.evaluate(&StrategySignal::entry(&probes[0], &parameters)),
            Ok(StrategyDecision::Enter)
        ));
    }

    #[test]
    fn learned_book_depth_parameter_selects_the_matching_insight() {
        let mut signal = source();
        signal.components.book_imbalance_by_depth = vec![
            DepthSignalValue { depth_levels: 1, value: Some(0.5) },
            DepthSignalValue { depth_levels: 5, value: Some(-0.5) },
        ];
        signal.components.momentum_by_window = vec![WindowedSignalValue {
            window_seconds: 10, value: Some(0.1), ..Default::default()
        }];
        let mut parameters = parameters(Strategy::LiveOrderBook);
        let baseline = live_scout_direction_candidates(&signal, Strategy::LiveOrderBook, &parameters);
        assert_eq!(baseline[0].direction, "bearish");
        parameters.insert("book_depth_levels".into(), 1.0);
        let learned = live_scout_direction_candidates(&signal, Strategy::LiveOrderBook, &parameters);
        assert_eq!(learned[0].direction, "bullish");
    }

    #[test]
    fn conflicting_live_momentum_inputs_do_not_create_two_opposing_scouts() {
        let mut signal = source();
        signal.components.momentum_by_window = vec![WindowedSignalValue {
            window_seconds: 10, value: Some(0.05), ..Default::default()
        }];
        signal.components.trend_volatility_units_by_window = vec![WindowedSignalValue {
            window_seconds: 300, value: Some(-1.0), ..Default::default()
        }];
        assert!(live_scout_direction_candidates(
            &signal,
            Strategy::LiveMomentum,
            &parameters(Strategy::LiveMomentum),
        ).is_empty());
    }
}

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
    fn live_signal_sources_remain_available_alongside_daily_candidates() {
        let daily = signal("trend-50-200", "bullish");
        let mut live_source = signal("unattributed", "bullish");
        live_source.strategy_candidate = false;

        let sources = live_strategy_source_signals(&[daily, live_source.clone()]);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].strategy.as_deref(), Some("unattributed"));
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

#[cfg(test)]
mod signal_freshness_tests {
    use super::*;

    fn signal(strategy: Option<&str>, timestamp: u64, stale: bool) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp,
            stale,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: strategy.is_some(),
            strategy: strategy.map(str::to_owned),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn live_signals_expire_at_five_seconds_and_daily_signals_at_ten_minutes() {
        let now = 10_000;
        assert!(signal_is_fresh(&signal(None, 9_995_000, false), now));
        assert!(!signal_is_fresh(&signal(None, 9_994_000, false), now));
        assert!(signal_is_fresh(
            &signal(Some("donchian-20-btc200"), 9_400_000, false),
            now,
        ));
        assert!(!signal_is_fresh(
            &signal(Some("donchian-20-btc200"), 9_399_000, false),
            now,
        ));
        assert!(!signal_is_fresh(&signal(None, 9_999_000, true), now));
    }
}
