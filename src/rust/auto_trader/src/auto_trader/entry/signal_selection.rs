
fn needs_pullback_confirmation(signal: &Signal) -> bool {
    !signal.strategy_candidate
        && matches!(
            signal.strategy.as_deref(),
            Some("live-order-book" | "live-vwap-dislocation")
        )
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
    signal
        .components
        .momentum_10s_percent
        .is_some_and(|momentum| momentum.is_finite() && side * momentum < 0.0)
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
        learning,
        risk_appetite,
        active_scout_count,
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
    let strength_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            if !passes_pullback_confirmation(signal) {
                return false;
            }
            let context = market_context(signal);
            let learned_floor = adaptive_strength_floor(&learning, &context);
            let floor = (adaptive_missed_opportunity_floor(
                learned_floor,
                "strength",
                &dashboard.missed_opportunity_bands,
            ) * risk_floor_scale(risk_appetite))
            .clamp(0.0, 1.0);
            signal.strength.is_finite() && signal.strength >= floor
        })
        .copied()
        .collect::<Vec<_>>();
    let exploration_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            if !passes_pullback_confirmation(signal) {
                return false;
            }
            let context = market_context(signal);
            !is_mixed_market_context(&context)
                && signal.strength.is_finite()
                && signal.strength >= MIN_EXPLORATION_STRENGTH * risk_floor_scale(risk_appetite)
                && signal.score.is_finite()
                && signal.score.abs()
                    >= (adaptive_exploration_score_floor(
                        &learning,
                        &context,
                        &dashboard.missed_opportunity_bands,
                    ) * risk_floor_scale(risk_appetite))
                    .max(0.0)
        })
        .copied()
        .collect::<Vec<_>>();
    let strict_candidate = strength_qualified
        .iter()
        .copied()
        .filter(|signal| {
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
        let exploratory = exploration_qualified
            .iter()
            .copied()
            .filter(|signal| {
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
                let research_supports = research_evidence.is_none_or(|evidence| {
                    evidence.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || evidence
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                research_supports
                    && side_evidence.is_none_or(|evidence| {
                        evidence.sample_count < MIN_EXPLORATION_EVIDENCE_SAMPLES
                            || (evidence
                                .average_net_return_percent
                                .is_some_and(|net| net > 0.0)
                                && evidence
                                    .positive_net_rate
                                    .is_some_and(|rate| rate >= NEGATIVE_EVIDENCE_FLOOR))
                    })
            })
            .max_by(|a, b| a.strength.total_cmp(&b.strength));
        let scout_candidate = if exploratory.is_none() {
            fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .filter(|signal| {
                    if !passes_pullback_confirmation(signal) {
                        return false;
                    }
                    let context = market_context(signal);
                    let (strength_floor, score_floor) =
                        adaptive_scout_thresholds(&learning, &context, active_scout_count);
                    let strength_floor =
                        (strength_floor * risk_floor_scale(risk_appetite)).clamp(0.0, 1.0);
                    let score_floor = (score_floor * risk_floor_scale(risk_appetite)).max(0.0);
                    if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS
                        || !signal.strength.is_finite()
                        || signal.strength < strength_floor
                        || !signal.score.is_finite()
                        || signal.score.abs() < score_floor
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
                    let measured_negative = side.is_some_and(|m| {
                        m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                            && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                                || m.positive_net_rate
                                    .is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR))
                    }) || research.is_some_and(|m| {
                        m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                            && m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                    });
                    !measured_negative || active_scout_count == 0
                })
                .max_by(|a, b| a.strength.total_cmp(&b.strength))
        } else {
            None
        };
        let Some(signal) = exploratory.or(scout_candidate) else {
            let reason =
                format_no_tradeable_bias(&input, &exploration_qualified, &strength_qualified);
            for candidate in fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
            {
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && is_entry_evidence_horizon(item.horizon_seconds)
                });
                let payload = signal_audit_payload(
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
                queue_auto_trader_audit(state, payload);
            }
            return Err(skipped(&reason));
        };
        (signal, true, exploratory.is_none())
    };
    Ok((signal, exploratory, scout))
}

struct PreparedSignals {
    signals: Vec<Signal>,
    inferred_direction_count: usize,
    neutral_direction_floor: f64,
}

