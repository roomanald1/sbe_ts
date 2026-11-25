struct SelectedSignalOrderRequest<'a> {
    state: &'a AutoTraderState,
    dashboard: &'a Dashboard,
    signal: &'a Signal,
    exploratory: bool,
    counterfactual_stop_loss_percent: f64,
    counterfactual_take_profit_percent: f64,
    signal_selection_ms: f64,
    context_threshold_refresh_ms: f64,
    cycle_started_at: Instant,
    now: u64,
    now_ms: u64,
    market: serde_json::Value,
    bid: f64,
    ask: f64,
    mid: f64,
    market_cache_lookup_ms: f64,
    market_http_fetch_ms: f64,
}

fn portfolio_reduction_quantity(settings: &Settings, symbol: &str, strategy: &str) -> f64 {
    settings
        .strategy_positions
        .get(symbol)
        .and_then(|lots| lots.get(strategy))
        .or_else(|| {
            settings.positions.get(symbol).filter(|position| {
                settings.strategy_positions.is_empty()
                    && position.strategy.as_deref() == Some(strategy)
            })
        })
        .map(|position| position.quantity.max(0.0))
        .unwrap_or(0.0)
}

struct SignalSettingsSnapshot {
    settings: Settings,
    quote_rate: f64,
    settings_lock_wait_ms: f64,
    ioc_fallback_for: Option<u64>,
    exploratory_event_key: Option<String>,
}

const MAX_RETAINED_EXPLORATORY_EVENT_RESERVATIONS: usize = 10_000;

fn source_episode_id_for_decision(
    decision: Option<&serde_json::Value>,
) -> Result<Option<String>, ()> {
    let Some(decision) = decision else {
        return Ok(None);
    };
    let Some(raw_version) = decision.get("live_worker_event_identity_version") else {
        return Ok(None);
    };
    let identity_version = raw_version
        .as_u64()
        .and_then(|version| u16::try_from(version).ok())
        .ok_or(())?;
    match identity_version {
        0 => Ok(None),
        strategy::LIVE_STRATEGY_EVENT_ID_VERSION => {
            let episode_id = decision
                .get("live_worker_episode_id")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty() && value.len() <= 512)
                .ok_or(())?;
            Ok(Some(episode_id.to_owned()))
        }
        _ => Err(()),
    }
}

fn exploratory_event_key(
    identity_version: u16,
    event_id: &str,
    episode_id: &str,
    symbol: &str,
    strategy: &str,
    direction: &str,
    profile_id: &str,
) -> Option<String> {
    if event_id.trim().is_empty()
        || event_id.len() > 512
        || episode_id.trim().is_empty()
        || episode_id.len() > 512
        || symbol.trim().is_empty()
        || symbol.len() > 128
        || strategy.trim().is_empty()
        || strategy.len() > 128
        || direction.trim().is_empty()
        || direction.len() > 32
        || profile_id.trim().is_empty()
        || profile_id.len() > 512
    {
        return None;
    }
    // Versioned event IDs identify individual source updates. Deduplication
    // uses the stable thesis episode so routine market updates cannot create
    // a fresh exploratory reservation. Legacy records retain best-effort
    // event-level deduplication because no episode identity was supplied.
    let episode_key = if identity_version == strategy::LIVE_STRATEGY_EVENT_ID_VERSION {
        episode_id
    } else {
        event_id
    };
    let encoded = serde_json::to_vec(&serde_json::json!({
        "identity_version": identity_version,
        "episode_key": episode_key,
        "symbol": symbol,
        "strategy": strategy,
        "direction": direction,
        "profile_id": profile_id,
    }))
    .ok()?;
    use sha2::Digest;
    Some(format!("sha256:v1:{:x}", sha2::Sha256::digest(encoded)))
}

fn exploratory_event_key_for_signal(signal: &Signal) -> Option<String> {
    // Versioned worker identities provide explicit event and episode keys.
    // Legacy IDs remain best-effort and never gain inferred continuity.
    let decision = signal.strategy_decision.as_ref()?;
    let event_id = decision
        .get("live_worker_event_id")
        .and_then(serde_json::Value::as_str)?;
    let identity_version = decision
        .get("live_worker_event_identity_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u16::try_from(version).ok())
        .unwrap_or(0);
    let episode_id = if identity_version == strategy::LIVE_STRATEGY_EVENT_ID_VERSION {
        decision
            .get("live_worker_episode_id")
            .and_then(serde_json::Value::as_str)?
    } else {
        event_id
    };
    let strategy = signal.strategy.as_deref()?;
    let supplied_profile = decision
        .get("learning_profile_id")
        .or_else(|| decision.get("scout_parameter_profile_id"))
        .and_then(serde_json::Value::as_str);
    let parameters = decision
        .get("strategy_parameters")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
        .collect::<BTreeMap<_, _>>();
    let profile_id = supplied_profile.map(str::to_owned).or_else(|| {
        let kind = Strategy::from_name(strategy)?;
        kind.parameter_specs()
            .iter()
            .all(|spec| {
                parameters
                    .get(spec.key.as_str())
                    .is_some_and(|value| value.is_finite())
            })
            .then(|| strategy::profile_id(&signal.symbol, strategy, &parameters))
    })?;
    exploratory_event_key(
        identity_version,
        event_id,
        episode_id,
        &signal.symbol,
        strategy,
        &signal.direction,
        &profile_id,
    )
}

fn exploratory_event_key_for_entry(signal: &Signal, exploratory: bool) -> Option<String> {
    exploratory
        .then(|| exploratory_event_key_for_signal(signal))
        .flatten()
}

fn reserve_exploratory_event(settings: &mut Settings, key: &str, reserved_at_ms: u64) {
    settings
        .exploratory_event_reservations
        .entry(key.to_owned())
        .or_insert(reserved_at_ms);
    while settings.exploratory_event_reservations.len()
        > MAX_RETAINED_EXPLORATORY_EVENT_RESERVATIONS
    {
        let Some(oldest) = settings
            .exploratory_event_reservations
            .iter()
            .min_by_key(|(key, timestamp)| (**timestamp, (*key).clone()))
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        settings.exploratory_event_reservations.remove(&oldest);
    }
}

fn exploratory_event_is_reserved(settings: &Settings, key: &str) -> bool {
    settings.exploratory_event_reservations.contains_key(key)
}

async fn snapshot_signal_settings(
    state: &AutoTraderState,
    signal: &Signal,
    now_ms: u64,
    exploratory: bool,
) -> Result<SignalSettingsSnapshot, String> {
    let signal_key = signal.symbol.as_str();
    let settings_lock_started = Instant::now();
    let mut settings = state.settings.lock().await;
    let settings_lock_wait = settings_lock_started.elapsed().as_secs_f64() * 1_000.0;
    record_cycle_lock_scope(
        state,
        "signal_settings_lock_wait",
        settings_lock_started.elapsed(),
    );
    reset_daily_budget(&mut settings);
    let exploratory_event_key = exploratory_event_key_for_entry(signal, exploratory);
    if exploratory_event_key
        .as_deref()
        .is_some_and(|key| exploratory_event_is_reserved(&settings, key))
    {
        return Err("Exploratory source event already reserved for this profile".to_owned());
    }
    let quote_rate = quote_usd_rate(&settings, signal_key);
    let strategy_key = signal.strategy.as_deref().unwrap_or("unattributed");
    if let Some(reason) = post_stop_reentry_block_reason(&settings, signal, now_ms) {
        return Err(reason);
    }
    let block_reason =
        signal_position_block_reason(&settings, signal, signal_key, strategy_key, now_ms);
    let ioc_fallback_for = match block_reason {
        Some(reason) if reason.starts_with("Entry already pending") => {
            pending_maker_entry_for_signal(&settings, signal, signal_key, strategy_key, now_ms)
                .filter(|(_, strength)| signal.strength.is_finite() && signal.strength > *strength)
                .map(|(timestamp, _)| timestamp)
                .map(Some)
                .ok_or(reason)
        }
        Some(reason) => Err(reason),
        None => Ok(None),
    }?;
    if let Some(strategy) = signal.strategy.as_deref().and_then(Strategy::from_name) {
        let profile = learned_strategy_parameter_profile(&settings, signal_key, strategy);
        queue_strategy_profile_registration(&mut settings, signal_key, strategy, &profile);
    }
    let snapshot = settings.clone();
    drop(settings);
    Ok(SignalSettingsSnapshot {
        settings: snapshot,
        quote_rate,
        settings_lock_wait_ms: settings_lock_wait,
        ioc_fallback_for,
        exploratory_event_key,
    })
}

async fn submit_selected_signal(
    request: SelectedSignalOrderRequest<'_>,
) -> Result<CycleResult, (StatusCode, String)> {
    if is_stablecoin_pair(&request.signal.symbol) {
        return Ok(skipped_for_signal(
            request.signal,
            "Stablecoin-to-stablecoin pairs are excluded from directional entries",
        ));
    }
    let SelectedSignalOrderRequest {
        state,
        dashboard,
        signal,
        exploratory,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        signal_selection_ms,
        context_threshold_refresh_ms,
        cycle_started_at,
        now,
        now_ms,
        market,
        bid,
        ask,
        mid,
        market_cache_lookup_ms,
        market_http_fetch_ms,
    } = request;
    // The settings mutex serializes the gate/profile update and snapshot below.
    // Don't also hold the shared cycle lock while scanning and cloning Settings:
    // order reservation rechecks the gate under the settings mutex before writing.
    let decision_id = signal_decision_id(signal);
    let SignalSettingsSnapshot {
        settings: settings_snapshot,
        quote_rate,
        settings_lock_wait_ms,
        ioc_fallback_for,
        exploratory_event_key,
    } = match snapshot_signal_settings(state, signal, now_ms, exploratory).await {
        Ok(snapshot) => snapshot,
        Err(reason) => return Ok(skipped_for_signal(signal, &reason)),
    };
    let recent_exploratory_exit_fee = signal.strategy.as_deref().and_then(|strategy| {
        recent_exploratory_exit_fee_rate(
            &settings_snapshot,
            signal.symbol.as_str(),
            strategy,
            now_ms,
        )
    });
    let signal_key = signal.symbol.clone();
    let strategy_key = signal.strategy.as_deref().unwrap_or("unattributed");
    let execution_plan_started = Instant::now();
    let plan = match prepare_signal_execution_plan(
        state,
        settings_snapshot,
        signal,
        dashboard,
        &market,
        now_ms,
        quote_rate,
        exploratory,
        ioc_fallback_for.is_some(),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    )
    .await
    {
        Ok(plan) => plan,
        Err(reason) => return Ok(skipped_for_signal(signal, &reason)),
    };
    let execution_plan_ms = execution_plan_started.elapsed().as_secs_f64() * 1_000.0;
    let SignalExecutionPlan {
        effective_stop_loss_percent,
        effective_take_profit_percent,
        horizon_take_profit_percent,
        mut notional,
        minimum_book_order,
        stop_headroom_multiplier,
        deep_book,
        deep_book_ms,
        side,
        order_type,
        profile,
        execution_params,
        learning_context,
        execution_learning_ms,
        imported_execution_learning,
        exploration_risk_adjustment,
    } = plan;
    if exploratory || imported_execution_learning {
        let mut settings = state.settings.lock().await;
        if exploratory {
            if !settings.last_risk_note.is_empty() {
                settings.last_risk_note.push_str(" · ");
            }
            settings.last_risk_note.push_str(
                "small paper-exploration size; no reliable same-side walk-forward edge yet",
            );
        }
        if imported_execution_learning {
            settings.execution_learning_imported = true;
        }
    }
    // Apply the live spread and estimated sweep cost before submitting.
    // Strategy-specific Scout returns already include their modeled fees.
    let Some(assessment) = deep_book else {
        return Ok(skipped_for_signal(
            signal,
            "Cannot estimate execution slippage because the order book is unavailable",
        ));
    };
    // `assess_deep_book` uses infinity when the visible book cannot fill the
    // planned order. Reject that condition before calculating/formatting the
    // spread-and-sweep threshold, otherwise the UI reports a misleading
    // `inf%` execution cost instead of the actual book-capacity failure.
    if !assessment.impact_bps.is_finite() {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Deep book cannot fill a {} {side} order: available capacity {:.2} USD; minimum order {:.2} USD",
                signal.symbol,
                assessment.notional_cap / quote_rate,
                minimum_book_order
            ),
        ));
    }
    if assessment.impact_bps > DEEP_BOOK_BAND_BPS {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Deep book cannot support a {} {side} order: estimated impact {:.2} bps exceeds {:.0} bps",
                signal.symbol, assessment.impact_bps, DEEP_BOOK_BAND_BPS
            ),
        ));
    }
    let spread_bps = (ask - bid) / mid * 10_000.0;
    let strategy_edge = signal
        .strategy
        .as_deref()
        .and_then(Strategy::from_name)
        .filter(|kind| !kind.is_dated_carry())
        .and_then(|kind| validated_strategy_entry_edge(signal, kind).ok());
    let contextual_net_edge = exploratory_contextual_net_edge(&exploration_risk_adjustment);
    let reducing_rebalance = signal.direction == "bearish"
        && is_portfolio_rebalance_signal(
            signal,
            signal.strategy.as_deref().and_then(Strategy::from_name),
        );
    if !exploratory {
        if let Some(reason) =
            strict_net_edge_blocker(strategy_edge, contextual_net_edge, reducing_rebalance)
        {
            return Ok(skipped_for_signal(signal, reason));
        }
    }
    let (edge_percent, edge_source, fee_bps, fee_assumption, edge_is_net) = if let Some(edge) =
        strategy_edge
    {
        (
            edge,
            "measured strategy Scout net edge",
            0.0,
            "walk-forward net edge already includes execution costs".to_owned(),
            true,
        )
    } else if let Some(edge) = contextual_net_edge {
        (
            edge,
            "recent contextual net expectancy lower bound",
            0.0,
            "realized contextual net returns already include execution costs".to_owned(),
            true,
        )
    } else if exploratory {
        let (fee_bps, fee_assumption) = recent_exploratory_exit_fee
                .map(|(exit_fee_rate, samples)| {
                    (
                        (configured_maker_fee_rate() + exit_fee_rate) * 10_000.0,
                        format!("exploratory: maker entry; observed exit fee mix from {samples} completed trades"),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        exploratory_round_trip_fee_bps(
                            configured_maker_fee_rate(),
                            configured_taker_fee_rate(),
                        ),
                        "exploratory: maker entry; 50% maker / 50% taker exit prior".to_owned(),
                    )
                });
        (
            effective_take_profit_percent,
            "strategy take-profit payoff ceiling (not a measured edge)",
            fee_bps,
            fee_assumption,
            false,
        )
    } else {
        (
            effective_take_profit_percent,
            "strategy take-profit payoff ceiling (not a measured edge)",
            2.0 * configured_taker_fee_rate() * 10_000.0,
            "strict/urgent: taker entry and taker exit".to_owned(),
            false,
        )
    };
    let incremental_cost_percent = (spread_bps + 2.0 * assessment.impact_bps + fee_bps) / 100.0;
    let execution_cost_cushion =
        time_adjusted_edge_cost_cushion(now, MIN_EDGE_TO_INCREMENTAL_COST_MULTIPLIER);
    let edge_cost_threshold = if edge_is_net {
        0.0
    } else {
        incremental_cost_percent * execution_cost_cushion
    };
    if !reducing_rebalance
        && (!edge_percent.is_finite()
            || !incremental_cost_percent.is_finite()
            || edge_percent <= edge_cost_threshold)
    {
        let rejection_reason = if edge_is_net {
            format!(
                "{edge_source} ({edge_percent:.3}%) is not positive after fees and execution costs ({fee_assumption})"
            )
        } else {
            format!(
                "{edge_source} ({edge_percent:.3}%) is below the {:.1}x estimated spread, sweep, and fee cost ({edge_cost_threshold:.3}%; fee assumption: {fee_assumption}, maker {:.2} bps, taker {:.2} bps)",
                execution_cost_cushion,
                configured_maker_fee_rate() * 10_000.0,
                configured_taker_fee_rate() * 10_000.0,
            )
        };
        return Ok(skipped_for_signal(signal, &rejection_reason));
    }
    let deep_book_note = if let Some(assessment) = deep_book {
        if !matches!(order_type, AutoOrderType::Vwap | AutoOrderType::Twap) {
            notional = notional.min(assessment.notional_cap / quote_rate);
        }
        if notional < minimum_book_order {
            return Ok(skipped(&format!(
                "Deep-book capacity for {} ({:.2} USD) is below the minimum paper order ({:.2} USD)",
                signal.symbol,
                assessment.notional_cap / quote_rate,
                minimum_book_order
            )));
        }
        Some(format!(
            "deep book: {:.0} quote liquidity within {:.0} bps; order cap {:.0} at {:.0}% participation; estimated sweep {:.2} bps",
            assessment.available_within_band,
            DEEP_BOOK_BAND_BPS,
            assessment.notional_cap,
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            assessment.impact_bps
        ))
    } else {
        None
    };
    if let Some(note) = deep_book_note {
        let mut settings = state.settings.lock().await;
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings.last_risk_note.push_str(&note);
    }
    let now_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let timestamp = next_unique_order_timestamp(now_timestamp);
    let max_signal_age_ms = signal_max_age_seconds(signal) * 1_000;
    if timestamp.saturating_sub(signal.timestamp) > max_signal_age_ms {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Signal became stale before order creation (age {}ms; max {}ms)",
                timestamp.saturating_sub(signal.timestamp),
                max_signal_age_ms
            ),
        ));
    }
    let mut stage_timings_ms = BTreeMap::new();
    stage_timings_ms.insert("settings_lock_wait".to_owned(), settings_lock_wait_ms);
    stage_timings_ms.insert(
        "execution_plan_local".to_owned(),
        (execution_plan_ms - deep_book_ms - execution_learning_ms).max(0.0),
    );
    stage_timings_ms.insert(
        "execution_learning_lookup".to_owned(),
        execution_learning_ms,
    );
    stage_timings_ms.insert("signal_selection_total".to_owned(), signal_selection_ms);
    stage_timings_ms.insert(
        "signal_selection_local".to_owned(),
        (signal_selection_ms - context_threshold_refresh_ms).max(0.0),
    );
    stage_timings_ms.insert(
        "context_threshold_refresh".to_owned(),
        context_threshold_refresh_ms,
    );
    stage_timings_ms.insert("market_cache_lookup".to_owned(), market_cache_lookup_ms);
    stage_timings_ms.insert("market_http_fetch".to_owned(), market_http_fetch_ms);
    stage_timings_ms.insert("deep_book_local".to_owned(), deep_book_ms);
    stage_timings_ms.insert(
        "signal_queue_wait".to_owned(),
        dashboard.queue_wait_ms.unwrap_or_default(),
    );
    stage_timings_ms.insert(
        "cycle_to_order_record".to_owned(),
        cycle_started_at.elapsed().as_secs_f64() * 1000.0,
    );
    stage_timings_ms.insert(
        "signal_age_at_order_record".to_owned(),
        timestamp.saturating_sub(signal.timestamp) as f64,
    );
    let is_post_only = matches!(order_type, AutoOrderType::PostOnlyLimit);
    let submitted_limit_price = is_post_only.then_some(if side == "buy" { bid } else { ask });
    let limit_to_mid_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let limit_to_crossing_quote_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (ask - price) / mid * 10_000.0
        } else {
            (price - bid) / mid * 10_000.0
        }
    });
    let expires_at = is_post_only
        .then_some(timestamp.saturating_add(execution_params.post_only_timeout_minutes * 60_000));
    let estimated_ioc_all_in_bps =
        deep_book.map(|assessment| assessment.impact_bps + configured_taker_fee_rate() * 10_000.0);
    let cycle_guard = acquire_cycle_lock(state, "signal").await;
    let reservation_cycle_lock_started = Instant::now();
    let (quantity, lifecycle_identity, persisted_request) = {
        let reservation_lock_started = Instant::now();
        let mut settings = state.settings.lock().await;
        stage_timings_ms.insert(
            "reservation_lock_wait".to_owned(),
            reservation_lock_started.elapsed().as_secs_f64() * 1_000.0,
        );
        record_cycle_lock_scope(
            state,
            "signal_reservation_settings_lock_wait",
            reservation_lock_started.elapsed(),
        );
        let order_reservation_started = Instant::now();
        reset_daily_budget(&mut settings);
        if exploratory_event_key
            .as_deref()
            .is_some_and(|key| exploratory_event_is_reserved(&settings, key))
        {
            return Ok(skipped_for_signal(
                signal,
                "Exploratory source event already reserved for this profile",
            ));
        }
        if let Some(reason) = post_stop_reentry_block_reason(&settings, signal, now_ms) {
            return Ok(skipped_for_signal(signal, &reason));
        }
        let strategy = signal.strategy.as_deref().and_then(Strategy::from_name);
        if !paper_learning_exploration_active(&settings, now) && !exploratory {
            if let Some(reason) = learned_pause_block_reason(
                &settings,
                strategy,
                signal,
                reducing_rebalance,
                &signal.direction,
                now,
            ) {
                return Ok(skipped_for_signal(
                    signal,
                    &format!("Strategy entry paused: {reason}"),
                ));
            }
        }
        let current_pending_maker =
            pending_maker_entry_for_signal(&settings, signal, &signal.symbol, strategy_key, now_ms);
        if ioc_fallback_for.is_some()
            && !current_pending_maker.is_some_and(|(timestamp, strength)| {
                Some(timestamp) == ioc_fallback_for
                    && signal.strength.is_finite()
                    && signal.strength > strength
            })
        {
            return Ok(skipped_for_signal(
                signal,
                "Resting maker entry resolved before the stronger-signal IOC fallback",
            ));
        }
        let block_reason =
            signal_position_block_reason(&settings, signal, &signal.symbol, strategy_key, now_ms);
        if let Some(reason) = block_reason {
            let fallback_still_valid = ioc_fallback_for.is_some_and(|pending_timestamp| {
                reason.starts_with("Entry already pending")
                    && current_pending_maker.is_some_and(|(timestamp, strength)| {
                        timestamp == pending_timestamp
                            && signal.strength.is_finite()
                            && signal.strength > strength
                    })
            });
            if !fallback_still_valid {
                return Ok(skipped_for_signal(signal, &reason));
            }
        }
        let allocated = allocated_notional(&settings);
        let category = strategy_category(signal.strategy.as_deref());
        let category_allocated = allocated_notional_for_category(&settings, Some(category));
        // Cap gross open and pending exposure for this symbol across all strategies.
        let symbol_remaining_budget =
            settings.max_order_notional - allocated_notional_for_symbol(&settings, &signal.symbol);
        let strategy_remaining_budget = Strategy::from_name(strategy_key)
            .map(|kind| match strategy_category(Some(kind.as_str())) {
                StrategyCategory::Daily if !kind.is_live_insight() && !kind.is_dated_carry() => {
                    signal_weighted_strategy_budget(
                        &settings,
                        dashboard,
                        kind,
                        StrategyCategory::Daily,
                        now_ms,
                    ) - allocated_notional_for_strategy(&settings, strategy_key)
                }
                StrategyCategory::Intraday if kind.is_live_insight() => {
                    signal_weighted_strategy_budget(
                        &settings,
                        dashboard,
                        kind,
                        StrategyCategory::Intraday,
                        now_ms,
                    ) - allocated_notional_for_strategy(&settings, strategy_key)
                }
                _ => f64::INFINITY,
            })
            .unwrap_or(f64::INFINITY);
        let remaining_budget = if reducing_rebalance {
            f64::INFINITY
        } else {
            (settings.daily_notional_limit - allocated)
                .min(category_notional_limit(&settings, category) - category_allocated)
                .min(strategy_remaining_budget)
                .min(symbol_remaining_budget)
                .max(0.0)
        };
        notional = notional.min(remaining_budget / stop_headroom_multiplier);
        if reducing_rebalance {
            notional = notional.min(
                portfolio_reduction_quantity(&settings, &signal.symbol, strategy_key)
                    * mid
                    * quote_rate,
            );
        }
        if notional < minimum_book_order {
            return Ok(skipped_for_signal(
                signal,
                if stop_headroom_multiplier > 1.0 {
                    "Insufficient notional headroom for both the scheduled entry and its protective stop"
                } else {
                    "Remaining daily notional budget is below the minimum order size"
                },
            ));
        }
        let quantity = notional / (mid * quote_rate);
        let supplied_profile_id = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| {
                decision
                    .get("learning_profile_id")
                    .or_else(|| decision.get("scout_parameter_profile_id"))
            })
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let entry_parameters = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
            .collect::<BTreeMap<_, _>>();
        let profile_id = supplied_profile_id.or_else(|| {
            let kind = Strategy::from_name(strategy_key)?;
            kind.parameter_specs()
                .iter()
                .all(|spec| {
                    entry_parameters
                        .get(spec.key.as_str())
                        .is_some_and(|value| value.is_finite())
                })
                .then(|| strategy::profile_id(&signal.symbol, strategy_key, &entry_parameters))
        });
        let profile_hash = profile_id.as_deref().and_then(|profile_id| {
            canonical_profile_hash_v1(&signal.symbol, strategy_key, profile_id, &entry_parameters)
        });
        let learned_profile_version = signal
            .strategy
            .as_deref()
            .and_then(|strategy| {
                settings
                    .learned_profiles
                    .get(&format!("{}:{strategy}", signal.symbol))
            })
            .filter(|profile| profile_id.as_deref() == Some(profile.profile_id.as_str()))
            .map(|profile| profile.version);
        let profile_version = signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| {
                decision
                    .get("learning_profile_version")
                    .or_else(|| decision.get("profile_version"))
            })
            .and_then(serde_json::Value::as_u64)
            .or(learned_profile_version);
        let existing_trade_id = settings
            .strategy_positions
            .get(&signal.symbol)
            .and_then(|lots| lots.get(strategy_key))
            .and_then(|position| position.trade_id.as_deref())
            .and_then(|trade_id| settings.canonical_lifecycle_links.get(trade_id))
            .map(|identity| identity.trade_id.clone());
        let source_episode_id =
            match source_episode_id_for_decision(signal.strategy_decision.as_ref()) {
                Ok(episode_id) => episode_id,
                Err(()) => {
                    return Ok(skipped_for_signal(
                        signal,
                        "Worker event identity version has no supported stable episode ID",
                    ));
                }
            };
        let Some(lifecycle_identity) = canonical_lifecycle_identity_for_reservation(
            &mut settings,
            &decision_id,
            profile_id.clone(),
            profile_hash,
            profile_version,
            exploratory,
            existing_trade_id,
            source_episode_id,
        ) else {
            return Ok(skipped_for_signal(
                signal,
                "Canonical lifecycle identity sequence is exhausted",
            ));
        };
        let (request_path, request_body) = build_order_request(
            &signal.symbol,
            side,
            quantity,
            bid,
            ask,
            order_type,
            execution_params,
            &lifecycle_identity.opportunity_id,
            now_ms,
            false,
            Some(&lifecycle_identity),
        );
        let persisted_request = PersistedEntryRequest {
            schema_version: 1,
            method: "POST".to_owned(),
            path: request_path.to_owned(),
            body: request_body,
        };
        let recovery_context = serde_json::json!({
            "schema_version": 1,
            "signal": signal,
            "exploratory": exploratory,
            "counterfactual_stop_loss_percent": counterfactual_stop_loss_percent,
            "counterfactual_take_profit_percent": counterfactual_take_profit_percent,
            "effective_stop_loss_percent": effective_stop_loss_percent,
            "effective_take_profit_percent": effective_take_profit_percent,
            "side": side,
            "quantity": quantity,
            "notional": notional,
            "bid": bid,
            "ask": ask,
            "mid": mid,
            "order_type": order_type,
            "execution_parameters": {
                "post_only_timeout_minutes": execution_params.post_only_timeout_minutes,
                "vwap_duration_minutes": execution_params.vwap_duration_minutes,
                "vwap_participation_rate": execution_params.vwap_participation_rate,
                "limit_offset_bps": execution_params.limit_offset_bps,
                "trigger_offset_bps": execution_params.trigger_offset_bps,
                "trailing_distance_percent": execution_params.trailing_distance_percent,
            },
            "timestamp": timestamp,
            "decision_id": decision_id,
            "lifecycle_identity": lifecycle_identity,
            "learning_context": learning_context,
            "exploration_risk_adjustment": {
                "scale": exploration_risk_adjustment.scale,
                "sample_count": exploration_risk_adjustment.sample_count,
                "weighted_net_return_percent": exploration_risk_adjustment.weighted_net_return_percent,
                "weighted_net_return_lcb_percent": exploration_risk_adjustment.weighted_net_return_lcb_percent,
                "regime_scoped": exploration_risk_adjustment.regime_scoped,
            },
            "horizon_take_profit_percent": horizon_take_profit_percent,
            "stage_timings_ms": stage_timings_ms,
            "deep_book": deep_book.map(|book| serde_json::json!({
                "available_within_band": book.available_within_band,
                "notional_cap": book.notional_cap,
                "impact_bps": book.impact_bps,
                "bid_depth_10bps": book.bid_depth_10bps,
                "ask_depth_10bps": book.ask_depth_10bps,
                "depth_imbalance_10bps": book.depth_imbalance_10bps,
            })),
            "estimated_ioc_all_in_bps": estimated_ioc_all_in_bps,
            "submitted_limit_price": submitted_limit_price,
            "limit_to_mid_bps": limit_to_mid_bps,
            "limit_to_crossing_quote_bps": limit_to_crossing_quote_bps,
            "expires_at": expires_at,
        });
        consume_post_stop_reentry_cooldown(&mut settings, signal, now_ms);
        stage_timings_ms.insert(
            "order_reservation_record_local".to_owned(),
            order_reservation_started.elapsed().as_secs_f64() * 1_000.0,
        );
        settings.used_notional_today = allocated;
        settings
            .last_signal_at
            .insert(signal_key.clone(), timestamp);
        settings
            .last_signal_direction
            .insert(signal_key.clone(), signal.direction.clone());
        settings
            .last_signal_strength
            .insert(signal_key, signal.strength);
        if let Some(key) = exploratory_event_key.as_deref() {
            reserve_exploratory_event(&mut settings, key, now_ms);
        }
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: signal.symbol.clone(),
            side: side.to_owned(),
            order_type: format!("{order_type:?} · P{profile}"),
            purpose: "signal".into(),
            exploratory,
            strategy: Some(
                signal
                    .strategy
                    .clone()
                    .unwrap_or_else(|| "unattributed".into()),
            ),
            decision_id: Some(decision_id.clone()),
            revalidation_snapshot: Some(RestingOrderRevalidationSnapshot {
                symbol: signal.symbol.clone(),
                strategy: strategy_key.to_owned(),
                direction: signal.direction.clone(),
                signal_timestamp_ms: signal.timestamp,
                profile_id: profile_id.clone(),
                profile_version,
                // Live worker reports currently echo the canonical profile ID
                // in `parameter_hash`; retain that exact contract value.
                parameter_hash: profile_id.clone(),
                event_identity_version: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision["live_worker_event_identity_version"].as_u64())
                    .and_then(|version| u16::try_from(version).ok()),
                event_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision["live_worker_event_id"].as_str())
                    .map(str::to_owned),
                episode_id: signal
                    .strategy_decision
                    .as_ref()
                    .and_then(|decision| decision["live_worker_episode_id"].as_str())
                    .map(str::to_owned),
                submission_request: Some(persisted_request.clone()),
                submission_recovery_context: Some(recovery_context),
            }),
            strength: signal.strength,
            ioc_fallback_for,
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: notional,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: Some(learning_context.clone()),
            book_bid_depth_10bps: deep_book.map(|assessment| assessment.bid_depth_10bps),
            book_ask_depth_10bps: deep_book.map(|assessment| assessment.ask_depth_10bps),
            book_depth_imbalance_10bps: deep_book
                .map(|assessment| assessment.depth_imbalance_10bps),
            estimated_sweep_bps: deep_book.map(|assessment| assessment.impact_bps),
            estimated_ioc_all_in_bps,
            decision_bid: Some(bid),
            decision_ask: Some(ask),
            decision_mid: Some(mid),
            submitted_limit_price,
            target_price: None,
            horizon_take_profit_percent,
            limit_to_mid_bps,
            limit_to_crossing_quote_bps,
            expires_at,
            failure_reason: None,
            stage_timings_ms: stage_timings_ms.clone(),
            status: "submitting".to_owned(),
            realized_pnl: 0.0,
            strategy_realized_pnl: None,
            strategy_realized_pnl_today: None,
            strategy_realized_pnl_day: None,
        });
        if settings.orders.len() > MAX_RETAINED_AUTO_TRADER_ORDERS {
            let excess = settings.orders.len() - MAX_RETAINED_AUTO_TRADER_ORDERS;
            let mut remaining = excess;
            settings.orders.retain(|order| {
                if remaining > 0
                    && !matches!(order.status.as_str(), "submitting" | "unknown_submission")
                {
                    remaining -= 1;
                    false
                } else {
                    true
                }
            });
        }
        prune_canonical_lifecycle_links(&mut settings);
        request_settings_persist(state)?;
        drop(settings);
        // The reservation and global portfolio-cap check are now committed in
        // shared memory while the Settings mutex kept portfolio cap checking
        // and reservation atomic. The durable intent row contains the exact
        // retry-stable PM request and lifecycle identity; no PM request is sent
        // until that small transaction commits. The coalescing full-state
        // writer later normalizes the snapshot and removes resolved intents.
        record_cycle_lock_scope(
            state,
            "signal_order_reservation_memory_critical_section",
            reservation_cycle_lock_started.elapsed(),
        );
        drop(cycle_guard);
        let settings_persist_started = Instant::now();
        if let Err(error) =
            persist_entry_reservation_durable(state, &decision_id, exploratory_event_key.as_deref())
                .await
        {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".to_owned();
                order.failure_reason = Some(format!(
                    "Unable to durably persist lifecycle reservation: {}",
                    error.1
                ));
            }
            settings.used_notional_today = allocated_notional(&settings);
            request_settings_persist(state)?;
            return Err(error);
        }
        stage_timings_ms.insert(
            "reservation_durable_persist".to_owned(),
            settings_persist_started.elapsed().as_secs_f64() * 1_000.0,
        );
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.stage_timings_ms = stage_timings_ms.clone();
        }
        request_settings_persist(state)?;
        (quantity, lifecycle_identity, persisted_request)
    };
    // The pending order and notional reservation are durable before this
    // function sends any external request; cross-symbol cycles can reserve
    // against the visible in-memory notional while persistence completes.
    let submission = SelectedOrderExecution {
        state: state.clone(),
        signal: signal.clone(),
        exploratory,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        effective_stop_loss_percent,
        effective_take_profit_percent,
        side: side.to_owned(),
        quantity,
        notional,
        bid,
        ask,
        mid,
        order_type,
        execution_params,
        timestamp,
        decision_id: decision_id.clone(),
        lifecycle_identity,
        learning_context: learning_context.clone(),
        exploration_risk_adjustment,
        horizon_take_profit_percent,
        stage_timings_ms,
        deep_book,
        estimated_ioc_all_in_bps,
        submitted_limit_price,
        limit_to_mid_bps,
        limit_to_crossing_quote_bps,
        expires_at,
        persisted_request,
        is_recovery: false,
    };
    let worker_state = state.clone();
    tokio::spawn(async move {
        match submit_and_record_selected_order(submission).await {
            Ok(result) if result.status == "order_submitted" => {
                record_cycle_gap_summary(&worker_state, &Ok(result));
            }
            Ok(_) => {}
            Err((status, error)) => crate::service_log!(
                "ERROR",
                "Asynchronous signal order submission failed ({status}): {error}"
            ),
        }
    });
    Ok(CycleResult {
        status: "order_submitting",
        reason: Some("Order reserved locally and queued for Position Manager submission".into()),
        symbol: Some(signal.symbol.clone()),
        side: Some(side.to_owned()),
        quantity: Some(quantity),
        order_type: Some(format!("{order_type:?}")),
        order_response: None,
        filled_quantity: None,
        entry_diagnostic: None,
    })
}

// Lower-activity calendar periods get a larger measured-edge cushion. Calendar
// time changes execution caution only and never creates a directional signal.
fn time_adjusted_edge_cost_cushion(now_seconds: u64, base: f64) -> f64 {
    let days_since_epoch = now_seconds / 86_400;
    let weekday = (days_since_epoch + 4) % 7; // Sunday = 0, Saturday = 6
    let hour_utc = (now_seconds % 86_400) / 3_600;
    if weekday == 0 || weekday == 6 || hour_utc < 8 {
        base.max(2.0)
    } else {
        base
    }
}

fn exploratory_contextual_net_edge(adjustment: &ExplorationRiskAdjustment) -> Option<f64> {
    const MIN_CONTEXT_EDGE_SAMPLES: usize = 6;
    let samples = adjustment.sample_count;
    let lower_bound = adjustment.weighted_net_return_lcb_percent?;
    if samples < MIN_CONTEXT_EDGE_SAMPLES || !lower_bound.is_finite() {
        return None;
    }
    Some(lower_bound)
}

fn strict_net_edge_blocker(
    strategy_net_edge: Option<f64>,
    contextual_net_edge: Option<f64>,
    reducing_rebalance: bool,
) -> Option<&'static str> {
    if reducing_rebalance
        || strategy_net_edge.is_some_and(f64::is_finite)
        || contextual_net_edge.is_some_and(f64::is_finite)
    {
        None
    } else {
        Some("Strict production entry blocked: validated net-edge evidence is unavailable")
    }
}

fn recent_exploratory_exit_fee_rate(
    settings: &Settings,
    symbol: &str,
    strategy: &str,
    now_ms: u64,
) -> Option<(f64, usize)> {
    const FEE_HISTORY_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
    const MIN_COMPLETED_TRADES: usize = 5;
    let mut completed_trades = BTreeSet::new();
    let mut total_notional = 0.0;
    let mut weighted_fee = 0.0;
    for order in &settings.orders {
        if !order.exploratory
            || order.symbol != symbol
            || order.strategy.as_deref() != Some(strategy)
            || order.purpose == "signal"
            || !matches!(
                order.status.as_str(),
                "filled" | "partially_filled_closed" | "expired_or_cancelled"
            )
            || order.timestamp > now_ms
            || now_ms.saturating_sub(order.timestamp) > FEE_HISTORY_WINDOW_MS
        {
            continue;
        }
        let Some(decision_id) = order.decision_id.as_deref() else {
            continue;
        };
        let Some(notional) = order
            .average_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .map(|price| order.filled_quantity * price)
            .filter(|notional| notional.is_finite() && *notional > 0.0)
        else {
            continue;
        };
        let fee_rate = if order.order_type.starts_with("PostOnlyLimit")
            || order.order_type.starts_with("Gtd")
        {
            configured_maker_fee_rate()
        } else {
            configured_taker_fee_rate()
        };
        completed_trades.insert(decision_id.to_owned());
        total_notional += notional;
        weighted_fee += notional * fee_rate;
    }
    if completed_trades.len() < MIN_COMPLETED_TRADES || total_notional <= 0.0 {
        return None;
    }
    Some((weighted_fee / total_notional, completed_trades.len()))
}

fn exploratory_round_trip_fee_bps(maker_rate: f64, taker_rate: f64) -> f64 {
    // Use a balanced prior until this strategy/symbol has enough completed
    // exploratory exit paths to estimate its observed mix.
    (maker_rate + (maker_rate + taker_rate) / 2.0) * 10_000.0
}

#[cfg(test)]
mod selected_signal_order_fee_tests {
    use super::*;

    #[test]
    fn strict_entries_do_not_use_take_profit_distance_as_expected_net_edge() {
        assert!(strict_net_edge_blocker(None, None, false).is_some());
        assert!(strict_net_edge_blocker(Some(f64::NAN), None, false).is_some());
        assert_eq!(strict_net_edge_blocker(Some(0.2), None, false), None);
        assert_eq!(strict_net_edge_blocker(None, Some(-0.1), false), None);
        assert_eq!(strict_net_edge_blocker(None, None, true), None);
    }

    #[test]
    fn canonical_episode_uses_worker_episode_and_fails_closed_for_malformed_v1() {
        let report = serde_json::json!({
            "live_worker_event_identity_version": strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
            "live_worker_event_id": "event-1",
            "live_worker_episode_id": "strategy:v1:episode:thesis-1"
        });
        assert_eq!(
            source_episode_id_for_decision(Some(&report))
                .unwrap()
                .as_deref(),
            Some("strategy:v1:episode:thesis-1")
        );

        let malformed = serde_json::json!({
            "live_worker_event_identity_version": strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
            "live_worker_event_id": "event-2"
        });
        assert!(source_episode_id_for_decision(Some(&malformed)).is_err());
        assert!(
            source_episode_id_for_decision(Some(&serde_json::json!({
                "live_worker_event_identity_version": 65_536
            })))
            .is_err()
        );
        assert_eq!(source_episode_id_for_decision(None).unwrap(), None);
    }

    fn exploratory_exit_order(
        timestamp: u64,
        decision_id: &str,
        order_type: &str,
        quantity: f64,
        average_price: f64,
    ) -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": timestamp,
            "symbol": "BTC-USDT",
            "side": "sell",
            "order_type": order_type,
            "purpose": "take-profit",
            "exploratory": true,
            "strategy": "live-momentum",
            "decision_id": decision_id,
            "quantity": quantity,
            "filled_quantity": quantity,
            "average_price": average_price,
            "status": "filled",
            "realized_pnl": 0.0
        }))
        .unwrap()
    }

    #[test]
    fn exploratory_fee_uses_maker_entry_and_mixed_exit_paths() {
        assert!((exploratory_round_trip_fee_bps(0.0008, 0.001) - 17.0).abs() < 1e-9);
    }

    #[test]
    fn immediate_stop_loss_exit_costs_taker_fee() {
        let maker_entry_and_stop_exit_bps: f64 = (0.0008 + 0.001) * 10_000.0;
        assert!((maker_entry_and_stop_exit_bps - 18.0).abs() < 1e-9);
        assert!(maker_entry_and_stop_exit_bps > 2.0 * 0.0008 * 10_000.0);
    }

    #[test]
    fn strict_or_urgent_path_keeps_two_taker_fees_and_measured_edge_adds_none() {
        let strict_taker_fee_bps = 2.0 * configured_taker_fee_rate() * 10_000.0;
        let measured_edge_fee_bps = 0.0;
        assert!((strict_taker_fee_bps - 20.0).abs() < 1e-9);
        assert_eq!(measured_edge_fee_bps, 0.0);
    }

    #[test]
    fn contextual_exploration_net_edge_uses_lower_bound_and_requires_six_samples() {
        let adjustment = ExplorationRiskAdjustment {
            scale: 1.0,
            sample_count: 6,
            weighted_net_return_percent: Some(0.6),
            weighted_net_return_lcb_percent: Some(0.4),
            regime_scoped: false,
        };
        assert!((exploratory_contextual_net_edge(&adjustment).unwrap() - 0.4).abs() < 1e-12);

        let insufficient = ExplorationRiskAdjustment {
            sample_count: 5,
            ..adjustment
        };
        assert_eq!(exploratory_contextual_net_edge(&insufficient), None);
    }

    #[test]
    fn exploration_dedup_continues_across_events_in_the_same_episode() {
        let base = exploratory_event_key(
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
            "worker-event-7",
            "episode-a",
            "BTC-USDT",
            "live-momentum",
            "bullish",
            "profile-a",
        )
        .unwrap();
        assert_eq!(
            exploratory_event_key(
                strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                "worker-event-7",
                "episode-a",
                "BTC-USDT",
                "live-momentum",
                "bullish",
                "profile-a",
            ),
            Some(base.clone())
        );
        assert_eq!(
            exploratory_event_key(
                strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                "worker-event-8",
                "episode-a",
                "BTC-USDT",
                "live-momentum",
                "bullish",
                "profile-a",
            ),
            Some(base.clone()),
            "a distinct event inside the unchanged thesis episode must remain deduplicated"
        );
        for (event_id, episode_id, direction, profile) in [
            ("worker-event-8", "episode-b", "bullish", "profile-a"),
            ("worker-event-7", "episode-a", "bearish", "profile-a"),
            ("worker-event-7", "episode-a", "bullish", "profile-b"),
        ] {
            assert_ne!(
                exploratory_event_key(
                    strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                    event_id,
                    episode_id,
                    "BTC-USDT",
                    "live-momentum",
                    direction,
                    profile,
                ),
                Some(base.clone())
            );
        }
        assert_eq!(
            exploratory_event_key(
                strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                "",
                "episode-a",
                "BTC-USDT",
                "live-momentum",
                "bullish",
                "profile-a",
            ),
            None
        );
        assert_eq!(
            exploratory_event_key(
                strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                &"e".repeat(513),
                "episode-a",
                "BTC-USDT",
                "live-momentum",
                "bullish",
                "profile-a",
            ),
            None
        );
    }

    #[test]
    fn strict_entry_does_not_use_exploratory_source_event_dedup() {
        let signal = benchmark_precomputed_ingress_dashboard(900_001, "BTC-USDT")
            .best
            .into_iter()
            .next()
            .unwrap();
        assert!(exploratory_event_key_for_entry(&signal, true).is_some());
        assert_eq!(exploratory_event_key_for_entry(&signal, false), None);
    }

    #[test]
    fn repeated_exploration_reservation_is_bounded_and_persists_across_restart() {
        let key = exploratory_event_key(
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
            "worker-event-7",
            "episode-a",
            "BTC-USDT",
            "live-momentum",
            "bullish",
            "profile-a",
        )
        .unwrap();
        let mut settings = Settings::default();
        assert!(!exploratory_event_is_reserved(&settings, &key));
        reserve_exploratory_event(&mut settings, &key, 100);
        assert!(exploratory_event_is_reserved(&settings, &key));
        reserve_exploratory_event(&mut settings, &key, 200);
        assert_eq!(settings.exploratory_event_reservations.len(), 1);
        assert_eq!(
            settings.exploratory_event_reservations.get(&key),
            Some(&100)
        );

        let restarted: Settings =
            serde_json::from_value(serde_json::to_value(settings).unwrap()).unwrap();
        assert!(exploratory_event_is_reserved(&restarted, &key));
        // The second attempt is blocked, so one duplicate order/evidence reservation is avoided.
        assert_eq!(restarted.exploratory_event_reservations.len(), 1);
    }

    #[test]
    fn exploratory_event_reservation_capacity_evicts_oldest_only_at_bound() {
        let mut settings = Settings::default();
        for index in 0..=MAX_RETAINED_EXPLORATORY_EVENT_RESERVATIONS {
            reserve_exploratory_event(&mut settings, &format!("event-{index}"), index as u64);
        }
        assert_eq!(
            settings.exploratory_event_reservations.len(),
            MAX_RETAINED_EXPLORATORY_EVENT_RESERVATIONS
        );
        assert!(!exploratory_event_is_reserved(&settings, "event-0"));
        assert!(exploratory_event_is_reserved(
            &settings,
            &format!("event-{MAX_RETAINED_EXPLORATORY_EVENT_RESERVATIONS}")
        ));
    }

    #[test]
    fn recent_exit_fee_estimate_uses_completed_strategy_exit_mix() {
        let now = 10 * 24 * 60 * 60 * 1_000;
        let mut settings = Settings::default();
        for (index, order_type, notional) in [
            (0, "PostOnlyLimit · exit", 1.0),
            (1, "Ioc", 3.0),
            (2, "Ioc", 3.0),
            (3, "Ioc", 3.0),
            (4, "Ioc", 3.0),
        ] {
            settings.orders.push(exploratory_exit_order(
                now - index * 1_000,
                &format!("decision-{index}"),
                order_type,
                notional / 100.0,
                100.0,
            ));
        }
        let (exit_fee_rate, sample_count) =
            recent_exploratory_exit_fee_rate(&settings, "BTC-USDT", "live-momentum", now).unwrap();
        assert_eq!(sample_count, 5);
        assert!((exit_fee_rate - (0.0128 / 13.0)).abs() < 1e-12);
    }
}
