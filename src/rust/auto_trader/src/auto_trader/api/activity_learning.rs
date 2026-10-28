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
    scout_open_positions: usize,
    scout_learning_samples: usize,
    scout_learning_win_rate: Option<f64>,
    scout_learning_realized_pnl: f64,
}

fn calculate_activity_learning_metrics(
    settings: &Settings,
    today_start: u64,
) -> ActivityLearningMetrics {
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
    let today = utc_day();
    let learning_samples = settings
        .learning
        .values()
        .map(|stats| learning_stats_for_day(stats, &today).samples)
        .sum();
    let scout_open_positions = settings
        .positions
        .values()
        .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
        .count();
    let scout_learning = settings
        .learning
        .iter()
        .filter(|(key, _)| key.starts_with("scout:"))
        .map(|(_, stats)| learning_stats_for_day(stats, &today))
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        });
    let scout_learning_win_rate = (scout_learning.samples > 0)
        .then_some(scout_learning.positive as f64 / scout_learning.samples as f64);
    let learning_positive = settings
        .learning
        .values()
        .map(|stats| learning_stats_for_day(stats, &today).positive)
        .sum::<usize>();
    let preferred_order_type = ALL_ORDER_TYPES
        .iter()
        .filter_map(|kind| {
            let suffix = format!(":{kind:?}");
            let stats = settings
                .learning
                .iter()
                .filter(|(key, _)| key.ends_with(&suffix))
                .map(|(_, stats)| learning_stats_for_day(stats, &today))
                .fold(LearningStats::default(), |mut total, stats| {
                    total.samples += stats.samples;
                    total.positive += stats.positive;
                    total.realized_pnl += stats.realized_pnl;
                    total
                });
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && stats.positive as f64 / stats.samples as f64 >= 0.55)
                .then_some((
                    format!("{kind:?}"),
                    stats.realized_pnl / stats.samples as f64,
                ))
        })
        .filter(|(_, pnl)| *pnl > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind);
    let learning_win_rate =
        (learning_samples > 0).then_some(learning_positive as f64 / learning_samples as f64);
    let scout_learning_samples = scout_learning.samples;
    let scout_learning_realized_pnl = scout_learning.realized_pnl;
    let parameter_profile_samples = std::array::from_fn(|profile| {
        settings
            .learning
            .iter()
            .filter(|(key, _)| key.contains(&format!(":P{profile}:")))
            .map(|(_, stats)| learning_stats_for_day(stats, &today).samples)
            .sum()
    });
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
        scout_open_positions,
        scout_learning_samples,
        scout_learning_win_rate,
        scout_learning_realized_pnl,
    }
}


