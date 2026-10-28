fn signal_position_block_reason(
    settings: &Settings,
    signal: &Signal,
    signal_key: &str,
    strategy_key: &str,
    now_ms: u64,
) -> Option<String> {
    let strategy_cooldown_ms = strategy_post_stop_cooldown_ms(strategy_key, &settings.orders);
    if let Some(stopped_at) = settings
        .orders
        .iter()
        .filter(|order| {
            order.symbol == signal_key
                && order.strategy.as_deref() == Some(strategy_key)
                && order.purpose == "stop-loss"
                && order.status == "filled"
                && order.realized_pnl < 0.0
        })
        .filter_map(|order| order.triggered_at)
        .max()
        .filter(|stopped_at| now_ms.saturating_sub(*stopped_at) < strategy_cooldown_ms)
    {
        let seconds_remaining =
            strategy_cooldown_ms.saturating_sub(now_ms.saturating_sub(stopped_at)) / 1000;
        return Some(format!("Post-stop cooldown: {signal_key} / {strategy_key} paused for {seconds_remaining}s after a losing protective stop"));
    }
    let category = strategy_category(Some(strategy_key));
    let open_position_count = total_open_position_count(settings);
    let category_position_count = category_open_position_count(settings, category);
    let already_open = settings.strategy_positions.get(signal_key)
        .and_then(|lots| lots.get(strategy_key))
        .is_some_and(|position| position.quantity.abs() > 1e-12);
    if !already_open && open_position_count >= settings.max_open_positions {
        return Some(format!("Total open-position cap reached ({}/{})", open_position_count, settings.max_open_positions));
    }
    if !already_open && category_position_count >= category_position_limit(settings, category) {
        let category_name = match category { StrategyCategory::Intraday => "intraday", StrategyCategory::Daily => "daily" };
        return Some(format!("{category_name} strategy position cap reached ({}/{})", category_position_count, category_position_limit(settings, category)));
    }
    if let Some(position) = settings
        .positions
        .get(signal_key)
        .filter(|position| position.quantity.abs() > 1e-12)
    {
        let signal_quantity = if signal.direction == "bullish" {
            1.0
        } else {
            -1.0
        };
        if position.quantity.signum() != signal_quantity {
            return Some(format!("Opposite signal for {signal_key}; waiting for its stop or target exit"));
        }
        let category_entries = position_entries_for_category(position, category);
        if category_entries >= settings.max_entries_per_auto_position {
            return Some(format!("Add-on limit reached for {signal_key} ({category_entries} {category:?} entries >= max {})", settings.max_entries_per_auto_position));
        }

    }
    None
}
