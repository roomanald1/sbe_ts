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
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not save Auto Trader settings: {error}"),
            )
        })
}
fn utc_day() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    days.to_string()
}
fn allocated_notional(settings: &Settings) -> f64 {
    let open_positions = settings
        .positions
        .iter()
        .map(|(symbol, position)| {
            let position_notional = position.quantity.abs()
                * position.average_entry_price.max(0.0)
                * quote_usd_rate(settings, symbol);
            let scheduled_position = matches!(position.entry_order_type.as_str(), "Vwap" | "Twap");
            position_notional * if scheduled_position { 2.0 } else { 1.0 }
        })
        .sum::<f64>();
    let mut pending = BTreeMap::<String, f64>::new();
    let mut pending_quantity = BTreeMap::<String, f64>::new();
    for order in &settings.orders {
        // Protective stop orders only close an existing position. A triggered
        // stop-market barrier that partially fills is terminal in Position
        // Management, but its residual quantity remains in the persisted order
        // record. Counting that residual as pending entry exposure double-counts
        // the already-netted position and can push allocation far above its cap.
        if order.purpose == "stop-loss"
            || !matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
            || order.quantity <= 0.0
        {
            continue;
        }
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let current_quantity = settings
            .positions
            .get(&order.symbol)
            .map(|position| position.quantity)
            .unwrap_or(0.0)
            + pending_quantity.get(&order.symbol).copied().unwrap_or(0.0);
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let increase_quantity = if current_quantity == 0.0 || current_quantity.signum() == direction
        {
            remaining
        } else {
            (remaining - current_quantity.abs()).max(0.0)
        };
        let reference_notional = if order.reserved_notional > 0.0 {
            order.reserved_notional
        } else {
            order.average_price.unwrap_or(0.0)
                * order.quantity
                * quote_usd_rate(settings, &order.symbol)
        };
        let scheduled_order = order.order_type.starts_with("Vwap") || order.order_type.starts_with("Twap");
        let stop_headroom_multiplier = if scheduled_order { 2.0 } else { 1.0 };
        *pending.entry(order.symbol.clone()).or_default() +=
            reference_notional * increase_quantity / order.quantity * stop_headroom_multiplier;
        *pending_quantity.entry(order.symbol.clone()).or_default() += direction * remaining;
    }
    open_positions + pending.values().sum::<f64>()
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
    let base = std::env::var("POSITION_MANAGEMENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
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
