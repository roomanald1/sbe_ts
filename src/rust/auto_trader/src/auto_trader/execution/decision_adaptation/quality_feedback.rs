const MIN_EXECUTION_QUALITY_SAMPLES: usize = 10;
const MIN_EXECUTION_QUALITY_FILLS: usize = 5;

#[cfg(test)]
fn execution_quality_key(context: &str, order_type: &str) -> String {
    format!("{context}:exec:{order_type}")
}

fn decay_execution_quality(stats: &mut ExecutionLearningStats, now_ms: u64) {
    if stats.updated_at_ms == 0 {
        stats.updated_at_ms = now_ms;
        return;
    }
    let elapsed = now_ms.saturating_sub(stats.updated_at_ms);
    if elapsed == 0 {
        return;
    }
    let weight = 0.5_f64.powf(elapsed as f64 / LEARNING_HALF_LIFE_MS as f64);
    stats.samples = (stats.samples as f64 * weight).round() as usize;
    stats.filled_samples = ((stats.filled_samples as f64 * weight).round() as usize)
        .min(stats.samples);
    stats.fill_rate_total *= weight;
    stats.cost_notional *= weight;
    stats.all_in_cost_notional_bps *= weight;
    stats.updated_at_ms = now_ms;
}

fn record_execution_order_quality(
    feedback_outbox: &mut Vec<strategies::ExecutionOutcomeEvent>,
    recorded_order_ids: &mut BTreeSet<String>,
    order: &TraderOrder,
) {
    let id = order.timestamp.to_string();
    if recorded_order_ids.contains(&id)
        || feedback_outbox.iter().any(|event| event.event_id == id)
        || !matches!(
            order.status.as_str(),
            "filled" | "partially_filled_closed" | "expired_or_cancelled" | "cancelled" | "failed" | "interrupted" | "accepted"
        )
        || (order.status == "accepted" && order.execution_order_id.is_some())
        || !matches!(order.purpose.as_str(), "signal" | "scout")
    {
        return;
    }
    let Some(context) = order.learning_context.as_deref() else {
        return;
    };
    let order_type = order.order_type.split(" · ").next().unwrap_or(&order.order_type);
    let mut cost_notional = 0.0;
    let mut all_in_cost_notional_bps = 0.0;
    if order.filled_quantity > 0.0 {
        if let (Some(price), Some(mid)) = (
            order.average_price.filter(|value| value.is_finite() && *value > 0.0),
            order.decision_mid.filter(|value| value.is_finite() && *value > 0.0),
        ) {
            let notional = order.filled_quantity * price;
            if notional.is_finite() && notional > 0.0 {
                let shortfall_bps = if order.side == "buy" {
                    (price / mid - 1.0) * 10_000.0
                } else {
                    (1.0 - price / mid) * 10_000.0
                };
                let fee_bps = order.fees_paid.abs() / notional * 10_000.0;
                let cost_bps = shortfall_bps + fee_bps;
                if cost_bps.is_finite() {
                    cost_notional = notional;
                    all_in_cost_notional_bps = cost_bps * notional;
                }
            }
        }
    }
    feedback_outbox.push(strategies::ExecutionOutcomeEvent {
        event_id: id.clone(),
        observed_at_ms: current_time_ms(),
        context: context.to_owned(),
        order_type: order_type.to_owned(),
        filled: order.filled_quantity > 0.0,
        fill_rate: order.fill_rate.clamp(0.0, 1.0),
        cost_notional,
        all_in_cost_notional_bps,
    });
    recorded_order_ids.insert(id);
    while recorded_order_ids.len() > 20_000 {
        if let Some(oldest) = recorded_order_ids.iter().next().cloned() {
            recorded_order_ids.remove(&oldest);
        }
    }
}

#[cfg(test)]
fn aggregate_execution_quality(
    learning: &BTreeMap<String, ExecutionLearningStats>,
    prefix: &str,
    suffix: &str,
) -> ExecutionLearningStats {
    learning
        .iter()
        .filter(|(key, _)| key.starts_with(prefix) && key.ends_with(suffix))
        .map(|(_, stats)| {
            let mut recent = stats.clone();
            decay_execution_quality(&mut recent, current_time_ms());
            recent
        })
        .fold(ExecutionLearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.filled_samples += stats.filled_samples;
            total.fill_rate_total += stats.fill_rate_total;
            total.cost_notional += stats.cost_notional;
            total.all_in_cost_notional_bps += stats.all_in_cost_notional_bps;
            total
        })
}

fn score_execution_order_type(
    signal: &Signal,
    risk_appetite: f64,
    notional: f64,
    baseline: AutoOrderType,
    learned: AutoOrderType,
    book: Option<DeepBookAssessment>,
    schedule_depth_threshold: f64,
) -> AutoOrderType {
    let side_depth = book.map(|assessment| {
        if signal.direction == "bullish" { assessment.ask_depth_10bps } else { assessment.bid_depth_10bps }
    }).unwrap_or(0.0);
    let depth_ratio = if side_depth > 0.0 { notional / side_depth } else { 0.0 };
    // Large parent orders use the scheduler to work child slices against volume.
    let large_order = depth_ratio >= schedule_depth_threshold;
    let urgency = (signal.components.momentum_10s_percent.unwrap_or(0.0).abs() / 0.30)
        .max(signal.components.pressure_without_price_move.unwrap_or(0.0).abs())
        .clamp(0.0, 1.0);
    // `learned` is returned only after its observed all-in cost beats the
    // baseline without materially worse fill rate. Preserve that choice for
    // normal-sized, non-urgent orders instead of letting the heuristic score
    // silently switch back to the baseline (or another candidate).
    if !large_order
        && learned != baseline
        && urgency < 0.80
        && !(learned == AutoOrderType::Ioc && baseline == AutoOrderType::PostOnlyLimit)
    {
        return learned;
    }
    let ranging = signal.ranging_probability.unwrap_or(0.0).clamp(0.0, 1.0);
    let trend = (signal.components.trend_5m_volatility_units.unwrap_or(0.0).abs() / 3.0).clamp(0.0, 1.0);
    let fee_bps = configured_taker_fee_rate() * 10_000.0;
    let impact_bps = book.map(|assessment| assessment.impact_bps).filter(|value| value.is_finite()).unwrap_or(0.0);
    let scaled_impact_bps = if side_depth > 0.0 {
        impact_bps * (notional / (side_depth * DEEP_BOOK_MAX_PARTICIPATION).max(1.0)).clamp(0.25, 4.0)
    } else { impact_bps };
    let candidates = [baseline, learned, AutoOrderType::Vwap, AutoOrderType::Twap,
        AutoOrderType::PostOnlyLimit, AutoOrderType::Gtd, AutoOrderType::Ioc, AutoOrderType::Market];
    let mut best = learned;
    let mut best_score = f64::NEG_INFINITY;
    for (index, candidate) in candidates.into_iter().enumerate() {
        if candidates[..index].contains(&candidate) { continue; }
        if large_order && !matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) { continue; }
        if candidate == AutoOrderType::Ioc
            && baseline == AutoOrderType::PostOnlyLimit
            && urgency < 0.80
        {
            continue;
        }
        let maker = matches!(candidate, AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd);
        let impact_multiplier = match candidate {
            AutoOrderType::Vwap | AutoOrderType::Twap => 0.45,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => 0.10,
            _ => 1.0,
        };
        let expected_cost_bps = (if maker { configured_fee_rates_maker_bps() } else { fee_bps })
            + scaled_impact_bps * impact_multiplier
            + if maker { 0.0 } else { signal.components.aggressor_cost_bps.unwrap_or(0.0).abs() * 0.25 };
        let mut score = if candidate == baseline { 1.0 } else { 0.0 };
        if candidate == learned { score += 0.35; }
        score += match candidate {
            AutoOrderType::Vwap => signal.strength.clamp(0.0, 1.0) * 0.6,
            AutoOrderType::Twap => ranging * 0.7,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => ranging * 0.8 - urgency * 0.7,
            AutoOrderType::Market | AutoOrderType::Ioc => urgency * 0.7 + trend * 0.2,
            _ => 0.0,
        };
        if matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) {
            score += ((depth_ratio - MIN_SCHEDULE_DEPTH_RATIO)
                / (schedule_depth_threshold - MIN_SCHEDULE_DEPTH_RATIO).max(0.01))
                .clamp(0.0, 1.0) * 2.0;
            score -= urgency * (risk_appetite / 100.0) * 0.7;
        }
        // Cost is a meaningful score component, but its bounded weight cannot
        // override a strong signal/regime fit by itself.
        score -= (expected_cost_bps / 10.0).clamp(0.0, 2.0) * 0.35;
        if score > best_score { best = candidate; best_score = score; }
    }
    best
}


const LOSS_CLUSTER_SIZE: usize = 3;
const LOSS_CLUSTER_WINDOW_MS: u64 = 6 * 60 * 60 * 1_000;
const LOSS_CLUSTER_SIZE_MULTIPLIER: f64 = 0.5;

fn has_recent_profile_loss_cluster(signal: &Signal, settings: &Settings, now_ms: u64) -> bool {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else { return false; };
    let parameters = signal_strategy_parameters(signal);
    let profile = strategies::profile_id(&signal.symbol, strategy.as_str(), &parameters);
    let key = format!("{profile}|{}", market_context(signal));
    let Some(returns) = settings.recent_profile_context_returns.get(&key) else { return false; };
    returns.len() >= LOSS_CLUSTER_SIZE && returns.iter().take(LOSS_CLUSTER_SIZE).all(|(timestamp, value)| {
        *timestamp <= now_ms && now_ms.saturating_sub(*timestamp) <= LOSS_CLUSTER_WINDOW_MS
            && value.is_finite() && *value < 0.0
    })
}

fn strategy_post_stop_cooldown_ms(signal: &Signal, strategy: &str, symbol: &str, settings: &Settings, now_ms: u64) -> u64 {
    // This is a learned strategy parameter. Its value is carried by the
    // entering Signal and therefore remains attributable to the Scout arm
    // which tested it. A manual profile override is already injected there.
    let profile_cooldown = signal_strategy_parameters(signal)
        .get("post_stop_cooldown_minutes")
        .copied()
        .filter(|value| value.is_finite() && (0.5..=10.0).contains(value))
        .map(|value| (value * 60_000.0) as u64);
    if has_recent_profile_loss_cluster(signal, settings, now_ms) {
        return MAX_POST_STOP_REENTRY_COOLDOWN_MS;
    }
    if let Some(cooldown) = profile_cooldown {
        return cooldown;
    }
    let Some(outcomes) = settings
        .learning
        .get(&strategy_trade_learning_key(symbol, strategy))
    else {
        return DEFAULT_POST_STOP_REENTRY_COOLDOWN_MS;
    };
    if outcomes.recent_trade_pnls.len() < MIN_POST_STOP_OUTCOMES_FOR_ADAPTATION {
        return DEFAULT_POST_STOP_REENTRY_COOLDOWN_MS;
    }
    let net_pnl_edge = recent_net_pnl_edge(outcomes).unwrap_or(0.0);
    let loss_pressure = ((0.1 - net_pnl_edge) / 1.1).clamp(0.0, 1.0);
    MIN_POST_STOP_REENTRY_COOLDOWN_MS
        + ((MAX_POST_STOP_REENTRY_COOLDOWN_MS - MIN_POST_STOP_REENTRY_COOLDOWN_MS) as f64
            * loss_pressure) as u64
}

#[cfg(test)]
mod cooldown_profile_tests {
    use super::*;

    #[test]
    fn learned_profile_cooldown_is_used_before_aggregate_history() {
        let signal = Signal {
            symbol: "BTC-USDT".into(), direction: "bullish".into(), strength: 1.0,
            timestamp: 1, stale: false, score: 1.0, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: true,
            strategy: Some("live-momentum".into()),
            strategy_decision: Some(serde_json::json!({"strategy_parameters": {
                "post_stop_cooldown_minutes": 4.5
            }})), market_regime: None, news_attribution: None, entry_gate: None,
        };
        assert_eq!(strategy_post_stop_cooldown_ms(&signal, "live-momentum", "BTC-USDT", &Settings::default(), 1), 270_000);
    }
}
