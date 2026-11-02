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
    // Closed spot trades cannot lose more than their entry notional. Reject
    // malformed returns before they enter the persisted rolling statistics.
    if !return_percent.is_finite()
        || return_percent < -100.0
        || !realized_usd.is_finite()
        || !fees_usd.is_finite()
    {
        return;
    }
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
    let excess = settings
        .closed_trade_returns_percent
        .len()
        .saturating_sub(settings.closed_trade_window);
    if excess > 0 {
        settings.closed_trade_returns_percent.drain(0..excess);
        settings.closed_trade_timestamps_ms.drain(0..excess);
        settings.closed_trade_realized_usd.drain(0..excess);
        settings.closed_trade_fees_usd.drain(0..excess);
    }
}

fn strategy_trade_learning_key(symbol: &str, strategy: &str) -> String {
    format!("trade:{symbol}:{strategy}")
}

fn strategy_shadow_learning_key(symbol: &str, strategy: &str, scenario: &str) -> String {
    format!("shadow:{symbol}:{strategy}:{scenario}")
}

fn strategy_parameter_probe_learning_key(symbol: &str, strategy: &str, parameter: &str, value: f64) -> String {
    format!("probe:{symbol}:{strategy}:{parameter}:{value:.8}")
}

fn record_strategy_trade_outcome(
    settings: &mut Settings,
    symbol: &str,
    strategy: Option<&str>,
    learning_context: Option<&str>,
    _scout: bool,
    realized_pnl_usd: f64,
) {
    if !realized_pnl_usd.is_finite() {
        return;
    }
    // Scout fills are deliberately small, but still provide strategy-specific
    // evidence for tuning the profile that produced them.
    if let Some(strategy) = strategy {
        let key = strategy_trade_learning_key(symbol, strategy);
        record_learning_outcome(settings.learning.entry(key).or_default(), realized_pnl_usd);
    }
    if let Some(context) = learning_context.filter(|context| !context.is_empty()) {
        let base_context = context.split(":P").next().unwrap_or(context);
        let key = format!("{base_context}:trade");
        record_learning_outcome(settings.learning.entry(key).or_default(), realized_pnl_usd);
    }
}

const MAX_STRATEGY_THRESHOLD_SAMPLES: usize = 64;

fn record_strategy_threshold_sample(
    settings: &mut Settings,
    symbol: &str,
    strategy: Option<&str>,
    _scout: bool,
    entry_decision: Option<&serde_json::Value>,
    net_return_percent: f64,
) {
    let Some(strategy) = strategy else {
        return;
    };
    let Some(entry_decision) = entry_decision else {
        return;
    };
    if !net_return_percent.is_finite() {
        return;
    }
    let Some((parameter_key, threshold, feature_value)) =
        threshold_observation(strategy, entry_decision)
    else {
        return;
    };
    let key = strategy_trade_learning_key(symbol, strategy);
    let samples = &mut settings.learning.entry(key).or_default().threshold_samples;
    samples.push(StrategyThresholdSample {
        timestamp_ms: current_time_ms(),
        parameter_key,
        threshold,
        feature_value,
        net_return_percent,
    });
    if samples.len() > MAX_STRATEGY_THRESHOLD_SAMPLES {
        let excess = samples.len() - MAX_STRATEGY_THRESHOLD_SAMPLES;
        samples.drain(..excess);
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
    stats.recent_trade_pnls.push((now_ms, realized_pnl));
    if stats.recent_trade_pnls.len() > STRATEGY_SIZING_RECENT_TRADES {
        let excess = stats.recent_trade_pnls.len() - STRATEGY_SIZING_RECENT_TRADES;
        stats.recent_trade_pnls.drain(0..excess);
    }
    if realized_pnl > 0.0 {
        stats.positive += 1;
        stats.daily_positive += 1;
    }
    stats.realized_pnl += realized_pnl;
    stats.updated_at_ms = now_ms;
}
