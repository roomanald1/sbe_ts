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
        configured.is_finite().then_some(configured.clamp(spec.min, spec.max))
    };
    let loss_activation = value(StrategyParameterKey::ThesisFailureLossActivationPercent)?;
    let confirmations = value(StrategyParameterKey::ThesisFailureConfirmationSamples)?
        .round() as u32;
    let minimum_signal_strength = value(StrategyParameterKey::ThesisFailureMinimumSignalStrength)?;
    Some((loss_activation, confirmations.max(1), minimum_signal_strength))
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
    (current_count.saturating_add(1).min(required), signal_timestamp_ms)
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
        "scout_profit_activation_percent",
        DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT,
        0.10,
        0.50,
    );
    ProfitProtectionParameters {
        activation_percent,
        giveback_percent: parameter(
            "profit_protection_giveback_percent",
            "scout_profit_giveback_percent",
            DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT,
            0.05,
            0.30,
        ),
        minimum_lock_percent: parameter(
            "profit_protection_minimum_lock_percent",
            "scout_profit_minimum_lock_percent",
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
        let already_exiting = settings.orders.iter().any(|order| {
            order.symbol == symbol
                && order.strategy.as_deref() == Some(strategy_key.as_str())
                && order.order_type == "Ioc"
                && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
        });
        !already_exiting && settings
            .strategy_positions
            .get(&symbol)
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
                });
                if current.price_path.len() > 4096 {
                    current.price_path.drain(..current.price_path.len() - 4096);
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
    let take_profit_percent =
        confidence_adjusted_take_profit_percent(
            &closed_trade_returns,
            take_profit_percent,
            stop_loss_percent,
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
    let (protection_peak, profit_protection_exit) =
        profit_protection_state(&position, net_return_percent, protection_parameters);
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
            settings.strategies.iter().any(|strategy| strategy.as_str() == name)
        });
        (enabled, position.strategy.clone())
    };
    let minimum_hold_days = position.strategy_decision.as_ref()
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
    // Evaluate signal state and strategy enablement through the strategy lifecycle evaluator.
    let strategy_decision = strategy_no_longer_eligible.as_deref().map(|strategy| {
        if let Some(regime) = research_regime.as_ref() {
            regime.evaluate_position(strategy, &symbol, strategy_enabled,
                position.strategy_decision.as_ref(), minimum_hold_active)
        } else {
            let parameters = BTreeMap::new();
            Strategy::from_name(strategy).map_or(
                if strategy_enabled { StrategyDecision::Hold } else { StrategyDecision::Exit },
                |kind| kind.evaluate(&StrategySignal::position(
                    &parameters, strategy_enabled, None, false,
                )).unwrap_or(StrategyDecision::Hold),
            )
        }
    }).unwrap_or(StrategyDecision::Hold);
    let strategy_disabled = strategy_decision == StrategyDecision::Exit && !strategy_enabled;
    let research_signal_active = research_regime
        .and_then(|snapshot| snapshot.signal_active(&strategy_key, &symbol));
    let thesis_failure_parameters = thesis_failure_parameters(&position);
    let loss_activation_reached = thesis_failure_parameters
        .is_some_and(|(activation, _, _)| net_return_percent <= -activation);
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
        && thesis_failure_parameters.is_some_and(|(_, required, _)| {
            position.thesis_failure_confirmation_count >= required
        });
    // Existing strategy invalidation remains immediate. A confirmed opposing
    // signal can also exit a losing position while the strategy remains active.
    let research_exit = strategy_decision == StrategyDecision::Exit && strategy_enabled;
    let exit_decision_context = serde_json::json!({
        "strategy_decision": format!("{strategy_decision:?}"),
        "minimum_hold_active": minimum_hold_active,
        "research_snapshot_available": research_regime.is_some(),
        "research_snapshot_as_of_ms": research_regime.map(|snapshot| snapshot.as_of_ms),
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
        "symbol_diagnostic": research_regime
            .and_then(|snapshot| snapshot.symbol_diagnostic(&strategy_key, &symbol)),
    });
    let strategy_rebalance = strategy_decision == StrategyDecision::Rebalance;
    let signal_managed_strategy = strategy_no_longer_eligible
        .as_deref()
        .and_then(Strategy::from_name)
        .is_some_and(|strategy| !strategy.uses_fixed_take_profit());
    let reason = if strategy_disabled {
        Some("strategy-disabled")
    } else if thesis_failure_confirmed {
        Some("thesis-failure")
    } else if research_exit {
        Some("research-regime-exit")
    } else if strategy_rebalance {
        Some("strategy-monthly-rebalance")
    } else if signal_managed_strategy {
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
                            "Deferring take-profit for {symbol}: five-minute profitable-close probability is {:.1}%",
                            probability.unwrap_or_default() * 100.0
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
            && order.order_type == "Ioc"
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
    });
    let position_is_current = !already_exiting && settings
        .strategy_positions
        .get(&symbol)
        .and_then(|lots| lots.get(&strategy_key))
        .is_some_and(|current| {
            (current.quantity - position.quantity).abs() <= 1e-12
                && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
        });
    let revalidation_elapsed = revalidation_started.elapsed();
    drop(settings);
    record_cycle_lock_scope(
        state,
        "position_exit_revalidation",
        revalidation_elapsed,
    );
    if !position_is_current {
        return Ok(None);
    }
    if reason == "stop-loss"
        && position.protective_stops.iter()
            .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
            .sum::<f64>() + 1e-12 >= position.quantity.abs()
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
    let side = if position.quantity > 0.0 { "sell" } else { "buy" };
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
            timestamp, &symbol, side, reason, &position,
            position.quantity.abs(), notional,
        ));
        order_record_elapsed = order_record_started.elapsed();
    }
    drop(_cycle);
    record_cycle_lock_scope(
        state,
        "position_exit_order_record",
        order_record_elapsed,
    );
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
        timestamp,
        exit_decision_context,
    )
    .await
}

#[cfg(test)]
mod position_exit_profit_protection_tests {
    use super::*;

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
        assert_eq!(defaults.activation_percent, DEFAULT_PROFIT_PROTECTION_ACTIVATION_PERCENT);
        assert_eq!(defaults.giveback_percent, DEFAULT_PROFIT_PROTECTION_GIVEBACK_PERCENT);
        assert_eq!(defaults.minimum_lock_percent, DEFAULT_PROFIT_PROTECTION_MINIMUM_LOCK_PERCENT);

        let custom = profit_protection_parameters(&position(1.0, serde_json::json!({
            "strategy_parameters": {
                "profit_protection_activation_percent": 9.0,
                "profit_protection_giveback_percent": 0.001,
                "profit_protection_minimum_lock_percent": 1.0
            }
        }), None));
        assert_eq!(custom.activation_percent, 0.50);
        assert_eq!(custom.giveback_percent, 0.05);
        assert_eq!(custom.minimum_lock_percent, 0.50);

        let legacy = profit_protection_parameters(&position(1.0, serde_json::json!({
            "strategy_parameters": {
                "scout_profit_activation_percent": 0.25,
                "scout_profit_giveback_percent": 0.1,
                "scout_profit_minimum_lock_percent": 0.05
            }
        }), None));
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
        assert_eq!(profit_protection_state(&open_position, 1.5, parameters), (Some(1.5), false));
        assert_eq!(profit_protection_state(&open_position, 1.0, parameters), (Some(1.0), false));
        assert_eq!(profit_protection_state(&open_position, 0.5, parameters), (Some(1.0), true));
        assert_eq!(profit_protection_state(&position(1.0, serde_json::json!({}), None), 0.5, parameters), (None, false));
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
        })).unwrap();
        let (long_peak, long_crossed) = profit_protection_state(&long, 0.0, parameters);
        assert!(long_peak.is_some_and(|peak| (0.89..0.90).contains(&peak)));
        assert!(long_crossed);

        let short: TraderPosition = serde_json::from_value(serde_json::json!({
            "quantity": -1.0, "average_entry_price": 100.0, "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "price_path": [{"timestamp":1,"bid":98.0,"ask":99.0}]
        })).unwrap();
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
        let signal = |strategy: &str, direction: &str, timestamp, stale| {
            Signal {
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
                news_attribution: None,
                entry_gate: None,
            }
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
            opposing_strategy_signal_strength(&wrong_strategy, "live-momentum", "BTC-USDT", 1.0, 10_000),
            None
        );
        let stale = vec![signal("live-momentum", "bearish", 9_000, true)];
        assert_eq!(
            opposing_strategy_signal_strength(&stale, "live-momentum", "BTC-USDT", 1.0, 10_000),
            None
        );
    }
}
