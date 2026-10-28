fn learned_exit_policy(signal: &Signal, walk_forward: &[WalkForward]) -> Option<(f64, f64)> {
    walk_forward
        .iter()
        .filter(|item| item.symbol == signal.symbol && matches!(item.horizon_seconds, 900 | 3_600))
        .filter_map(|item| {
            let policy = if signal.direction == "bullish" {
                item.learned_long_exit_policy.as_ref()
            } else {
                item.learned_short_exit_policy.as_ref()
            }?;
            (policy.validation_samples >= 8
                && policy.validation_average_net_return_percent > 0.0
                && policy.validation_positive_net_rate >= 0.40
                && policy.stop_loss_percent.is_finite()
                && policy.take_profit_percent.is_finite())
            .then_some((
                policy.validation_average_net_return_percent,
                policy.stop_loss_percent,
                policy.take_profit_percent,
            ))
        })
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, stop, target)| (stop, target))
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
    (safe_target * (1.0 - risk) + SMALL_NET_TAKE_PROFIT_PERCENT * risk)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT)
}

fn volatility_adjusted_stop_loss_percent(base_stop_percent: f64, volatility_1m_percent: Option<f64>) -> f64 {
    let base = base_stop_percent.clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    let Some(volatility) = volatility_1m_percent.filter(|value| value.is_finite() && *value > 0.0) else {
        return base;
    };
    // volatility_1m_percent is realized one-minute movement; allow 1.5x the
    // square-root-of-time five-minute move, with bounded exposure.
    let volatility_stop = volatility * 5.0_f64.sqrt() * 1.5;
    base.max(volatility_stop).clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT)
}

fn volatility_size_scale(base_stop_percent: f64, effective_stop_percent: f64) -> f64 {
    (base_stop_percent / effective_stop_percent).clamp(0.0, 1.0)
}

fn win_rate_supports_extended_hold(returns: &[f64]) -> bool {
    returns.len() >= EXTENDED_HOLD_MIN_SAMPLES
        && returns.iter().filter(|value| **value > 0.0).count() as f64 / returns.len() as f64
            >= EXTENDED_HOLD_MIN_WIN_RATE
}

fn confidence_adjusted_take_profit_percent(returns: &[f64], candidate: f64) -> f64 {
    if win_rate_supports_extended_hold(returns) {
        candidate.clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT)
    } else {
        EARLY_NET_TAKE_PROFIT_PERCENT
    }
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
            Strategy::LiveInsights
                | Strategy::LiveMomentum
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
    let candidate = risk_adjusted_take_profit_percent(settings.stop_loss_percent, settings.risk_appetite);
    settings.take_profit_percent = confidence_adjusted_take_profit_percent(
        &settings.closed_trade_returns_percent,
        candidate,
    );
}
