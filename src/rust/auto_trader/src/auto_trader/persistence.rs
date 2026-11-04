fn settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AUTO_TRADER_STATE_FILE") {
        return PathBuf::from(path);
    }
    std::env::var_os("POSITIONS_FILE")
        .map(PathBuf::from)
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.join("auto_trader_state.json"))
        })
        .unwrap_or_else(|| PathBuf::from("auto_trader_state.json"))
}
fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), (StatusCode, String)> {
    persist_settings_snapshot(path, settings)
}

fn settings_binary_path(json_path: &PathBuf) -> PathBuf {
    json_path.with_extension("msgpack")
}

fn persist_settings_snapshot(
    path: &PathBuf,
    settings: &Settings,
) -> Result<(), (StatusCode, String)> {
    let bytes = rmp_serde::to_vec_named(settings)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    persist_settings_bytes(path, bytes)
}

fn persist_settings_bytes(
    path: &PathBuf,
    bytes: Vec<u8>,
) -> Result<(), (StatusCode, String)> {
    let temporary = path.with_extension("msgpack.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not save Auto Trader settings: {error}"),
            )
        })
}

fn record_settings_persist_sample(
    timings: &std::sync::Mutex<BTreeMap<String, Vec<f64>>>,
    phase: &str,
    elapsed_ms: f64,
) {
    if let Ok(mut timings) = timings.lock() {
        let values = timings.entry(phase.to_owned()).or_default();
        values.push(elapsed_ms);
        if values.len() > 500 {
            values.remove(0);
        }
    }
}

fn request_settings_persist(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    state.settings_persist_notify.notify_one();
    let _ = state.state_updates.send(());
    Ok(())
}

fn start_settings_persistence_worker(state: &AutoTraderState) {
    let settings = Arc::clone(&state.settings);
    let path = state.settings_path.clone();
    let notify = Arc::clone(&state.settings_persist_notify);
    let timings = Arc::clone(&state.settings_persist_timings_ms);
    tokio::spawn(async move {
        loop {
            notify.notified().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            let lock_wait_started = Instant::now();
            let settings_guard = Arc::clone(&settings).lock_owned().await;
            record_settings_persist_sample(
                &timings,
                "snapshot_lock_wait",
                lock_wait_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let copy_started = Instant::now();
            let settings_snapshot =
                match tokio::task::spawn_blocking(move || settings_guard.clone()).await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        crate::service_log!(
                            "ERROR",
                            "Auto Trader state snapshot copier failed: {error}"
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        notify.notify_one();
                        continue;
                    }
                };
            record_settings_persist_sample(
                &timings,
                "snapshot_copy",
                copy_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let serialization_started = Instant::now();
            let bytes = match tokio::task::spawn_blocking(move || {
                rmp_serde::to_vec_named(&settings_snapshot)
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
            })
            .await
            {
                Ok(Ok(bytes)) => bytes,
                Ok(Err((_, error))) => {
                    crate::service_log!("ERROR", "Auto Trader state serialization failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                    continue;
                }
                Err(error) => {
                    crate::service_log!("ERROR", "Auto Trader state serializer failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                    continue;
                }
            };
            record_settings_persist_sample(
                &timings,
                "snapshot_serialize",
                serialization_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let path = path.clone();
            let write_started = Instant::now();
            match tokio::task::spawn_blocking(move || persist_settings_bytes(&path, bytes)).await {
                Ok(Ok(())) => record_settings_persist_sample(
                    &timings,
                    "snapshot_write",
                    write_started.elapsed().as_secs_f64() * 1_000.0,
                ),
                Ok(Err((_, error))) => {
                    crate::service_log!(
                        "ERROR",
                        "Auto Trader state snapshot write failed: {error}"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                }
                Err(error) => {
                    crate::service_log!(
                        "ERROR",
                        "Auto Trader state snapshot writer failed: {error}"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                }
            }
        }
    });
}

fn utc_day() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    days.to_string()
}
fn scheduled_stop_headroom_multiplier(order_type: &str, strategy: Option<&str>) -> f64 {
    let scheduled = order_type.starts_with("Vwap") || order_type.starts_with("Twap");
    if scheduled && Strategy::from_name(strategy.unwrap_or_default())
        .is_some_and(Strategy::uses_protective_stops)
    {
        2.0
    } else {
        1.0
    }
}

fn allocated_notional(settings: &Settings) -> f64 {
    allocated_notional_for_category(settings, None)
}

fn allocated_notional_for_symbol(settings: &Settings, symbol: &str) -> f64 {
    let quote_rate = quote_usd_rate(settings, symbol);
    let open_exposure = if let Some(lots) = settings.strategy_positions.get(symbol) {
        lots.values()
            .filter(|position| !position.scout_entry)
            .map(|position| position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate)
            .sum()
    } else {
        settings.positions.get(symbol).filter(|position| !position.scout_entry).map_or(0.0, |position| {
            position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate
        })
    };
    let mut pending_exposure = 0.0;
    let mut pending_quantity = 0.0;
    let current_quantity = settings.positions.get(symbol).filter(|position| !position.scout_entry).map_or(0.0, |position| position.quantity);
    for order in settings.orders.iter().filter(|order| {
        order.symbol == symbol
            && order.purpose == "signal"
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
            && order.quantity > order.filled_quantity + 1e-12
    }) {
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let net_quantity = current_quantity + pending_quantity;
        let exposure_increase = if net_quantity == 0.0 || net_quantity.signum() == direction {
            remaining
        } else {
            (remaining - net_quantity.abs()).max(0.0)
        };
        let reference_notional = if order.reserved_notional > 0.0 {
            order.reserved_notional
        } else {
            order.average_price.unwrap_or(0.0) * order.quantity * quote_rate
        };
        pending_exposure += reference_notional * exposure_increase / order.quantity;
        pending_quantity += direction * remaining;
    }
    open_exposure + pending_exposure
}

fn allocated_notional_for_category(settings: &Settings, category: Option<StrategyCategory>) -> f64 {
    let open_positions = if let Some(category) = category {
        let mut lots_total = 0.0;
        for (symbol, lots) in &settings.strategy_positions {
            for (strategy, position) in lots {
                if strategy_category(Some(strategy)) != category || position.scout_entry {
                    continue;
                }
                let value = position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol);
                lots_total += value * scheduled_stop_headroom_multiplier(
                    &position.entry_order_type,
                    Some(strategy.as_str()),
                );
            }
        }
        if settings.strategy_positions.is_empty() {
            settings
                .positions
                .iter()
                .filter(|(_, position)| !position.scout_entry && strategy_category(position.strategy.as_deref()) == category)
                .map(|(symbol, position)| {
                    position.quantity.abs()
                        * position.average_entry_price.max(0.0)
                        * quote_usd_rate(settings, symbol)
                        * scheduled_stop_headroom_multiplier(
                            &position.entry_order_type,
                            position.strategy.as_deref(),
                        )
                })
                .sum()
        } else {
            lots_total
        }
    } else if !settings.strategy_positions.is_empty() {
        settings.strategy_positions.iter().flat_map(|(symbol, lots)| {
            lots.iter().filter(|(_, position)| !position.scout_entry).map(move |(strategy, position)| {
                let value = position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol);
                value * scheduled_stop_headroom_multiplier(
                    &position.entry_order_type,
                    Some(strategy.as_str()),
                )
            })
        }).sum()
    } else {
        settings.positions.iter().filter(|(_, position)| !position.scout_entry).map(|(symbol, position)| {
            let value = position.quantity.abs()
                * position.average_entry_price.max(0.0)
                * quote_usd_rate(settings, symbol);
            value * scheduled_stop_headroom_multiplier(
                &position.entry_order_type,
                position.strategy.as_deref(),
            )
        }).sum()
    };
    let mut pending_exposure = 0.0;
    let mut pending_quantity = BTreeMap::<String, f64>::new();
    let mut pending_category_quantity = BTreeMap::<(String, String), f64>::new();
    for order in &settings.orders {
        if matches!(order.purpose.as_str(), "stop-loss" | "scout")
            || !matches!(
                order.status.as_str(),
                "submitting" | "accepted" | "resting" | "partially_filled"
            )
            || order.quantity <= 0.0
        {
            continue;
        }
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let current_quantity = if let Some(category) = category {
            let category_key = strategy_category_key(category);
            settings
                .strategy_positions
                .get(&order.symbol)
                .map(|lots| {
                    lots.iter()
                        .filter(|(strategy, position)| {
                            !position.scout_entry && strategy_category(Some(strategy.as_str())) == category
                        })
                        .map(|(_, position)| position.quantity)
                        .sum::<f64>()
                })
                .unwrap_or_else(|| {
                    settings
                        .positions
                        .get(&order.symbol)
                        .filter(|position| {
                            !position.scout_entry && strategy_category(position.strategy.as_deref()) == category
                        })
                        .filter(|position| !position.scout_entry)
                        .map(|position| position.quantity)
                        .unwrap_or(0.0)
                })
                + pending_category_quantity
                    .get(&(order.symbol.clone(), category_key.to_owned()))
                    .copied()
                    .unwrap_or(0.0)
        } else {
            settings
                .positions
                .get(&order.symbol)
                .filter(|position| !position.scout_entry)
                .map(|position| position.quantity)
                .unwrap_or(0.0)
                + pending_quantity.get(&order.symbol).copied().unwrap_or(0.0)
        };
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let increase_quantity = if current_quantity == 0.0 || current_quantity.signum() == direction
        {
            remaining
        } else {
            (remaining - current_quantity.abs()).max(0.0)
        };
        if category.is_none_or(|category| strategy_category(order.strategy.as_deref()) == category)
        {
            let reference_notional = if order.reserved_notional > 0.0 {
                order.reserved_notional
            } else {
                order.average_price.unwrap_or(0.0)
                    * order.quantity
                    * quote_usd_rate(settings, &order.symbol)
            };
            pending_exposure += reference_notional * increase_quantity / order.quantity
                * scheduled_stop_headroom_multiplier(&order.order_type, order.strategy.as_deref());
            if let Some(category) = category {
                *pending_category_quantity
                    .entry((
                        order.symbol.clone(),
                        strategy_category_key(category).to_owned(),
                    ))
                    .or_default() += direction * remaining;
            }
        }
        *pending_quantity.entry(order.symbol.clone()).or_default() += direction * remaining;
    }
    open_positions + pending_exposure
}

fn total_open_position_count(settings: &Settings) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
            .count();
    }
    settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.values())
        .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
        .count()
}

fn category_open_position_count(settings: &Settings, category: StrategyCategory) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| {
                position.quantity.abs() > 1e-12
                    && !position.scout_entry
                    && strategy_category(position.strategy.as_deref()) == category
            })
            .count();
    }
    settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.iter())
        .filter(|(strategy, position)| {
            strategy_category(Some(strategy.as_str())) == category
                && position.quantity.abs() > 1e-12
                && !position.scout_entry
        })
        .count()
}

fn category_position_limit(settings: &Settings, category: StrategyCategory) -> usize {
    match category {
        StrategyCategory::Intraday => settings.max_intraday_open_positions,
        StrategyCategory::Daily => settings.max_daily_open_positions,
    }
}

fn category_notional_limit(settings: &Settings, category: StrategyCategory) -> f64 {
    match category {
        StrategyCategory::Intraday => settings.intraday_notional_limit,
        StrategyCategory::Daily => settings.daily_strategy_notional_limit,
    }
}

fn is_independent_daily_strategy(strategy: &str) -> bool {
    Strategy::from_name(strategy)
        .is_some_and(|kind| !kind.is_live_insight() && !kind.is_dated_carry())
}

fn enabled_daily_strategy_count(settings: &Settings) -> usize {
    settings
        .strategies
        .iter()
        .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
        .count()
        .max(1)
}

fn daily_strategy_notional_share(settings: &Settings) -> f64 {
    settings.daily_strategy_notional_limit / enabled_daily_strategy_count(settings) as f64
}

fn daily_strategy_budget(settings: &Settings, strategy: &str) -> f64 {
    if settings.daily_strategy_budgets.is_empty() {
        return daily_strategy_notional_share(settings);
    }
    if let Some(budget) = settings.daily_strategy_budgets.get(strategy) {
        return *budget;
    }
    let enabled = settings
        .strategies
        .iter()
        .filter(|candidate| is_independent_daily_strategy(candidate.as_str()))
        .collect::<Vec<_>>();
    let missing = enabled
        .iter()
        .filter(|candidate| !settings.daily_strategy_budgets.contains_key(candidate.as_str()))
        .count();
    if missing == 0 || !enabled.iter().any(|candidate| candidate.as_str() == strategy) {
        return 0.0;
    }
    let configured = enabled
        .iter()
        .filter_map(|candidate| settings.daily_strategy_budgets.get(candidate.as_str()))
        .sum::<f64>();
    (settings.daily_strategy_notional_limit - configured).max(0.0) / missing as f64
}

fn enabled_intraday_strategy_count(settings: &Settings) -> usize {
    settings
        .strategies
        .iter()
        .filter(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
        .count()
        .max(1)
}

fn intraday_strategy_notional_share(settings: &Settings) -> f64 {
    settings.intraday_notional_limit / enabled_intraday_strategy_count(settings) as f64
}

fn intraday_strategy_budget(settings: &Settings, strategy: &str) -> f64 {
    if settings.intraday_strategy_budgets.is_empty() {
        return intraday_strategy_notional_share(settings);
    }
    if let Some(budget) = settings.intraday_strategy_budgets.get(strategy) {
        return *budget;
    }
    let enabled = settings
        .strategies
        .iter()
        .filter(|candidate| candidate.is_live_insight() && !candidate.is_legacy_composite())
        .collect::<Vec<_>>();
    let missing = enabled
        .iter()
        .filter(|candidate| !settings.intraday_strategy_budgets.contains_key(candidate.as_str()))
        .count();
    if missing == 0 || !enabled.iter().any(|candidate| candidate.as_str() == strategy) {
        return 0.0;
    }
    let configured = enabled
        .iter()
        .filter_map(|candidate| settings.intraday_strategy_budgets.get(candidate.as_str()))
        .sum::<f64>();
    (settings.intraday_notional_limit - configured).max(0.0) / missing as f64
}

fn allocated_notional_for_strategy(settings: &Settings, strategy: &str) -> f64 {
    let open_positions = if settings.strategy_positions.is_empty() {
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.strategy.as_deref() == Some(strategy))
            .map(|(symbol, position)| {
                position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol)
                    * scheduled_stop_headroom_multiplier(
                        &position.entry_order_type,
                        position.strategy.as_deref(),
                    )
            })
            .sum::<f64>()
    } else {
        settings
            .strategy_positions
            .iter()
            .filter_map(|(symbol, lots)| {
                lots.get(strategy).map(|position| {
                    position.quantity.abs()
                        * position.average_entry_price.max(0.0)
                        * quote_usd_rate(settings, symbol)
                        * scheduled_stop_headroom_multiplier(
                            &position.entry_order_type,
                            Some(strategy),
                        )
                })
            })
            .sum::<f64>()
    };
    let pending_orders = settings
        .orders
        .iter()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && order.purpose != "stop-loss"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > 0.0
        })
        .map(|order| {
            let remaining_fraction = ((order.quantity - order.filled_quantity).max(0.0)
                / order.quantity)
                .clamp(0.0, 1.0);
            let reference = if order.reserved_notional > 0.0 {
                order.reserved_notional
            } else {
                order.average_price.unwrap_or(0.0)
                    * order.quantity
                    * quote_usd_rate(settings, &order.symbol)
            };
            reference
                * remaining_fraction
                * scheduled_stop_headroom_multiplier(&order.order_type, order.strategy.as_deref())
        })
        .sum::<f64>();
    open_positions + pending_orders
}

fn daily_strategy_open_position_count(settings: &Settings, strategy: &str) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| {
                position.quantity.abs() > 1e-12 && !position.scout_entry && position.strategy.as_deref() == Some(strategy)
            })
            .count();
    }
    settings
        .strategy_positions
        .values()
        .filter(|lots| {
            lots.get(strategy)
                .is_some_and(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
        })
        .count()
}

fn pending_entry_strategy_keys(settings: &Settings) -> BTreeSet<(String, String)> {
    settings
        .orders
        .iter()
        .filter(|order| {
            order.purpose == "signal"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > order.filled_quantity + 1e-12
        })
        .filter_map(|order| {
            let strategy = order.strategy.as_deref()?;
            let existing = settings
                .strategy_positions
                .get(&order.symbol)
                .and_then(|lots| lots.get(strategy))
                .is_some_and(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
                || (settings.strategy_positions.is_empty()
                    && settings
                        .positions
                        .get(&order.symbol)
                        .is_some_and(|position| {
                            position.quantity.abs() > 1e-12
                                && !position.scout_entry
                                && position.strategy.as_deref() == Some(strategy)
                        }));
            (!existing).then(|| (order.symbol.clone(), strategy.to_owned()))
        })
        .collect()
}

fn pending_entry_position_count_for_category(
    settings: &Settings,
    category: StrategyCategory,
) -> usize {
    pending_entry_strategy_keys(settings)
        .iter()
        .filter(|(_, strategy)| strategy_category(Some(strategy)) == category)
        .count()
}

fn pending_entry_position_count_for_strategy(settings: &Settings, strategy: &str) -> usize {
    pending_entry_strategy_keys(settings)
        .iter()
        .filter(|(_, candidate)| candidate == &strategy)
        .count()
}

fn daily_strategy_position_limit(settings: &Settings) -> usize {
    settings
        .max_daily_open_positions
        .div_ceil(enabled_daily_strategy_count(settings))
        .max(1)
}

fn quote_usd_rate(settings: &Settings, symbol: &str) -> f64 {
    quote_usd_rate_for_map(&settings.usd_rates, symbol)
}

fn quote_usd_rate_for_map(rates: &BTreeMap<String, f64>, symbol: &str) -> f64 {
    symbol
        .split_once('-')
        .and_then(|(_, quote)| rates.get(quote))
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        // Unknown quote currencies reserve conservatively until the USD-rate feed is available.
        .unwrap_or(10.0)
}

async fn refresh_usd_rates(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let base =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let response = state
        .client
        .get(format!("{base}/positions/usd-rates"))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json::<serde_json::Value>()
        .await
        .map_err(upstream_error)?;
    let Some(rates) = response.get("rates").and_then(serde_json::Value::as_object) else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "USD rate response has no rates map".into(),
        ));
    };
    let mut parsed = default_usd_rates();
    for (currency, value) in rates {
        if let Some(rate) = value
            .as_f64()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
        {
            parsed.insert(currency.clone(), rate);
        }
    }
    parsed.insert("USD".into(), 1.0);
    let mut settings = state.settings.lock().await;
    settings.usd_rates = parsed;
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    Ok(())
}

fn reset_daily_budget(settings: &mut Settings) {
    let day = utc_day();
    if settings.utc_day != day {
        settings.utc_day = day.clone();
    }
    settings.used_notional_today = allocated_notional(settings);
    if settings.pnl_utc_day != day {
        settings.pnl_utc_day = day;
        settings.daily_realized_pnl = 0.0;
        settings.daily_loss_limit_reduced = false;
    }
}

#[cfg(test)]
mod daily_strategy_allocator_tests {
    use super::*;

    #[test]
    fn daily_budget_and_position_slots_are_split_across_selected_daily_strategies() {
        let mut settings = Settings::default();
        settings.daily_strategy_notional_limit = 600.0;
        settings.max_daily_open_positions = 10;
        settings.strategies = vec![
            Strategy::Donchian55Btc200,
            Strategy::Trend50200,
            Strategy::LiveMomentum,
            Strategy::DatedCarry,
        ];
        assert_eq!(enabled_daily_strategy_count(&settings), 2);
        assert_eq!(daily_strategy_notional_share(&settings), 300.0);
        assert_eq!(daily_strategy_position_limit(&settings), 5);

        settings.strategies.push(Strategy::Rsi2OversoldRebound);
        assert_eq!(enabled_daily_strategy_count(&settings), 3);
        assert_eq!(daily_strategy_notional_share(&settings), 200.0);
        assert_eq!(daily_strategy_position_limit(&settings), 4);
    }
}

#[cfg(test)]
mod persistence_allocation_tests {
    use super::*;

    fn position(strategy: &str, quantity: f64, price: f64) -> TraderPosition {
        serde_json::from_value(serde_json::json!({
            "strategy": strategy,
            "quantity": quantity,
            "average_entry_price": price,
            "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "entry_order_type": "Limit"
        }))
        .unwrap()
    }

    fn pending_order(symbol: &str, strategy: &str, side: &str, quantity: f64) -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": 1,
            "symbol": symbol,
            "side": side,
            "order_type": "Limit",
            "purpose": "signal",
            "strategy": strategy,
            "quantity": quantity,
            "filled_quantity": 1.0,
            "reserved_notional": 100.0,
            "average_price": 10.0,
            "status": "accepted",
            "realized_pnl": 0.0
        }))
        .unwrap()
    }

    fn settings_with_rates() -> Settings {
        let mut settings = Settings::default();
        settings.usd_rates.insert("USDT".into(), 1.0);
        settings
    }

    #[test]
    fn allocated_symbol_exposure_counts_open_lots_and_only_incremental_pending_risk() {
        let mut settings = settings_with_rates();
        settings.positions.insert("ETH-USDT".into(), position("live-momentum", 2.0, 10.0));
        settings.orders.push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 100.0);

        settings.orders[0].side = "sell".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 60.0);
        settings.orders[0].purpose = "stop-loss".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 20.0);
    }

    #[test]
    fn category_allocation_and_open_counts_support_strategy_lots_and_legacy_positions() {
        let mut settings = settings_with_rates();
        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "donchian-20-btc200".into(), position("donchian-20-btc200", 2.0, 10.0));
        settings.strategy_positions.entry("BTC-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", -1.0, 50.0));
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)), 20.0);
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Intraday)), 50.0);
        assert_eq!(allocated_notional(&settings), 70.0);
        assert_eq!(total_open_position_count(&settings), 2);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Daily), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);

        settings.strategy_positions.clear();
        settings.positions.insert("ETH-USDT".into(), position("donchian-20-btc200", 2.0, 10.0));
        settings.positions.insert("BTC-USDT".into(), position("live-momentum", -1.0, 50.0));
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)), 20.0);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);
    }

    #[test]
    fn scout_positions_do_not_consume_real_open_position_caps() {
        let mut settings = settings_with_rates();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings.strategy_positions.entry("ETH-USDT".into()).or_default()
            .insert("live-momentum".into(), scout.clone());
        settings.strategy_positions.entry("BTC-USDT".into()).or_default()
            .insert("live-trade-flow".into(), position("live-trade-flow", 1.0, 50.0));
        settings.strategy_positions.entry("SOL-USDT".into()).or_default()
            .insert("trend-50-200".into(), scout);

        assert_eq!(total_open_position_count(&settings), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Daily), 0);

        settings.strategy_positions.clear();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings.positions.insert("ETH-USDT".into(), scout);
        assert_eq!(total_open_position_count(&settings), 0);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 0);
    }

    #[test]
    fn strategy_budgets_allocate_configured_remainder_and_exposure() {
        let mut settings = settings_with_rates();
        settings.strategies = vec![
            Strategy::Donchian20Btc200,
            Strategy::Donchian55Btc200,
            Strategy::LiveMomentum,
            Strategy::LiveTradeFlow,
        ];
        settings.daily_strategy_notional_limit = 300.0;
        settings.daily_strategy_budgets.insert("donchian-20-btc200".into(), 80.0);
        assert_eq!(enabled_daily_strategy_count(&settings), 2);
        assert_eq!(daily_strategy_notional_share(&settings), 150.0);
        assert_eq!(daily_strategy_budget(&settings, "donchian-20-btc200"), 80.0);
        assert_eq!(daily_strategy_budget(&settings, "donchian-55-btc200"), 220.0);
        assert_eq!(daily_strategy_budget(&settings, "not-enabled"), 0.0);

        settings.intraday_notional_limit = 120.0;
        settings.intraday_strategy_budgets.insert("live-momentum".into(), 45.0);
        assert_eq!(enabled_intraday_strategy_count(&settings), 2);
        assert_eq!(intraday_strategy_notional_share(&settings), 60.0);
        assert_eq!(intraday_strategy_budget(&settings, "live-momentum"), 45.0);
        assert_eq!(intraday_strategy_budget(&settings, "live-trade-flow"), 75.0);

        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", 2.0, 10.0));
        settings.orders.push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(allocated_notional_for_strategy(&settings, "live-momentum"), 100.0);
        assert_eq!(daily_strategy_open_position_count(&settings, "live-momentum"), 1);
        assert_eq!(daily_strategy_open_position_count(&settings, "donchian-20-btc200"), 0);
    }

    #[test]
    fn pending_entries_are_unique_and_existing_strategy_positions_are_excluded() {
        let mut settings = settings_with_rates();
        settings.orders = vec![
            pending_order("ETH-USDT", "live-momentum", "buy", 5.0),
            pending_order("ETH-USDT", "live-momentum", "buy", 3.0),
            pending_order("BTC-USDT", "donchian-20-btc200", "buy", 2.0),
        ];
        let keys = pending_entry_strategy_keys(&settings);
        assert_eq!(keys.len(), 2);
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday), 1);
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Daily), 1);
        assert_eq!(pending_entry_position_count_for_strategy(&settings, "live-momentum"), 1);
        settings.orders.push(pending_order("SOL-USDT", "live-order-book", "buy", 2.0));
        settings.orders.last_mut().unwrap().purpose = "scout".into();
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday), 1);

        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", 1.0, 10.0));
        assert_eq!(pending_entry_position_count_for_strategy(&settings, "live-momentum"), 0);
    }

    #[test]
    fn category_limits_and_quote_conversion_use_safe_fallbacks() {
        let mut settings = settings_with_rates();
        settings.max_daily_open_positions = 7;
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_strategy_notional_limit = 320.0;
        settings.intraday_notional_limit = 90.0;
        assert_eq!(category_position_limit(&settings, StrategyCategory::Daily), 7);
        assert_eq!(category_notional_limit(&settings, StrategyCategory::Daily), 320.0);
        assert_eq!(category_notional_limit(&settings, StrategyCategory::Intraday), 90.0);
        assert_eq!(daily_strategy_position_limit(&settings), 4);
        assert_eq!(quote_usd_rate(&settings, "ETH-USDT"), 1.0);
        assert_eq!(quote_usd_rate_for_map(&BTreeMap::new(), "ETH-XYZ"), 10.0);
        assert_eq!(quote_usd_rate_for_map(&BTreeMap::from([("XYZ".into(), f64::NAN)]), "ETH-XYZ"), 10.0);
        assert_eq!(scheduled_stop_headroom_multiplier("Vwap", Some("live-momentum")), 2.0);
        assert_eq!(scheduled_stop_headroom_multiplier("Limit", Some("live-momentum")), 1.0);
        assert!(!is_independent_daily_strategy("live-momentum"));
        assert!(is_independent_daily_strategy("donchian-20-btc200"));
    }
}
