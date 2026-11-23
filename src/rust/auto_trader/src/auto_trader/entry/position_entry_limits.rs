
const LOSS_ENTRY_FLOOR_LIFT_MS: u64 = 30 * 60 * 1_000;
const LOSS_ENTRY_FLOOR_MULTIPLIER: f64 = 1.5;

fn entry_floor_lift_key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}|{strategy}")
}

fn mark_symbol_entry_floor_lift(
    settings: &mut Settings,
    symbol: &str,
    strategy: &str,
    realized_pnl: f64,
) {
    let now_ms = current_time_ms();
    settings.entry_floor_lift_until_ms.retain(|_, until| *until > now_ms);
    let key = entry_floor_lift_key(symbol, strategy);
    if realized_pnl.is_finite()
        && realized_pnl < 0.0
        && Strategy::from_name(strategy).is_some_and(|kind| kind.is_live_insight())
    {
        settings.entry_floor_lift_until_ms.insert(
            key,
            now_ms.saturating_add(LOSS_ENTRY_FLOOR_LIFT_MS),
        );
    } else {
        settings.entry_floor_lift_until_ms.remove(&key);
    }
}

fn clears_temporary_entry_floor(signal: &Signal) -> bool {
    let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) else {
        return true;
    };
    if !strategy.is_live_insight() {
        return true;
    }
    let parameters = signal_strategy_parameters(signal);
    let side = match signal.direction.as_str() {
        "bullish" => 1.0,
        "bearish" => -1.0,
        _ => return false,
    };
    let threshold = |key: StrategyParameterKey| {
        parameters
            .get(key.as_str())
            .copied()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value * LOSS_ENTRY_FLOOR_MULTIPLIER)
    };
    let directional = match strategy {
        Strategy::LiveMomentum => {
            let Some(floor) = threshold(StrategyParameterKey::MinimumMomentum) else {
                return false;
            };
            return signal
                .components
                .momentum_10s_percent
                .into_iter()
                .chain(signal.components.trend_5m_volatility_units)
                .any(|value| value.is_finite() && side * value > floor);
        }
        Strategy::LiveTradeFlow => (
            threshold(StrategyParameterKey::MinimumFlowImbalance),
            signal.components.trade_flow_imbalance.map(|value| side * value),
        ),
        Strategy::LiveOrderBook => (
            threshold(StrategyParameterKey::MinimumBookImbalance),
            signal.components.book_imbalance.map(|value| side * value),
        ),
        Strategy::LiveVwapDislocation => (
            threshold(StrategyParameterKey::MinimumVwapDislocationBps),
            signal
                .components
                .mispricing_vs_vwap_bps
                .map(|value| side * value),
        ),
        Strategy::LiveAbsorptionPressure => (
            threshold(StrategyParameterKey::MinimumAbsorptionPressure),
            signal
                .components
                .pressure_without_price_move
                .map(|value| -side * value),
        ),
        Strategy::LiveRangeMeanReversion => (
            threshold(StrategyParameterKey::MinimumSignalStrength),
            Some(signal.strength),
        ),
        _ => return true,
    };
    directional
        .0
        .zip(directional.1)
        .is_some_and(|(floor, value)| value.is_finite() && value >= floor)
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
    let floor_lift_active = settings
        .entry_floor_lift_until_ms
        .get(&entry_floor_lift_key(signal_key, strategy_key))
        .is_some_and(|until_ms| now_ms < *until_ms);
    if floor_lift_active && !portfolio_rebalance && !clears_temporary_entry_floor(signal) {
        return Some(format!(
            "Temporary post-loss entry floor: {signal_key} / {strategy_key} must exceed 1.5× its learned threshold"
        ));
    }
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
            && order.expires_at.is_none_or(|expires_at| expires_at > now_ms)
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

fn pending_maker_entry_for_signal(
    settings: &Settings,
    signal: &Signal,
    signal_key: &str,
    strategy_key: &str,
    now_ms: u64,
) -> Option<(u64, f64)> {
    let side = if signal.direction == "bullish" { "buy" } else { "sell" };
    settings.orders.iter().filter_map(|order| {
        let eligible = order.symbol == signal_key
            && order.strategy.as_deref() == Some(strategy_key)
            && order.side == side
            && order.purpose == "signal"
            && order.order_type.starts_with("PostOnlyLimit")
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting")
            && order.expires_at.is_none_or(|expires_at| expires_at > now_ms)
            && order.filled_quantity <= 1e-12
            && order.quantity > 1e-12
            && !settings.orders.iter().any(|candidate| {
                candidate.ioc_fallback_for == Some(order.timestamp)
            });
        eligible.then_some((order.timestamp, order.strength))
    }).max_by(|left, right| left.1.total_cmp(&right.1))
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
