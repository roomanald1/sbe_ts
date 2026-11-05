struct ActivityLearningMetrics {
    exit_learning_samples: usize,
    closed_trade_average_return_percent: Option<f64>,
    closed_trade_wins: usize,
    closed_trade_losses: usize,
    closed_trade_flat: usize,
    closed_trade_realized_usd: Option<f64>,
    closed_trade_fees_usd: Option<f64>,
    learning_samples: usize,
    learning_win_rate: Option<f64>,
    preferred_order_type: Option<String>,
    parameter_profile_samples: [usize; 3],
}

fn calculate_activity_learning_metrics(
    settings: &Settings,
    today_start: u64,
) -> ActivityLearningMetrics {
    // Ignore legacy/corrupt return samples. In a spot position, net return
    // cannot be materially below -100%; an invalid sample must not dominate
    // the daily average (or the win/loss counts).
    let today_closed_returns = settings
        .closed_trade_returns_percent
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            settings
                .closed_trade_timestamps_ms
                .get(*index)
                .is_some_and(|timestamp| *timestamp >= today_start)
        })
        .map(|(_, value)| *value)
        .filter(|value| value.is_finite() && *value >= -100.0)
        .collect::<Vec<_>>();
    let exit_learning_samples = today_closed_returns.len();
    let closed_trade_average_return_percent = (exit_learning_samples > 0)
        .then(|| today_closed_returns.iter().sum::<f64>() / exit_learning_samples as f64);
    let closed_trade_wins = today_closed_returns
        .iter()
        .filter(|value| **value > 1e-12)
        .count();
    let closed_trade_losses = today_closed_returns
        .iter()
        .filter(|value| **value < -1e-12)
        .count();
    let closed_trade_flat = exit_learning_samples - closed_trade_wins - closed_trade_losses;
    let today_closed_indices = settings
        .closed_trade_timestamps_ms
        .iter()
        .enumerate()
        .filter(|(_, timestamp)| **timestamp >= today_start)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let closed_trade_realized_usd = (settings.closed_trade_timestamps_ms.len()
        == settings.closed_trade_returns_percent.len()
        && settings.closed_trade_realized_usd.len() == settings.closed_trade_returns_percent.len()
        && !today_closed_indices.is_empty())
    .then(|| {
        today_closed_indices
            .iter()
            .map(|index| settings.closed_trade_realized_usd[*index])
            .sum()
    });
    let closed_trade_fees_usd = (settings.closed_trade_timestamps_ms.len()
        == settings.closed_trade_returns_percent.len()
        && settings.closed_trade_fees_usd.len() == settings.closed_trade_returns_percent.len()
        && !today_closed_indices.is_empty())
    .then(|| {
        today_closed_indices
            .iter()
            .map(|index| settings.closed_trade_fees_usd[*index])
            .sum()
    });
    // The per-strategy and per-context learning maps both record the same
    // completed position. Use the closed-position ledger for a user-facing
    // trade count so a trade is counted once, and its return (including fees)
    // determines whether it was positive.
    let learning_samples = exit_learning_samples;
    let preferred_order_type = ALL_ORDER_TYPES.iter().filter_map(|kind| {
        let quality_suffix = format!(":exec:{kind:?}");
        let stats = settings.learned_execution_stats.iter()
            .filter(|(key, _)| key.ends_with(&quality_suffix))
            .fold((0usize, 0usize, 0.0, 0.0, 0.0), |mut total, (_, stats)| {
                total.0 += stats.samples;
                total.1 += stats.filled_samples;
                total.2 += stats.fill_rate_total;
                total.3 += stats.cost_notional;
                total.4 += stats.all_in_cost_notional_bps;
                total
            });
        (stats.0 >= MIN_EXECUTION_QUALITY_SAMPLES
            && stats.1 >= MIN_EXECUTION_QUALITY_FILLS
            && stats.3 > 0.0
            && stats.2 / stats.0 as f64 >= 0.60)
            .then_some((format!("{kind:?}"), stats.4 / stats.3))
    }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(kind, _)| kind);
    let parameter_profile_samples = std::array::from_fn(|profile| {
        settings.learned_execution_stats.iter()
            .filter(|(key, stats)| key.contains(&format!(":P{profile}:")) && stats.updated_at_ms >= today_start)
            .map(|(_, stats)| stats.samples)
            .sum()
    });
    let learning_win_rate = (learning_samples > 0)
        .then_some(closed_trade_wins as f64 / learning_samples as f64);
    ActivityLearningMetrics {
        exit_learning_samples,
        closed_trade_average_return_percent,
        closed_trade_wins,
        closed_trade_losses,
        closed_trade_flat,
        closed_trade_realized_usd,
        closed_trade_fees_usd,
        learning_samples,
        learning_win_rate,
        preferred_order_type,
        parameter_profile_samples,
    }
}
