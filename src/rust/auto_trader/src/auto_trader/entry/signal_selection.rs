
fn needs_pullback_confirmation(signal: &Signal) -> bool {
    !signal.strategy_candidate
        && matches!(
            signal.strategy.as_deref(),
            Some("live-order-book" | "live-vwap-dislocation")
        )
}

fn minimum_counter_momentum_percent(signal: &Signal) -> Option<f64> {
    let strategy = signal.strategy.as_deref().and_then(Strategy::from_name)?;
    let spec = strategy.parameter_specs()
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
    signal.components.momentum_10s_percent.is_some_and(|momentum| {
        momentum.is_finite() && side * momentum < -minimum_counter_momentum_percent
    })
}

fn select_cycle_signal<'a>(
    input: SignalSelectionInput<'a>,
) -> Result<(&'a Signal, bool, bool), CycleResult> {
    let SignalSelectionInput {
        state,
        dashboard,
        fresh_directional,
        neutral_scout_candidates,
        now,
        learning_thresholds,
        risk_appetite,
        active_scout_count,
        scout_max_active_positions,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        ..
    } = input;
    if let Some(signal) = fresh_directional
        .iter()
        .copied()
        .filter(|signal| signal.strategy_candidate)
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
    {
        return Ok((signal, false, false));
    }
    if input.delegate_scout_fallback {
        let reason = format_no_tradeable_bias(&input, &[], &[]);
        return Err(skipped(&reason));
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
    let (signal, exploratory, scout) = if let Some(signal) = strict_candidate {
        (signal, false, false)
    } else {
        let scout_candidate = fresh_directional
            .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .filter(|signal| {
                    // Scouts are deliberately small probes. A missing pullback
                    // confirmation keeps a signal out of normal entry, but must not
                    // prevent a scout from sampling a directionally qualified opportunity.
                    if scout_walk_forward_thresholds(signal).is_none() {
                        return false;
                    }
                    let live_scout_only = is_live_scout_only_signal(signal);
                    let (strength_floor, score_floor) =
                        adaptive_scout_thresholds(active_scout_count, scout_max_active_positions);
                    let strength_floor =
                        (strength_floor * risk_floor_scale(risk_appetite)).clamp(0.0, 1.0);
                    let score_floor = (score_floor * risk_floor_scale(risk_appetite)).max(0.0);
                    if active_scout_count >= scout_max_active_positions
                        || !signal.strength.is_finite()
                        || !signal.score.is_finite()
                        || (!live_scout_only
                            && (signal.strength < strength_floor || signal.score.abs() < score_floor))
                    {
                        return false;
                    }
                    let evidence = dashboard.walk_forward.iter().find(|item| {
                        item.symbol == signal.symbol && is_entry_evidence_horizon(item.horizon_seconds)
                    });
                    let side = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.all_signal_long
                        } else {
                            &item.all_signal_short
                        }
                    });
                    let research = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.research_logistic_long
                        } else {
                            &item.research_logistic_short
                        }
                    });
                    let (minimum_samples, side_minimum_samples, positive_net_floor) =
                        scout_walk_forward_thresholds(signal).unwrap_or((usize::MAX, usize::MAX, 1.0));
                    let negative_at = |metrics: &SideWalkForwardMetrics, minimum_samples| {
                        metrics.sample_count >= minimum_samples
                            && (metrics.average_net_return_percent.is_none_or(|net| net <= 0.0)
                                || metrics.positive_net_rate.is_none_or(|rate| rate < positive_net_floor))
                    };
                    let measured_negative = side.is_some_and(|metrics| negative_at(metrics, minimum_samples))
                        || research.is_some_and(|metrics| negative_at(metrics, side_minimum_samples));
                    !measured_negative || active_scout_count == 0
                })
            .max_by(|a, b| a.strength.total_cmp(&b.strength));
        let Some(signal) = scout_candidate else {
            let reason =
                format_no_tradeable_bias(&input, &[], &strength_qualified);
            for candidate in fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
            {
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && is_entry_evidence_horizon(item.horizon_seconds)
                });
                let mut payload = signal_audit_payload(
                    candidate,
                    "decision",
                    now.saturating_mul(1000),
                    "skipped",
                    Some(&reason),
                    Some(true),
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    evidence,
                );
                payload["entry_gate"] = candidate_entry_audit_value(
                    candidate,
                    dashboard,
                    learning_thresholds,
                    risk_appetite,
                    active_scout_count,
                    scout_max_active_positions,
                    "rejected",
                );
                queue_auto_trader_audit(state, payload);
            }
            return Err(skipped(&reason));
        };
        (signal, false, true)
    };
    Ok((signal, exploratory, scout))
}

struct PreparedSignals {
    signals: Vec<Signal>,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
}

#[cfg(test)]
mod signal_selection_behavior_tests {
    use super::*;

    static EMPTY_LEARNING_THRESHOLDS: std::sync::LazyLock<BTreeMap<String, strategies::ContextThresholds>> =
        std::sync::LazyLock::new(BTreeMap::new);
    static EMPTY_POSITIONS: std::sync::LazyLock<BTreeMap<String, TraderPosition>> =
        std::sync::LazyLock::new(BTreeMap::new);

    fn signal(strength: f64, score: f64, strategy: Option<&str>, candidate: bool) -> Signal {
        Signal {
            symbol: "BTC-USDT".into(), direction: "bullish".into(), strength,
            timestamp: 100, stale: false, score, ranging_probability: None,
            components: SignalComponents { trend_5m_volatility_units: Some(2.0), ..SignalComponents::default() },
            strategy_candidate: candidate, strategy: strategy.map(str::to_owned),
            strategy_decision: None, market_regime: None, news_attribution: None, entry_gate: None,
        }
    }

    fn state() -> AutoTraderState {
        let (state_updates, _) = tokio::sync::broadcast::channel(8);
        let audit_tx = AuditQueue::new(DECISION_AUDIT_QUEUE_CAPACITY);
        let execution_audit_tx = AuditQueue::new(EXECUTION_AUDIT_QUEUE_CAPACITY);
        AutoTraderState {
            client: reqwest::Client::new(),
            latest_daily_strategy_snapshot: Arc::new(Mutex::new(None)),
            strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            settings: Arc::new(Mutex::new(Settings::default())),
            settings_persist_notify: Arc::new(Notify::new()), state_updates,
            settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
            latest_execution_parameters: Arc::new(Mutex::new(None)), cycle_lock: Arc::new(Mutex::new(())),
            scout_lifecycle_last_update_ms: Arc::new(AtomicU64::new(0)),
            pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())), daily_signal_notify: Arc::new(Notify::new()),
            pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())), live_signal_notify: Arc::new(Notify::new()),
            signal_book_prewarms: Arc::new(Mutex::new(BTreeMap::new())), audit_tx, execution_audit_tx,
            audit_health: Arc::new(AtomicU64::new(3)), audit_failure_latched: Arc::new(AtomicBool::new(false)),
            gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
            insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
            cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
            cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            settings_path: PathBuf::new(), dated_carry: Arc::new(Mutex::new(DatedCarryBook::default())),
            dated_carry_work_lock: Arc::new(Mutex::new(())), dated_carry_path: PathBuf::new(),
        }
    }

    fn input<'a>(
        state: &'a AutoTraderState, dashboard: &'a Dashboard, signals: &'a [Signal],
        fresh: &'a [&'a Signal], neutral: &'a [Signal], risk: f64, active_scouts: usize,
    ) -> SignalSelectionInput<'a> {
        SignalSelectionInput {
            state, dashboard, signals, fresh_directional: fresh, neutral_scout_candidates: neutral,
            now: 100, inferred_direction_count: 0, neutral_direction_floor: 0.15,
            stale_signal_count: 0, stalest_signal_age_seconds: 0, stale_symbols: &[],
            non_directional_count: 0, position_blocked_count: 0, position_block_details: &[],
            risk_appetite: risk, active_scout_count: active_scouts,
            learning_thresholds: &EMPTY_LEARNING_THRESHOLDS,
            delegate_scout_fallback: false,
            scout_max_active_positions: DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS,
            open_positions: &EMPTY_POSITIONS, max_entries_per_auto_position: 3,
            counterfactual_stop_loss_percent: 0.5, counterfactual_take_profit_percent: 0.8,
        }
    }

    fn dashboard(walk_forward: Vec<WalkForward>) -> Dashboard {
        Dashboard { best: Vec::new(), walk_forward: Arc::new(walk_forward),
            missed_opportunity_bands: Vec::new(), markets: Arc::new(BTreeMap::new()),
            queued_at: None, queue_wait_ms: None }
    }

    #[test]
    fn daily_strategy_candidates_take_priority_and_pullback_strategies_require_counter_momentum() {
        let state = state();
        let daily = signal(0.2, 0.2, Some("donchian-20-btc200"), true);
        let live = signal(0.9, 0.9, None, false);
        let signals = vec![daily.clone(), live.clone()];
        let fresh = vec![&daily, &live];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(&state, &empty_dashboard, &signals, &fresh, &[], 50.0, 0)).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert_eq!(selected.0.symbol, "BTC-USDT");
        assert!(selected.0.strategy_candidate);
        assert!(!selected.1 && !selected.2);

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
    fn scout_can_probe_pullback_strategy_without_pullback_confirmation() {
        let state = state();
        let candidate = signal(0.9, 0.9, Some("live-order-book"), false);
        assert!(needs_pullback_confirmation(&candidate));
        assert!(!passes_pullback_confirmation(&candidate));
        let signals = vec![candidate.clone()];
        let fresh = vec![&candidate];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(
            &state, &empty_dashboard, &signals, &fresh, &[], 50.0, 0,
        ))
        .unwrap_or_else(|_| panic!("a small scout should sample a qualified signal without pullback confirmation"));
        assert_eq!(selected.0.strategy.as_deref(), Some("live-order-book"));
        assert!(!selected.1 && selected.2);
    }

    #[test]
    fn selects_strict_evidence_before_scouting_unvalidated_signals() {
        let state = state();
        let strict = signal(0.9, 0.9, None, false);
        let evidence = serde_json::from_value::<WalkForward>(serde_json::json!({
            "symbol":"BTC-USDT", "horizon_seconds":300,
            "all_signal_long":{"sample_count":40,"direction_match_rate":0.8,"average_net_return_percent":0.5,"positive_net_rate":0.8,"positive_net_rate_lower_95":0.6},
            "research_logistic_long":{"sample_count":40,"average_net_return_percent":0.3}
        })).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        let signals = vec![strict.clone()]; let fresh = vec![&strict];
        let strict_dashboard = dashboard(vec![evidence]);
        let selected = select_cycle_signal(input(&state, &strict_dashboard, &signals, &fresh, &[], 50.0, 0)).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert_eq!(selected.0.symbol, strict.symbol);
        assert!(!selected.1 && !selected.2);

        let unvalidated = signal(0.9, 0.9, None, false);
        let unvalidated_signals = vec![unvalidated.clone()]; let unvalidated_fresh = vec![&unvalidated];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(&state, &empty_dashboard, &unvalidated_signals, &unvalidated_fresh, &[], 50.0, 0)).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert!(!selected.1);
        assert!(selected.2);
    }

    #[test]
    fn live_scout_uses_its_own_entry_rule_without_aggregate_score_floors() {
        let state = state();
        let mut candidate = signal(0.01, 0.001, Some("live-trade-flow"), false);
        candidate.strategy_decision = Some(serde_json::json!({
            "scout_only": true,
            "strategy_parameters": {
                "scout_minimum_walk_forward_samples": 10.0,
                "scout_minimum_positive_net_rate": 0.45
            }
        }));
        let signals = vec![candidate.clone()];
        let fresh = vec![&candidate];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(
            &state, &empty_dashboard, &signals, &fresh, &[], 50.0, 0,
        )).unwrap_or_else(|_| panic!("a strategy-qualified scout should not need aggregate score floors"));
        assert_eq!(selected.0.strategy.as_deref(), Some("live-trade-flow"));
        assert!(selected.2);
    }

    #[test]
    fn live_scout_walk_forward_thresholds_are_strategy_parameters() {
        let state = state();
        let mut candidate = signal(0.5, 0.5, Some("live-trade-flow"), false);
        candidate.strategy_decision = Some(serde_json::json!({
            "scout_only": true,
            "strategy_parameters": {
                "scout_minimum_walk_forward_samples": 20.0,
                "scout_minimum_positive_net_rate": 0.45
            }
        }));
        let signals = vec![candidate.clone()];
        let fresh = vec![&candidate];
        let evidence = serde_json::from_value::<WalkForward>(serde_json::json!({
            "symbol":"BTC-USDT", "horizon_seconds":300,
            "all_signal_long":{"sample_count":15,"average_net_return_percent":-0.2,"positive_net_rate":0.0},
            "research_logistic_long":{"sample_count":15,"average_net_return_percent":-0.2}
        })).unwrap();
        let dashboard = dashboard(vec![evidence]);
        let selected = select_cycle_signal(input(&state, &dashboard, &signals, &fresh, &[], 50.0, 0))
            .unwrap_or_else(|_| panic!("the 20-sample Scout threshold should permit this 15-sample probe"));
        assert_eq!(selected.0.strategy.as_deref(), Some("live-trade-flow"));
        assert!(selected.2);

        candidate.strategy_decision.as_mut().unwrap()["strategy_parameters"]["scout_minimum_walk_forward_samples"] = serde_json::json!(10.0);
        let signals = vec![candidate.clone()];
        let fresh = vec![&candidate];
        let first_probe = select_cycle_signal(input(&state, &dashboard, &signals, &fresh, &[], 50.0, 0))
            .unwrap_or_else(|_| panic!("negative history should permit one Scout probe when inventory is clear"));
        assert!(first_probe.2);
        assert!(select_cycle_signal(input(&state, &dashboard, &signals, &fresh, &[], 50.0, 1)).is_err());
    }

    #[test]
    fn scout_fallback_respects_inventory_and_measured_negative_history() {
        let state = state();
        let candidate = signal(0.5, 0.5, None, false);
        let signals = vec![candidate.clone()]; let fresh = vec![&candidate];
        let empty_dashboard = dashboard(Vec::new());
        let selected = select_cycle_signal(input(&state, &empty_dashboard, &signals, &fresh, &[], 50.0, 0)).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert!(!selected.1 && selected.2);

        let neutral = signal(0.5, 0.5, None, false);
        let none_fresh: Vec<&Signal> = Vec::new();
        let neutral_candidates = vec![neutral.clone()];
        let selected = select_cycle_signal(input(&state, &empty_dashboard, &[], &none_fresh, &neutral_candidates, 50.0, 0)).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        assert!(!selected.1 && selected.2);
        assert!(select_cycle_signal(input(&state, &empty_dashboard, &[], &none_fresh, &neutral_candidates, 50.0, DEFAULT_SCOUT_MAX_ACTIVE_POSITIONS)).is_err());

        let evidence = serde_json::from_value::<WalkForward>(serde_json::json!({
            "symbol":"BTC-USDT", "horizon_seconds":300,
            "all_signal_long":{"sample_count":20,"average_net_return_percent":-0.1,"positive_net_rate":0.2}
        })).unwrap_or_else(|_| panic!("expected signal selection to succeed"));
        let negative_dashboard = dashboard(vec![evidence]);
        assert!(select_cycle_signal(input(&state, &negative_dashboard, &[], &none_fresh, &neutral_candidates, 50.0, 1)).is_err());
    }
}
