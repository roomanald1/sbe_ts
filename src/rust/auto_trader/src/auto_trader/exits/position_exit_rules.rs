#[derive(Clone, Copy)]
struct ProfitProtectionParameters {
    activation_percent: f64,
    giveback_percent: f64,
    minimum_lock_percent: f64,
}

fn profit_protection_parameters(position: &TraderPosition) -> ProfitProtectionParameters {
    let strategy_parameters = position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"));
    let parameter = |key: &str, legacy_key: &str, default: f64, min: f64, max: f64| {
        strategy_parameters
            .and_then(|parameters| parameters.get(key).or_else(|| parameters.get(legacy_key)))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(default)
            .clamp(min, max)
    };
    let activation_percent = parameter(
        "profit_protection_activation_percent",
        "shadow_profit_activation_percent",
        DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT,
        0.10,
        0.50,
    );
    ProfitProtectionParameters {
        activation_percent,
        giveback_percent: parameter(
            "profit_protection_giveback_percent",
            "shadow_profit_giveback_percent",
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT,
            0.05,
            0.30,
        ),
        minimum_lock_percent: parameter(
            "profit_protection_minimum_lock_percent",
            "shadow_profit_minimum_lock_percent",
            DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT,
            0.0,
            activation_percent,
        ),
    }
}

fn profit_protection_state(
    position: &TraderPosition,
    current_return_percent: f64,
    parameters: ProfitProtectionParameters,
) -> (Option<f64>, bool) {
    let long = position.quantity > 0.0;
    let mut peak = position.profit_protection_peak_return_percent;
    let fee_rate = configured_taker_fee_rate();
    let mut crossed_floor = false;

    // Seed persisted state from the already collected path, so positions that
    // were open before rollout retain their observed peak-profit protection.
    if peak.is_none() {
        for sample in &position.price_path {
            let price = if long { sample.bid } else { sample.ask };
            if !price.is_finite() || price <= 0.0 || position.average_entry_price <= 0.0 {
                continue;
            }
            let notional = position.quantity.abs() * position.average_entry_price;
            let net = ((price - position.average_entry_price) * position.quantity
                - position.entry_fees
                - position.quantity.abs() * price * fee_rate)
                / notional
                * 100.0;
            if !net.is_finite() {
                continue;
            }
            if net >= parameters.activation_percent {
                peak = Some(peak.map_or(net, |prior| prior.max(net)));
            }
            if peak.is_some_and(|high| {
                net <= (high - parameters.giveback_percent).max(parameters.minimum_lock_percent)
            }) {
                crossed_floor = true;
                break;
            }
        }
    }

    if current_return_percent >= parameters.activation_percent {
        peak = Some(peak.map_or(current_return_percent, |prior| prior.max(current_return_percent)));
    }
    if peak.is_some_and(|high| {
        current_return_percent
            <= (high - parameters.giveback_percent).max(parameters.minimum_lock_percent)
    }) {
        crossed_floor = true;
    }
    (peak, crossed_floor)
}

async fn evaluate_position_exit(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: String,
    strategy_key: String,
    mut position: TraderPosition,
    inputs_by_symbol: &mut BTreeMap<String, serde_json::Value>,
    research_regime: Option<&DailyStrategySnapshot>,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let position_is_current = {
        let settings = state.settings.lock().await;
        settings.strategy_positions.get(&symbol)
            .and_then(|lots| lots.get(&strategy_key))
            .is_some_and(|current| {
                (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            })
    };
    if !position_is_current {
        return Ok(None);
    }
    let Some(inputs) = inputs_by_symbol.get(&symbol).cloned() else {
        return Ok(None);
    };
    let market = &inputs["markets"][&symbol];
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid
        .zip(ask)
        .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
    else {
        return Ok(None);
    };
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings.strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy_key)) {
            let should_sample = current
                .price_path
                .last()
                .is_none_or(|sample| observed_at.saturating_sub(sample.timestamp) >= 5_000);
            if should_sample {
                current.price_path.push(TradePriceSample { timestamp: observed_at, bid, ask });
                if current.price_path.len() > 4096 {
                    current.price_path.drain(..current.price_path.len() - 4096);
                }
                position.price_path = current.price_path.clone();
                persist_settings(&state.settings_path, &settings)?;
            } else {
                position.price_path = current.price_path.clone();
            }
        }
    }
    let is_long = position.quantity > 0.0;
    let exit_price = if is_long { bid } else { ask };
    let entry_notional = position.quantity.abs() * position.average_entry_price;
    let net_open_pnl = (exit_price - position.average_entry_price) * position.quantity
        - position.entry_fees
        - position.quantity.abs() * exit_price * configured_taker_fee_rate();
    let return_fraction = if entry_notional > 0.0 {
        net_open_pnl / entry_notional
    } else {
        0.0
    };
    let (stop_loss_percent, configured_take_profit_percent, closed_trade_returns) = {
        let settings = state.settings.lock().await;
        (
            settings.stop_loss_percent,
            settings.take_profit_percent,
            settings.closed_trade_returns_percent.clone(),
        )
    };
    let stop_loss_percent = position.stop_loss_percent.unwrap_or(stop_loss_percent);
    let take_profit_percent = position
        .take_profit_percent
        .or_else(|| {
            position
                .horizon_take_profit_percent
                .map(|target| target.max(configured_take_profit_percent))
        })
        .unwrap_or(configured_take_profit_percent);
    let take_profit_percent = confidence_adjusted_take_profit_percent(
        &closed_trade_returns,
        take_profit_percent,
    );
    let take_profit_percent = age_adjusted_take_profit_percent(
        position.strategy.as_deref(),
        position.entry_timestamp_ms,
        observed_at,
        take_profit_percent,
    );
    let protection_parameters = profit_protection_parameters(&position);
    let net_return_percent = return_fraction * 100.0;
    let previous_protection_peak = position.profit_protection_peak_return_percent;
    let (protection_peak, profit_protection_exit) = profit_protection_state(
        &position,
        net_return_percent,
        protection_parameters,
    );
    if protection_peak != previous_protection_peak {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings.strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy_key)) {
            if (current.quantity - position.quantity).abs() <= 1e-12
                && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            {
                current.profit_protection_peak_return_percent = protection_peak;
                position.profit_protection_peak_return_percent = protection_peak;
                persist_settings(&state.settings_path, &settings)?;
            }
        }
    }
    let (strategy_disabled, strategy_no_longer_eligible) = {
        let settings = state.settings.lock().await;
        let strategy_disabled = position.strategy.as_ref().is_some_and(|name| {
            let enabled = settings.strategies.iter().any(|strategy| strategy.as_str() == name);
            Strategy::from_name(name).map_or(!enabled, |strategy| strategy.should_exit_when_disabled(enabled))
        });
        (strategy_disabled, position.strategy.clone())
    };
    let research_exit = strategy_no_longer_eligible.as_deref().is_some_and(|strategy| {
        research_regime.is_some_and(|regime| regime.strategy_exit_required(strategy, &symbol))
    });
    let donchian_signal_managed =
        strategy_no_longer_eligible.as_deref() == Some("donchian-55-btc200");
    let reason = if strategy_disabled {
        Some("strategy-disabled")
    } else if research_exit {
        Some("research-regime-exit")
    } else if donchian_signal_managed {
        None
    } else if return_fraction <= -stop_loss_percent / 100.0 {
        Some("stop-loss")
    } else if profit_protection_exit {
        Some("profit-protection")
    } else if return_fraction >= take_profit_percent / 100.0 {
        Some("take-profit")
    } else {
        None
    };
    let Some(reason) = reason else {
        return Ok(None);
    };
    if reason == "take-profit" {
        let probability = profitable_close_probability_5m(market, &position);
        if win_rate_supports_extended_hold(&closed_trade_returns)
            && probability.is_some_and(|value| value >= TAKE_PROFIT_HOLD_PROBABILITY_FLOOR)
        {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "System clock is before Unix epoch".into(),
                    )
                })?
                .as_millis() as u64;
            let mut settings = state.settings.lock().await;
            if let Some(current) = settings.strategy_positions.get_mut(&symbol).and_then(|lots| lots.get_mut(&strategy_key)) {
                let was_deferred = current.take_profit_deferred_at_ms.is_some();
                let deferred_at = *current.take_profit_deferred_at_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(deferred_at) < MAX_TAKE_PROFIT_HOLD_MS {
                    if !was_deferred {
                        persist_settings(&state.settings_path, &settings)?;
                        crate::service_log!(
                            "INFO",
                            "Deferring take-profit for {symbol}: five-minute profitable-close probability is {:.1}%",
                            probability.unwrap_or_default() * 100.0
                        );
                    }
                    return Ok(None);
                }
            }
        }
    }
    submit_position_exit_order(
        client,
        base,
        state,
        symbol,
        strategy_key,
        position,
        bid,
        ask,
        reason,
        return_fraction,
        stop_loss_percent,
        take_profit_percent,
    )
    .await
}

