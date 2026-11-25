
const LOSS_ENTRY_FLOOR_LIFT_MS: u64 = 30 * 60 * 1_000;
const LOSS_ENTRY_FLOOR_MULTIPLIER: f64 = 1.5;

fn entry_floor_lift_key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}|{strategy}")
}

const MAX_POST_STOP_REENTRY_COOLDOWNS: usize = 10_000;

fn post_stop_reentry_key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}|{strategy}")
}

fn signal_timestamp_ms(signal: &Signal) -> u64 {
    if signal.timestamp < 1_000_000_000_000 {
        signal.timestamp.saturating_mul(1_000)
    } else {
        signal.timestamp
    }
}

fn post_stop_cooldown_duration_ms(
    strategy_name: &str,
    strategy_decision: Option<&serde_json::Value>,
) -> Option<u64> {
    let strategy = Strategy::from_name(strategy_name)?;
    let spec = strategy
        .parameter_specs()
        .into_iter()
        .find(|spec| spec.key == StrategyParameterKey::PostStopCooldownMinutes)?;
    let supplied = strategy_decision
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(|parameters| parameters.get(spec.key.as_str()))
        .and_then(serde_json::Value::as_f64)
        .filter(|value| {
            if !value.is_finite() || *value < spec.min || *value > spec.max || spec.step <= 0.0 {
                return false;
            }
            let steps = (*value - spec.min) / spec.step;
            (steps - steps.round()).abs() <= 1e-6
        })
        .unwrap_or(spec.default);
    Some((supplied * 60_000.0).round() as u64)
}

fn record_post_stop_reentry_cooldown(
    settings: &mut Settings,
    symbol: &str,
    strategy_name: &str,
    position: &TraderPosition,
    stopped_at_ms: u64,
) -> bool {
    let Some(duration_ms) = post_stop_cooldown_duration_ms(
        strategy_name,
        position.strategy_decision.as_ref(),
    ) else {
        return false;
    };
    let decision = position.strategy_decision.as_ref();
    let lifecycle_profile_id = position
        .entry_decision_id
        .as_deref()
        .and_then(|decision_id| settings.canonical_lifecycle_links.get(decision_id))
        .and_then(|identity| identity.profile_id.clone());
    let profile_id = lifecycle_profile_id.or_else(|| {
        decision
            .and_then(|value| {
                value
                    .get("learning_profile_id")
                    .or_else(|| value.get("scout_parameter_profile_id"))
            })
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    });
    let event_identity_version = decision
        .and_then(|value| value.get("live_worker_event_identity_version"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u16::try_from(version).ok())
        .unwrap_or(0);
    let stopped_event_id = decision
        .and_then(|value| value.get("live_worker_event_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let stopped_episode_id = decision
        .and_then(|value| value.get("live_worker_episode_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let key = post_stop_reentry_key(symbol, strategy_name);
    settings.post_stop_reentry_cooldowns.insert(
        key,
        PostStopReentryCooldown {
            stopped_at_ms,
            cooldown_until_ms: stopped_at_ms.saturating_add(duration_ms),
            profile_id,
            event_identity_version,
            stopped_event_id,
            stopped_episode_id,
        },
    );
    while settings.post_stop_reentry_cooldowns.len() > MAX_POST_STOP_REENTRY_COOLDOWNS {
        let Some(oldest) = settings
            .post_stop_reentry_cooldowns
            .iter()
            .min_by_key(|(key, value)| (value.stopped_at_ms, (*key).clone()))
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        settings.post_stop_reentry_cooldowns.remove(&oldest);
    }
    true
}

fn post_stop_reentry_block_reason(
    settings: &Settings,
    signal: &Signal,
    now_ms: u64,
) -> Option<String> {
    let strategy_name = signal.strategy.as_deref()?;
    let cooldown = settings
        .post_stop_reentry_cooldowns
        .get(&post_stop_reentry_key(&signal.symbol, strategy_name))?;
    if now_ms < cooldown.cooldown_until_ms {
        return Some(format!(
            "Post-stop re-entry cooldown active for {} / {} / {}",
            signal.symbol,
            strategy_name,
            cooldown.profile_id.as_deref().unwrap_or("unknown profile")
        ));
    }
    let decision = signal.strategy_decision.as_ref();
    let identity_version = decision
        .and_then(|value| value.get("live_worker_event_identity_version"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u16::try_from(version).ok());
    let event_id = decision
        .and_then(|value| value.get("live_worker_event_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    let episode_id = decision
        .and_then(|value| value.get("live_worker_episode_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    let fresh_event = identity_version == Some(strategy::LIVE_STRATEGY_EVENT_ID_VERSION)
        && (cooldown.event_identity_version == 0
            || identity_version == Some(cooldown.event_identity_version))
        && signal_timestamp_ms(signal) > cooldown.stopped_at_ms
        && event_id.is_some_and(|event_id| cooldown.stopped_event_id.as_deref() != Some(event_id))
        && episode_id
            .is_some_and(|episode_id| cooldown.stopped_episode_id.as_deref() != Some(episode_id));
    (!fresh_event).then(|| {
        format!(
            "Post-stop cooldown elapsed; a fresh versioned worker event is required for {} / {} / {}",
            signal.symbol,
            strategy_name,
            cooldown.profile_id.as_deref().unwrap_or("unknown profile")
        )
    })
}

fn consume_post_stop_reentry_cooldown(settings: &mut Settings, signal: &Signal, now_ms: u64) {
    if post_stop_reentry_block_reason(settings, signal, now_ms).is_none() {
        if let Some(strategy_name) = signal.strategy.as_deref() {
            settings
                .post_stop_reentry_cooldowns
                .remove(&post_stop_reentry_key(&signal.symbol, strategy_name));
        }
    }
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

#[cfg(test)]
mod post_stop_reentry_cooldown_tests {
    use super::*;

    fn cooldown_signal() -> Signal {
        let mut signal = benchmark_precomputed_ingress_dashboard(900_021, "BTC-USDT")
            .best
            .into_iter()
            .next()
            .unwrap();
        signal.timestamp = 1_000_000_001_500;
        signal
    }

    fn insert_cooldown(settings: &mut Settings) {
        settings.post_stop_reentry_cooldowns.insert(
            post_stop_reentry_key("BTC-USDT", "live-momentum"),
            PostStopReentryCooldown {
                stopped_at_ms: 1_000_000_000_000,
                cooldown_until_ms: 1_000_000_002_000,
                profile_id: Some("profile-a".into()),
                event_identity_version: strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                stopped_event_id: Some("stop-event".into()),
                stopped_episode_id: Some("stop-episode".into()),
            },
        );
    }

    fn set_signal_identity(signal: &mut Signal, event_id: &str, episode_id: &str, version: u16) {
        let decision = signal.strategy_decision.get_or_insert_with(|| serde_json::json!({}));
        decision["live_worker_event_id"] = serde_json::json!(event_id);
        decision["live_worker_episode_id"] = serde_json::json!(episode_id);
        decision["live_worker_event_identity_version"] = serde_json::json!(version);
    }

    #[test]
    fn stop_fill_cooldown_uses_bounded_profile_parameter_and_default() {
        let strategy = Strategy::LiveMomentum;
        let mut position = TraderPosition::default();
        position.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": {
                "post_stop_cooldown_minutes": 4.0
            }
        }));
        assert_eq!(
            post_stop_cooldown_duration_ms(strategy.as_str(), position.strategy_decision.as_ref()),
            Some(4 * 60_000)
        );
        position.strategy_decision.as_mut().unwrap()["strategy_parameters"]["post_stop_cooldown_minutes"] =
            serde_json::json!(20.0);
        assert_eq!(
            post_stop_cooldown_duration_ms(strategy.as_str(), position.strategy_decision.as_ref()),
            Some(2 * 60_000)
        );
        assert_eq!(post_stop_cooldown_duration_ms("unknown", None), None);
    }

    #[test]
    fn post_stop_cooldown_requires_timer_expiry_and_a_new_versioned_event_and_episode() {
        let mut settings = Settings::default();
        insert_cooldown(&mut settings);
        let mut signal = cooldown_signal();
        set_signal_identity(
            &mut signal,
            "stop-event",
            "stop-episode",
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
        );
        assert!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_001_999).is_some());
        assert!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_002_000).is_some());

        set_signal_identity(
            &mut signal,
            "fresh-event",
            "stop-episode",
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
        );
        assert!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_002_000).is_some());
        set_signal_identity(
            &mut signal,
            "stop-event",
            "fresh-episode",
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
        );
        assert!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_002_000).is_some());
        set_signal_identity(
            &mut signal,
            "fresh-event",
            "fresh-episode",
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
        );
        assert_eq!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_002_000), None);
        consume_post_stop_reentry_cooldown(&mut settings, &signal, 1_000_000_002_000);
        assert!(settings.post_stop_reentry_cooldowns.is_empty());
    }

    #[test]
    fn unversioned_or_pre_stop_signal_cannot_satisfy_fresh_event_gate() {
        let mut settings = Settings::default();
        insert_cooldown(&mut settings);
        let mut signal = cooldown_signal();
        signal.timestamp = 999_999_999;
        set_signal_identity(&mut signal, "fresh-event", "fresh-episode", 0);
        assert!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_003_000).is_some());

        signal.timestamp = 1_000_000_003_000;
        set_signal_identity(
            &mut signal,
            "fresh-event",
            "fresh-episode",
            strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
        );
        assert_eq!(post_stop_reentry_block_reason(&settings, &signal, 1_000_000_003_000), None);
    }

    #[test]
    fn only_a_fully_filled_stop_that_closes_position_starts_cooldown() {
        assert!(!completed_stop_position_fill("stop-loss", 0.5, 1.0, false));
        assert!(!completed_stop_position_fill("stop-loss", 1.0, 1.0, false));
        assert!(!completed_stop_position_fill("take-profit", 1.0, 1.0, true));
        assert!(completed_stop_position_fill("stop-loss", 1.0, 1.0, true));
    }

    #[test]
    fn completed_stop_cooldown_uses_profile_value_and_survives_settings_restart() {
        let mut settings = Settings::default();
        let position = TraderPosition {
            strategy_decision: Some(serde_json::json!({
                "learning_profile_id": "profile-a",
                "strategy_parameters": { "post_stop_cooldown_minutes": 4.0 },
                "live_worker_event_identity_version": strategy::LIVE_STRATEGY_EVENT_ID_VERSION,
                "live_worker_event_id": "stopped-event",
                "live_worker_episode_id": "stopped-episode",
            })),
            ..TraderPosition::default()
        };
        let stop_time_ms = 1_000_000_000_000;
        assert!(!completed_stop_position_fill("stop-loss", 0.5, 1.0, false));
        assert!(settings.post_stop_reentry_cooldowns.is_empty());
        assert!(completed_stop_position_fill("stop-loss", 1.0, 1.0, true));
        assert!(record_post_stop_reentry_cooldown(
            &mut settings,
            "BTC-USDT",
            "live-momentum",
            &position,
            stop_time_ms,
        ));
        let cooldown = settings
            .post_stop_reentry_cooldowns
            .get(&post_stop_reentry_key("BTC-USDT", "live-momentum"))
            .unwrap();
        assert_eq!(cooldown.cooldown_until_ms, stop_time_ms + 4 * 60_000);
        assert_eq!(cooldown.profile_id.as_deref(), Some("profile-a"));

        let restarted: Settings =
            serde_json::from_value(serde_json::to_value(settings).unwrap()).unwrap();
        assert!(post_stop_reentry_block_reason(
            &restarted,
            &cooldown_signal(),
            stop_time_ms + 1,
        )
        .is_some());
    }
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
