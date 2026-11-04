
fn mark_symbol_reentry_cooldown(settings: &mut Settings, symbol: &str) {
    let now_ms = current_time_ms();
    settings.reentry_cooldown_until_ms.retain(|_, until| *until > now_ms);
    settings.reentry_cooldown_until_ms.insert(
        symbol.to_owned(),
        now_ms.saturating_add(DEFAULT_REENTRY_COOLDOWN_MS),
    );
}

fn strategy_exit_pending(order: &TraderOrder, symbol: &str, strategy: &str) -> bool {
    order.symbol == symbol
        && order.strategy.as_deref() == Some(strategy)
        && order.order_type == "Ioc"
        && !matches!(order.purpose.as_str(), "signal" | "scout")
        && matches!(
            order.status.as_str(),
            "submitting" | "accepted" | "resting" | "partially_filled"
        )
}

fn signal_position_block_reason(
    settings: &Settings,
    signal: &Signal,
    signal_key: &str,
    strategy_key: &str,
    now_ms: u64,
) -> Option<String> {
    let portfolio_rebalance = signal.strategy.as_deref() == Some("risk-adjusted-trend-portfolio")
        && signal.strategy_decision.as_ref()
            .and_then(|decision| decision.get("portfolio_rebalance_delta_usd"))
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|delta| delta.is_finite() && delta > 0.0);
    if settings.orders.iter().any(|order| strategy_exit_pending(order, signal_key, strategy_key)) {
        return Some(format!("Exit already pending for {signal_key} / {strategy_key}"));
    }
    let strategy_cooldown_ms = strategy_post_stop_cooldown_ms(strategy_key, signal_key, settings);
    if !portfolio_rebalance { if let Some(stopped_at) = settings
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
    }}
    let category = strategy_category(Some(strategy_key));
    let open_position_count = total_open_position_count(settings)
        + pending_entry_strategy_keys(settings).len();
    let category_position_count = category_open_position_count(settings, category)
        + pending_entry_position_count_for_category(settings, category);
    let strategy_position = settings.strategy_positions.get(signal_key)
        .and_then(|lots| lots.get(strategy_key))
        .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
        .or_else(|| settings.positions.get(signal_key)
            .filter(|position| settings.strategy_positions.is_empty()
                && position.quantity.abs() > 1e-12
                && !position.scout_entry
                && position.strategy.as_deref() == Some(strategy_key)));
    let already_open = strategy_position.is_some();
    if !already_open && !portfolio_rebalance {
        if let Some(until_ms) = settings.reentry_cooldown_until_ms.get(signal_key)
            .filter(|until_ms| now_ms < **until_ms)
        {
            let seconds_remaining = until_ms.saturating_sub(now_ms).div_ceil(1000);
            return Some(format!(
                "Symbol re-entry cooldown: {signal_key} paused for {seconds_remaining}s after a completed position"
            ));
        }
    }
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
            && order.purpose == "signal"
            && matches!(
            order.status.as_str(),
            "submitting" | "accepted" | "resting" | "partially_filled"
        )
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
        .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
    {
        let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
        if position.quantity.signum() != signal_quantity && !portfolio_rebalance {
            return Some(format!("Opposite signal for {signal_key}; waiting for the other strategy lots to close"));
        }
    }
    if let Some(position) = strategy_position {
        let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
        if position.quantity.signum() != signal_quantity && !portfolio_rebalance {
            return Some(format!("Opposite signal for {signal_key} / {strategy_key}"));
        }
        let strategy_entries = position.entries_by_category
            .get(strategy_category_key(category))
            .copied()
            .unwrap_or(position.entries);
        if !portfolio_rebalance && strategy_entries >= settings.max_entries_per_auto_position {
            return Some(format!("Add-on limit reached for {signal_key} / {strategy_key} ({strategy_entries} entries >= max {})", settings.max_entries_per_auto_position));
        }
    }
    None
}

#[cfg(test)]
mod pending_exit_tests {
    use super::*;

    #[test]
    fn blocks_reentry_while_strategy_exit_is_pending() {
        let mut order = new_position_exit_order(
            1,
            "BCH-USDT",
            "sell",
            "research-regime-exit",
            &TraderPosition {
                strategy: Some("risk-adjusted-trend-portfolio".into()),
                ..TraderPosition::default()
            },
            1.0,
            100.0,
        );
        order.status = "accepted".into();
        assert!(strategy_exit_pending(
            &order,
            "BCH-USDT",
            "risk-adjusted-trend-portfolio"
        ));
        assert!(!strategy_exit_pending(
            &order,
            "BCH-USDT",
            "another-strategy"
        ));
        order.purpose = "signal".into();
        assert!(!strategy_exit_pending(
            &order,
            "BCH-USDT",
            "risk-adjusted-trend-portfolio"
        ));
    }
}
