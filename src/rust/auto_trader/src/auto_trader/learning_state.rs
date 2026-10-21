const LEARNING_HALF_LIFE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn utc_day_start_ms() -> u64 {
    current_time_ms() / 86_400_000 * 86_400_000
}

fn decay_learning_stats(stats: &mut LearningStats, now_ms: u64) {
    if stats.updated_at_ms == 0 {
        // Legacy persisted profiles have no timestamp; begin their decay clock
        // when first read after this version is deployed.
        stats.updated_at_ms = now_ms;
        return;
    }
    let elapsed_ms = now_ms.saturating_sub(stats.updated_at_ms);
    if elapsed_ms == 0 {
        return;
    }
    let weight = 0.5_f64.powf(elapsed_ms as f64 / LEARNING_HALF_LIFE_MS as f64);
    stats.samples = (stats.samples as f64 * weight).round() as usize;
    stats.positive = ((stats.positive as f64 * weight).round() as usize).min(stats.samples);
    stats.realized_pnl *= weight;
    stats.updated_at_ms = now_ms;
}

fn decay_learning_map(learning: &mut BTreeMap<String, LearningStats>, now_ms: u64) {
    for stats in learning.values_mut() {
        decay_learning_stats(stats, now_ms);
    }
}

fn record_closed_trade_outcome(
    settings: &mut Settings,
    return_percent: f64,
    realized_usd: f64,
    fees_usd: f64,
) {
    // These parallel arrays are a persisted legacy format. Discard an already
    // misaligned window rather than attaching new money values to old returns.
    let count = settings.closed_trade_returns_percent.len();
    if settings.closed_trade_timestamps_ms.len() != count
        || settings.closed_trade_realized_usd.len() != count
        || settings.closed_trade_fees_usd.len() != count
    {
        settings.closed_trade_returns_percent.clear();
        settings.closed_trade_timestamps_ms.clear();
        settings.closed_trade_realized_usd.clear();
        settings.closed_trade_fees_usd.clear();
    }
    settings.closed_trade_returns_percent.push(return_percent);
    settings.closed_trade_timestamps_ms.push(current_time_ms());
    settings.closed_trade_realized_usd.push(realized_usd);
    settings.closed_trade_fees_usd.push(fees_usd);
    let excess = settings.closed_trade_returns_percent.len().saturating_sub(settings.closed_trade_window);
    if excess > 0 {
        settings.closed_trade_returns_percent.drain(0..excess);
        settings.closed_trade_timestamps_ms.drain(0..excess);
        settings.closed_trade_realized_usd.drain(0..excess);
        settings.closed_trade_fees_usd.drain(0..excess);
    }
}

fn realized_usd(settings: &Settings, symbol: &str, quote_amount: f64) -> f64 {
    quote_amount * quote_usd_rate(settings, symbol)
}

fn learning_stats_for_day(stats: &LearningStats, day: &str) -> LearningStats {
    if stats.daily_utc_day == day {
        stats.clone()
    } else {
        LearningStats::default()
    }
}

fn record_learning_outcome(stats: &mut LearningStats, realized_pnl: f64) {
    let now_ms = current_time_ms();
    decay_learning_stats(stats, now_ms);
    let day = utc_day();
    if stats.daily_utc_day != day {
        stats.daily_utc_day = day;
        stats.daily_samples = 0;
        stats.daily_positive = 0;
        stats.daily_realized_pnl = 0.0;
    }
    stats.daily_samples += 1;
    stats.daily_realized_pnl += realized_pnl;
    stats.samples += 1;
    if realized_pnl > 0.0 {
        stats.positive += 1;
        stats.daily_positive += 1;
    }
    stats.realized_pnl += realized_pnl;
    stats.updated_at_ms = now_ms;
}
