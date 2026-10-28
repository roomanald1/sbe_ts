async fn set_enabled(
    State(state): State<AutoTraderState>,
    Json(request): Json<EnabledRequest>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if request.enabled {
        let position_url = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        sync_auto_positions(&state.client, &position_url, &state).await?;
    }
    state.enabled.store(request.enabled, Ordering::Relaxed);
    let mut settings = state.settings.lock().await;
    settings.enabled = request.enabled;
    if let Err(error) = request_settings_persist(&state) {
        crate::service_log!(
            "ERROR",
            "Could not persist Auto Trader enabled state: {}",
            error.1
        );
    }
    Ok(Json(status(&state, &settings)))
}
async fn update_settings(
    State(state): State<AutoTraderState>,
    Json(request): Json<SettingsRequest>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if !request.max_order_notional.is_finite()
        || request.max_order_notional <= 0.0
        || !request.daily_notional_limit.is_finite()
        || request.daily_notional_limit < request.max_order_notional
        || !request.daily_realized_loss_limit.is_finite()
        || request.daily_realized_loss_limit <= 0.0
        || !request.exploration_percent.is_finite()
        || !(0.0..=100.0).contains(&request.exploration_percent)
        || request
            .risk_appetite
            .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        || request
            .vwap_duration_minutes
            .is_some_and(|value| !(5..=390).contains(&value))
        || request
            .vwap_participation_percent
            .is_some_and(|value| !value.is_finite() || !(1.0..=50.0).contains(&value))
        || request
            .limit_offset_bps
            .is_some_and(|value| !value.is_finite() || !(-10.0..=10.0).contains(&value))
        || request
            .trigger_offset_bps
            .is_some_and(|value| !value.is_finite() || !(5.0..=100.0).contains(&value))
        || request
            .trailing_distance_percent
            .is_some_and(|value| !value.is_finite() || !(0.05..=1.0).contains(&value))
        || request
            .max_entries_per_auto_position
            .is_some_and(|value| !(1..=10).contains(&value))
        || request
            .closed_trade_window
            .is_some_and(|value| !(1..=5000).contains(&value))
        || !(1..=20).contains(&request.max_open_positions)
        || request
            .max_intraday_open_positions
            .is_some_and(|value| value > 20)
        || request
            .max_daily_open_positions
            .is_some_and(|value| value > 20)
        || request
            .intraday_notional_limit
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        || request
            .daily_strategy_notional_limit
            .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err((StatusCode::BAD_REQUEST, "Order and total notional limits must be positive with total notional at least the per-order limit; daily loss cap must be positive, exploration must be between 0% and 100%, and max open positions must be between 1 and 20, and entries per position must be between 1 and 10".into()));
    }
    if request.strategies.as_ref().is_some_and(|strategies| {
        strategies.is_empty() || strategies.iter().all(|strategy| strategy.is_live_insight())
    }) && request.signal_strategies_enabled == Some(false)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Enable the generic insight strategy or select at least one daily strategy".into(),
        ));
    }
    if let Some(overrides) = request.strategy_parameter_overrides.as_ref() {
        validate_strategy_parameter_overrides(overrides)
            .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    }
    if request
        .reset_strategy_profile
        .as_ref()
        .is_some_and(|profile| {
            profile.symbol.trim().is_empty() || Strategy::from_name(&profile.strategy).is_none()
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "A symbol and known strategy are required to reset a strategy profile".into(),
        ));
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if request.daily_notional_limit < settings.used_notional_today {
        return Err((
            StatusCode::CONFLICT,
            "Total notional limit cannot be less than the Auto Trader's current allocation".into(),
        ));
    }
    settings.max_order_notional = request.max_order_notional;
    let strategies_were_provided = request.strategies.is_some();
    if let Some(strategies) = request.strategies {
        let mut unique = Vec::new();
        for strategy in strategies {
            if !unique.contains(&strategy) {
                unique.push(strategy);
            }
        }
        settings.strategies = unique;
    } else if let Some(strategy) = request.strategy {
        settings.strategies = vec![strategy];
    }
    if let Some(enabled) = request.signal_strategies_enabled {
        settings.signal_strategies_enabled = enabled;
        if enabled {
            for strategy in
                Strategy::live_strategies().filter(|strategy| !strategy.is_legacy_composite())
            {
                if !settings.strategies.contains(&strategy) {
                    settings.strategies.push(strategy);
                }
            }
        } else {
            settings
                .strategies
                .retain(|strategy| !strategy.is_live_insight());
        }
    } else if strategies_were_provided {
        settings.signal_strategies_enabled = settings
            .strategies
            .iter()
            .any(|strategy| strategy.is_live_insight());
    }
    settings.daily_notional_limit = request.daily_notional_limit;
    settings.daily_realized_loss_limit = request.daily_realized_loss_limit;
    settings.exploration_percent = request.exploration_percent;
    if let Some(value) = request.risk_appetite {
        settings.risk_appetite = value;
    }
    settings.max_open_positions = request.max_open_positions;
    if let Some(value) = request.max_intraday_open_positions {
        settings.max_intraday_open_positions = value;
    }
    if let Some(value) = request.max_daily_open_positions {
        settings.max_daily_open_positions = value;
    }
    if let Some(value) = request.intraday_notional_limit {
        settings.intraday_notional_limit = value;
    }
    if let Some(value) = request.daily_strategy_notional_limit {
        settings.daily_strategy_notional_limit = value;
    }
    if let Some(value) = request.max_entries_per_auto_position {
        settings.max_entries_per_auto_position = value;
    }
    if let Some(value) = request.vwap_duration_minutes {
        settings.vwap_duration_minutes = value;
    }
    if let Some(value) = request.vwap_participation_percent {
        settings.vwap_participation_percent = value;
    }
    if let Some(value) = request.limit_offset_bps {
        settings.limit_offset_bps = value;
    }
    if let Some(value) = request.trigger_offset_bps {
        settings.trigger_offset_bps = value;
    }
    if let Some(value) = request.trailing_distance_percent {
        settings.trailing_distance_percent = value;
    }
    if let Some(value) = request.closed_trade_window {
        settings.closed_trade_window = value;
        let excess = settings
            .closed_trade_returns_percent
            .len()
            .saturating_sub(value);
        if excess > 0 {
            settings.closed_trade_returns_percent.drain(0..excess);
        }
        let timestamp_excess = settings
            .closed_trade_timestamps_ms
            .len()
            .saturating_sub(value);
        if timestamp_excess > 0 {
            settings
                .closed_trade_timestamps_ms
                .drain(0..timestamp_excess);
        }
    }
    if let Some(overrides) = request.strategy_parameter_overrides {
        settings.strategy_parameter_overrides = overrides;
    }
    if let Some(profile) = request.reset_strategy_profile {
        let marker = format!(":{}:{}:", profile.symbol, profile.strategy);
        settings.learning.retain(|key, _| !key.contains(&marker));
        let prefix = format!("{}:{}:", profile.symbol, profile.strategy);
        settings
            .strategy_parameter_overrides
            .retain(|key, _| !key.starts_with(&prefix));
    }
    if request.reset_learning {
        settings.learning.clear();
        settings.strategy_parameter_overrides.clear();
        settings.closed_trade_returns_percent.clear();
        settings.closed_trade_timestamps_ms.clear();
        settings.closed_trade_realized_usd.clear();
        settings.closed_trade_fees_usd.clear();
    }
    request_settings_persist(&state)?;
    drop(settings);

    if request.reset_learning {
        *state.latest_execution_parameters.lock().await = None;
    }
    let settings = state.settings.lock().await;
    Ok(Json(status(&state, &settings)))
}
