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
        &closed_trade_learning_returns(settings),
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
}
