// Preserve a pause while a snapshot has no value for its strategy. A fresh
// positive multiplier is allowed through after time-decayed evidence recovers.
fn preserve_paused_strategy_multipliers(
    previous: &BTreeMap<String, f64>,
    mut incoming: BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    for (strategy, multiplier) in previous {
        if multiplier.is_finite()
            && *multiplier <= 0.0
            && !incoming
                .get(strategy)
                .is_some_and(|value| value.is_finite() && *value > 0.0)
        {
            incoming.insert(strategy.clone(), 0.0);
        }
    }
    incoming
}

fn should_apply_learned_profile_snapshot(
    previous: Option<&strategy::StrategyProfile>,
    incoming: &strategy::StrategyProfile,
) -> bool {
    previous.is_none_or(|previous| {
        previous.profile_id != incoming.profile_id || incoming.version > previous.version
    })
}

fn sync_registered_profile_ids(settings: &mut Settings, incoming: BTreeSet<String>) -> bool {
    if settings.registered_profile_ids == incoming {
        return false;
    }
    settings.registered_profile_ids = incoming;
    true
}

async fn apply_learning_snapshot(
    state: &AutoTraderState,
    mut snapshot: strategy::LearnedProfileSnapshot,
) {
    snapshot.profiles.retain(|profile| strategy::is_autotrader_symbol_allowed(&profile.symbol));
    snapshot.scout_parameter_variants.retain(|variant| strategy::is_autotrader_symbol_allowed(&variant.symbol));
    let allowed_profile_ids = snapshot.registered_profile_ids.clone();
    snapshot.walk_forward_evidence.retain(|key, _| {
        key.split('|').next().is_some_and(|identity| {
            strategy::is_autotrader_symbol_allowed(identity)
                || allowed_profile_ids.contains(identity)
        })
    });
    snapshot.recent_profile_context_returns.retain(|key, _| key.split('|').next().is_some_and(|profile_id| allowed_profile_ids.contains(profile_id)));
    snapshot.missed_opportunity_bands.retain(|band| band.symbol.as_deref().is_none_or(strategy::is_autotrader_symbol_allowed));
    let bands = snapshot
        .missed_opportunity_bands
        .iter()
        .take(2_000)
        .map(|band| MissedOpportunityGateBand {
            gate: match band.gate.as_str() {
                "score" => MissedOpportunityGate::Score,
                "strength" => MissedOpportunityGate::Strength,
                _ => MissedOpportunityGate::Other,
            },
            threshold: band.threshold,
            symbol: band.symbol.clone(),
            direction: band.direction.clone(),
            strategy: band.strategy.clone(),
            market_context: band.market_context.clone(),
            horizon_seconds: band.horizon_seconds,
            sample_count: band.sample_count,
            average_net_return_percent: band.average_net_return_percent,
            positive_net_rate: band.positive_net_rate,
            time_to_profit_sample_count: band.time_to_profit_sample_count,
            time_to_profit_rate: band.time_to_profit_rate,
            average_time_to_profit_seconds: band.average_time_to_profit_seconds,
            stop_target_sample_count: band.stop_target_sample_count,
            stop_target_average_net_return_percent: band.stop_target_average_net_return_percent,
            stop_target_positive_net_rate: band.stop_target_positive_net_rate,
        })
        .collect::<Vec<_>>();
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    changed |= sync_registered_profile_ids(&mut settings, snapshot.registered_profile_ids);
    if settings.learned_scout_parameter_variants != snapshot.scout_parameter_variants {
        settings.learned_scout_parameter_variants = snapshot.scout_parameter_variants;
        changed = true;
    }
    for profile in snapshot.profiles {
        let key = format!("{}:{}", profile.symbol, profile.strategy);
        if queue_snapshot_profile_registration(&mut settings, &profile) {
            changed = true;
        }
        if should_apply_learned_profile_snapshot(settings.learned_profiles.get(&key), &profile) {
            settings.learned_profiles.insert(key, profile);
            changed = true;
        }
    }
    let multipliers = preserve_paused_strategy_multipliers(
        &settings.learned_strategy_size_multipliers,
        snapshot.strategy_size_multipliers,
    );
    if settings.learned_strategy_size_multipliers != multipliers {
        settings.learned_strategy_size_multipliers = multipliers;
        changed = true;
    }
    if settings.recent_profile_context_returns != snapshot.recent_profile_context_returns {
        settings.recent_profile_context_returns = snapshot.recent_profile_context_returns;
        changed = true;
    }
    if settings.contextual_stop_loss_percents != snapshot.contextual_stop_loss_percents {
        settings.contextual_stop_loss_percents = snapshot.contextual_stop_loss_percents;
        changed = true;
    }
    if settings.learned_context_stats != snapshot.context_learning {
        settings.learned_context_stats = snapshot.context_learning;
        changed = true;
    }
    if settings.learned_context_thresholds != snapshot.context_thresholds {
        settings.learned_context_thresholds = snapshot.context_thresholds;
        changed = true;
    }
    if settings.learned_walk_forward_evidence != snapshot.walk_forward_evidence {
        settings.learned_walk_forward_evidence = snapshot.walk_forward_evidence;
        changed = true;
    }
    if settings.learned_execution_stats != snapshot.execution_learning {
        settings.learned_execution_stats = snapshot.execution_learning;
        changed = true;
    }
    drop(settings);
    *state.latest_missed_opportunity_bands.lock().await = Arc::new(bands);
    if changed {
        notify_live_profile_updates(state);
        if let Err(error) = request_settings_persist(state) {
            crate::service_log!(
                "WARN",
                "Could not persist pushed Learning snapshot: {}",
                error.1
            );
        }
    }
}

fn queue_snapshot_profile_registration(
    settings: &mut Settings,
    profile: &strategy::StrategyProfile,
) -> bool {
    let Some(kind) = Strategy::from_name(&profile.strategy) else {
        return false;
    };
    if kind.is_legacy_composite()
        || strategy::profile_id(&profile.symbol, &profile.strategy, &profile.parameters)
            != profile.profile_id
        || settings
            .registered_profile_ids
            .contains(&profile.profile_id)
        || settings
            .profile_registration_outbox
            .iter()
            .any(|registration| registration.profile.profile_id == profile.profile_id)
    {
        return false;
    }
    let specs = kind
        .parameter_specs()
        .into_iter()
        .filter(|spec| profile.parameters.contains_key(spec.key.as_str()))
        .map(|spec| strategy::ParameterSpec {
            key: spec.key.as_str().to_owned(),
            label: spec.label.to_owned(),
            default: spec.default,
            min: spec.min,
            max: spec.max,
            step: spec.step,
            unit: spec.unit.to_owned(),
            learning_direction: spec.learning_direction,
        })
        .collect();
    settings
        .profile_registration_outbox
        .push(strategy::StrategyProfileRegistration {
            profile: profile.clone(),
            specs,
        });
    true
}

fn next_profile_registration_for_feedback(
    registrations: &[strategy::StrategyProfileRegistration],
    feedback: Option<&FeedbackEvent>,
) -> Option<strategy::StrategyProfileRegistration> {
    match feedback {
        None => registrations.first().cloned(),
        Some(
            FeedbackEvent::TradeOutcome { profile_id, .. }
            | FeedbackEvent::ScoutOutcome { profile_id, .. },
        ) => registrations
            .iter()
            .find(|registration| &registration.profile.profile_id == profile_id)
            .cloned(),
        // Missed-opportunity delivery has no registration prerequisite, so
        // drain an unrelated registration alongside it instead of letting a
        // large missed-opportunity backlog starve profile registration.
        Some(FeedbackEvent::MissedOpportunityCandidate { .. }) => registrations.first().cloned(),
    }
}

fn next_feedback_event(feedback: &[FeedbackEvent]) -> Option<&FeedbackEvent> {
    // Trade and Scout outcomes must not wait behind missed-opportunity jobs:
    // Learning can reject those jobs while its separate replay queue is full.
    feedback
        .iter()
        .find(|event| {
            matches!(
                event,
                FeedbackEvent::TradeOutcome { .. } | FeedbackEvent::ScoutOutcome { .. }
            )
        })
        .or_else(|| feedback.first())
}

#[cfg(test)]
mod strategy_pause_snapshot_tests {
    use super::*;

    #[test]
    fn pause_survives_missing_snapshot_but_decayed_positive_evidence_clears_it() {
        let prior = BTreeMap::from([
            ("live-momentum".to_owned(), 0.0),
            ("live-trade-flow".to_owned(), 0.8),
        ]);
        let incoming = BTreeMap::from([
            ("live-momentum".to_owned(), 1.0),
            ("live-trade-flow".to_owned(), 1.2),
        ]);
        let merged = preserve_paused_strategy_multipliers(&prior, incoming.clone());
        assert_eq!(merged["live-momentum"], 1.0);
        assert_eq!(merged["live-trade-flow"], 1.2);
        assert_eq!(
            preserve_paused_strategy_multipliers(&merged, BTreeMap::new())["live-momentum"],
            0.0
        );
        assert_eq!(
            preserve_paused_strategy_multipliers(&BTreeMap::new(), incoming)["live-momentum"],
            1.0
        );
    }

    #[test]
    fn changed_profile_identity_replaces_stale_high_version_snapshot() {
        let profile = |profile_id: &str, version| strategy::StrategyProfile {
            profile_id: profile_id.into(),
            version,
            symbol: "LINK-USDT".into(),
            strategy: "live-order-book".into(),
            parameters: BTreeMap::new(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let previous = profile("old-profile", 1_299);
        assert!(should_apply_learned_profile_snapshot(
            Some(&previous),
            &profile("new-profile", 0),
        ));
        assert!(!should_apply_learned_profile_snapshot(
            Some(&previous),
            &profile("old-profile", 1_298),
        ));
        assert!(should_apply_learned_profile_snapshot(
            Some(&previous),
            &profile("old-profile", 1_300),
        ));
        assert!(should_apply_learned_profile_snapshot(
            None,
            &profile("new-profile", 0),
        ));
    }

    #[test]
    fn registered_profile_snapshot_drops_active_profiles_without_variant_registration() {
        let mut settings = Settings::default();
        settings
            .registered_profile_ids
            .extend(["registered".into(), "stale-active-only".into()]);
        assert!(sync_registered_profile_ids(
            &mut settings,
            BTreeSet::from(["registered".into(), "newly-registered".into()]),
        ));
        assert_eq!(
            settings.registered_profile_ids,
            BTreeSet::from(["registered".into(), "newly-registered".into()]),
        );
        assert!(!sync_registered_profile_ids(
            &mut settings,
            BTreeSet::from(["registered".into(), "newly-registered".into()]),
        ));
    }

    #[test]
    fn unregistered_active_learning_profile_is_queued_before_it_can_collect_scout_evidence() {
        let strategy = Strategy::LiveMomentum;
        let parameters = strategy
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile = strategy::StrategyProfile {
            profile_id: strategy::profile_id("LINK-USDT", strategy.as_str(), &parameters),
            version: 0,
            symbol: "LINK-USDT".into(),
            strategy: strategy.as_str().into(),
            parameters,
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let mut settings = Settings::default();
        assert!(queue_snapshot_profile_registration(&mut settings, &profile));
        assert_eq!(settings.profile_registration_outbox.len(), 1);
        assert_eq!(
            settings.profile_registration_outbox[0].profile.profile_id,
            profile.profile_id
        );
        assert!(
            !settings
                .registered_profile_ids
                .contains(&profile.profile_id)
        );
        assert!(!queue_snapshot_profile_registration(
            &mut settings,
            &profile
        ));
        assert_eq!(settings.profile_registration_outbox.len(), 1);
    }

    #[test]
    fn feedback_is_not_blocked_by_unrelated_profile_registrations() {
        let registration = |id: &str| strategy::StrategyProfileRegistration {
            profile: strategy::StrategyProfile {
                profile_id: id.into(),
                version: 0,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::new(),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![],
        };
        let registrations = vec![registration("unrelated"), registration("needed")];
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade".into(),
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "needed".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: -1.0,
            net_return_percent: -0.1,
            threshold_observations: vec![],
            replay_context: None,
        };
        assert_eq!(
            next_profile_registration_for_feedback(&registrations, Some(&event))
                .unwrap()
                .profile
                .profile_id,
            "needed"
        );
        assert!(
            next_profile_registration_for_feedback(&registrations[..1], Some(&event)).is_none()
        );
        assert_eq!(
            next_profile_registration_for_feedback(&registrations, None)
                .unwrap()
                .profile
                .profile_id,
            "unrelated"
        );
    }

    #[test]
    fn missed_opportunity_backlog_does_not_starve_profile_registration() {
        let registration = strategy::StrategyProfileRegistration {
            profile: strategy::StrategyProfile {
                profile_id: "active-profile".into(),
                version: 0,
                symbol: "LINK-USDT".into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::new(),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![],
        };
        let missed = FeedbackEvent::MissedOpportunityCandidate {
            event_id: "missed".into(),
            observed_at_ms: 1,
            symbol: "LINK-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "active-profile".into(),
            parameters: BTreeMap::new(),
            direction: "bullish".into(),
            strength: 0.5,
            score: 0.5,
            learning_context: "long:trend:LINK-USDT:live-momentum".into(),
        };
        let registrations = [registration];
        assert_eq!(
            next_profile_registration_for_feedback(&registrations, Some(&missed))
                .unwrap()
                .profile
                .profile_id,
            "active-profile"
        );
    }

    #[test]
    fn trade_outcomes_bypass_missed_opportunities_at_the_feedback_queue_head() {
        let missed = FeedbackEvent::MissedOpportunityCandidate {
            event_id: "missed".into(),
            observed_at_ms: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "missed-profile".into(),
            parameters: BTreeMap::new(),
            direction: "bullish".into(),
            strength: 0.5,
            score: 0.5,
            learning_context: "long:trend:BTC-USDT:live-momentum".into(),
        };
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "trade".into(),
            observed_at_ms: 2,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "trade-profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 0.1,
            threshold_observations: vec![],
            replay_context: None,
        };

        assert!(matches!(
            next_feedback_event(&[missed.clone(), trade.clone()]),
            Some(FeedbackEvent::TradeOutcome { .. })
        ));
        assert!(matches!(
            next_feedback_event(&[missed.clone()]),
            Some(FeedbackEvent::MissedOpportunityCandidate { .. })
        ));
        assert!(next_feedback_event(&[]).is_none());
    }
}

pub fn router() -> Router {
    router_with_socket().0
}

pub fn router_with_socket() -> (Router, AutoTraderSocketApi) {
    let state_file_path = settings_path();
    let settings_path = settings_database_path(&state_file_path);
    let database_state = read_settings_database(&settings_path).unwrap_or_else(|error| {
        panic!(
            "Could not read AutoTrader SQLite state {}: {error}",
            settings_path.display()
        )
    });
    let mut settings = database_state.unwrap_or_default();
    migrate_closed_trade_metrics(&mut settings);
    // The previous release defaulted this new cap to three, which can strand existing books above the cap.
    if settings.max_open_positions == 3 {
        settings.max_open_positions = default_max_open_positions();
    }
    for (symbol, position) in settings.positions.iter_mut() {
        if position.trade_id.is_none() {
            position.trade_id = position.entry_decision_id.clone();
        }
        if position.quantity.abs() > 1e-12 && position.strategy.is_none() {
            position.strategy = strategy_from_entry_context(&position.entry_context);
        }
        if position.quantity.abs() > 1e-12
            && position.entries > settings.max_entries_per_auto_position
        {
            let expected_strategy = position.strategy.as_deref();
            let actual_entry_count = settings
                .orders
                .iter()
                .filter(|order| {
                    order.symbol == *symbol
                        && matches!(order.purpose.as_str(), "signal" | "scout")
                        && order.filled_quantity > 1e-12
                })
                .filter(|order| match expected_strategy {
                    Some(strategy) => order.strategy.as_deref() == Some(strategy),
                    None => order.strategy.as_deref().is_none_or(|strategy| {
                        strategy == "generic-insight-signals" || strategy == "unattributed"
                    }),
                })
                .filter_map(|order| order.decision_id.as_ref())
                .collect::<BTreeSet<_>>()
                .len();
            position.entries = actual_entry_count
                .max(1)
                .min(settings.max_entries_per_auto_position as usize)
                as u32;
        } else if position.quantity.abs() > 1e-12 && position.entries == 0 {
            position.entries = 1;
        }
    }
    let legacy_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| (symbol.clone(), position.clone()))
        .collect::<Vec<_>>();
    for (symbol, position) in legacy_positions {
        let strategy = position
            .strategy
            .clone()
            .unwrap_or_else(|| "unattributed".into());
        let lots = settings
            .strategy_positions
            .entry(symbol.clone())
            .or_default();
        if lots.is_empty() {
            lots.insert(strategy, position.clone());
        }
        let mut entries_by_category = BTreeMap::<String, u32>::new();
        for (strategy, lot) in lots.iter() {
            let category = strategy_category_key(strategy_category(Some(strategy)));
            *entries_by_category.entry(category.to_owned()).or_default() += lot.entries;
        }
        if let Some(aggregate) = settings.positions.get_mut(&symbol) {
            aggregate.entries_by_category = entries_by_category;
        }
    }
    for lots in settings.strategy_positions.values_mut() {
        for position in lots.values_mut() {
            if position.trade_id.is_none() {
                position.trade_id = position.entry_decision_id.clone();
            }
        }
    }
    let strategy_by_symbol = settings
        .positions
        .iter()
        .filter_map(|(symbol, position)| {
            position
                .strategy
                .as_ref()
                .map(|strategy| (symbol.clone(), strategy.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for order in &mut settings.orders {
        if order.purpose == "stop-loss"
            && order.strategy.as_deref() == Some("generic-insight-signals")
        {
            if let Some(strategy) = strategy_by_symbol.get(&order.symbol) {
                order.strategy = Some(strategy.clone());
            }
        }
    }
    if let Err(error) = persist_settings(&settings_path, &settings) {
        crate::service_log!(
            "ERROR",
            "Could not persist AutoTrader SQLite state: {}",
            error.1
        );
    }
    settings.used_notional_today = allocated_notional(&settings);
    if settings.utc_day != utc_day() {
        settings.utc_day = utc_day();
    }
    let enabled = std::env::var("AUTO_TRADER_ENABLED")
        .map(|value| value == "true")
        .unwrap_or(settings.enabled);
    settings.enabled = enabled;
    let audit_tx = AuditQueue::new(
        DECISION_AUDIT_QUEUE_CAPACITY,
        DECISION_AUDIT_QUEUE_MAX_BYTES,
    );
    let execution_audit_tx = AuditQueue::new(
        EXECUTION_AUDIT_QUEUE_CAPACITY,
        EXECUTION_AUDIT_QUEUE_MAX_BYTES,
    );
    let audit_health = Arc::new(AtomicU64::new(3));
    let audit_failure_latched = Arc::new(AtomicBool::new(false));
    let execution_queue = execution_audit_tx.clone();
    let execution_health = Arc::clone(&audit_health);
    tokio::spawn(async move {
        loop {
            let payload = execution_queue.recv().await;
            store_execution_audit_until_saved(&payload, &execution_health, 2).await;
            if execution_queue.depth() == 0 {
                execution_health.fetch_or(2, Ordering::Relaxed);
            }
        }
    });
    let decision_queue = audit_tx.clone();
    let decision_health = Arc::clone(&audit_health);
    tokio::spawn(async move {
        loop {
            let payload = decision_queue.recv().await;
            store_execution_audit_until_saved(&payload, &decision_health, 1).await;
            if decision_queue.depth() == 0 {
                decision_health.fetch_or(1, Ordering::Relaxed);
            }
        }
    });
    let carry_legacy_path = std::env::var_os("POSITIONS_FILE")
        .map(PathBuf::from)
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.join("auto_trader_state.dated_carry.json"))
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| dated_carry_legacy_path(&state_file_path));
    let carry_database_book = read_dated_carry_database(&settings_path)
        .unwrap_or_else(|error| panic!("Could not read AutoTrader dated-carry state: {error}"));
    let carry_book = match carry_database_book {
        Some(book) => book,
        None => {
            let legacy_bytes = std::fs::read(&carry_legacy_path).ok();
            let legacy = legacy_bytes
                .as_deref()
                .and_then(|bytes| serde_json::from_slice::<DatedCarryBook>(&bytes).ok());
            let was_migrated = legacy.is_some();
            let book = legacy.unwrap_or_default();
            if legacy_bytes.is_some() || !carry_legacy_path.exists() {
                persist_dated_carry_book(&settings_path, &book).unwrap_or_else(|error| {
                    panic!("Could not migrate dated-carry state to SQLite: {error}")
                });
                if was_migrated {
                    let _ = std::fs::remove_file(&carry_legacy_path);
                }
            }
            book
        }
    };
    let (state_updates, _) = tokio::sync::broadcast::channel(64);
    let (live_profile_updates, _) = tokio::sync::broadcast::channel(16);
    let state = AutoTraderState {
        latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
        strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
        latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_worker_decisions: Arc::new(Mutex::new(BTreeMap::new())),
        seen_worker_decision_events: Arc::new(Mutex::new(BTreeMap::new())),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        settings_persist_notify: Arc::new(Notify::new()),
        price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
        state_updates,
        live_profile_updates,
        settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
        daily_signal_notify: Arc::new(Notify::new()),
        pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
        live_signal_notify: Arc::new(Notify::new()),
        audit_tx,
        execution_audit_tx,
        audit_health,
        audit_failure_latched,
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        stale_signal_candidates_total: Arc::new(AtomicU64::new(0)),
        stale_signal_cycles_total: Arc::new(AtomicU64::new(0)),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path: settings_path.clone(),
        dated_carry: Arc::new(Mutex::new(carry_book)),
        dated_carry_work_lock: Arc::new(Mutex::new(())),
        dated_carry_path: settings_path.clone(),
    };
    start_settings_persistence_worker(&state);
    start_learning_feedback_worker(&state);
    start_post_exit_observation_worker(&state);
    let carry_sync_book = state.dated_carry.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let book = carry_sync_book.lock().await.clone();
            if let Err(error) = sync_dated_carry_ledger(&book).await {
                crate::service_log!("WARN", "Dated-carry position-manager sync failed: {error}");
            }
        }
    });
    let rates_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = refresh_usd_rates(&rates_worker_state).await {
                crate::service_log!(
                    "WARN",
                    "USD allocation rates refresh failed; using cached rates: {}",
                    error.1
                );
            }
        }
    });

    let signal_worker_state = state.clone();
    tokio::spawn(async move {
        let mut prefer_daily = true;
        // Let the keyed queues absorb a short burst, then merge the freshest
        // reports into bounded dashboards before evaluating them. A small
        // number of these coalesced cycles can run in parallel; this avoids
        // repeating whole-dashboard work for every individual report.
        let cycle_slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SIGNAL_CYCLES));
        loop {
            let daily_notification = signal_worker_state.daily_signal_notify.notified();
            let live_notification = signal_worker_state.live_signal_notify.notified();
            tokio::pin!(daily_notification);
            tokio::pin!(live_notification);

            let stale_live_candidates = {
                let mut queue = signal_worker_state.pending_live_signals.lock().await;
                discard_expired_live_signal_dashboards(&mut queue, current_time_ms())
            };
            if stale_live_candidates > 0 {
                signal_worker_state
                    .stale_signal_candidates_total
                    .fetch_add(stale_live_candidates as u64, Ordering::Relaxed);
            }
            let has_pending = {
                !signal_worker_state
                    .pending_daily_signals
                    .lock()
                    .await
                    .is_empty()
                    || !signal_worker_state
                        .pending_live_signals
                        .lock()
                        .await
                        .is_empty()
            };
            if has_pending {
                // Wait for a cycle slot before removing a dashboard from the
                // coalescing queues. Otherwise a stale snapshot can sit outside
                // the queues while waiting, even as newer reports replace it.
                let Ok(permit) = Arc::clone(&cycle_slots).acquire_owned().await else {
                    continue;
                };
                tokio::time::sleep(SIGNAL_BATCH_COALESCE_WINDOW).await;
                // The queue may age while this task waits for a slot. Check it
                // again after coalescing, immediately before dequeue.
                let stale_live_candidates = {
                    let mut queue = signal_worker_state.pending_live_signals.lock().await;
                    discard_expired_live_signal_dashboards(&mut queue, current_time_ms())
                };
                if stale_live_candidates > 0 {
                    signal_worker_state
                        .stale_signal_candidates_total
                        .fetch_add(stale_live_candidates as u64, Ordering::Relaxed);
                }
                let pending = if prefer_daily {
                    let daily = {
                        let mut queue = signal_worker_state.pending_daily_signals.lock().await;
                        take_next_pending_signal_batch(&mut queue, MAX_SIGNAL_DASHBOARDS_PER_CYCLE)
                    };
                    if let Some(dashboard) = daily {
                        Some((PendingSignalKind::Daily, dashboard))
                    } else {
                        let live = {
                            let mut queue = signal_worker_state.pending_live_signals.lock().await;
                            take_next_pending_signal_batch(
                                &mut queue,
                                MAX_SIGNAL_DASHBOARDS_PER_CYCLE,
                            )
                        };
                        live.map(|dashboard| (PendingSignalKind::Live, dashboard))
                    }
                } else {
                    let live = {
                        let mut queue = signal_worker_state.pending_live_signals.lock().await;
                        take_next_pending_signal_batch(&mut queue, MAX_SIGNAL_DASHBOARDS_PER_CYCLE)
                    };
                    if let Some(dashboard) = live {
                        Some((PendingSignalKind::Live, dashboard))
                    } else {
                        let daily = {
                            let mut queue = signal_worker_state.pending_daily_signals.lock().await;
                            take_next_pending_signal_batch(
                                &mut queue,
                                MAX_SIGNAL_DASHBOARDS_PER_CYCLE,
                            )
                        };
                        daily.map(|dashboard| (PendingSignalKind::Daily, dashboard))
                    }
                };

                if let Some((kind, dashboard)) = pending {
                    prefer_daily = kind == PendingSignalKind::Live;
                    let enabled = {
                        let settings = signal_worker_state.settings.lock().await;
                        signal_worker_state.enabled.load(Ordering::Relaxed)
                            && match kind {
                                PendingSignalKind::Daily => {
                                    settings.strategies.iter().any(|strategy| {
                                        !strategy.is_live_insight() && !strategy.is_dated_carry()
                                    })
                                }
                                PendingSignalKind::Live => {
                                    settings.signal_strategies_enabled
                                        || settings
                                            .strategies
                                            .iter()
                                            .any(|strategy| !strategy.is_dated_carry())
                                }
                            }
                    };
                    if enabled {
                        let worker = match kind {
                            PendingSignalKind::Daily => "Daily strategy",
                            PendingSignalKind::Live => "Live insight",
                        };
                        let state = signal_worker_state.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            if let Err(error) = run_cycle(&state, dashboard).await {
                                crate::service_log!("ERROR", "{worker} cycle failed: {}", error.1);
                            }
                        });
                    }
                } else {
                    drop(permit);
                }
            } else {
                tokio::select! {
                    _ = &mut daily_notification => {},
                    _ = &mut live_notification => {},
                }
            }
        }
    });

    let position_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if position_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_position_maintenance(&position_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader position maintenance failed: {}",
                        error.1
                    );
                }
            }
        }
    });

    let hedge_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if hedge_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_hedge_evaluation(&hedge_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader hedge evaluation failed: {}",
                        error.1
                    );
                }
            }
        }
    });
    let socket_api = AutoTraderSocketApi {
        state: state.clone(),
    };
    let router = Router::new()
        .route("/positions/auto-trader", get(get_status).put(set_enabled))
        .route(
            "/positions/auto-trader/dated-carry",
            get(get_dated_carry).put(update_dated_carry).post(close_dated_carry),
        )
        .route(
            "/positions/auto-trader/parameters",
            get(get_decision_parameters),
        )
        .route(
            "/positions/auto-trader/parameters/v1",
            get(get_compact_decision_parameters),
        )
        .route(
            "/positions/auto-trader/parameters/scout-variants",
            get(get_scout_parameter_variants),
        )
        .route(
            "/internal/strategy/daily-scan-parameters",
            get(get_daily_strategy_parameters),
        )
        .route(
            "/positions/auto-trader/missed-opportunity-bands",
            get(get_missed_opportunity_bands),
        )
        .route(
            "/positions/auto-trader/settings",
            axum::routing::put(update_settings),
        )
        .route("/positions/auto-trader/activity", get(get_activity))
        .route(
            "/internal/strategy/open-position-counts",
            get(get_strategy_open_position_counts),
        )
        .route(
            "/positions/auto-trader/hedge-candidates",
            get(get_portfolio_hedge_candidates),
        )
        .route(
            "/positions/auto-trader/learning-summary",
            get(get_learning_summary),
        )
        .with_state(state);
    (router, socket_api)
}

async fn get_learning_summary(
    State(state): State<AutoTraderState>,
) -> axum::Json<serde_json::Value> {
    let settings = state.settings.lock().await;
    let mut summaries = BTreeMap::<String, (usize, usize, usize, usize, f64)>::new();
    for profile in settings.learned_profiles.values() {
        let totals = summaries.entry(profile.strategy.clone()).or_default();
        totals.0 = totals.0.saturating_add(profile.learning_samples);
        totals.1 = totals.1.saturating_add(profile.positive_samples);
        if let Some(evidence) = settings.learned_walk_forward_evidence.get(&profile.profile_id) {
            totals.2 = totals.2.saturating_add(evidence.sample_count);
            totals.3 = totals.3.saturating_add(
                (evidence.positive_net_rate.unwrap_or_default() * evidence.sample_count as f64)
                    .round() as usize,
            );
            totals.4 += evidence
                .average_net_return_percent
                .unwrap_or_default()
                * evidence.sample_count as f64;
        }
    }
    axum::Json(serde_json::Value::Array(
        summaries
            .into_iter()
            .map(|(strategy, (samples, positive, scout_samples, scout_positive, scout_net))| {
                serde_json::json!({
                    "strategy": strategy,
                    "learning_samples": samples,
                    "positive_rate": (samples > 0).then_some(positive as f64 / samples as f64),
                    "scout_outcome_samples": scout_samples,
                    "scout_positive_rate": (scout_samples > 0).then_some(scout_positive as f64 / scout_samples as f64),
                    "scout_average_net_return_percent": (scout_samples > 0).then_some(scout_net / scout_samples as f64),
                })
            })
            .collect(),
    ))
}

fn feedback_event_id(event: &FeedbackEvent) -> &str {
    match event {
        FeedbackEvent::ScoutOutcome { event_id, .. }
        | FeedbackEvent::TradeOutcome { event_id, .. }
        | FeedbackEvent::MissedOpportunityCandidate { event_id, .. } => event_id,
    }
}

#[cfg(test)]
fn scout_feedback_parameter_keys(
    strategy: Strategy,
    parameter_key: Option<&str>,
) -> BTreeSet<String> {
    match parameter_key {
        Some(key) if key.starts_with("combo:") => parse_scout_combination(key, strategy)
            .map(|parameters| parameters.into_keys().collect())
            .unwrap_or_default(),
        Some(key)
            if strategy
                .parameter_specs()
                .iter()
                .any(|spec| spec.key.as_str() == key) =>
        {
            BTreeSet::from([key.to_owned()])
        }
        _ => BTreeSet::new(),
    }
}

fn start_learning_feedback_worker(state: &AutoTraderState) {
    let learning_url = std::env::var("LEARNING_RSOCKET_URL").ok();
    if learning_url.is_none() {
        return;
    }

    let publisher_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let registration = {
                let settings = publisher_state.settings.lock().await;
                let feedback = next_feedback_event(&settings.feedback_outbox);
                next_profile_registration_for_feedback(
                    &settings.profile_registration_outbox,
                    feedback,
                )
            };
            if let Some(registration) = registration {
                let response = crate::service_rpc::learning_request(
                    strategy::ScoutWorkerHttpMethod::Post,
                    "/profiles",
                    Some(serde_json::to_value(&registration).unwrap_or(serde_json::Value::Null)),
                )
                .await;
                match response {
                    Ok(response) if (200..300).contains(&response.status) => {
                        let profile_id = registration.profile.profile_id;
                        let mut settings = publisher_state.settings.lock().await;
                        settings
                            .profile_registration_outbox
                            .retain(|pending| pending.profile.profile_id != profile_id);
                        settings.registered_profile_ids.insert(profile_id);
                        drop(settings);
                        if let Err(error) = request_settings_persist(&publisher_state) {
                            crate::service_log!(
                                "WARN",
                                "Could not persist profile-registration acknowledgement: {}",
                                error.1
                            );
                        }
                    }
                    Ok(response) => crate::service_log!(
                        "WARN",
                        "Learning rejected profile registration {} with status {}: {}",
                        registration.profile.profile_id,
                        response.status,
                        response.body,
                    ),
                    Err(error) => crate::service_log!(
                        "WARN",
                        "Learning profile registration {} failed: {}",
                        registration.profile.profile_id,
                        error,
                    ),
                }
            }
            let event = {
                let settings = publisher_state.settings.lock().await;
                next_feedback_event(&settings.feedback_outbox).cloned()
            };
            if let Some(event) = event {
                let response = crate::service_rpc::learning_request(
                    strategy::ScoutWorkerHttpMethod::Post,
                    "/feedback",
                    Some(serde_json::to_value(&event).unwrap_or(serde_json::Value::Null)),
                )
                .await;
                if response.is_ok_and(|response| (200..300).contains(&response.status)) {
                    let event_id = feedback_event_id(&event).to_owned();
                    let mut settings = publisher_state.settings.lock().await;
                    settings
                        .feedback_outbox
                        .retain(|pending| feedback_event_id(pending) != event_id);
                    drop(settings);
                    if let Err(error) = request_settings_persist(&publisher_state) {
                        crate::service_log!(
                            "WARN",
                            "Could not persist Scout/trade feedback acknowledgement: {}",
                            error.1
                        );
                    }
                }
                continue;
            }
            let execution_event = publisher_state
                .settings
                .lock()
                .await
                .execution_feedback_outbox
                .first()
                .cloned();
            if let Some(event) = execution_event {
                let response = crate::service_rpc::learning_request(
                    strategy::ScoutWorkerHttpMethod::Post,
                    "/execution-outcomes",
                    Some(serde_json::to_value(&event).unwrap_or(serde_json::Value::Null)),
                )
                .await;
                if response.is_ok_and(|response| (200..300).contains(&response.status)) {
                    let event_id = event.event_id.clone();
                    let mut settings = publisher_state.settings.lock().await;
                    settings
                        .execution_feedback_outbox
                        .retain(|pending| pending.event_id != event_id);
                    drop(settings);
                    if let Err(error) = request_settings_persist(&publisher_state) {
                        crate::service_log!(
                            "WARN",
                            "Could not persist execution-learning acknowledgement: {}",
                            error.1
                        );
                    }
                }
            }
        }
    });

}

async fn receive_strategy_decision(
    State(state): State<AutoTraderState>,
    Json(mut report): Json<strategy::LiveStrategyDecisionReport>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !strategy::is_autotrader_symbol_allowed(&report.symbol) {
        return Ok(StatusCode::NO_CONTENT);
    }
    if report.strategy == Strategy::DatedCarry {
        return receive_dated_carry_worker_report(state, report).await;
    }
    if report.contract_version != strategy::LIVE_EVALUATION_CONTRACT_VERSION
        || report.event_id.is_empty()
        || report.strategy.as_str() != report.signal.strategy.as_deref().unwrap_or_default()
        || report.symbol != report.signal.symbol
        || report.event_time_ms != report.signal.timestamp
        || report.parameter_hash != report.profile_id
        || report.signal.stale
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid strategy decision report".into(),
        ));
    }
    let now_ms = current_time_ms();
    if now_ms.saturating_sub(report.event_time_ms) > 10_000 || report.event_time_ms > now_ms + 5_000
    {
        return Err((StatusCode::CONFLICT, "Strategy decision is stale".into()));
    }
    let profile_symbol = report
        .signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| {
            decision
                .get("learning_profile_symbol")
                .or_else(|| decision.get("scout_parameter_profile_symbol"))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&report.signal.symbol)
        .to_owned();
    let enabled = {
        let settings = state.settings.lock().await;
        let signal_parameters = report
            .signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .map(|parameters| {
                parameters
                    .iter()
                    .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                    .collect::<BTreeMap<_, _>>()
            });
        let parameters_match = signal_parameters.as_ref().is_some_and(|parameters| {
            strategy::profile_id(&profile_symbol, report.strategy.as_str(), parameters)
                == report.profile_id
        });
        let version_match = if let Some(configured) = settings
            .active_live_strategy_profiles
            .get(report.strategy.as_str())
        {
            configured.iter().any(|profile| {
                profile.profile_id == report.profile_id
                    && profile.version == report.profile_version
                    && profile.profile_symbol == profile_symbol
            })
        } else {
            let profile = live_profile_for_symbol(&settings, &profile_symbol, report.strategy);
            profile.profile_id == report.profile_id && profile.version == report.profile_version
        };
        state.enabled.load(Ordering::Relaxed)
            && settings.strategies.contains(&report.strategy)
            && parameters_match
            && version_match
    };
    if !enabled {
        return Err((
            StatusCode::CONFLICT,
            "Strategy or profile is no longer enabled".into(),
        ));
    }
    let event_key = (
        report.event_id.clone(),
        report.symbol.clone(),
        report.strategy.as_str().to_owned(),
    );
    let mut selected = state.seen_worker_decision_events.lock().await;
    selected.retain(|_, timestamp| now_ms.saturating_sub(*timestamp) <= 60_000);
    if selected.contains_key(&event_key) {
        return Ok(StatusCode::ACCEPTED);
    }
    if selected.len() >= 8_192 {
        if let Some(oldest) = selected
            .iter()
            .min_by_key(|(_, timestamp)| *timestamp)
            .map(|(key, _)| key.clone())
        {
            selected.remove(&oldest);
        }
    }
    selected.insert(event_key, now_ms);
    drop(selected);
    let decision_key = (report.symbol.clone(), report.strategy.as_str().to_owned());
    {
        let mut decisions = state.latest_worker_decisions.lock().await;
        decisions
            .retain(|_, previous| now_ms.saturating_sub(previous.event_time_ms) <= 10 * 60_000);
        if decisions.len() >= 512 && !decisions.contains_key(&decision_key) {
            if let Some(oldest) = decisions
                .iter()
                .min_by_key(|(_, report)| report.event_time_ms)
                .map(|(key, _)| key.clone())
            {
                decisions.remove(&oldest);
            }
        }
        decisions.insert(decision_key.clone(), report.clone());
    }
    {
        let mut signals = state.latest_live_strategy_signals.lock().await;
        if signals.len() >= 512 && !signals.contains_key(&decision_key) {
            if let Some(oldest) = signals.keys().next().cloned() {
                signals.remove(&oldest);
            }
        }
        signals.insert(decision_key, vec![report.signal.clone()]);
    }
    if report.decision != strategy::LiveStrategyDecision::Enter {
        if report.decision == strategy::LiveStrategyDecision::Reject {
            let evaluated_reason = strategy_entry_rejection_reason(
                &report.signal,
                report.strategy,
                &signal_strategy_parameters(&report.signal),
            );
            let reason = report
                .reason
                .clone()
                .filter(|reason| {
                    !reason.contains("no rule explanation")
                        && !reason.contains("did not provide a rule explanation")
                        && !reason.contains("did not satisfy its entry rules")
                        && !reason.contains("the signal did not satisfy its entry rules")
                        && !reason.contains("entry conditions were not met")
                        && !reason.contains("Live strategy evaluator rejected the signal")
                })
                .unwrap_or(evaluated_reason);
            let audit_reason = reason.clone();
            let blocker = StrategyBlocker {
                strategy: report.strategy.as_str().to_owned(),
                symbol: report.symbol.clone(),
                reason: reason.clone(),
                timestamp_ms: report.event_time_ms,
            };
            {
                let mut blockers = state
                    .latest_strategy_blockers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(existing) = blockers.get_mut(&blocker.strategy) {
                    if existing.timestamp_ms < blocker.timestamp_ms {
                        *existing = blocker;
                    } else if existing.timestamp_ms == blocker.timestamp_ms {
                        let detail = format!("{} · {}", blocker.symbol, blocker.reason);
                        if !existing.reason.contains(&detail) {
                            existing.reason.push_str("; ");
                            existing.reason.push_str(&detail);
                        }
                        existing.symbol = "MULTIPLE".to_owned();
                    }
                } else {
                    blockers.insert(blocker.strategy.clone(), blocker);
                }
            }
            record_cycle_gap_summary(&state, &Ok(skipped(&reason)));
            let mut settings = state.settings.lock().await;
            if now_ms >= settings.last_decision_at {
                settings.last_decision = reason;
                settings.last_decision_at = now_ms;
            }
            let (stop_loss_percent, take_profit_percent) =
                (settings.stop_loss_percent, settings.take_profit_percent);
            drop(settings);
            queue_strategy_rejection_audit(
                &state,
                &report.signal,
                report.strategy.as_str(),
                &audit_reason,
                report.event_time_ms,
                stop_loss_percent,
                take_profit_percent,
                "live_worker",
            );
        }
        if report.decision == strategy::LiveStrategyDecision::Reject
            && report.strategy.is_live_insight()
        {
            let parameters = report
                .signal
                .strategy_decision
                .as_ref()
                .and_then(|decision| decision.get("strategy_parameters"))
                .and_then(serde_json::Value::as_object)
                .map(|parameters| {
                    parameters
                        .iter()
                        .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            let signal_timestamp_ms = if report.event_time_ms < 1_000_000_000_000 {
                report.event_time_ms.saturating_mul(1_000)
            } else {
                report.event_time_ms
            };
            let event_id = format!(
                "missed:{}:{}:{}:{}",
                report.symbol,
                report.strategy.as_str(),
                report.profile_id,
                signal_timestamp_ms
            );
            let feedback = FeedbackEvent::MissedOpportunityCandidate {
                event_id: event_id.clone(),
                observed_at_ms: signal_timestamp_ms,
                symbol: report.symbol.clone(),
                strategy: report.strategy.as_str().to_owned(),
                profile_id: report.profile_id.clone(),
                parameters,
                direction: report.signal.direction.clone(),
                strength: report.signal.strength,
                score: report.signal.score,
                learning_context: market_context(&report.signal),
            };
            let mut settings = state.settings.lock().await;
            if settings.feedback_outbox.len() < 10_000
                && !settings.feedback_outbox.iter().any(|pending| matches!(pending,
                    FeedbackEvent::MissedOpportunityCandidate { event_id: pending_id, .. } if pending_id == &event_id))
            {
                settings.feedback_outbox.push(feedback);
                drop(settings);
                let _ = request_settings_persist(&state);
            }
        }
        return Ok(StatusCode::ACCEPTED);
    }
    if let Some(decision) = report
        .signal
        .strategy_decision
        .as_mut()
        .and_then(serde_json::Value::as_object_mut)
    {
        decision.insert("live_worker_decision".into(), serde_json::json!("enter"));
        decision.insert(
            "live_worker_event_id".into(),
            serde_json::json!(report.event_id),
        );
        decision.insert(
            "live_worker_profile_version".into(),
            serde_json::json!(report.profile_version),
        );
        decision.insert(
            "strategy_walk_forward".into(),
            serde_json::to_value(report.walk_forward_evidence).unwrap_or(serde_json::Value::Null),
        );
    }
    report.signal.strategy_candidate = true;
    let daily_strategy = is_independent_daily_strategy(report.strategy.as_str());
    let dashboard = Dashboard {
        best: vec![report.signal.clone()],
        missed_opportunity_bands: Arc::new(Vec::new()),
        markets: Arc::new(BTreeMap::new()),
        queued_at: None,
        queue_wait_ms: None,
    };
    if daily_strategy {
        let mut queue = state.pending_daily_signals.lock().await;
        queue_pending_signal_with_key(
            &mut queue,
            PendingSignalKind::Daily,
            format!("{}|{}", report.symbol, report.profile_id),
            report.symbol,
            dashboard,
            max_pending_live_signal_items(),
        );
        state.daily_signal_notify.notify_one();
    } else {
        let mut queue = state.pending_live_signals.lock().await;
        queue_pending_signal_with_key(
            &mut queue,
            PendingSignalKind::Live,
            format!("{}|{}", report.symbol, report.profile_id),
            report.symbol,
            dashboard,
            max_pending_live_signal_items(),
        );
        state.live_signal_notify.notify_one();
    }
    Ok(StatusCode::ACCEPTED)
}

async fn receive_dated_carry_worker_report(
    state: AutoTraderState,
    report: strategy::LiveStrategyDecisionReport,
) -> Result<StatusCode, (StatusCode, String)> {
    let now_ms = current_time_ms();
    if report.contract_version != strategy::LIVE_EVALUATION_CONTRACT_VERSION
        || report.event_id.is_empty()
        || now_ms.saturating_sub(report.event_time_ms) > 120_000
        || report.event_time_ms > now_ms + 5_000
    {
        return Err((
            StatusCode::CONFLICT,
            "Dated-carry worker result is stale or invalid".into(),
        ));
    }
    if let Some(scan) = report.dated_carry_scan {
        return receive_dated_carry_signal(
            State(state),
            Json(DatedCarrySignal::Scan {
                emitted_at_ms: scan.emitted_at_ms,
                candidates: scan.candidates,
                last_error: scan.last_error,
            }),
        )
        .await;
    }
    if let Some(candidate) = report.dated_carry_candidate {
        let fallback_profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
        let profile_symbol = report
            .signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_symbol"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&fallback_profile_symbol)
            .to_owned();
        let parameters = report
            .signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .map(|parameters| {
                parameters
                    .iter()
                    .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        if report.symbol != candidate.spot_symbol
            || report.signal.strategy.as_deref() != Some(Strategy::DatedCarry.as_str())
            || report.profile_id
                != strategy::profile_id(&profile_symbol, Strategy::DatedCarry.as_str(), &parameters)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Dated-carry entry profile identity is invalid".into(),
            ));
        }
        let profile_is_current = {
            let settings = state.settings.lock().await;
            if let Some(configured) = settings
                .active_live_strategy_profiles
                .get(Strategy::DatedCarry.as_str())
            {
                configured.iter().any(|profile| {
                    profile.profile_id == report.profile_id
                        && profile.version == report.profile_version
                        && profile.profile_symbol == profile_symbol
                })
            } else {
                let profile =
                    live_profile_for_symbol(&settings, &profile_symbol, Strategy::DatedCarry);
                profile.profile_id == report.profile_id && profile.version == report.profile_version
            }
        };
        if !profile_is_current {
            return Err((
                StatusCode::CONFLICT,
                "Dated-carry profile is no longer active".into(),
            ));
        }
        return receive_dated_carry_signal(
            State(state),
            Json(DatedCarrySignal::Entry {
                emitted_at_ms: report.event_time_ms,
                candidate,
                worker_decision: report.decision,
            }),
        )
        .await;
    }
    if let Some(settlement) = report.dated_carry_settlement {
        let fallback_profile_symbol = settlement.spot_symbol.replace("USDT", "-USDT");
        let profile_symbol = report
            .signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("learning_profile_symbol"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&fallback_profile_symbol);
        let parameters = report
            .signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .map(|parameters| {
                parameters
                    .iter()
                    .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        if report.decision != strategy::LiveStrategyDecision::Exit
            || report.signal.strategy.as_deref() != Some(Strategy::DatedCarry.as_str())
            || report.symbol != settlement.spot_symbol
            || report.profile_id
                != strategy::profile_id(profile_symbol, Strategy::DatedCarry.as_str(), &parameters)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Dated-carry settlement decision is invalid".into(),
            ));
        }
        let profile_is_current = {
            let settings = state.settings.lock().await;
            if let Some(configured) = settings
                .active_live_strategy_profiles
                .get(Strategy::DatedCarry.as_str())
            {
                configured.iter().any(|profile| {
                    profile.profile_id == report.profile_id
                        && profile.version == report.profile_version
                        && profile.profile_symbol == profile_symbol
                })
            } else {
                let profile =
                    live_profile_for_symbol(&settings, profile_symbol, Strategy::DatedCarry);
                profile.profile_id == report.profile_id && profile.version == report.profile_version
            }
        };
        if !profile_is_current {
            return Err((
                StatusCode::CONFLICT,
                "Dated-carry settlement profile is stale".into(),
            ));
        }
        return receive_dated_carry_signal(
            State(state),
            Json(DatedCarrySignal::Settlement {
                emitted_at_ms: settlement.emitted_at_ms,
                spot_symbol: settlement.spot_symbol,
                future_symbol: settlement.future_symbol,
                expiry_ms: settlement.expiry_ms,
                spot_exit_twap: settlement.spot_exit_twap,
                settlement_price: settlement.settlement_price,
                settlement_source: settlement.settlement_source,
                worker_decision: report.decision,
            }),
        )
        .await;
    }
    Err((
        StatusCode::BAD_REQUEST,
        "Dated-carry report has no decision payload".into(),
    ))
}

fn migrate_closed_trade_metrics(settings: &mut Settings) {
    if settings.closed_trade_metrics_version >= 3 {
        return;
    }
    let count = settings.closed_trade_returns_percent.len();
    if settings.closed_trade_timestamps_ms.len() != count
        || settings.closed_trade_realized_usd.len() != count
        || settings.closed_trade_fees_usd.len() != count
    {
        settings.closed_trade_returns_percent.clear();
        settings.closed_trade_timestamps_ms.clear();
        settings.closed_trade_realized_usd.clear();
        settings.closed_trade_fees_usd.clear();
    } else {
        let mut valid = Vec::with_capacity(count);
        for index in 0..count {
            let return_percent = settings.closed_trade_returns_percent[index];
            let realized = settings.closed_trade_realized_usd[index];
            let fees = settings.closed_trade_fees_usd[index];
            if return_percent.is_finite()
                && return_percent >= -100.0
                && realized.is_finite()
                && fees.is_finite()
            {
                valid.push((
                    return_percent,
                    settings.closed_trade_timestamps_ms[index],
                    realized,
                    fees,
                ));
            }
        }
        settings.closed_trade_returns_percent = valid.iter().map(|item| item.0).collect();
        settings.closed_trade_timestamps_ms = valid.iter().map(|item| item.1).collect();
        settings.closed_trade_realized_usd = valid.iter().map(|item| item.2).collect();
        settings.closed_trade_fees_usd = valid.iter().map(|item| item.3).collect();
    }
    settings.closed_trade_metrics_version = 3;
}

#[cfg(test)]
mod closed_trade_metrics_migration_tests {
    use super::*;

    #[test]
    fn clears_legacy_outcomes_without_changing_ledger_pnl() {
        let mut settings = Settings {
            closed_trade_metrics_version: 0,
            closed_trade_returns_percent: vec![-2405.0],
            closed_trade_timestamps_ms: vec![123],
            closed_trade_realized_usd: vec![-0.89],
            closed_trade_fees_usd: vec![1.99],
            daily_realized_pnl: -11.15,
            ..Settings::default()
        };
        migrate_closed_trade_metrics(&mut settings);
        assert!(settings.closed_trade_returns_percent.is_empty());
        assert!(settings.closed_trade_timestamps_ms.is_empty());
        assert!(settings.closed_trade_realized_usd.is_empty());
        assert!(settings.closed_trade_fees_usd.is_empty());
        assert_eq!(settings.daily_realized_pnl, -11.15);
        migrate_closed_trade_metrics(&mut settings);
        assert_eq!(settings.closed_trade_metrics_version, 3);
    }

    #[test]
    fn scout_parameter_feedback_targets_only_the_tested_parameter_values() {
        let strategy = Strategy::from_name("live-momentum").unwrap();
        let specs = strategy.parameter_specs();
        let tested = specs[0].key.as_str();
        assert_eq!(
            scout_feedback_parameter_keys(strategy, Some(tested)),
            BTreeSet::from([tested.to_owned()])
        );
        assert!(scout_feedback_parameter_keys(strategy, None).is_empty());

        let full_profile = specs
            .iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let combination = BTreeMap::from([
            (specs[0].key.as_str().to_owned(), specs[0].default),
            (specs[1].key.as_str().to_owned(), specs[1].default),
        ]);
        let key = strategy::combination_key(&combination, &full_profile).unwrap();
        assert_eq!(
            scout_feedback_parameter_keys(strategy, Some(&key)),
            combination.keys().cloned().collect()
        );
    }
}
