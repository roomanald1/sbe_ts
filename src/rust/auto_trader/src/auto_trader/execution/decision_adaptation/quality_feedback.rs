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
    feedback_outbox: &mut Vec<strategy::ExecutionOutcomeEvent>,
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
    feedback_outbox.push(strategy::ExecutionOutcomeEvent {
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

#[derive(Clone, Copy)]
struct ExplorationRiskAdjustment {
    scale: f64,
    sample_count: usize,
    weighted_net_return_percent: Option<f64>,
    weighted_net_return_lcb_percent: Option<f64>,
    regime_scoped: bool,
}

fn weighted_exploration_risk_scale(samples: &[(u64, f64)], now_ms: u64) -> (f64, f64) {
    const LOSS_HALF_LIFE_MS: f64 = 3.0 * 60.0 * 60.0 * 1_000.0;
    const WIN_HALF_LIFE_MS: f64 = 60.0 * 60.0 * 1_000.0;
    let (weight, loss_weight, weighted_return) = samples.iter().fold((0.0, 0.0, 0.0), |(weight, loss_weight, total), (timestamp, net)| {
        let age = now_ms.saturating_sub(*timestamp) as f64;
        // Loss evidence decays more slowly than wins, so recovery requires
        // sustained positive realized outcomes.
        let half_life = if *net < 0.0 { LOSS_HALF_LIFE_MS } else { WIN_HALF_LIFE_MS };
        let sample_weight = 0.5_f64.powf(age / half_life);
        (weight + sample_weight, loss_weight + if *net < 0.0 { sample_weight } else { 0.0 }, total + sample_weight * net)
    });
    if weight <= 0.0 {
        return (1.0, 0.0);
    }
    let mean = weighted_return / weight;
    let confidence = ((loss_weight - 1.0) / 7.0).clamp(0.0, 1.0);
    let shortfall = (-mean / 0.50).clamp(0.0, 1.0);
    ((1.0 - 0.65 * confidence * shortfall).clamp(0.35, 1.0), mean)
}

fn weighted_exploration_net_return_lcb_percent(
    samples: &[(u64, f64)],
    now_ms: u64,
) -> Option<f64> {
    const LOSS_HALF_LIFE_MS: f64 = 3.0 * 60.0 * 60.0 * 1_000.0;
    const WIN_HALF_LIFE_MS: f64 = 60.0 * 60.0 * 1_000.0;
    let (weight, squared_weight, weighted_return, weighted_square) = samples.iter().fold(
        (0.0, 0.0, 0.0, 0.0),
        |(weight, squared_weight, weighted_return, weighted_square), (timestamp, net)| {
            if !net.is_finite() {
                return (weight, squared_weight, weighted_return, weighted_square);
            }
            let age = now_ms.saturating_sub(*timestamp) as f64;
            let half_life = if *net < 0.0 {
                LOSS_HALF_LIFE_MS
            } else {
                WIN_HALF_LIFE_MS
            };
            let sample_weight = 0.5_f64.powf(age / half_life);
            (
                weight + sample_weight,
                squared_weight + sample_weight * sample_weight,
                weighted_return + sample_weight * net,
                weighted_square + sample_weight * net * net,
            )
        },
    );
    if weight <= 0.0 || squared_weight <= 0.0 {
        return None;
    }
    let mean = weighted_return / weight;
    let effective_samples = weight * weight / squared_weight;
    let variance = (weighted_square / weight - mean * mean).max(0.0);
    let variance = if variance <= 1e-12 { 0.0 } else { variance };
    let standard_error = (variance / effective_samples).sqrt();
    Some(mean - 1.645 * standard_error)
}

impl ExplorationRiskAdjustment {
    fn neutral() -> Self {
        Self {
            scale: 1.0,
            sample_count: 0,
            weighted_net_return_percent: None,
            weighted_net_return_lcb_percent: None,
            regime_scoped: false,
        }
    }
}

fn contextual_exploration_risk_adjustment(
    signal: &Signal,
    settings: &Settings,
    now_ms: u64,
) -> ExplorationRiskAdjustment {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name)
        .filter(|strategy| strategy.is_live_insight())
    else {
        return ExplorationRiskAdjustment::neutral();
    };
    let direction = match signal.direction.as_str() {
        "bullish" => "long",
        "bearish" => "short",
        _ => return ExplorationRiskAdjustment::neutral(),
    };
    const WINDOW_MS: u64 = 6 * 60 * 60 * 1_000;
    let as_of_ms = now_ms.min(signal.timestamp);
    let mut samples = Vec::<(u64, f64, String)>::new();
    for (key, returns) in &settings.recent_profile_context_returns {
        let Some((_, context)) = key.split_once('|') else { continue; };
        let mut parts = context.split(':');
        let (Some(side), Some(regime), Some(symbol), Some(kind)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else { continue; };
        if side != direction || symbol != signal.symbol || kind != strategy.as_str() {
            continue;
        }
        for &(timestamp, net_return) in returns {
            if timestamp < as_of_ms && as_of_ms.saturating_sub(timestamp) <= WINDOW_MS
                && net_return.is_finite()
            {
                samples.push((timestamp, net_return, regime.to_owned()));
            }
        }
    }
    let requested_regime = signal.market_regime.as_deref()
        .filter(|regime| matches!(*regime, "range" | "trend" | "volatile" | "mixed"));
    let regime_samples = requested_regime.map(|regime| {
        samples.iter().filter(|(_, _, sample_regime)| sample_regime == regime)
            .cloned().collect::<Vec<_>>()
    });
    let regime_scoped = regime_samples.as_ref().is_some_and(|subset| subset.len() >= 6);
    if regime_scoped {
        samples = regime_samples.unwrap_or_default();
    }
    samples.sort_unstable_by_key(|sample| std::cmp::Reverse(sample.0));
    samples.dedup_by_key(|sample| sample.0);
    samples.truncate(64);
    if samples.len() < 3 {
        return ExplorationRiskAdjustment::neutral();
    }
    let risk_samples = samples.iter().map(|(timestamp, net, _)| (*timestamp, *net)).collect::<Vec<_>>();
    let (scale, mean) = weighted_exploration_risk_scale(&risk_samples, now_ms);
    let net_return_lcb = weighted_exploration_net_return_lcb_percent(&risk_samples, as_of_ms);
    ExplorationRiskAdjustment {
        scale,
        sample_count: samples.len(),
        weighted_net_return_percent: Some(mean),
        weighted_net_return_lcb_percent: net_return_lcb,
        regime_scoped,
    }
}

#[cfg(test)]
mod exploration_risk_tests {
    use super::*;

    #[test]
    fn large_supported_net_losses_cut_risk_and_keep_a_probe_floor() {
        let now = 20_000_000;
        let losses = vec![(now, -0.5); 12];
        let (scale, mean) = weighted_exploration_risk_scale(&losses, now);
        assert!(mean <= -0.25);
        assert_eq!(scale, 0.35);
    }

    #[test]
    fn risk_recovery_is_slower_than_a_fresh_win() {
        let now = 20_000_000;
        let prior_losses = vec![(now - 4 * 60 * 60 * 1_000, -0.5); 8];
        let (loss_scale, _) = weighted_exploration_risk_scale(&prior_losses, now);
        let mut with_one_win = prior_losses.clone();
        with_one_win.push((now, 0.2));
        let (one_win_scale, _) = weighted_exploration_risk_scale(&with_one_win, now);
        let mut with_sustained_recovery = prior_losses;
        with_sustained_recovery.extend(vec![(now, 0.2); 8]);
        let (recovered_scale, _) = weighted_exploration_risk_scale(&with_sustained_recovery, now);
        assert!(loss_scale < 1.0);
        assert!(one_win_scale > loss_scale);
        assert!(one_win_scale < recovered_scale);
        assert!(recovered_scale <= 1.0);
    }

    #[test]
    fn recent_net_edge_lower_bound_requires_a_positive_conservative_estimate() {
        let now = 20_000_000;
        let supported_wins = vec![(now, 0.2); 6];
        assert!(
            (weighted_exploration_net_return_lcb_percent(&supported_wins, now).unwrap() - 0.2)
                .abs()
                < 1e-12
        );

        let mixed = vec![(now, 0.2); 5]
            .into_iter()
            .chain([(now, -1.0)])
            .collect::<Vec<_>>();
        assert!(
            weighted_exploration_net_return_lcb_percent(&mixed, now)
                .is_some_and(|lower_bound| lower_bound < 0.0)
        );
    }
}

fn has_recent_profile_loss_cluster(signal: &Signal, settings: &Settings, now_ms: u64) -> bool {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else { return false; };
    let parameters = signal_strategy_parameters(signal);
    let profile = strategy::profile_id(&signal.symbol, strategy.as_str(), &parameters);
    let key = format!("{profile}|{}", market_context(signal));
    let Some(returns) = settings.recent_profile_context_returns.get(&key) else { return false; };
    returns.len() >= LOSS_CLUSTER_SIZE && returns.iter().take(LOSS_CLUSTER_SIZE).all(|(timestamp, value)| {
        *timestamp <= now_ms && now_ms.saturating_sub(*timestamp) <= LOSS_CLUSTER_WINDOW_MS
            && value.is_finite() && *value < 0.0
    })
}
