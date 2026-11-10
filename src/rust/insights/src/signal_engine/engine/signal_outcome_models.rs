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
    if !evaluation.counterfactual_quote_path.is_empty() {
        return quote_path_stop_target_return(
            evaluation,
            direction,
            stop_loss_percent,
            take_profit_percent,
        );
    }
    let (entry_bid, entry_ask) = evaluation.entry_bid.zip(evaluation.entry_ask)?;
    if !entry_bid.is_finite() || !entry_ask.is_finite() || entry_bid <= 0.0 || entry_ask < entry_bid
    {
        return None;
    }
    let half_spread = (entry_ask - entry_bid) * 0.5;
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
        (true, true) => {
            let (target_timestamp, stop_timestamp) = match direction {
                "bullish" => (
                    evaluation.path_high_timestamp,
                    evaluation.path_low_timestamp,
                ),
                "bearish" => (
                    evaluation.path_low_timestamp,
                    evaluation.path_high_timestamp,
                ),
                _ => return None,
            };
            if target_timestamp > 0 && target_timestamp < stop_timestamp {
                Some(take_profit_percent)
            } else {
                Some(-stop_loss_percent)
            }
        }
        (false, true) => Some(-stop_loss_percent),
        (true, false) => Some(take_profit_percent),
        (false, false) => terminal_net.filter(|value| value.is_finite()),
    }
}

const COUNTERFACTUAL_DISPLAYED_DEPTH_FRACTION: f64 = 0.10;

fn quote_path_stop_target_return(
    evaluation: &SignalEvaluation,
    direction: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> Option<f64> {
    if !matches!(direction, "bullish" | "bearish") {
        return None;
    }
    let entry = evaluation.counterfactual_quote_path.first()?;
    let entry_price = quote_path_entry_price(entry, direction)?;
    let mut terminal = None;
    for sample in &evaluation.counterfactual_quote_path {
        if sample.timestamp <= entry.timestamp {
            continue;
        }
        let exit_price = quote_path_exit_price(sample, direction)?;
        let net = directional_net_return(
            direction,
            entry_price,
            exit_price,
            evaluation.taker_fee_rate,
        )?;
        if net <= -stop_loss_percent || net >= take_profit_percent {
            return Some(net);
        }
        terminal = Some(net);
    }
    terminal
}

pub(super) fn strategy_invalidation_counterfactual_return(
    evaluation: &SignalEvaluation,
    direction: &str,
    invalidation_timestamp: u64,
) -> Option<f64> {
    let entry = evaluation.counterfactual_quote_path.first()?;
    let entry_price = quote_path_entry_price(entry, direction)?;
    let sample = evaluation
        .counterfactual_quote_path
        .iter()
        .find(|sample| sample.timestamp >= invalidation_timestamp)?;
    let exit_price = quote_path_exit_price(sample, direction)?;
    directional_net_return(
        direction,
        entry_price,
        exit_price,
        evaluation.taker_fee_rate,
    )
}

fn quote_path_entry_price(
    sample: &CounterfactualQuoteSample,
    direction: &str,
 ) -> Option<f64> {
    let entry_price = match direction {
        "bullish" => sample.ask_execution_price,
        "bearish" => sample.bid_execution_price,
        _ => return None,
    };
    entry_price.filter(|price| price.is_finite() && *price > 0.0)
}

fn quote_path_exit_price(
    sample: &CounterfactualQuoteSample,
    direction: &str,
) -> Option<f64> {
    let exit_price = match direction {
        "bullish" => sample.bid_execution_price,
        "bearish" => sample.ask_execution_price,
        _ => return None,
    };
    exit_price.filter(|price| price.is_finite() && *price > 0.0)
}

pub(super) fn depth_adjusted_quote_execution_price(levels: &[(f64, f64)]) -> Option<f64> {
    let order_quantity = levels
        .iter()
        .filter_map(|(price, size)| {
            (price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0).then_some(*size)
        })
        .sum::<f64>()
        * COUNTERFACTUAL_DISPLAYED_DEPTH_FRACTION;
    if !order_quantity.is_finite() || order_quantity <= 0.0 {
        return None;
    }
    quote_path_vwap(levels, order_quantity)
}

fn quote_path_vwap(levels: &[(f64, f64)], order_quantity: f64) -> Option<f64> {
    let mut remaining = order_quantity;
    let mut notional = 0.0;
    for (price, size) in levels.iter().copied() {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 {
            continue;
        }
        let filled = remaining.min(size);
        notional += price * filled;
        remaining -= filled;
        if remaining <= 1e-12 {
            return Some(notional / order_quantity);
        }
    }
    None
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

pub fn configured_signal_taker_fee_rate() -> f64 {
    let configured = std::env::var("SIGNALS_TAKER_FEE_RATE")
        .or_else(|_| std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE"))
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(f64::abs)
        .unwrap_or(0.001);
    configured.clamp(0.0, 0.02)
}

#[cfg(test)]
mod outcome_model_tests {
    use super::*;

    #[test]
    fn microprice_and_directional_returns_handle_size_direction_and_invalid_inputs() {
        assert_eq!(sample_microprice(99.0, 3.0, 101.0, 1.0), 100.5);
        assert!(
            (directional_net_return("bullish", 100.0, 102.0, 0.001).unwrap() - 1.8).abs() < 1e-10
        );
        assert!(
            (directional_net_return("bearish", 100.0, 98.0, 0.001).unwrap() - 1.8).abs() < 1e-10
        );
        assert_eq!(directional_net_return("neutral", 100.0, 98.0, 0.0), None);
        assert_eq!(directional_net_return("bullish", 0.0, 98.0, 0.0), None);
    }

    #[test]
    fn depth_adjusted_execution_price_accounts_for_visible_book_depth() {
        let levels = [
            (100.0, 0.2),
            (99.5, 9.8),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
        ];
        let execution_price = depth_adjusted_quote_execution_price(&levels).unwrap();
        assert!((execution_price - 99.6).abs() < 1e-10);
    }

    #[test]
    fn stop_target_counterfactual_respects_observed_barrier_path_order() {
        let evaluation = SignalEvaluation {
            symbol: "BTC-USDT".into(),
            signal_timestamp: 1,
            evaluated_timestamp: 2,
            horizon_seconds: 60,
            signal_direction: "bullish".into(),
            signal_score: 0.5,
            ranging_probability: None,
            reference_entry_price: 100.0,
            reference_exit_price: 100.0,
            entry_bid: Some(99.9),
            entry_ask: Some(100.1),
            entry_quote_timestamp: Some(1),
            exit_bid: None,
            exit_ask: None,
            exit_quote_timestamp: None,
            taker_fee_rate: 0.0,
            long_gross_percent: 0.0,
            short_gross_percent: 0.0,
            long_net_percent: Some(0.1),
            short_net_percent: Some(0.1),
            path_high: 102.0,
            path_high_timestamp: 2,
            path_low: 98.0,
            path_low_timestamp: 2,
            counterfactual_quote_path: Vec::new(),
            long_time_to_positive_net_seconds: None,
            short_time_to_positive_net_seconds: None,
        };
        let mut with_quote_path = evaluation.clone();
        with_quote_path
            .counterfactual_quote_path
            .push(CounterfactualQuoteSample::default());
        assert_eq!(
            bincode::serialize(&evaluation).unwrap(),
            bincode::serialize(&with_quote_path).unwrap()
        );
        assert_eq!(
            stop_target_counterfactual_return(&evaluation, "bullish", 0.5, 0.8),
            Some(-0.5)
        );
        let target_first = SignalEvaluation {
            path_high_timestamp: 2,
            path_low_timestamp: 3,
            ..evaluation.clone()
        };
        assert_eq!(
            stop_target_counterfactual_return(&target_first, "bullish", 0.5, 0.8),
            Some(0.8)
        );
        let stop_first = SignalEvaluation {
            path_high_timestamp: 3,
            path_low_timestamp: 2,
            ..evaluation.clone()
        };
        assert_eq!(
            stop_target_counterfactual_return(&stop_first, "bullish", 0.5, 0.8),
            Some(-0.5)
        );
        assert_eq!(
            stop_target_counterfactual_return(&evaluation, "bullish", 0.0, 0.8),
            None
        );
    }

    #[test]
    fn quote_path_counterfactual_walks_depth_and_uses_the_first_executable_barrier() {
        let evaluation = SignalEvaluation {
            symbol: "BTC-USDT".into(),
            signal_timestamp: 1_000,
            evaluated_timestamp: 3_000,
            horizon_seconds: 2,
            signal_direction: "bullish".into(),
            signal_score: 0.5,
            ranging_probability: None,
            reference_entry_price: 100.0,
            reference_exit_price: 100.0,
            entry_bid: Some(99.9),
            entry_ask: Some(100.1),
            entry_quote_timestamp: Some(1_000),
            exit_bid: Some(100.5),
            exit_ask: Some(100.6),
            exit_quote_timestamp: Some(3_000),
            taker_fee_rate: 0.0,
            long_gross_percent: 0.5,
            short_gross_percent: -0.5,
            long_net_percent: Some(0.5),
            short_net_percent: Some(-0.5),
            path_high: 101.0,
            path_high_timestamp: 3_000,
            path_low: 99.0,
            path_low_timestamp: 2_000,
            counterfactual_quote_path: vec![
                CounterfactualQuoteSample {
                    timestamp: 1_000,
                    bid_execution_price: Some(99.9),
                    ask_execution_price: Some(100.1),
                },
                CounterfactualQuoteSample {
                    timestamp: 2_000,
                    bid_execution_price: Some(100.58),
                    ask_execution_price: Some(101.0),
                },
                CounterfactualQuoteSample {
                    timestamp: 3_000,
                    bid_execution_price: Some(100.6),
                    ask_execution_price: Some(100.7),
                },
            ],
            long_time_to_positive_net_seconds: None,
            short_time_to_positive_net_seconds: None,
        };
        let result = stop_target_counterfactual_return(&evaluation, "bullish", 0.5, 0.6)
            .expect("quote path should produce an executable return");
        assert!(result < 0.6);
        assert!(result > 0.4);
    }
}
