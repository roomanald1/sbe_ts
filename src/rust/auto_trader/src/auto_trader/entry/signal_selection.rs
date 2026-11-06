fn needs_pullback_confirmation(signal: &Signal) -> bool {
    !signal.strategy_candidate
        && matches!(
            signal.strategy.as_deref(),
            Some("live-order-book" | "live-vwap-dislocation")
        )
}

fn minimum_counter_momentum_percent(signal: &Signal) -> Option<f64> {
    let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
    let spec = strategy
        .parameter_specs()
        .into_iter()
        .find(|spec| spec.key == StrategyParameterKey::MinimumCounterMomentumPercent)?;
    let value = signal
        .strategy_decision
        .as_ref()?
        .get("strategy_parameters")?
        .get(StrategyParameterKey::MinimumCounterMomentumPercent.as_str())?
        .as_f64()?;
    let step_position = (value - spec.min) / spec.step;
    (value.is_finite()
        && value >= spec.min
        && value <= spec.max
        && (step_position - step_position.round()).abs() <= 1e-6)
        .then_some(value)
}

fn passes_pullback_confirmation(signal: &Signal) -> bool {
    if !needs_pullback_confirmation(signal) {
        return true;
    }
    let side = match signal.direction.as_str() {
        "bullish" => 1.0,
        "bearish" => -1.0,
        _ => return false,
    };
    let Some(minimum_counter_momentum_percent) = minimum_counter_momentum_percent(signal) else {
        return false;
    };
    signal
        .components
        .momentum_10s_percent
        .is_some_and(|momentum| {
            momentum.is_finite() && side * momentum < -minimum_counter_momentum_percent
        })
}

fn select_cycle_signal<'a>(
    input: SignalSelectionInput<'a>,
) -> Result<(&'a Signal, bool), CycleResult> {
    let SignalSelectionInput {
        dashboard,
        fresh_directional,
        learning_thresholds,
        risk_appetite,
        ..
    } = input;
    if let Some(signal) = fresh_directional
        .iter()
        .copied()
        .filter(|signal| {
            signal.strategy_candidate
                && !is_live_scout_only_signal(signal)
                && signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision.get("scout_parameter_probe"))
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
        })
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
    {
        return Ok((signal, false));
    }
    let strength_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            if is_live_scout_only_signal(signal) || !passes_pullback_confirmation(signal) {
                return false;
            }
            let context = market_context(signal);
            let learned_floor = learning_thresholds
                .get(&context)
                .map(|thresholds| thresholds.strength_floor)
                .unwrap_or(MIN_STRENGTH);
            let floor = (adaptive_missed_opportunity_floor(
                learned_floor,
                MissedOpportunityGate::Strength,
                &dashboard.missed_opportunity_bands,
            ) * risk_floor_scale(risk_appetite))
            .clamp(0.0, 1.0);
            signal.strength.is_finite() && signal.strength >= floor
        })
        .copied()
        .collect::<Vec<_>>();
    let strict_candidate = strength_qualified
        .iter()
        .copied()
        .filter(|signal| {
            if is_live_scout_only_signal(signal) {
                return false;
            }
            // Insights' walk-forward rows use generic directional outcomes
            // grouped by symbol and horizon. They do not replay a named
            // strategy or its parameter profile, so they cannot validate or
            // reject strategy-attributed signals.
            if signal.strategy.is_some() || signal.strategy_candidate {
                return true;
            }
            let walk_forward = dashboard.walk_forward.iter().find(|item| {
                item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
            });
            let side_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.all_signal_long
                } else {
                    &item.all_signal_short
                }
            });
            let research_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.research_logistic_long
                } else {
                    &item.research_logistic_short
                }
            });
            side_evidence.is_some_and(|evidence| {
                let research_supports = research_evidence.is_none_or(|research| {
                    research.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || research
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                evidence.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                    && evidence
                        .direction_match_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                    && evidence
                        .average_net_return_percent
                        .is_some_and(|net| net > 0.0)
                    && evidence
                        .positive_net_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_RATE)
                    && evidence
                        .positive_net_rate_lower_95
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
                    && research_supports
            })
        })
        .max_by(|a, b| a.strength.total_cmp(&b.strength));
    if let Some(signal) = strict_candidate {
        return Ok((signal, false));
    }
    let exploration_strength_floor = MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite);
    let exploration_score_floor = MIN_EXPLORATION_SCORE * risk_floor_scale(risk_appetite);
    if let Some(signal) = fresh_directional
        .iter()
        .copied()
        .filter(|signal| {
            signal.strategy.is_none()
                && !signal.strategy_candidate
                && !is_live_scout_only_signal(signal)
                && passes_pullback_confirmation(signal)
                && signal.strength.is_finite()
                && signal.strength >= exploration_strength_floor
                && signal.score.is_finite()
                && signal.score.abs() >= exploration_score_floor
        })
        .max_by(|a, b| {
            (a.strength * a.score.abs()).total_cmp(&(b.strength * b.score.abs()))
        })
    {
        return Ok((signal, true));
    }
    let reason = format_no_tradeable_bias(&input, &[], &strength_qualified);
    Err(skipped(&reason))
}

struct PreparedSignals {
    signals: Vec<Signal>,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
}

#[cfg(test)]
mod signal_selection_behavior_tests {
    use super::*;

    static EMPTY_LEARNING_THRESHOLDS: std::sync::LazyLock<
        BTreeMap<String, strategies::ContextThresholds>,
    > = std::sync::LazyLock::new(BTreeMap::new);
    static EMPTY_POSITIONS: std::sync::LazyLock<BTreeMap<String, TraderPosition>> =
        std::sync::LazyLock::new(BTreeMap::new);

    fn signal(strength: f64, score: f64, strategy: Option<&str>, candidate: bool) -> Signal {
        Signal {
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength,
            timestamp: 100,
            stale: false,
            score,
            ranging_probability: None,
            components: SignalComponents {
                trend_5m_volatility_units: Some(2.0),
                ..SignalComponents::default()
            },
            strategy_candidate: candidate,
            strategy: strategy.map(str::to_owned),
            strategy_decision: None,
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    fn input<'a>(
        dashboard: &'a Dashboard,
        signals: &'a [Signal],
        fresh: &'a [&'a Signal],
        risk: f64,
    ) -> SignalSelectionInput<'a> {
        SignalSelectionInput {
            dashboard,
            signals,
            fresh_directional: fresh,
            now: 100,
            inferred_direction_count: 0,
            neutral_direction_floor: 0.15,
            stale_signal_count: 0,
            stalest_signal_age_seconds: 0,
            stale_symbols: &[],
            non_directional_count: 0,
            position_blocked_count: 0,
            position_block_details: &[],
            risk_appetite: risk,
            learning_thresholds: &EMPTY_LEARNING_THRESHOLDS,
            open_positions: &EMPTY_POSITIONS,
            max_entries_per_auto_position: 3,
        }
    }

    fn dashboard(walk_forward: Vec<WalkForward>) -> Dashboard {
        Dashboard {
            best: Vec::new(),
            walk_forward: Arc::new(walk_forward),
            missed_opportunity_bands: Vec::new(),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        }
    }

    #[test]
    fn daily_strategy_candidates_take_priority_and_pullback_strategies_require_counter_momentum() {
        let daily = signal(0.2, 0.2, Some("donchian-20-btc200"), true);
        let live = signal(0.9, 0.9, None, false);
        let signals = vec![daily.clone(), live.clone()];
        let fresh = vec![&daily, &live];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(&empty_dashboard, &signals, &fresh, 50.0))
            .unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert_eq!(selected.0.symbol, "BTC-USDT");
        assert!(selected.0.strategy_candidate);
        assert!(!selected.1);

        let mut pullback = signal(0.9, 0.9, Some("live-order-book"), false);
        assert!(needs_pullback_confirmation(&pullback));
        pullback.components.momentum_10s_percent = Some(-0.1);
        assert!(!passes_pullback_confirmation(&pullback));
        pullback.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": { "minimum_counter_momentum_percent": 0.0 }
        }));
        assert!(passes_pullback_confirmation(&pullback));
        pullback.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": { "minimum_counter_momentum_percent": 0.02 }
        }));
        pullback.components.momentum_10s_percent = Some(-0.01);
        assert!(!passes_pullback_confirmation(&pullback));
        pullback.components.momentum_10s_percent = Some(-0.03);
        assert!(passes_pullback_confirmation(&pullback));
        pullback.direction = "neutral".into();
        assert!(!passes_pullback_confirmation(&pullback));
    }

    #[test]
    fn selects_strict_evidence_before_capped_exploration_without_history() {
        let strict = signal(0.9, 0.9, None, false);
        let evidence = serde_json::from_value::<WalkForward>(serde_json::json!({
            "symbol":"BTC-USDT", "horizon_seconds":300,
            "all_signal_long":{"sample_count":40,"direction_match_rate":0.8,"average_net_return_percent":0.5,"positive_net_rate":0.8,"positive_net_rate_lower_95":0.6},
            "research_logistic_long":{"sample_count":40,"average_net_return_percent":0.3}
        })).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        let signals = vec![strict.clone()];
        let fresh = vec![&strict];
        let strict_dashboard = dashboard(vec![evidence]);
        let selected = select_cycle_signal(input(&strict_dashboard, &signals, &fresh, 50.0))
            .unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert_eq!(selected.0.symbol, strict.symbol);
        assert!(!selected.1);

        let unvalidated = signal(0.9, 0.9, None, false);
        let unvalidated_signals = vec![unvalidated.clone()];
        let unvalidated_fresh = vec![&unvalidated];
        let empty_dashboard = dashboard(Vec::new());
        let (selected, exploratory) = select_cycle_signal(input(
            &empty_dashboard,
            &unvalidated_signals,
            &unvalidated_fresh,
            50.0,
        ))
        .unwrap_or_else(|_| panic!("eligible paper exploration should be selected"));
        assert_eq!(selected.symbol, unvalidated.symbol);
        assert!(exploratory);

        let weak = signal(0.9, 0.29, None, false);
        let weak_signals = vec![weak.clone()];
        let weak_fresh = vec![&weak];
        assert!(select_cycle_signal(input(
            &empty_dashboard,
            &weak_signals,
            &weak_fresh,
            50.0,
        ))
        .is_err());
    }

    #[test]
    fn generic_signal_walk_forward_does_not_veto_strategy_attributed_signals() {
        let strategy_signal = signal(0.9, 0.9, Some("live-momentum"), false);
        let signals = vec![strategy_signal.clone()];
        let fresh = vec![&strategy_signal];
        let no_strategy_evidence = dashboard(Vec::new());

        let selected = select_cycle_signal(input(&no_strategy_evidence, &signals, &fresh, 50.0));

        let selected = selected
            .unwrap_or_else(|_| panic!("generic signal evidence must not veto a named strategy"));
        assert_eq!(selected.0.strategy.as_deref(), Some("live-momentum"));
    }
}
