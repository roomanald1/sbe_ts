#[derive(Clone, Copy)]
struct ProfitProtectionParameters {
    activation_percent: f64,
    giveback_percent: f64,
    minimum_lock_percent: f64,
}

fn thesis_failure_parameters(position: &TraderPosition) -> Option<(f64, u32, f64)> {
    let strategy = position.strategy.as_deref().and_then(Strategy::from_name)?;
    if !strategy.is_live_insight() {
        return None;
    }
    let specs = strategy.parameter_specs();
    let value = |key: StrategyParameterKey| {
        let spec = specs.iter().find(|spec| spec.key == key)?;
        let configured = position
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(|parameters| parameters.get(key.as_str()))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(spec.default);
        configured
            .is_finite()
            .then_some(configured.clamp(spec.min, spec.max))
    };
    let loss_activation = value(StrategyParameterKey::ThesisFailureLossActivationPercent)?;
    let confirmations =
        value(StrategyParameterKey::ThesisFailureConfirmationSamples)?.round() as u32;
    let minimum_signal_strength = value(StrategyParameterKey::ThesisFailureMinimumSignalStrength)?;
    Some((
        loss_activation,
        confirmations.max(1),
        minimum_signal_strength,
    ))
}

fn opposing_strategy_signal_strength(
    signals: &[Signal],
    strategy: &str,
    symbol: &str,
    position_quantity: f64,
    now_ms: u64,
) -> Option<(f64, u64)> {
    let signal = signals
        .iter()
        .filter(|signal| {
            signal.strategy.as_deref() == Some(strategy)
                && signal.symbol == symbol
                && signal.strategy_candidate
                && !signal.stale
                && signal.strength.is_finite()
                && signal.timestamp <= now_ms
                && now_ms.saturating_sub(signal.timestamp) <= MAX_SIGNAL_AGE_SECONDS * 1_000
        })
        .max_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.strength.total_cmp(&right.strength))
        })?;
    let opposes_position = if position_quantity > 0.0 {
        signal.direction == "bearish"
    } else if position_quantity < 0.0 {
        signal.direction == "bullish"
    } else {
        false
    };
    let strength = signal.strength;
    (opposes_position && strength.is_finite())
        .then_some((strength.clamp(0.0, 1.0), signal.timestamp))
}

fn position_strategy_parameters(position: &TraderPosition) -> BTreeMap<String, f64> {
    position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(serde_json::Value::as_object)
        .map(|values| {
            values
                .iter()
                .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MaximumHoldStatus {
    status: &'static str,
    source: &'static str,
    maximum_hold_days: Option<f64>,
    maximum_hold_ms: Option<u64>,
    age_ms: Option<u64>,
    expired: bool,
}

fn maximum_hold_status(position: &TraderPosition, now_ms: u64) -> MaximumHoldStatus {
    let Some(strategy) = position.strategy.as_deref().and_then(Strategy::from_name) else {
        return MaximumHoldStatus {
            status: "not_applicable",
            source: "none",
            maximum_hold_days: None,
            maximum_hold_ms: None,
            age_ms: None,
            expired: false,
        };
    };
    let specs = strategy.parameter_specs();
    let Some(spec) = specs
        .iter()
        .find(|spec| spec.key == StrategyParameterKey::MaximumHoldDays)
    else {
        return MaximumHoldStatus {
            status: "not_applicable",
            source: "none",
            maximum_hold_days: None,
            maximum_hold_ms: None,
            age_ms: None,
            expired: false,
        };
    };
    let configured = position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(|parameters| parameters.get(spec.key.as_str()));
    let (days, source) = match configured {
        Some(value) => {
            let Some(days) = value.as_f64() else {
                return MaximumHoldStatus {
                    status: "invalid_profile_parameter",
                    source: "entry_profile",
                    maximum_hold_days: None,
                    maximum_hold_ms: None,
                    age_ms: None,
                    expired: false,
                };
            };
            (days, "entry_profile")
        }
        None => (spec.default, "catalog_default"),
    };
    if !days.is_finite() || days < spec.min || days > spec.max {
        return MaximumHoldStatus {
            status: "invalid_profile_parameter",
            source,
            maximum_hold_days: Some(days),
            maximum_hold_ms: None,
            age_ms: None,
            expired: false,
        };
    }
    let maximum_hold_ms = (days * 86_400_000.0).ceil() as u64;
    if maximum_hold_ms == 0 || position.entry_timestamp_ms == 0 {
        return MaximumHoldStatus {
            status: "unknown_entry_timestamp",
            source,
            maximum_hold_days: Some(days),
            maximum_hold_ms: Some(maximum_hold_ms),
            age_ms: None,
            expired: false,
        };
    }
    if position.entry_timestamp_ms > now_ms {
        return MaximumHoldStatus {
            status: "invalid_entry_timestamp",
            source,
            maximum_hold_days: Some(days),
            maximum_hold_ms: Some(maximum_hold_ms),
            age_ms: None,
            expired: false,
        };
    }
    let age_ms = now_ms.saturating_sub(position.entry_timestamp_ms);
    let expired = age_ms >= maximum_hold_ms;
    MaximumHoldStatus {
        status: if expired { "expired" } else { "within_limit" },
        source,
        maximum_hold_days: Some(days),
        maximum_hold_ms: Some(maximum_hold_ms),
        age_ms: Some(age_ms),
        expired,
    }
}

fn position_exit_reason(
    strategy_disabled: bool,
    thesis_failure_confirmed: bool,
    research_exit: bool,
    strategy_rebalance: bool,
    signal_managed_strategy: bool,
    immediate_exit_return_fraction: f64,
    stop_loss_percent: f64,
    profit_protection_exit: bool,
    maximum_hold_expired: bool,
    profit_exit_return_fraction: f64,
    take_profit_percent: f64,
) -> Option<&'static str> {
    if strategy_disabled {
        Some("strategy-disabled")
    } else if thesis_failure_confirmed {
        Some("thesis-failure")
    } else if research_exit {
        Some("research-regime-exit")
    } else if strategy_rebalance {
        Some("strategy-monthly-rebalance")
    } else if signal_managed_strategy {
        maximum_hold_expired.then_some("maximum-hold")
    } else if immediate_exit_return_fraction <= -stop_loss_percent / 100.0 {
        Some("stop-loss")
    } else if profit_protection_exit {
        Some("profit-protection")
    } else if maximum_hold_expired {
        Some("maximum-hold")
    } else if profit_exit_return_fraction >= take_profit_percent / 100.0 {
        Some("take-profit")
    } else {
        None
    }
}

fn position_uses_learned_exit_policy(position: &TraderPosition) -> bool {
    position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("learned_exit_protection_policy"))
        .is_some_and(serde_json::Value::is_object)
}

fn next_thesis_failure_confirmation(
    current_count: u32,
    last_signal_timestamp_ms: u64,
    signal_timestamp_ms: Option<u64>,
    invalidated: bool,
    loss_activation_reached: bool,
    required: u32,
) -> (u32, u64) {
    let Some(signal_timestamp_ms) = signal_timestamp_ms.filter(|timestamp| *timestamp > 0) else {
        return (0, 0);
    };
    if !invalidated || !loss_activation_reached {
        return (0, 0);
    }
    if signal_timestamp_ms <= last_signal_timestamp_ms {
        return (current_count.min(required), last_signal_timestamp_ms);
    }
    (
        current_count.saturating_add(1).min(required),
        signal_timestamp_ms,
    )
}

fn profit_protection_parameters(position: &TraderPosition) -> ProfitProtectionParameters {
    let strategy_parameters = position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"));
    let parameter = |key: &str, legacy_key: &str, default: f64| {
        strategy_parameters
            .and_then(|parameters| parameters.get(key).or_else(|| parameters.get(legacy_key)))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(default)
    };
    let activation_percent = parameter(
        "profit_protection_activation_percent",
        "scout_profit_activation_percent",
        DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT,
    )
    .clamp(0.10, 0.50);
    ProfitProtectionParameters {
        activation_percent,
        giveback_percent: parameter(
            "profit_protection_giveback_percent",
            "scout_profit_giveback_percent",
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT,
        )
        .clamp(0.05, 0.30),
        minimum_lock_percent: parameter(
            "profit_protection_minimum_lock_percent",
            "scout_profit_minimum_lock_percent",
            DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT,
        )
        .clamp(0.0, 0.20)
        .min(activation_percent),
    }
}

fn position_exit_fee_rate(position: &TraderPosition, profit_exit: bool) -> f64 {
    if position.exploratory && profit_exit {
        configured_maker_fee_rate()
    } else {
        configured_taker_fee_rate()
    }
}

fn net_exit_return_fraction(position: &TraderPosition, exit_price: f64, exit_fee_rate: f64) -> f64 {
    let entry_notional = position.quantity.abs() * position.average_entry_price;
    if !entry_notional.is_finite() || entry_notional <= 0.0 {
        return 0.0;
    }
    let gross_open_pnl = (exit_price - position.average_entry_price) * position.quantity;
    let exit_fees = position.quantity.abs() * exit_price * exit_fee_rate;
    (gross_open_pnl - position.entry_fees - exit_fees) / entry_notional
}

fn profit_protection_state(
    position: &TraderPosition,
    current_return_percent: f64,
    parameters: ProfitProtectionParameters,
) -> (Option<f64>, bool) {
    let long = position.quantity > 0.0;
    let mut peak = position.profit_protection_peak_return_percent;
    let fee_rate = position_exit_fee_rate(position, true);
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
        peak = Some(peak.map_or(current_return_percent, |prior| {
            prior.max(current_return_percent)
        }));
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
    state: &AutoTraderState,
    symbol: String,
    strategy_key: String,
    mut position: TraderPosition,
    inputs_by_symbol: &mut BTreeMap<String, serde_json::Value>,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let position_is_current = {
        let settings = state.settings.lock().await;
        let already_exiting = settings.orders.iter().any(|order| {
            order.symbol == symbol
                && order.strategy.as_deref() == Some(strategy_key.as_str())
                && order.purpose != "signal"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
        });
        !already_exiting
            && settings
                .strategy_positions
                .get(&symbol)
                .and_then(|lots| lots.get(&strategy_key))
                .is_some_and(|current| {
                    (current.quantity - position.quantity).abs() <= 1e-12
                        && (current.average_entry_price - position.average_entry_price).abs()
                            <= 1e-12
                })
    };
    if !position_is_current {
        return Ok(None);
    }
    let Some(inputs) = inputs_by_symbol.get(&symbol).cloned() else {
        return Ok(None);
    };
    let market = inputs;
    let bid = market["bid"].as_f64();
    let ask = market["ask"].as_f64();
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
    let source_updated_at_ms = market["updated_at_ms"].as_u64();
    let freshness_contract_version = market["symbol_freshness"]["contract_version"]
        .as_u64()
        .and_then(|version| u16::try_from(version).ok());
    let freshness_max_age_ms = market["symbol_freshness"]["max_age_ms"].as_u64();
    let source_fresh = match (
        freshness_contract_version,
        market["symbol_freshness"]["state"].as_str(),
        source_updated_at_ms,
        freshness_max_age_ms,
    ) {
        (Some(1), Some("fresh"), Some(updated_at), Some(max_age)) => Some(
            market["symbol"].as_str() == Some(symbol.as_str())
                && updated_at <= observed_at
                && observed_at - updated_at <= max_age,
        ),
        (Some(1), _, _, _) => Some(false),
        _ => None,
    };
    {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings
            .strategy_positions
            .get_mut(&symbol)
            .and_then(|lots| lots.get_mut(&strategy_key))
        {
            let should_sample = current
                .price_path
                .last()
                .is_none_or(|sample| observed_at.saturating_sub(sample.timestamp) >= 5_000);
            if should_sample {
                current.price_path.push(TradePriceSample {
                    timestamp: observed_at,
                    bid,
                    ask,
                    source_updated_at_ms,
                    freshness_contract_version,
                    freshness_max_age_ms,
                    source_fresh,
                });
                current.price_path_total_samples = current
                    .price_path_total_samples
                    .max(current.price_path.len() as u64)
                    .saturating_add(1);
                if current.price_path.len() > 4096 {
                    current.price_path.drain(..current.price_path.len() - 4096);
                    current.price_path_truncated = true;
                }
                position.price_path = current.price_path.clone();
                request_price_path_persist_if_due(state)?;
            } else {
                position.price_path = current.price_path.clone();
            }
        }
    }
    let is_long = position.quantity > 0.0;
    let exit_price = if is_long { bid } else { ask };
    let immediate_exit_fee_rate = position_exit_fee_rate(&position, false);
    let immediate_exit_return_fraction =
        net_exit_return_fraction(&position, exit_price, immediate_exit_fee_rate);
    let profit_exit_fee_rate = position_exit_fee_rate(&position, true);
    let profit_exit_return_fraction =
        net_exit_return_fraction(&position, exit_price, profit_exit_fee_rate);
    let (stop_loss_percent, configured_take_profit_percent, closed_trade_returns) = {
        let settings = state.settings.lock().await;
        (
            settings.stop_loss_percent,
            settings.take_profit_percent,
            closed_trade_learning_returns(&settings),
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
    let learned_exit_policy = position_uses_learned_exit_policy(&position);
    let take_profit_percent = if learned_exit_policy {
        take_profit_percent
    } else {
        confidence_adjusted_take_profit_percent(
            &closed_trade_returns,
            take_profit_percent,
            stop_loss_percent,
        )
    };
    let take_profit_percent = if learned_exit_policy {
        take_profit_percent
    } else {
        age_adjusted_take_profit_percent(
            position.strategy.as_deref(),
            position.entry_timestamp_ms,
            observed_at,
            take_profit_percent,
        )
    };
    let protection_parameters = profit_protection_parameters(&position);
    let immediate_net_return_percent = immediate_exit_return_fraction * 100.0;
    let profit_exit_net_return_percent = profit_exit_return_fraction * 100.0;
    let previous_protection_peak = position.profit_protection_peak_return_percent;
    let (protection_peak, profit_protection_exit) = profit_protection_state(
        &position,
        profit_exit_net_return_percent,
        protection_parameters,
    );
    if protection_peak != previous_protection_peak {
        let mut settings = state.settings.lock().await;
        if let Some(current) = settings
            .strategy_positions
            .get_mut(&symbol)
            .and_then(|lots| lots.get_mut(&strategy_key))
        {
            if (current.quantity - position.quantity).abs() <= 1e-12
                && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            {
                current.profit_protection_peak_return_percent = protection_peak;
                position.profit_protection_peak_return_percent = protection_peak;
                request_settings_persist(state)?;
            }
        }
    }
    let (strategy_enabled, strategy_no_longer_eligible) = {
        let settings = state.settings.lock().await;
        let enabled = position.strategy.as_ref().is_some_and(|name| {
            settings
                .strategies
                .iter()
                .any(|strategy| strategy.as_str() == name)
        });
        (enabled, position.strategy.clone())
    };
    let expected_profile_id = position.strategy.as_deref().map(|strategy_name| {
        let parameters = position_strategy_parameters(&position);
        let profile_symbol = position
            .strategy_decision
            .as_ref()
            .and_then(|decision| {
                decision
                    .get("learning_profile_symbol")
                    .or_else(|| decision.get("scout_parameter_profile_symbol"))
            })
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&symbol);
        strategy::profile_id(profile_symbol, strategy_name, &parameters)
    });
    let minimum_hold_days = position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(|parameters| parameters.get("minimum_hold_days"))
        .and_then(serde_json::Value::as_f64)
        .filter(|days| days.is_finite() && *days > 0.0)
        .unwrap_or(3.0);
    let minimum_hold_active = strategy_no_longer_eligible.as_deref()
        == Some("donchian-20-btc200-hold3")
        && position.entry_timestamp_ms > 0
        && observed_at.saturating_sub(position.entry_timestamp_ms)
            < (minimum_hold_days * 86_400_000.0) as u64;
    // Strategy workers own entry and exit decisions from their Insights
    // subscriptions. AutoTrader consumes the latest profile-attributed result.
    let strategy_decision = if !strategy_enabled {
        StrategyDecision::Exit
    } else if minimum_hold_active {
        StrategyDecision::Hold
    } else if let Some(strategy_name) = strategy_no_longer_eligible.as_deref() {
        if let Some(strategy) = Strategy::from_name(strategy_name) {
            let maximum_age_ms = if strategy.is_live_insight() {
                10_000
            } else {
                90_000
            };
            state
                .latest_worker_decisions
                .lock()
                .await
                .get(&(
                    symbol.clone(),
                    strategy_name.to_owned(),
                    expected_profile_id.clone().unwrap_or_default(),
                ))
                .filter(|report| {
                    expected_profile_id
                        .as_ref()
                        .is_some_and(|profile_id| &report.profile_id == profile_id)
                        && observed_at.saturating_sub(report.event_time_ms) <= maximum_age_ms
                })
                .map(|report| match report.decision {
                    strategy::LiveStrategyDecision::Exit => StrategyDecision::Exit,
                    strategy::LiveStrategyDecision::Rebalance => StrategyDecision::Rebalance,
                    strategy::LiveStrategyDecision::Enter
                    | strategy::LiveStrategyDecision::Hold
                    | strategy::LiveStrategyDecision::Reject => StrategyDecision::Hold,
                })
                .unwrap_or(StrategyDecision::Hold)
        } else {
            StrategyDecision::Hold
        }
    } else {
        StrategyDecision::Hold
    };
    let strategy_disabled = strategy_decision == StrategyDecision::Exit && !strategy_enabled;
    let latest_worker_report = if let Some(profile_id) = expected_profile_id.as_ref() {
        state
            .latest_worker_decisions
            .lock()
            .await
            .get(&(symbol.clone(), strategy_key.clone(), profile_id.clone()))
            .cloned()
    } else {
        None
    };
    let research_signal_active = latest_worker_report
        .as_ref()
        .map(|report| report.decision != strategy::LiveStrategyDecision::Exit);
    let thesis_failure_parameters = thesis_failure_parameters(&position);
    let loss_activation_reached = thesis_failure_parameters
        .is_some_and(|(activation, _, _)| immediate_net_return_percent <= -activation);
    let latest_live_signals = state
        .latest_live_strategy_signals
        .lock()
        .await
        .get(&(symbol.clone(), strategy_key.clone()))
        .cloned()
        .unwrap_or_default();
    let opposing_signal = opposing_strategy_signal_strength(
        &latest_live_signals,
        &strategy_key,
        &symbol,
        position.quantity,
        observed_at,
    );
    let thesis_invalidation = strategy_enabled
        && !minimum_hold_active
        && thesis_failure_parameters.is_some_and(|(_, _, minimum_strength)| {
            opposing_signal.is_some_and(|(strength, _)| strength >= minimum_strength)
        });
    let signal_timestamp_ms = opposing_signal.map(|(_, timestamp)| timestamp);
    if let Some((_, required, _)) = thesis_failure_parameters {
        let (next_count, next_signal_timestamp_ms) = next_thesis_failure_confirmation(
            position.thesis_failure_confirmation_count,
            position.thesis_failure_last_signal_timestamp_ms,
            signal_timestamp_ms,
            thesis_invalidation,
            loss_activation_reached,
            required,
        );
        if next_count != position.thesis_failure_confirmation_count
            || next_signal_timestamp_ms != position.thesis_failure_last_signal_timestamp_ms
        {
            let mut settings = state.settings.lock().await;
            if let Some(current) = settings
                .strategy_positions
                .get_mut(&symbol)
                .and_then(|lots| lots.get_mut(&strategy_key))
            {
                if (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
                {
                    current.thesis_failure_confirmation_count = next_count;
                    current.thesis_failure_last_signal_timestamp_ms = next_signal_timestamp_ms;
                    position.thesis_failure_confirmation_count = next_count;
                    position.thesis_failure_last_signal_timestamp_ms = next_signal_timestamp_ms;
                    request_settings_persist(state)?;
                }
            }
        }
    }
    let thesis_failure_confirmed = loss_activation_reached
        && thesis_invalidation
        && thesis_failure_parameters
            .is_some_and(|(_, required, _)| position.thesis_failure_confirmation_count >= required);
    // Existing strategy invalidation remains immediate. A confirmed opposing
    // signal can also exit a losing position while the strategy remains active.
    let research_exit = strategy_decision == StrategyDecision::Exit && strategy_enabled;
    let maximum_hold = maximum_hold_status(&position, observed_at);
    let forecast_strategy = position
        .strategy
        .as_deref()
        .unwrap_or(strategy_key.as_str());
    let forecast_report_max_age_ms = Strategy::from_name(forecast_strategy)
        .map(|strategy| {
            if strategy.is_live_insight() {
                10_000
            } else {
                90_000
            }
        })
        .unwrap_or(0);
    let forecast_market_regime = market["market_regime"].as_str().or_else(|| {
        latest_worker_report
            .as_ref()
            .filter(|report| {
                report.event_time_ms <= observed_at
                    && observed_at.saturating_sub(report.event_time_ms)
                        <= forecast_report_max_age_ms
            })
            .and_then(|report| report.signal.market_regime.as_deref())
    });
    let unvalidated_price_sample_probability =
        unvalidated_profitable_close_probability_5m(&market, &position);
    let validated_close_forecast = validated_close_forecast_5m(
        &market,
        &symbol,
        forecast_strategy,
        expected_profile_id.as_deref(),
        if position.quantity > 0.0 {
            "long"
        } else {
            "short"
        },
        forecast_market_regime,
        observed_at,
    );
    let exit_decision_context = serde_json::json!({
        "strategy_decision": format!("{strategy_decision:?}"),
        "minimum_hold_active": minimum_hold_active,
        "maximum_hold": {
            "status": maximum_hold.status,
            "source": maximum_hold.source,
            "maximum_hold_days": maximum_hold.maximum_hold_days,
            "maximum_hold_ms": maximum_hold.maximum_hold_ms,
            "age_ms": maximum_hold.age_ms,
            "expired": maximum_hold.expired,
        },
        "take_profit_forecast": {
            "status": validated_close_forecast.status,
            "forecast_id": validated_close_forecast.forecast_id,
            "model_id": validated_close_forecast.model_id,
            "validation_id": validated_close_forecast.validation_id,
            "as_of_ms": validated_close_forecast.as_of_ms,
            "age_ms": validated_close_forecast.age_ms,
            "max_age_ms": validated_close_forecast.max_age_ms,
            "validation_effective_samples": validated_close_forecast.validation_effective_samples,
            "calibration_error": validated_close_forecast.calibration_error,
            "probability_lower_bound": validated_close_forecast.probability_lower_bound,
            "net_return_lower_bound_percent": validated_close_forecast.net_return_lower_bound_percent,
            "unvalidated_price_sample_projection_probability": unvalidated_price_sample_probability,
            "only_validated_forecasts_affect_take_profit": true,
        },
        "strategy_worker_report_available": latest_worker_report.is_some(),
        "strategy_worker_report_event_id": latest_worker_report.as_ref().map(|report| report.event_id.clone()),
        "strategy_worker_report_timestamp_ms": latest_worker_report.as_ref().map(|report| report.event_time_ms),
        "symbol_evaluated": research_signal_active.is_some(),
        "signal_active": research_signal_active,
        "opposing_signal_strength": opposing_signal.map(|(strength, _)| strength),
        "opposing_signal_timestamp_ms": opposing_signal.map(|(_, timestamp)| timestamp),
        "thesis_failure_loss_activation_percent": thesis_failure_parameters.map(|(activation, _, _)| activation),
        "thesis_failure_required_confirmations": thesis_failure_parameters.map(|(_, count, _)| count),
        "thesis_failure_minimum_signal_strength": thesis_failure_parameters.map(|(_, _, strength)| strength),
        "thesis_failure_confirmation_count": position.thesis_failure_confirmation_count,
        "thesis_failure_last_signal_timestamp_ms": position.thesis_failure_last_signal_timestamp_ms,
        "thesis_failure_confirmed": thesis_failure_confirmed,
        "immediate_exit_net_return_percent": immediate_net_return_percent,
        "profit_exit_net_return_percent": profit_exit_net_return_percent,
        "immediate_exit_fee_rate": immediate_exit_fee_rate,
        "profit_exit_fee_rate": profit_exit_fee_rate,
        "symbol_diagnostic": latest_worker_report
            .as_ref()
            .and_then(|report| report.signal.strategy_decision.clone()),
    });
    let strategy_rebalance = strategy_decision == StrategyDecision::Rebalance;
    let signal_managed_strategy = strategy_no_longer_eligible
        .as_deref()
        .and_then(Strategy::from_name)
        .is_some_and(|strategy| !strategy.uses_fixed_take_profit());
    let reason = position_exit_reason(
        strategy_disabled,
        thesis_failure_confirmed,
        research_exit,
        strategy_rebalance,
        signal_managed_strategy,
        immediate_exit_return_fraction,
        stop_loss_percent,
        profit_protection_exit,
        maximum_hold.expired,
        profit_exit_return_fraction,
        take_profit_percent,
    );
    let Some(reason) = reason else {
        return Ok(None);
    };
    let return_fraction = if matches!(reason, "take-profit" | "profit-protection") {
        profit_exit_return_fraction
    } else {
        immediate_exit_return_fraction
    };
    if reason == "take-profit" {
        if win_rate_supports_extended_hold(&closed_trade_returns)
            && validated_close_forecast.eligible
            && validated_close_forecast
                .probability_lower_bound
                .is_some_and(|value| value >= TAKE_PROFIT_HOLD_PROBABILITY_FLOOR)
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
            if let Some(current) = settings
                .strategy_positions
                .get_mut(&symbol)
                .and_then(|lots| lots.get_mut(&strategy_key))
            {
                let was_deferred = current.take_profit_deferred_at_ms.is_some();
                let deferred_at = *current.take_profit_deferred_at_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(deferred_at) < MAX_TAKE_PROFIT_HOLD_MS {
                    if !was_deferred {
                        request_settings_persist(state)?;
                        crate::service_log!(
                            "INFO",
                            "Deferring take-profit for {symbol}: validated forecast {} has five-minute profitable-close probability lower bound {:.1}%",
                            validated_close_forecast
                                .forecast_id
                                .as_deref()
                                .unwrap_or("unknown"),
                            validated_close_forecast
                                .probability_lower_bound
                                .unwrap_or_default()
                                * 100.0
                        );
                    }
                    return Ok(None);
                }
            }
        }
    }
    // Serialize only the state-changing exit order. Position and market
    // evaluation above may overlap with signal processing, so verify the
    // captured position is still current once the lock has been acquired.
    let _cycle = acquire_cycle_lock(state, "position_maintenance").await;
    let settings_lock_started = Instant::now();
    let settings = state.settings.lock().await;
    record_cycle_lock_scope(
        state,
        "position_exit_revalidation_settings_lock_wait",
        settings_lock_started.elapsed(),
    );
    let revalidation_started = Instant::now();
    let already_exiting = settings.orders.iter().any(|order| {
        order.symbol == symbol
            && order.strategy.as_deref() == Some(strategy_key.as_str())
            && order.purpose != "signal"
            && matches!(
                order.status.as_str(),
                "submitting" | "accepted" | "resting" | "partially_filled"
            )
    });
    let position_is_current = !already_exiting
        && settings
            .strategy_positions
            .get(&symbol)
            .and_then(|lots| lots.get(&strategy_key))
            .is_some_and(|current| {
                (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            });
    let revalidation_elapsed = revalidation_started.elapsed();
    drop(settings);
    record_cycle_lock_scope(state, "position_exit_revalidation", revalidation_elapsed);
    if !position_is_current {
        return Ok(None);
    }
    if reason == "stop-loss"
        && position
            .protective_stops
            .iter()
            .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
            .sum::<f64>()
            + 1e-12
            >= position.quantity.abs()
    {
        return Ok(None);
    }
    // Reserve the exit while holding the cycle lock, then release it before
    // protective-stop cancellation and order submission perform network I/O.
    let reservation_prepare_started = Instant::now();
    let now_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let timestamp = next_unique_order_timestamp(now_timestamp);
    let side = if position.quantity > 0.0 {
        "sell"
    } else {
        "buy"
    };
    let notional = position.quantity.abs() * (bid + (ask - bid) * 0.5);
    record_cycle_lock_scope(
        state,
        "position_exit_reservation_prepare",
        reservation_prepare_started.elapsed(),
    );
    let settings_lock_started = Instant::now();
    let order_record_elapsed;
    {
        let mut settings = state.settings.lock().await;
        record_cycle_lock_scope(
            state,
            "position_exit_reservation_settings_lock_wait",
            settings_lock_started.elapsed(),
        );
        let order_record_started = Instant::now();
        reset_daily_budget(&mut settings);
        settings.used_notional_today = allocated_notional(&settings);
        settings.orders.push(new_position_exit_order(
            timestamp,
            &symbol,
            side,
            reason,
            &position,
            position.quantity.abs(),
            notional,
        ));
        order_record_elapsed = order_record_started.elapsed();
    }
    drop(_cycle);
    record_cycle_lock_scope(state, "position_exit_order_record", order_record_elapsed);
    submit_position_exit_order(
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
        timestamp,
        exit_decision_context,
    )
    .await
}

#[cfg(test)]
mod position_exit_profit_protection_tests {
    use super::*;

    #[test]
    fn maximum_hold_uses_the_entry_profile_and_expires_at_the_exact_boundary() {
        let mut position = position(
            1.0,
            serde_json::json!({
                "strategy_parameters": {"maximum_hold_days": 1.5}
            }),
            None,
        );
        position.strategy = Some("rsi2-oversold-rebound".into());
        position.entry_timestamp_ms = 1_000;

        let before = maximum_hold_status(&position, 1_000 + 129_599_999);
        assert_eq!(before.status, "within_limit");
        assert_eq!(before.source, "entry_profile");
        assert_eq!(before.maximum_hold_days, Some(1.5));
        assert_eq!(before.maximum_hold_ms, Some(129_600_000));
        assert!(!before.expired);

        let at_boundary = maximum_hold_status(&position, 1_000 + 129_600_000);
        assert_eq!(at_boundary.status, "expired");
        assert_eq!(at_boundary.age_ms, Some(129_600_000));
        assert!(at_boundary.expired);
    }

    #[test]
    fn maximum_hold_uses_catalog_default_only_for_missing_legacy_parameter() {
        let mut position = position(1.0, serde_json::json!({"strategy_parameters": {}}), None);
        position.strategy = Some("rsi2-oversold-rebound".into());
        position.entry_timestamp_ms = 1_000;
        let status = maximum_hold_status(&position, 1_000 + 5 * 86_400_000);
        assert_eq!(status.status, "expired");
        assert_eq!(status.source, "catalog_default");
        assert_eq!(status.maximum_hold_days, Some(5.0));
        assert_eq!(status.age_ms, Some(5 * 86_400_000));
    }

    #[test]
    fn maximum_hold_does_not_infer_duration_from_unknown_or_invalid_inputs() {
        let mut position = position(
            1.0,
            serde_json::json!({"strategy_parameters": {"maximum_hold_days": 1.0}}),
            None,
        );
        position.strategy = Some("rsi2-oversold-rebound".into());
        assert_eq!(
            maximum_hold_status(&position, 10 * 86_400_000).status,
            "unknown_entry_timestamp"
        );
        assert!(!maximum_hold_status(&position, 10 * 86_400_000).expired);

        position.entry_timestamp_ms = 1;
        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["maximum_hold_days"] =
            serde_json::json!(0.5);
        let invalid = maximum_hold_status(&position, 10 * 86_400_000);
        assert_eq!(invalid.status, "invalid_profile_parameter");
        assert!(!invalid.expired);

        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["maximum_hold_days"] =
            serde_json::json!(1.0);
        let future = maximum_hold_status(&position, 0);
        assert_eq!(future.status, "invalid_entry_timestamp");
        assert!(!future.expired);

        position.strategy = Some("live-momentum".into());
        assert_eq!(
            maximum_hold_status(&position, u64::MAX).status,
            "not_applicable"
        );
    }

    #[test]
    fn maximum_hold_exit_preserves_prior_reasons_and_hard_stop_priority() {
        let reason = |signal_managed_strategy,
                      immediate_exit_return_fraction,
                      profit_protection_exit,
                      maximum_hold_expired,
                      research_exit| {
            position_exit_reason(
                false,
                false,
                research_exit,
                false,
                signal_managed_strategy,
                immediate_exit_return_fraction,
                0.5,
                profit_protection_exit,
                maximum_hold_expired,
                0.0,
                0.8,
            )
        };
        assert_eq!(reason(true, 0.0, false, true, false), Some("maximum-hold"));
        assert_eq!(reason(true, 0.0, false, false, false), None);
        assert_eq!(reason(false, -0.005, true, true, false), Some("stop-loss"));
        assert_eq!(
            reason(false, 0.0, true, true, false),
            Some("profit-protection")
        );
        assert_eq!(
            reason(true, 0.0, false, true, true),
            Some("research-regime-exit")
        );
    }

    #[test]
    fn exploratory_profit_exits_use_maker_cost_but_immediate_exits_use_taker_cost() {
        let exploratory = TraderPosition {
            exploratory: true,
            ..TraderPosition::default()
        };
        let strict = TraderPosition::default();
        assert_eq!(
            position_exit_fee_rate(&exploratory, true),
            configured_maker_fee_rate()
        );
        assert_eq!(
            position_exit_fee_rate(&exploratory, false),
            configured_taker_fee_rate()
        );
        assert_eq!(
            position_exit_fee_rate(&strict, true),
            configured_taker_fee_rate()
        );
    }

    #[test]
    fn profit_protection_activation_uses_net_return_after_entry_and_exit_fees() {
        let parameters = ProfitProtectionParameters {
            activation_percent: 0.15,
            giveback_percent: 0.05,
            minimum_lock_percent: 0.0,
        };
        let mut long = position(1.0, serde_json::json!({}), None);
        long.entry_fees = 0.10;
        let below_net_floor = net_exit_return_fraction(&long, 100.30, 0.001);
        assert!((below_net_floor * 100.0 - 0.0997).abs() < 1e-9);
        assert_eq!(
            profit_protection_state(&long, below_net_floor * 100.0, parameters),
            (None, false),
            "gross return above 0.15% does not activate below 0.15% after both fees"
        );
        let above_net_floor = net_exit_return_fraction(&long, 100.351, 0.001);
        assert!(above_net_floor * 100.0 > parameters.activation_percent);
        assert!(
            profit_protection_state(&long, above_net_floor * 100.0, parameters)
                .0
                .is_some()
        );

        let mut short = position(-1.0, serde_json::json!({}), None);
        short.entry_fees = 0.10;
        let short_net = net_exit_return_fraction(&short, 99.649, 0.001);
        assert!(short_net * 100.0 > parameters.activation_percent);
        assert!(
            profit_protection_state(&short, short_net * 100.0, parameters)
                .0
                .is_some()
        );
    }

    fn position(quantity: f64, decision: serde_json::Value, peak: Option<f64>) -> TraderPosition {
        serde_json::from_value(serde_json::json!({
            "quantity": quantity,
            "average_entry_price": 100.0,
            "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "strategy_decision": decision,
            "profit_protection_peak_return_percent": peak
        }))
        .unwrap()
    }

    #[test]
    fn strategy_parameters_and_legacy_aliases_are_bounded() {
        let defaults = profit_protection_parameters(&position(1.0, serde_json::json!({}), None));
        assert_eq!(
            defaults.activation_percent,
            DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT
        );
        assert_eq!(
            defaults.giveback_percent,
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT
        );
        assert_eq!(
            defaults.minimum_lock_percent,
            DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT
        );

        let custom = profit_protection_parameters(&position(
            1.0,
            serde_json::json!({
                "strategy_parameters": {
                    "profit_protection_activation_percent": 9.0,
                    "profit_protection_giveback_percent": 0.001,
                    "profit_protection_minimum_lock_percent": 1.0
                }
            }),
            None,
        ));
        assert_eq!(custom.activation_percent, 0.50);
        assert_eq!(custom.giveback_percent, 0.05);
        assert_eq!(custom.minimum_lock_percent, 0.20);

        let legacy = profit_protection_parameters(&position(
            1.0,
            serde_json::json!({
                "strategy_parameters": {
                    "scout_profit_activation_percent": 0.25,
                    "scout_profit_giveback_percent": 0.1,
                    "scout_profit_minimum_lock_percent": 0.05
                }
            }),
            None,
        ));
        assert_eq!(legacy.activation_percent, 0.25);
        assert_eq!(legacy.giveback_percent, 0.1);
        assert_eq!(legacy.minimum_lock_percent, 0.05);
    }

    #[test]
    fn peak_profit_is_raised_and_giveback_crosses_the_minimum_lock_floor() {
        let open_position = position(1.0, serde_json::json!({}), Some(1.0));
        let parameters = ProfitProtectionParameters {
            activation_percent: 1.0,
            giveback_percent: 0.5,
            minimum_lock_percent: 0.25,
        };
        assert_eq!(
            profit_protection_state(&open_position, 1.5, parameters),
            (Some(1.5), false)
        );
        assert_eq!(
            profit_protection_state(&open_position, 1.0, parameters),
            (Some(1.0), false)
        );
        assert_eq!(
            profit_protection_state(&open_position, 0.5, parameters),
            (Some(1.0), true)
        );
        assert_eq!(
            profit_protection_state(&position(1.0, serde_json::json!({}), None), 0.5, parameters),
            (None, false)
        );
    }

    #[test]
    fn legacy_price_path_seeds_long_and_short_profit_peaks_and_ignores_bad_marks() {
        let parameters = ProfitProtectionParameters {
            activation_percent: 0.1,
            giveback_percent: 0.2,
            minimum_lock_percent: 0.0,
        };
        let long: TraderPosition = serde_json::from_value(serde_json::json!({
            "quantity": 1.0, "average_entry_price": 100.0, "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "price_path": [
                {"timestamp":1,"bid":-1.0,"ask":101.0},
                {"timestamp":2,"bid":101.0,"ask":102.0},
                {"timestamp":3,"bid":100.5,"ask":101.0}
            ]
        }))
        .unwrap();
        let (long_peak, long_crossed) = profit_protection_state(&long, 0.0, parameters);
        assert!(long_peak.is_some_and(|peak| (0.89..0.90).contains(&peak)));
        assert!(long_crossed);

        let short: TraderPosition = serde_json::from_value(serde_json::json!({
            "quantity": -1.0, "average_entry_price": 100.0, "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "price_path": [{"timestamp":1,"bid":98.0,"ask":99.0}]
        }))
        .unwrap();
        let (short_peak, short_crossed) = profit_protection_state(&short, 0.0, parameters);
        assert!(short_peak.is_some_and(|peak| (0.90..1.0).contains(&peak)));
        assert!(short_crossed);
    }

    #[test]
    fn thesis_failure_requires_distinct_invalidated_snapshots_inside_the_loss_zone() {
        let first = next_thesis_failure_confirmation(0, 0, Some(100), true, true, 2);
        assert_eq!(first, (1, 100));
        assert_eq!(
            next_thesis_failure_confirmation(first.0, first.1, Some(100), true, true, 2),
            first,
            "reprocessing one snapshot must not count twice"
        );
        let second = next_thesis_failure_confirmation(first.0, first.1, Some(200), true, true, 2);
        assert_eq!(second, (2, 200));
        assert_eq!(
            next_thesis_failure_confirmation(second.0, second.1, Some(300), false, true, 2),
            (0, 0),
            "an active signal resets invalidation evidence"
        );
        assert_eq!(
            next_thesis_failure_confirmation(first.0, first.1, Some(200), true, false, 2),
            (0, 0),
            "losses below activation do not accumulate early-exit evidence"
        );
    }

    #[test]
    fn thesis_failure_defaults_come_from_each_strategy_parameter_spec() {
        let mut position = position(
            1.0,
            serde_json::json!({
                "strategy": "live-momentum",
                "strategy_parameters": {}
            }),
            None,
        );
        position.strategy = Some("live-momentum".into());
        let parameters = thesis_failure_parameters(&position).unwrap();
        let specs = Strategy::LiveMomentum.parameter_specs();
        let activation = specs
            .iter()
            .find(|spec| spec.key == StrategyParameterKey::ThesisFailureLossActivationPercent)
            .unwrap();
        let confirmations = specs
            .iter()
            .find(|spec| spec.key == StrategyParameterKey::ThesisFailureConfirmationSamples)
            .unwrap();
        let minimum_strength = specs
            .iter()
            .find(|spec| spec.key == StrategyParameterKey::ThesisFailureMinimumSignalStrength)
            .unwrap();
        assert_eq!(
            parameters,
            (
                activation.default,
                confirmations.default as u32,
                minimum_strength.default
            )
        );
    }

    #[test]
    fn thesis_failure_uses_only_fresh_opposing_strategy_signals() {
        let signal = |strategy: &str, direction: &str, timestamp, stale| Signal {
            range_location: None,
            symbol: "BTC-USDT".into(),
            direction: direction.into(),
            strength: 0.8,
            timestamp,
            stale,
            score: 0.8,
            ranging_probability: None,
            components: Default::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: None,
            market_regime: None,
            market_state: None,
            news_attribution: None,
            entry_gate: None,
        };
        let bearish = vec![signal("live-momentum", "bearish", 9_500, false)];
        assert_eq!(
            opposing_strategy_signal_strength(&bearish, "live-momentum", "BTC-USDT", 1.0, 10_000),
            Some((0.8, 9_500))
        );
        assert_eq!(
            opposing_strategy_signal_strength(&bearish, "live-momentum", "BTC-USDT", -1.0, 10_000),
            None
        );
        let wrong_strategy = vec![signal("live-trade-flow", "bearish", 9_500, false)];
        assert_eq!(
            opposing_strategy_signal_strength(
                &wrong_strategy,
                "live-momentum",
                "BTC-USDT",
                1.0,
                10_000
            ),
            None
        );
        let stale = vec![signal("live-momentum", "bearish", 9_000, true)];
        assert_eq!(
            opposing_strategy_signal_strength(&stale, "live-momentum", "BTC-USDT", 1.0, 10_000),
            None
        );
    }
}
