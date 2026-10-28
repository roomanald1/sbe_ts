fn sample_microprice(bid: f64, bid_size: f64, ask: f64, ask_size: f64) -> f64 {
    (ask * bid_size + bid * ask_size) / (bid_size + ask_size)
}

fn directional_net_return(
    direction: &str,
    entry_price: f64,
    exit_price: f64,
    fee_rate: f64,
) -> Option<f64> {
    if !entry_price.is_finite()
        || !exit_price.is_finite()
        || entry_price <= 0.0
        || exit_price <= 0.0
    {
        return None;
    }
    let gross_return = match direction {
        "bullish" => (exit_price / entry_price - 1.0) * 100.0,
        "bearish" => ((entry_price - exit_price) / entry_price) * 100.0,
        _ => return None,
    };
    Some(gross_return - 2.0 * fee_rate * 100.0)
}

fn stop_target_counterfactual_return(
    evaluation: &SignalEvaluation,
    direction: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> Option<f64> {
    if !stop_loss_percent.is_finite()
        || stop_loss_percent <= 0.0
        || !take_profit_percent.is_finite()
        || take_profit_percent <= 0.0
    {
        return None;
    }
    let (entry_bid, entry_ask) = evaluation.entry_bid.zip(evaluation.entry_ask)?;
    if !entry_bid.is_finite() || !entry_ask.is_finite() || entry_bid <= 0.0 || entry_ask < entry_bid
    {
        return None;
    }
    let half_spread = (entry_ask - entry_bid) / 2.0;
    let (best_net, worst_net, terminal_net) = match direction {
        "bullish" => (
            directional_net_return(
                "bullish",
                entry_ask,
                evaluation.path_high - half_spread,
                evaluation.taker_fee_rate,
            )?,
            directional_net_return(
                "bullish",
                entry_ask,
                evaluation.path_low - half_spread,
                evaluation.taker_fee_rate,
            )?,
            evaluation.long_net_percent,
        ),
        "bearish" => (
            directional_net_return(
                "bearish",
                entry_bid,
                evaluation.path_low + half_spread,
                evaluation.taker_fee_rate,
            )?,
            directional_net_return(
                "bearish",
                entry_bid,
                evaluation.path_high + half_spread,
                evaluation.taker_fee_rate,
            )?,
            evaluation.short_net_percent,
        ),
        _ => return None,
    };
    let target_hit = best_net >= take_profit_percent;
    let stop_hit = worst_net <= -stop_loss_percent;
    match (target_hit, stop_hit) {
        // If both barriers were touched and ordering is unavailable, count the
        // stop first. This keeps the counterfactual from overstating edge.
        (_, true) => Some(-stop_loss_percent),
        (true, false) => Some(take_profit_percent),
        (false, false) => terminal_net.filter(|value| value.is_finite()),
    }
}

fn fit_learned_exit_policy(
    samples: &[SignalOutcome],
    evaluations: &HashMap<(String, u64, u64), SignalEvaluation>,
    direction: &str,
) -> Option<LearnedExitPolicy> {
    let rows = samples
        .iter()
        .filter(|sample| sample.direction == direction)
        .filter_map(|sample| {
            evaluations
                .get(&(
                    sample.symbol.clone(),
                    sample.signal_timestamp,
                    sample.horizon_seconds,
                ))
                .map(|evaluation| (sample, evaluation))
        })
        .collect::<Vec<_>>();
    if rows.len() < 28 {
        return None;
    }
    let split = rows.len() * 7 / 10;
    let (training, validation) = rows.split_at(split);
    if training.len() < 20 || validation.len() < 8 {
        return None;
    }
    let evaluate = |set: &[(&SignalOutcome, &SignalEvaluation)], stop: f64, target: f64| {
        let returns = set
            .iter()
            .filter_map(|(_, evaluation)| {
                stop_target_counterfactual_return(evaluation, direction, stop, target)
            })
            .collect::<Vec<_>>();
        if returns.len() < 8 {
            return None;
        }
        Some((
            returns.len(),
            returns.iter().sum::<f64>() / returns.len() as f64,
            returns.iter().filter(|value| **value > 0.0).count() as f64 / returns.len() as f64,
        ))
    };
    let baseline_train = evaluate(training, 0.50, 0.80)?;
    let mut best: Option<(f64, f64, f64)> = None;
    for stop in [0.30, 0.40, 0.50, 0.65, 0.80] {
        for target in [0.50, 0.65, 0.80, 1.00, 1.25, 1.50] {
            if target < stop * 1.25 || (stop == 0.50 && target == 0.80) {
                continue;
            }
            let Some((_, average, _)) = evaluate(training, stop, target) else {
                continue;
            };
            if average <= baseline_train.1 + 0.02 {
                continue;
            }
            if best.is_none_or(|(_, _, best_average)| average > best_average) {
                best = Some((stop, target, average));
            }
        }
    }
    let (stop, target, _) = best?;
    let candidate = evaluate(validation, stop, target)?;
    let baseline = evaluate(validation, 0.50, 0.80)?;
    if candidate.1 <= baseline.1 + 0.02 || candidate.1 <= 0.0 || candidate.2 < 0.40 {
        return None;
    }
    Some(LearnedExitPolicy {
        stop_loss_percent: stop,
        take_profit_percent: target,
        validation_samples: candidate.0,
        validation_average_net_return_percent: candidate.1,
        validation_positive_net_rate: candidate.2,
    })
}

fn configured_signal_taker_fee_rate() -> f64 {
    let configured = std::env::var("SIGNALS_TAKER_FEE_RATE")
        .or_else(|_| std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE"))
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(f64::abs)
        .unwrap_or(0.001);
    configured.clamp(0.0, 0.02)
}
