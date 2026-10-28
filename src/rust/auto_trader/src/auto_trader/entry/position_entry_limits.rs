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
    let open_position_count = total_open_position_count(settings)
        + pending_entry_strategy_keys(settings).len();
    let category_position_count = category_open_position_count(settings, category)
        + pending_entry_position_count_for_category(settings, category);
    let strategy_position = settings.strategy_positions.get(signal_key)
        .and_then(|lots| lots.get(strategy_key))
        .filter(|position| position.quantity.abs() > 1e-12)
        .or_else(|| settings.positions.get(signal_key)
            .filter(|position| settings.strategy_positions.is_empty()
                && position.quantity.abs() > 1e-12
                && position.strategy.as_deref() == Some(strategy_key)));
    let already_open = strategy_position.is_some();
    if !already_open && open_position_count >= settings.max_open_positions {
        return Some(format!("Total open-position cap reached ({}/{})", open_position_count, settings.max_open_positions));
    }
    if !already_open && category_position_count >= category_position_limit(settings, category) {
        let category_name = match category { StrategyCategory::Intraday => "intraday", StrategyCategory::Daily => "daily" };
        return Some(format!("{category_name} strategy position cap reached ({}/{})", category_position_count, category_position_limit(settings, category)));
    }
    if is_independent_daily_strategy(strategy_key) && !already_open {
        let strategy_positions = daily_strategy_open_position_count(settings, strategy_key)
            + pending_entry_position_count_for_strategy(settings, strategy_key);
        let strategy_limit = daily_strategy_position_limit(settings);
        if strategy_positions >= strategy_limit {
            return Some(format!("Daily strategy position share is full for {strategy_key} ({strategy_positions}/{strategy_limit})"));
        }
    }
    let signal_side = if signal.direction == "bullish" { "buy" } else { "sell" };
    let pending_entries = settings.orders.iter().filter(|order| {
        order.symbol == signal_key
            && matches!(order.purpose.as_str(), "signal" | "scout")
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
            && order.quantity > order.filled_quantity + 1e-12
    });
    for order in pending_entries {
        if order.strategy.as_deref() == Some(strategy_key) {
            return Some(format!("Entry already pending for {signal_key} / {strategy_key}"));
        }
        if order.side != signal_side {
            return Some(format!("Conflicting strategy entry is pending for {signal_key}"));
        }
    }
    if let Some(position) = settings.positions.get(signal_key)
        .filter(|position| position.quantity.abs() > 1e-12)
    {
        let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
        if position.quantity.signum() != signal_quantity {
            return Some(format!("Opposite signal for {signal_key}; waiting for the other strategy lots to close"));
        }
    }
    if let Some(position) = strategy_position {
        let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
        if position.quantity.signum() != signal_quantity {
            return Some(format!("Opposite signal for {signal_key} / {strategy_key}"));
        }
        let strategy_entries = position.entries_by_category
            .get(strategy_category_key(category))
            .copied()
            .unwrap_or(position.entries);
        if strategy_entries >= settings.max_entries_per_auto_position {
            return Some(format!("Add-on limit reached for {signal_key} / {strategy_key} ({strategy_entries} entries >= max {})", settings.max_entries_per_auto_position));
        }
    }
    None
}
