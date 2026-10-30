const MIN_EXECUTION_QUALITY_SAMPLES: usize = 10;
const MIN_EXECUTION_QUALITY_FILLS: usize = 5;

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
    execution_learning: &mut BTreeMap<String, ExecutionLearningStats>,
    recorded_order_ids: &mut BTreeSet<String>,
    order: &TraderOrder,
) {
    let id = order.timestamp.to_string();
    if recorded_order_ids.contains(&id)
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
    let stats = execution_learning
        .entry(execution_quality_key(context, order_type))
        .or_default();
    let now = current_time_ms();
    decay_execution_quality(stats, now);
    stats.samples += 1;
    stats.fill_rate_total += order.fill_rate.clamp(0.0, 1.0);
    if order.filled_quantity > 0.0 {
        stats.filled_samples += 1;
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
                    stats.cost_notional += notional;
                    stats.all_in_cost_notional_bps += cost_bps * notional;
                }
            }
        }
    }
    stats.updated_at_ms = now;
    recorded_order_ids.insert(id);
    while recorded_order_ids.len() > 20_000 {
        if let Some(oldest) = recorded_order_ids.iter().next().cloned() {
            recorded_order_ids.remove(&oldest);
        }
    }
}

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


fn strategy_post_stop_cooldown_ms(strategy: &str, orders: &[TraderOrder]) -> u64 {
    let outcomes = orders
        .iter()
        .rev()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && matches!(
                    order.purpose.as_str(),
                    "stop-loss" | "take-profit" | "profit-protection" | "shadow-profit-protection" | "strategy-disabled" | "research-regime-exit"
                )
                && order.status == "filled"
                && order.filled_quantity > 0.0
                && order.realized_pnl.is_finite()
        })
        .take(POST_STOP_OUTCOME_WINDOW)
        .collect::<Vec<_>>();

    if outcomes.len() < MIN_POST_STOP_OUTCOMES_FOR_ADAPTATION {
        return DEFAULT_POST_STOP_REENTRY_COOLDOWN_MS;
    }

    let win_rate = outcomes
        .iter()
        .filter(|order| order.realized_pnl > 0.0)
        .count() as f64
        / outcomes.len() as f64;
    let loss_pressure = ((0.55 - win_rate) / 0.35).clamp(0.0, 1.0);
    MIN_POST_STOP_REENTRY_COOLDOWN_MS
        + ((MAX_POST_STOP_REENTRY_COOLDOWN_MS - MIN_POST_STOP_REENTRY_COOLDOWN_MS) as f64
            * loss_pressure) as u64
}
