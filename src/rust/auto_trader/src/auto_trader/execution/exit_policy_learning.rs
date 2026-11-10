const MIN_VALIDATED_EXIT_POLICY_SAMPLES: usize = 20;
const STOP_WIDENING_PER_VALIDATION_TIER_PERCENT: f64 = 0.15;
const MAX_STOP_WIDENING_TIERS: usize = 3;
const MIN_POST_EXIT_ADAPTATION_SAMPLES: usize = 20;
const MAX_POST_EXIT_ADJUSTMENT_PERCENT: f64 = 0.5;
const MAX_POST_EXIT_SAMPLE_PERCENT: f64 = 2.0;

fn post_exit_policy_adjustments(
    history: &[serde_json::Value],
    symbol: &str,
    strategy: &str,
    direction: &str,
) -> (f64, f64, usize) {
    let matching_samples = |require_symbol: bool, trailing_only: bool| {
        history
            .iter()
            .filter(|item| {
                item["strategy"].as_str() == Some(strategy)
                    && (!require_symbol || item["symbol"].as_str() == Some(symbol))
                    && match direction {
                        "bullish" => item["side"].as_str() == Some("sell"),
                        "bearish" => item["side"].as_str() == Some("buy"),
                        _ => false,
                    }
                    && if trailing_only {
                        item["reason"].as_str() == Some("profit-protection")
                    } else {
                        matches!(
                            item["reason"].as_str(),
                            Some("take-profit" | "profit-protection")
                        )
                    }
            })
            .filter_map(|item| {
                item["best_post_exit_return_percent"]
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(|value| value.clamp(0.0, MAX_POST_EXIT_SAMPLE_PERCENT))
            })
            .collect::<Vec<_>>()
    };
    let validated_mean = |trailing_only: bool| {
        let symbol_samples = matching_samples(true, trailing_only);
        let samples = if symbol_samples.len() >= MIN_POST_EXIT_ADAPTATION_SAMPLES {
            symbol_samples
        } else {
            matching_samples(false, trailing_only)
        };
        (samples.len() >= MIN_POST_EXIT_ADAPTATION_SAMPLES).then(|| {
            (
                samples.iter().sum::<f64>() / samples.len() as f64,
                samples.len(),
            )
        })
    };
    let target = validated_mean(false);
    let trailing = validated_mean(true);
    (
        target
            .map(|(mean, _)| (mean * 0.25).clamp(0.0, MAX_POST_EXIT_ADJUSTMENT_PERCENT))
            .unwrap_or_default(),
        trailing
            .map(|(mean, _)| (mean * 0.25).clamp(0.0, MAX_POST_EXIT_ADJUSTMENT_PERCENT))
            .unwrap_or_default(),
        target.map_or(0, |(_, samples)| samples),
    )
}

fn learned_exit_policy(signal: &Signal, walk_forward: &[WalkForward]) -> Option<(f64, f64, usize)> {
    walk_forward
        .iter()
        .filter(|item| item.symbol == signal.symbol && matches!(item.horizon_seconds, 900 | 3_600))
        .filter_map(|item| {
            let policy = if signal.direction == "bullish" {
                item.learned_long_exit_policy.as_ref()
            } else {
                item.learned_short_exit_policy.as_ref()
            }?;
            (policy.validation_samples >= MIN_VALIDATED_EXIT_POLICY_SAMPLES
                && policy.validation_average_net_return_percent > 0.0
                && policy.validation_positive_net_rate >= 0.40
                && policy.stop_loss_percent.is_finite()
                && policy.take_profit_percent.is_finite())
            .then_some((
                policy.validation_average_net_return_percent,
                policy.stop_loss_percent,
                policy.take_profit_percent,
                policy.validation_samples,
            ))
        })
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, stop, target, samples)| (stop, target, samples))
}

fn horizon_adjusted_take_profit_percent(
    signal: &Signal,
    walk_forward: &[WalkForward],
    base_target_percent: f64,
) -> Option<f64> {
    walk_forward
        .iter()
        .filter(|item| item.symbol == signal.symbol && matches!(item.horizon_seconds, 900 | 3_600))
        .filter(|item| {
            let metrics = if signal.direction == "bullish" {
                &item.all_signal_long
            } else {
                &item.all_signal_short
            };
            metrics.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                && metrics
                    .direction_match_rate
                    .is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                && metrics.positive_net_rate.is_some_and(|rate| rate >= 0.60)
                && metrics
                    .positive_net_rate_lower_95
                    .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
                && metrics
                    .average_net_return_percent
                    .is_some_and(|average| average > base_target_percent)
        })
        .filter_map(|item| {
            if signal.direction == "bullish" {
                item.all_signal_long.average_net_return_percent
            } else {
                item.all_signal_short.average_net_return_percent
            }
        })
        .max_by(f64::total_cmp)
        .map(|average_return| {
            (average_return * 0.75)
                .max(base_target_percent)
                .min(MAX_TAKE_PROFIT_PERCENT)
        })
}

fn risk_adjusted_take_profit_percent(stop_loss_percent: f64, risk_appetite: f64) -> f64 {
    let safe_target = default_take_profit_percent()
        .max(stop_loss_percent * MIN_EXIT_REWARD_RISK_RATIO)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT);
    let risk = risk_appetite.clamp(0.0, 100.0) / 100.0;
    minimum_reward_risk_take_profit_percent(
        stop_loss_percent,
        safe_target * (1.0 - risk) + SMALL_NET_TAKE_PROFIT_PERCENT * risk,
    )
}

fn minimum_reward_risk_take_profit_percent(stop_loss_percent: f64, target_percent: f64) -> f64 {
    let reward_risk_floor =
        (stop_loss_percent.max(0.0) * MIN_EXIT_REWARD_RISK_RATIO).min(MAX_TAKE_PROFIT_PERCENT);
    target_percent
        .max(reward_risk_floor)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT)
}

fn volatility_adjusted_stop_loss_percent(
    base_stop_percent: f64,
    volatility_1m_percent: Option<f64>,
) -> f64 {
    let base = base_stop_percent.clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    let Some(volatility) = volatility_1m_percent.filter(|value| value.is_finite() && *value > 0.0)
    else {
        return base;
    };
    // volatility_1m_percent is realized one-minute movement; allow 1.5x the
    // square-root-of-time five-minute move, with bounded exposure.
    let volatility_stop = volatility * 5.0_f64.sqrt() * 1.5;
    base.max(volatility_stop)
        .clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT)
}

fn volatility_size_scale(base_stop_percent: f64, effective_stop_percent: f64) -> f64 {
    (base_stop_percent / effective_stop_percent).clamp(0.0, 1.0)
}

fn win_rate_supports_extended_hold(returns: &[f64]) -> bool {
    returns.len() >= EXTENDED_HOLD_MIN_SAMPLES
        && returns.iter().filter(|value| **value > 0.0).count() as f64 / returns.len() as f64
            >= EXTENDED_HOLD_MIN_WIN_RATE
}

fn confidence_adjusted_take_profit_percent(
    returns: &[f64],
    candidate: f64,
    stop_loss_percent: f64,
) -> f64 {
    let target = if win_rate_supports_extended_hold(returns) {
        candidate
    } else {
        EARLY_NET_TAKE_PROFIT_PERCENT
    };
    minimum_reward_risk_take_profit_percent(stop_loss_percent, target)
}

fn age_adjusted_take_profit_percent(
    strategy_name: Option<&str>,
    entry_timestamp_ms: u64,
    now_ms: u64,
    target_percent: f64,
) -> f64 {
    let Some(strategy) = strategy_name.and_then(Strategy::from_name) else {
        return target_percent;
    };
    if entry_timestamp_ms == 0
        || !matches!(
            strategy,
            Strategy::LiveMomentum
                | Strategy::LiveTradeFlow
                | Strategy::LiveOrderBook
                | Strategy::LiveVwapDislocation
                | Strategy::LiveAbsorptionPressure
        )
    {
        return target_percent;
    }

    let age_ms = now_ms.saturating_sub(entry_timestamp_ms);
    if age_ms <= LIVE_SIGNAL_TP_DECAY_START_MS {
        return target_percent;
    }
    let decay_duration = (LIVE_SIGNAL_TP_DECAY_END_MS - LIVE_SIGNAL_TP_DECAY_START_MS).max(1);
    let progress = (age_ms.saturating_sub(LIVE_SIGNAL_TP_DECAY_START_MS) as f64
        / decay_duration as f64)
        .clamp(0.0, 1.0);
    let floor = MIN_TAKE_PROFIT_PERCENT;
    let starting_target = target_percent.max(floor);
    starting_target + (floor - starting_target) * progress
}

fn adapt_exit_policy(settings: &mut Settings) {
    settings.stop_loss_percent = settings
        .stop_loss_percent
        .clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    let candidate =
        risk_adjusted_take_profit_percent(settings.stop_loss_percent, settings.risk_appetite);
    settings.take_profit_percent = confidence_adjusted_take_profit_percent(
        &settings.closed_trade_returns_percent,
        candidate,
        settings.stop_loss_percent,
    );
}

#[cfg(test)]
mod exit_policy_learning_tests {
    use super::*;

    #[test]
    fn risk_and_volatility_adjustments_stay_within_policy_bounds() {
        assert_eq!(volatility_adjusted_stop_loss_percent(1.0, None), 1.0);
        assert!(volatility_adjusted_stop_loss_percent(1.0, Some(1.0)) > 1.0);
        assert_eq!(
            volatility_adjusted_stop_loss_percent(1.0, Some(f64::NAN)),
            1.0
        );
        assert_eq!(volatility_size_scale(2.0, 4.0), 0.5);
        assert_eq!(volatility_size_scale(2.0, 1.0), 1.0);
        let conservative = risk_adjusted_take_profit_percent(2.0, 0.0);
        let aggressive = risk_adjusted_take_profit_percent(2.0, 100.0);
        assert!(conservative >= aggressive);
        assert_eq!(
            minimum_reward_risk_take_profit_percent(1.0, 0.1),
            1.0 * MIN_EXIT_REWARD_RISK_RATIO
        );
        assert_eq!(
            minimum_reward_risk_take_profit_percent(200.0, 1.0),
            MAX_TAKE_PROFIT_PERCENT
        );
    }

    #[test]
    fn extended_hold_requires_enough_positive_outcomes_and_respects_reward_risk() {
        let mixed = vec![1.0, -1.0, 1.0];
        let strong = vec![1.0; EXTENDED_HOLD_MIN_SAMPLES];
        assert!(!win_rate_supports_extended_hold(&mixed));
        assert!(win_rate_supports_extended_hold(&strong));
        assert_eq!(
            confidence_adjusted_take_profit_percent(&mixed, 3.0, 1.0),
            minimum_reward_risk_take_profit_percent(1.0, EARLY_NET_TAKE_PROFIT_PERCENT)
        );
        assert_eq!(
            confidence_adjusted_take_profit_percent(&strong, 3.0, 1.0),
            minimum_reward_risk_take_profit_percent(1.0, 3.0)
        );
    }

    #[test]
    fn learned_exit_policy_requires_validated_positive_evidence_for_matching_side() {
        let signal: Signal = serde_json::from_value(serde_json::json!({
            "symbol":"ETH-USDT", "direction":"bullish", "strength":1.0,
            "timestamp":1, "stale":false
        }))
        .unwrap();
        let accepted: WalkForward = serde_json::from_value(serde_json::json!({
            "symbol":"ETH-USDT", "horizon_seconds":900,
            "learned_long_exit_policy": {
                "stop_loss_percent":1.5, "take_profit_percent":3.0,
                "validation_samples":20, "validation_average_net_return_percent":0.4,
                "validation_positive_net_rate":0.5
            }
        }))
        .unwrap();
        assert_eq!(
            learned_exit_policy(&signal, std::slice::from_ref(&accepted)),
            Some((1.5, 3.0, 20))
        );
        let insufficient: WalkForward = serde_json::from_value(serde_json::json!({
            "symbol":"ETH-USDT", "horizon_seconds":900,
            "learned_long_exit_policy": {
                "stop_loss_percent":1.5, "take_profit_percent":3.0,
                "validation_samples":19, "validation_average_net_return_percent":0.4,
                "validation_positive_net_rate":0.5
            }
        }))
        .unwrap();
        assert_eq!(learned_exit_policy(&signal, &[insufficient]), None);
    }

    #[test]
    fn post_exit_observations_adjust_only_matching_strategy_direction_with_minimum_samples() {
        let history = (0..20)
            .map(|index| {
                serde_json::json!({
                    "event_id": format!("tp-{index}"),
                    "symbol": "BTC-USDT",
                    "strategy": "live-momentum",
                    "side": "sell",
                    "reason": "take-profit",
                    "best_post_exit_return_percent": 0.8
                })
            })
            .chain((0..20).map(|index| {
                serde_json::json!({
                    "event_id": format!("protection-{index}"),
                    "symbol": "BTC-USDT",
                    "strategy": "live-momentum",
                    "side": "sell",
                    "reason": "profit-protection",
                    "best_post_exit_return_percent": 0.4
                })
            }))
            .collect::<Vec<_>>();
        let (target_increase, trailing_increase, sample_count) =
            post_exit_policy_adjustments(&history, "BTC-USDT", "live-momentum", "bullish");
        assert_eq!(sample_count, 40);
        assert!((target_increase - 0.15).abs() < 1e-10);
        assert!((trailing_increase - 0.1).abs() < 1e-10);

        assert_eq!(
            post_exit_policy_adjustments(&history[..19], "BTC-USDT", "live-momentum", "bullish"),
            (0.0, 0.0, 0)
        );
        assert_eq!(
            post_exit_policy_adjustments(&history, "BTC-USDT", "live-momentum", "bearish"),
            (0.0, 0.0, 0)
        );
    }
}
