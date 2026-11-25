// Keep daily-strategy pauses, while clearing legacy strategy-wide zeros for
// live strategies. Their risk now belongs to the entry's local context.
fn preserve_paused_strategy_multipliers(
    previous: &BTreeMap<String, f64>,
    mut incoming: BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    for (strategy, multiplier) in &mut incoming {
        if Strategy::from_name(strategy).is_some_and(|kind| kind.is_live_insight()) {
            *multiplier = 1.0;
        }
    }
    for (strategy, multiplier) in previous {
        if Strategy::from_name(strategy).is_some_and(|kind| kind.is_live_insight()) {
            incoming.insert(strategy.clone(), 1.0);
            continue;
        }
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
    let exit_protection_policies =
        apply_exit_protection_policies(snapshot.exit_protection_policies, current_time_ms());
    snapshot
        .profiles
        .retain(|profile| strategy::is_autotrader_symbol_allowed(&profile.symbol));
    snapshot
        .scout_parameter_variants
        .retain(|variant| strategy::is_autotrader_symbol_allowed(&variant.symbol));
    let allowed_profile_ids = snapshot.registered_profile_ids.clone();
    snapshot.walk_forward_evidence.retain(|key, _| {
        key.split('|').next().is_some_and(|identity| {
            strategy::is_autotrader_symbol_allowed(identity)
                || allowed_profile_ids.contains(identity)
        })
    });
    snapshot.recent_profile_context_returns.retain(|key, _| {
        key.split('|')
            .next()
            .is_some_and(|profile_id| allowed_profile_ids.contains(profile_id))
    });
    snapshot.missed_opportunity_bands.retain(|band| {
        band.symbol
            .as_deref()
            .is_none_or(strategy::is_autotrader_symbol_allowed)
    });
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
    if settings.learned_exit_protection_policies != exit_protection_policies {
        settings.learned_exit_protection_policies = exit_protection_policies;
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
    settings
        .profile_registration_enqueued_at_ms
        .insert(profile.profile_id.clone(), current_time_ms());
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

fn next_feedback_event(feedback: &[FeedbackEvent], missed_lane: bool) -> Option<&FeedbackEvent> {
    let is_missed =
        |event: &&FeedbackEvent| matches!(event, FeedbackEvent::MissedOpportunityCandidate { .. });
    if missed_lane {
        feedback.iter().find(is_missed)
    } else {
        feedback.iter().find(|event| !is_missed(event))
    }
}

const FEEDBACK_OUTBOX_CAPACITY: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
enum MissedFeedbackEnqueueResult {
    Queued,
    EvictedOldestMissed,
    CoalescedExistingCase,
    Duplicate,
    Throttled,
    DroppedOutcomeCapacity,
}

fn missed_opportunity_case_key(event: &FeedbackEvent) -> Option<String> {
    let FeedbackEvent::MissedOpportunityCandidate {
        profile_id,
        direction,
        learning_context,
        ..
    } = event
    else {
        return None;
    };
    Some(format!("{profile_id}|{direction}|{learning_context}"))
}

fn missed_opportunity_case_cooldown_ms(settings: &Settings, event: &FeedbackEvent) -> u64 {
    const UNDER_COVERED_COOLDOWN_MS: u64 = 5 * 60 * 1_000;
    const COVERED_COOLDOWN_MS: u64 = 30 * 60 * 1_000;
    let FeedbackEvent::MissedOpportunityCandidate {
        profile_id,
        parameters,
        learning_context,
        ..
    } = event
    else {
        return UNDER_COVERED_COOLDOWN_MS;
    };
    let regime = learning_context
        .split(':')
        .nth(1)
        .filter(|regime| matches!(*regime, "range" | "trend" | "volatile" | "mixed"));
    let evidence_key = regime.map_or_else(
        || profile_id.clone(),
        |regime| format!("{profile_id}|regime:{regime}"),
    );
    let Some(evidence) = settings.learned_walk_forward_evidence.get(&evidence_key) else {
        return UNDER_COVERED_COOLDOWN_MS;
    };
    let minimum_samples = parameters
        .get(StrategyParameterKey::ScoutMinimumWalkForwardSamples.as_str())
        .copied()
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value.round() as usize)
        .unwrap_or(10);
    let maximum_age_ms = parameters
        .get(StrategyParameterKey::ScoutMaximumEvidenceAgeSeconds.as_str())
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| (value as u64).saturating_mul(1_000))
        .unwrap_or(24 * 60 * 60 * 1_000);
    if evidence.sample_count < minimum_samples
        || evidence.last_sample_timestamp_ms == 0
        || current_time_ms().saturating_sub(evidence.last_sample_timestamp_ms) > maximum_age_ms
    {
        UNDER_COVERED_COOLDOWN_MS
    } else {
        COVERED_COOLDOWN_MS
    }
}

fn enqueue_scheduled_missed_opportunity_feedback(
    settings: &mut Settings,
    event: FeedbackEvent,
    now_ms: u64,
) -> MissedFeedbackEnqueueResult {
    const MAX_SCHEDULED_CASES: usize = 20_000;
    let Some(case_key) = missed_opportunity_case_key(&event) else {
        return MissedFeedbackEnqueueResult::Duplicate;
    };
    let cooldown_ms = missed_opportunity_case_cooldown_ms(settings, &event);
    if settings
        .missed_opportunity_last_enqueued_at_ms
        .get(&case_key)
        .is_some_and(|last| now_ms.saturating_sub(*last) < cooldown_ms)
    {
        return MissedFeedbackEnqueueResult::Throttled;
    }
    let result = enqueue_missed_opportunity_feedback(&mut settings.feedback_outbox, event);
    if matches!(
        result,
        MissedFeedbackEnqueueResult::Queued
            | MissedFeedbackEnqueueResult::EvictedOldestMissed
            | MissedFeedbackEnqueueResult::CoalescedExistingCase
    ) {
        settings
            .missed_opportunity_last_enqueued_at_ms
            .insert(case_key, now_ms);
        while settings.missed_opportunity_last_enqueued_at_ms.len() > MAX_SCHEDULED_CASES {
            let Some(oldest) = settings
                .missed_opportunity_last_enqueued_at_ms
                .iter()
                .min_by_key(|(_, timestamp)| *timestamp)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            settings
                .missed_opportunity_last_enqueued_at_ms
                .remove(&oldest);
        }
    }
    result
}

fn enqueue_missed_opportunity_feedback(
    outbox: &mut Vec<FeedbackEvent>,
    event: FeedbackEvent,
) -> MissedFeedbackEnqueueResult {
    let FeedbackEvent::MissedOpportunityCandidate { event_id, .. } = &event else {
        return MissedFeedbackEnqueueResult::Duplicate;
    };
    if outbox
        .iter()
        .any(|pending| feedback_event_id(pending) == event_id)
    {
        return MissedFeedbackEnqueueResult::Duplicate;
    }
    let case_key = missed_opportunity_case_key(&event);
    if let Some(index) = outbox.iter().position(|pending| {
        matches!(pending, FeedbackEvent::MissedOpportunityCandidate { .. })
            && missed_opportunity_case_key(pending) == case_key
    }) {
        outbox[index] = event;
        return MissedFeedbackEnqueueResult::CoalescedExistingCase;
    }
    let mut evicted = false;
    if outbox.len() >= FEEDBACK_OUTBOX_CAPACITY {
        // Keep the newest missed-opportunity observations useful while never
        // evicting acknowledged-value outcomes (trade and scout feedback).
        let Some(oldest_missed) = outbox.iter().position(|pending| {
            matches!(pending, FeedbackEvent::MissedOpportunityCandidate { .. })
        }) else {
            return MissedFeedbackEnqueueResult::DroppedOutcomeCapacity;
        };
        outbox.remove(oldest_missed);
        evicted = true;
    }
    outbox.push(event);
    if evicted {
        MissedFeedbackEnqueueResult::EvictedOldestMissed
    } else {
        MissedFeedbackEnqueueResult::Queued
    }
}

#[cfg(test)]
mod strategy_pause_snapshot_tests {
    use super::*;

    #[test]
    fn live_strategy_zeros_are_cleared_from_learning_snapshots() {
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
        assert_eq!(merged["live-trade-flow"], 1.0);
        assert_eq!(
            preserve_paused_strategy_multipliers(&prior, BTreeMap::new())["live-momentum"],
            1.0
        );
        assert_eq!(
            preserve_paused_strategy_multipliers(&merged, BTreeMap::new())["live-momentum"],
            1.0
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
        assert!(!settings
            .registered_profile_ids
            .contains(&profile.profile_id));
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
            schema_version: None,
            trade_id: None,
            opportunity_id: None,
            episode_id: None,
            profile_hash: None,
            profile_version: Some(4),
            execution_mode: None,
            entry_policy: None,
            source: None,
            entry_profile_id: None,
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
            schema_version: None,
            trade_id: None,
            opportunity_id: None,
            episode_id: None,
            profile_hash: None,
            profile_version: None,
            execution_mode: None,
            entry_policy: None,
            source: None,
            entry_profile_id: None,
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
            next_feedback_event(&[missed.clone(), trade.clone()], false),
            Some(FeedbackEvent::TradeOutcome { .. })
        ));
        assert!(matches!(
            next_feedback_event(&[missed.clone(), trade.clone()], true),
            Some(FeedbackEvent::MissedOpportunityCandidate { .. })
        ));
        assert!(matches!(
            next_feedback_event(&[missed.clone()], true),
            Some(FeedbackEvent::MissedOpportunityCandidate { .. })
        ));
        let mut later_missed = missed.clone();
        if let FeedbackEvent::MissedOpportunityCandidate { event_id, .. } = &mut later_missed {
            *event_id = "missed-later".into();
        }
        let mut feedback = vec![missed.clone(), trade.clone(), later_missed];
        assert_eq!(
            feedback_event_id(next_feedback_event(&feedback, true).unwrap()),
            "missed"
        );
        assert!(rotate_feedback_event_to_back(&mut feedback, "missed"));
        assert_eq!(
            feedback_event_id(next_feedback_event(&feedback, true).unwrap()),
            "missed-later"
        );
        assert!(matches!(
            next_feedback_event(&feedback, false),
            Some(FeedbackEvent::TradeOutcome { .. })
        ));
        assert!(next_feedback_event(&[], false).is_none());
    }

    #[test]
    fn queue_lifecycle_stats_count_attempts_acknowledgements_rejections_and_drops() {
        let mut stats = LearningFeedbackQueueStats::default();
        stats.attempts = 3;
        apply_feedback_delivery_stats(&mut stats, FeedbackDeliveryResult::Acknowledged, 10);
        apply_feedback_delivery_stats(
            &mut stats,
            FeedbackDeliveryResult::Rejected {
                status: 422,
                reason: "invalid sample".into(),
            },
            20,
        );
        apply_feedback_delivery_stats(
            &mut stats,
            FeedbackDeliveryResult::TransientError {
                reason: "timeout".into(),
            },
            30,
        );
        apply_feedback_terminal_drop_stats(
            &mut stats,
            "evicted oldest",
            FeedbackTerminalDropKind::EvictedOldest,
            40,
        );
        assert_eq!(stats.acknowledged, 1);
        assert_eq!(stats.rejected, 1);
        assert_eq!(stats.transient_errors, 1);
        assert_eq!(stats.terminal_drops, 1);
        assert_eq!(stats.evicted_oldest, 1);
        assert_eq!(stats.latest_reason.as_deref(), Some("evicted oldest"));
        assert_eq!(stats.latest_reason_at_ms, Some(40));

        let mut permanent = LearningFeedbackQueueStats::default();
        apply_feedback_terminal_drop_stats(
            &mut permanent,
            "permanent HTTP 422 rejection",
            FeedbackTerminalDropKind::PermanentRejection,
            50,
        );
        assert_eq!(permanent.terminal_drops, 1);
        assert_eq!(permanent.permanent_rejections, 1);
        assert_eq!(permanent.outcome_capacity_rejections, 0);
    }

    #[test]
    fn permanent_feedback_rejections_are_skipped_without_losing_trade_outcomes() {
        assert!(is_permanent_learning_rejection(400));
        assert!(is_permanent_learning_rejection(413));
        assert!(is_permanent_learning_rejection(422));
        assert!(!is_permanent_learning_rejection(429));
        assert!(!is_permanent_learning_rejection(503));

        let mut feedback = vec![
            FeedbackEvent::MissedOpportunityCandidate {
                event_id: "bad-missed".into(),
                observed_at_ms: 1,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "profile".into(),
                parameters: BTreeMap::new(),
                direction: "bullish".into(),
                strength: 0.5,
                score: 0.5,
                learning_context: "long:trend:BTC-USDT:live-momentum".into(),
            },
            FeedbackEvent::TradeOutcome {
                event_id: "trade-outcome".into(),
                schema_version: None,
                trade_id: None,
                opportunity_id: None,
                episode_id: None,
                profile_hash: None,
                profile_version: None,
                execution_mode: None,
                entry_policy: None,
                source: None,
                entry_profile_id: None,
                observed_at_ms: 2,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "profile".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: false,
                realized_pnl_usd: 1.0,
                net_return_percent: 0.1,
                threshold_observations: vec![],
                replay_context: None,
            },
        ];

        feedback.swap(0, 1);
        assert_eq!(feedback_event_id(&feedback[0]), "trade-outcome");
        assert!(rotate_feedback_event_to_back(
            &mut feedback,
            "trade-outcome"
        ));
        assert_eq!(feedback_event_id(&feedback[0]), "bad-missed");
        assert_eq!(feedback_event_id(&feedback[1]), "trade-outcome");
    }

    #[test]
    fn missed_opportunity_retention_evicts_oldest_missed_without_evicting_outcomes() {
        let missed = |id: String| FeedbackEvent::MissedOpportunityCandidate {
            event_id: id.clone(),
            observed_at_ms: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            direction: "bullish".into(),
            strength: 0.5,
            score: 0.5,
            learning_context: format!("long:trend:BTC-USDT:live-momentum:{id}"),
        };
        let mut outbox = (0..FEEDBACK_OUTBOX_CAPACITY)
            .map(|index| missed(format!("missed-{index}")))
            .collect::<Vec<_>>();
        assert_eq!(
            enqueue_missed_opportunity_feedback(&mut outbox, missed("newest".into())),
            MissedFeedbackEnqueueResult::EvictedOldestMissed
        );
        assert_eq!(outbox.len(), FEEDBACK_OUTBOX_CAPACITY);
        assert!(!outbox
            .iter()
            .any(|event| feedback_event_id(event) == "missed-0"));
        assert!(outbox
            .iter()
            .any(|event| feedback_event_id(event) == "newest"));

        outbox.clear();
        outbox.resize_with(FEEDBACK_OUTBOX_CAPACITY, || FeedbackEvent::TradeOutcome {
            event_id: "trade".into(),
            schema_version: None,
            trade_id: None,
            opportunity_id: None,
            episode_id: None,
            profile_hash: None,
            profile_version: None,
            execution_mode: None,
            entry_policy: None,
            source: None,
            entry_profile_id: None,
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 0.1,
            threshold_observations: vec![],
            replay_context: None,
        });
        assert_eq!(
            enqueue_missed_opportunity_feedback(&mut outbox, missed("cannot-fit".into())),
            MissedFeedbackEnqueueResult::DroppedOutcomeCapacity
        );
        assert_eq!(outbox.len(), FEEDBACK_OUTBOX_CAPACITY);
        assert!(outbox
            .iter()
            .all(|event| matches!(event, FeedbackEvent::TradeOutcome { .. })));
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
        latest_worker_diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
        seen_worker_decision_events: Arc::new(Mutex::new(BTreeMap::new())),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        activity_snapshot: Arc::new(Mutex::new(None)),
        settings_persist_notify: Arc::new(Notify::new()),
        settings_persist_write_lock: Arc::new(Mutex::new(())),
        settings_persist_revision: Arc::new(AtomicU64::new(0)),
        settings_persist_durable_revision: Arc::new(AtomicU64::new(0)),
        settings_persist_durable_waiters: Arc::new(AtomicU64::new(0)),
        settings_persist_batch_notify: Arc::new(Notify::new()),
        durable_entry_reservation_batch: Arc::new(Mutex::new(Vec::new())),
        durable_entry_reservation_batch_scheduled: Arc::new(AtomicBool::new(false)),
        durable_feedback_event_ids: Arc::new(Mutex::new(BTreeSet::new())),
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
    start_activity_snapshot_worker(&state);
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
            get(get_dated_carry)
                .put(update_dated_carry)
                .post(close_dated_carry),
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
            "/internal/strategy/daily-diagnostics",
            axum::routing::post(update_daily_strategy_diagnostics),
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
        .route("/internal/monitoring-metrics", get(get_monitoring_metrics))
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
        .route(
            "/internal/learning-feedback-queues",
            get(get_learning_feedback_queue_status),
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
        if let Some(evidence) = settings
            .learned_walk_forward_evidence
            .get(&profile.profile_id)
        {
            totals.2 = totals.2.saturating_add(evidence.sample_count);
            totals.3 = totals.3.saturating_add(
                (evidence.positive_net_rate.unwrap_or_default() * evidence.sample_count as f64)
                    .round() as usize,
            );
            totals.4 += evidence.average_net_return_percent.unwrap_or_default()
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

async fn get_learning_feedback_queue_status(
    State(state): State<AutoTraderState>,
) -> axum::Json<serde_json::Value> {
    let settings = state.settings.lock().await;
    let now_ms = current_time_ms();
    let ages = |class: &str| {
        settings
            .feedback_outbox
            .iter()
            .filter_map(|event| {
                let event_class = match event {
                    FeedbackEvent::MissedOpportunityCandidate { .. } => "missed_opportunity",
                    FeedbackEvent::ScoutOutcome { .. } => "scout_outcome",
                    FeedbackEvent::TradeOutcome { .. } => "trade_outcome",
                };
                if (class == "trade_and_scout" && event_class == "missed_opportunity")
                    || (class != "trade_and_scout" && class != event_class)
                {
                    return None;
                }
                let observed_at = match event {
                    FeedbackEvent::MissedOpportunityCandidate { observed_at_ms, .. }
                    | FeedbackEvent::ScoutOutcome { observed_at_ms, .. }
                    | FeedbackEvent::TradeOutcome { observed_at_ms, .. } => *observed_at_ms,
                };
                Some(now_ms.saturating_sub(observed_at))
            })
            .max()
    };
    let execution_oldest_age_ms = settings
        .execution_feedback_outbox
        .iter()
        .map(|event| now_ms.saturating_sub(event.observed_at_ms))
        .max();
    axum::Json(serde_json::json!({
        "feedback_capacity": FEEDBACK_OUTBOX_CAPACITY,
        "feedback_depth": settings.feedback_outbox.len(),
        "trade_and_scout_depth": settings.feedback_outbox.iter().filter(|event| !matches!(event, FeedbackEvent::MissedOpportunityCandidate { .. })).count(),
        "trade_and_scout_oldest_age_ms": ages("trade_and_scout"),
        "trade_outcome_depth": settings.feedback_outbox.iter().filter(|event| matches!(event, FeedbackEvent::TradeOutcome { .. })).count(),
        "trade_outcome_oldest_age_ms": ages("trade_outcome"),
        "scout_outcome_depth": settings.feedback_outbox.iter().filter(|event| matches!(event, FeedbackEvent::ScoutOutcome { .. })).count(),
        "scout_outcome_oldest_age_ms": ages("scout_outcome"),
        "missed_opportunity_depth": settings.feedback_outbox.iter().filter(|event| matches!(event, FeedbackEvent::MissedOpportunityCandidate { .. })).count(),
        "missed_opportunity_oldest_age_ms": ages("missed_opportunity"),
        "execution_depth": settings.execution_feedback_outbox.len(),
        "execution_oldest_age_ms": execution_oldest_age_ms,
        "profile_registration_depth": settings.profile_registration_outbox.len(),
        "profile_registration_oldest_age_ms": settings.profile_registration_outbox.iter().filter_map(|item| settings.profile_registration_enqueued_at_ms.get(&item.profile.profile_id)).map(|queued_at| now_ms.saturating_sub(*queued_at)).max(),
        "profile_registration_missing_enqueue_time_count": settings.profile_registration_outbox.iter().filter(|item| !settings.profile_registration_enqueued_at_ms.contains_key(&item.profile.profile_id)).count(),
        "lifecycle": settings.learning_feedback_stats.clone(),
    }))
}

enum FeedbackDeliveryResult {
    Acknowledged,
    Rejected { status: u16, reason: String },
    TransientError { reason: String },
}

#[derive(Debug, PartialEq, Eq)]
enum FeedbackDeliveryOutcome {
    Acknowledged,
    PermanentRejection { status: u16, reason: String },
    Retained,
}

fn is_permanent_learning_rejection(status: u16) -> bool {
    matches!(status, 400 | 413 | 422)
}

fn bounded_feedback_reason(reason: String) -> String {
    reason.chars().take(512).collect()
}

fn apply_feedback_delivery_stats(
    stats: &mut LearningFeedbackQueueStats,
    result: FeedbackDeliveryResult,
    now_ms: u64,
) {
    match result {
        FeedbackDeliveryResult::Acknowledged => {
            stats.acknowledged = stats.acknowledged.saturating_add(1);
        }
        FeedbackDeliveryResult::Rejected { status, reason } => {
            stats.rejected = stats.rejected.saturating_add(1);
            stats.latest_reason = Some(bounded_feedback_reason(format!("HTTP {status}: {reason}")));
            stats.latest_reason_at_ms = Some(now_ms);
        }
        FeedbackDeliveryResult::TransientError { reason } => {
            stats.transient_errors = stats.transient_errors.saturating_add(1);
            stats.latest_reason = Some(bounded_feedback_reason(reason));
            stats.latest_reason_at_ms = Some(now_ms);
        }
    }
}

async fn record_feedback_attempt(state: &AutoTraderState, class: &str) {
    let mut settings = state.settings.lock().await;
    let stats = settings
        .learning_feedback_stats
        .entry(class.to_owned())
        .or_default();
    stats.attempts = stats.attempts.saturating_add(1);
    stats.last_attempt_at_ms = Some(current_time_ms());
    drop(settings);
    let _ = request_settings_persist(state);
}

async fn record_feedback_delivery(
    state: &AutoTraderState,
    class: &str,
    result: FeedbackDeliveryResult,
) {
    let mut settings = state.settings.lock().await;
    let stats = settings
        .learning_feedback_stats
        .entry(class.to_owned())
        .or_default();
    apply_feedback_delivery_stats(stats, result, current_time_ms());
    drop(settings);
    let _ = request_settings_persist(state);
}

async fn record_feedback_terminal_drop(
    state: &AutoTraderState,
    class: &str,
    reason: &str,
    drop_kind: FeedbackTerminalDropKind,
) {
    let mut settings = state.settings.lock().await;
    let stats = settings
        .learning_feedback_stats
        .entry(class.to_owned())
        .or_default();
    apply_feedback_terminal_drop_stats(stats, reason, drop_kind, current_time_ms());
    drop(settings);
    let _ = request_settings_persist(state);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FeedbackTerminalDropKind {
    EvictedOldest,
    OutcomeCapacity,
    PermanentRejection,
}

fn apply_feedback_terminal_drop_stats(
    stats: &mut LearningFeedbackQueueStats,
    reason: &str,
    drop_kind: FeedbackTerminalDropKind,
    now_ms: u64,
) {
    stats.terminal_drops = stats.terminal_drops.saturating_add(1);
    match drop_kind {
        FeedbackTerminalDropKind::EvictedOldest => {
            stats.evicted_oldest = stats.evicted_oldest.saturating_add(1);
        }
        FeedbackTerminalDropKind::OutcomeCapacity => {
            stats.outcome_capacity_rejections = stats.outcome_capacity_rejections.saturating_add(1);
        }
        FeedbackTerminalDropKind::PermanentRejection => {
            stats.permanent_rejections = stats.permanent_rejections.saturating_add(1);
        }
    }
    stats.latest_reason = Some(bounded_feedback_reason(reason.to_owned()));
    stats.latest_reason_at_ms = Some(now_ms);
}

async fn deliver_learning_payload(
    state: &AutoTraderState,
    class: &str,
    path: &str,
    payload: serde_json::Value,
) -> FeedbackDeliveryOutcome {
    record_feedback_attempt(state, class).await;
    match crate::service_rpc::learning_request(
        strategy::ScoutWorkerHttpMethod::Post,
        path,
        Some(payload),
    )
    .await
    {
        Ok(response) if (200..300).contains(&response.status) => {
            record_feedback_delivery(state, class, FeedbackDeliveryResult::Acknowledged).await;
            FeedbackDeliveryOutcome::Acknowledged
        }
        Ok(response) => {
            let status = response.status;
            let reason = response.body.to_string();
            record_feedback_delivery(
                state,
                class,
                FeedbackDeliveryResult::Rejected {
                    status,
                    reason: reason.clone(),
                },
            )
            .await;
            crate::service_log!(
                "WARN",
                "Learning rejected {class} with status {status}: {reason}"
            );
            if is_permanent_learning_rejection(status) {
                FeedbackDeliveryOutcome::PermanentRejection { status, reason }
            } else {
                FeedbackDeliveryOutcome::Retained
            }
        }
        Err(error) => {
            let reason = error.to_string();
            record_feedback_delivery(
                state,
                class,
                FeedbackDeliveryResult::TransientError {
                    reason: reason.clone(),
                },
            )
            .await;
            crate::service_log!("WARN", "Learning delivery for {class} failed: {reason}");
            FeedbackDeliveryOutcome::Retained
        }
    }
}

fn feedback_event_id(event: &FeedbackEvent) -> &str {
    match event {
        FeedbackEvent::ScoutOutcome { event_id, .. }
        | FeedbackEvent::TradeOutcome { event_id, .. }
        | FeedbackEvent::MissedOpportunityCandidate { event_id, .. } => event_id,
    }
}

async fn ensure_feedback_event_durable(
    state: &AutoTraderState,
    event_id: &str,
) -> Result<bool, (StatusCode, String)> {
    if state
        .durable_feedback_event_ids
        .lock()
        .await
        .contains(event_id)
    {
        return Ok(false);
    }
    request_settings_persist(state)?;
    persist_settings_snapshot_durable(state).await?;
    state
        .durable_feedback_event_ids
        .lock()
        .await
        .insert(event_id.to_owned());
    Ok(true)
}

#[cfg(test)]
mod durable_feedback_event_tests {
    use super::*;

    #[tokio::test]
    async fn canonical_feedback_is_committed_once_before_first_delivery() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "auto-trader-durable-feedback-{}-{nonce}.sqlite",
            std::process::id(),
        ));
        let mut state = benchmark_snapshot_state(Settings::default());
        state.settings_path = path.clone();
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:durable-before-send".into(),
            schema_version: Some(1),
            trade_id: Some("trade-v1".into()),
            opportunity_id: Some("opportunity-v1".into()),
            episode_id: Some("episode-v1".into()),
            profile_hash: None,
            profile_version: Some(4),
            execution_mode: Some("paper".into()),
            entry_policy: Some("strict".into()),
            source: Some("auto_trader_entry_reservation".into()),
            observed_at_ms: 2,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "derived-profile".into(),
            entry_profile_id: Some("entry-profile".into()),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: vec![],
            replay_context: None,
        };
        state.settings.lock().await.feedback_outbox.push(event);

        assert!(
            ensure_feedback_event_durable(&state, "trade:durable-before-send")
                .await
                .unwrap()
        );
        assert!(
            !ensure_feedback_event_durable(&state, "trade:durable-before-send")
                .await
                .unwrap()
        );
        let restored = read_settings_database(&path).unwrap().unwrap();
        let Some(FeedbackEvent::TradeOutcome {
            trade_id,
            opportunity_id,
            episode_id,
            profile_version,
            entry_profile_id,
            ..
        }) = restored.feedback_outbox.first()
        else {
            panic!("canonical feedback was not committed before delivery")
        };
        assert_eq!(trade_id.as_deref(), Some("trade-v1"));
        assert_eq!(opportunity_id.as_deref(), Some("opportunity-v1"));
        assert_eq!(episode_id.as_deref(), Some("episode-v1"));
        assert_eq!(*profile_version, Some(4));
        assert_eq!(entry_profile_id.as_deref(), Some("entry-profile"));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}

fn rotate_feedback_event_to_back(feedback: &mut Vec<FeedbackEvent>, event_id: &str) -> bool {
    let Some(index) = feedback
        .iter()
        .position(|event| feedback_event_id(event) == event_id)
    else {
        return false;
    };
    if index + 1 < feedback.len() {
        let event = feedback.remove(index);
        feedback.push(event);
    }
    true
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
        let mut service_lane = 0u8;
        loop {
            ticker.tick().await;
            let lane = service_lane;
            service_lane = (service_lane + 1) % 4;
            if lane == 0 {
                let registration = {
                    let settings = publisher_state.settings.lock().await;
                    next_profile_registration_for_feedback(
                        &settings.profile_registration_outbox,
                        None,
                    )
                };
                if let Some(registration) = registration {
                    let delivery = deliver_learning_payload(
                        &publisher_state,
                        "profile_registration",
                        "/profiles",
                        serde_json::to_value(&registration).unwrap_or(serde_json::Value::Null),
                    )
                    .await;
                    if delivery == FeedbackDeliveryOutcome::Acknowledged {
                        let profile_id = registration.profile.profile_id;
                        let mut settings = publisher_state.settings.lock().await;
                        settings
                            .profile_registration_outbox
                            .retain(|pending| pending.profile.profile_id != profile_id);
                        settings
                            .profile_registration_enqueued_at_ms
                            .remove(&profile_id);
                        settings.registered_profile_ids.insert(profile_id);
                        drop(settings);
                        if let Err(error) = request_settings_persist(&publisher_state) {
                            crate::service_log!(
                                "WARN",
                                "Could not persist profile-registration acknowledgement: {}",
                                error.1
                            );
                        }
                    } else if matches!(delivery, FeedbackDeliveryOutcome::PermanentRejection { .. })
                    {
                        let mut settings = publisher_state.settings.lock().await;
                        if let Some(index) =
                            settings
                                .profile_registration_outbox
                                .iter()
                                .position(|pending| {
                                    pending.profile.profile_id == registration.profile.profile_id
                                })
                        {
                            if index + 1 < settings.profile_registration_outbox.len() {
                                let rejected = settings.profile_registration_outbox.remove(index);
                                settings.profile_registration_outbox.push(rejected);
                                drop(settings);
                                let _ = request_settings_persist(&publisher_state);
                            }
                        }
                    }
                }
            }
            if lane == 1 || lane == 3 {
                let event = {
                    let settings = publisher_state.settings.lock().await;
                    next_feedback_event(&settings.feedback_outbox, lane == 3).cloned()
                };
                if let Some(event) = event {
                    let event_id = feedback_event_id(&event).to_owned();
                    if let Err(error) =
                        ensure_feedback_event_durable(&publisher_state, &event_id).await
                    {
                        crate::service_log!(
                            "WARN",
                            "Could not durably persist Learning feedback before delivery: {}",
                            error.1
                        );
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                    let class = match &event {
                        FeedbackEvent::MissedOpportunityCandidate { .. } => "missed_opportunity",
                        FeedbackEvent::ScoutOutcome { .. } => "scout_outcome",
                        FeedbackEvent::TradeOutcome { .. } => "trade_outcome",
                    };
                    match deliver_learning_payload(
                        &publisher_state,
                        class,
                        "/feedback",
                        serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                    )
                    .await
                    {
                        FeedbackDeliveryOutcome::Acknowledged => {
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
                            publisher_state
                                .durable_feedback_event_ids
                                .lock()
                                .await
                                .remove(&event_id);
                        }
                        FeedbackDeliveryOutcome::PermanentRejection { status, reason } => {
                            let mut settings = publisher_state.settings.lock().await;
                            if matches!(&event, FeedbackEvent::MissedOpportunityCandidate { .. }) {
                                settings
                                    .feedback_outbox
                                    .retain(|pending| feedback_event_id(pending) != event_id);
                                drop(settings);
                                publisher_state
                                    .durable_feedback_event_ids
                                    .lock()
                                    .await
                                    .remove(&event_id);
                                record_feedback_terminal_drop(
                                    &publisher_state,
                                    class,
                                    &format!(
                                        "permanent HTTP {status} rejection for {event_id}: {reason}"
                                    ),
                                    FeedbackTerminalDropKind::PermanentRejection,
                                )
                                .await;
                            } else {
                                rotate_feedback_event_to_back(
                                    &mut settings.feedback_outbox,
                                    &event_id,
                                );
                                drop(settings);
                            }
                            let _ = request_settings_persist(&publisher_state);
                        }
                        FeedbackDeliveryOutcome::Retained => {
                            // A transient 5xx/queue-full response must not pin
                            // this event at the head of its lane forever.
                            // Rotate it so later independent candidates get a
                            // delivery attempt when Learning capacity returns.
                            let event_id = feedback_event_id(&event).to_owned();
                            let mut settings = publisher_state.settings.lock().await;
                            let rotated = rotate_feedback_event_to_back(
                                &mut settings.feedback_outbox,
                                &event_id,
                            );
                            drop(settings);
                            if rotated {
                                let _ = request_settings_persist(&publisher_state);
                            }
                        }
                    }
                }
            }
            if lane == 2 {
                let execution_event = publisher_state
                    .settings
                    .lock()
                    .await
                    .execution_feedback_outbox
                    .first()
                    .cloned();
                if let Some(event) = execution_event {
                    match deliver_learning_payload(
                        &publisher_state,
                        "execution_outcome",
                        "/execution-outcomes",
                        serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                    )
                    .await
                    {
                        FeedbackDeliveryOutcome::Acknowledged => {
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
                        FeedbackDeliveryOutcome::PermanentRejection { .. } => {
                            let event_id = event.event_id.clone();
                            let mut settings = publisher_state.settings.lock().await;
                            if let Some(index) = settings
                                .execution_feedback_outbox
                                .iter()
                                .position(|pending| pending.event_id == event_id)
                            {
                                if index + 1 < settings.execution_feedback_outbox.len() {
                                    let rejected = settings.execution_feedback_outbox.remove(index);
                                    settings.execution_feedback_outbox.push(rejected);
                                }
                            }
                            drop(settings);
                            let _ = request_settings_persist(&publisher_state);
                        }
                        FeedbackDeliveryOutcome::Retained => {}
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
        report.profile_id.clone(),
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
    let decision_key = (
        report.symbol.clone(),
        report.strategy.as_str().to_owned(),
        report.profile_id.clone(),
    );
    let signal_key = (report.symbol.clone(), report.strategy.as_str().to_owned());
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
        let mut diagnostics = state.latest_worker_diagnostics.lock().await;
        if diagnostics
            .get(&decision_key)
            .is_some_and(|diagnostic| diagnostic.event_time_ms <= report.event_time_ms)
        {
            diagnostics.remove(&decision_key);
        }
    }
    {
        let mut signals = state.latest_live_strategy_signals.lock().await;
        if signals.len() >= 512 && !signals.contains_key(&signal_key) {
            if let Some(oldest) = signals.keys().next().cloned() {
                signals.remove(&oldest);
            }
        }
        signals.insert(signal_key, vec![report.signal.clone()]);
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
            && strategy::is_autotrader_symbol_allowed(&report.symbol)
            && matches!(report.signal.direction.as_str(), "bullish" | "bearish")
            && report.signal.strength.is_finite()
            && report.signal.score.is_finite()
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
            match enqueue_scheduled_missed_opportunity_feedback(
                &mut settings,
                feedback,
                current_time_ms(),
            ) {
                MissedFeedbackEnqueueResult::Queued => {
                    drop(settings);
                    let _ = request_settings_persist(&state);
                }
                MissedFeedbackEnqueueResult::CoalescedExistingCase => {
                    drop(settings);
                    let _ = request_settings_persist(&state);
                }
                MissedFeedbackEnqueueResult::EvictedOldestMissed => {
                    drop(settings);
                    record_feedback_terminal_drop(
                        &state,
                        "missed_opportunity",
                        "Evicted oldest missed-opportunity candidate at queue capacity",
                        FeedbackTerminalDropKind::EvictedOldest,
                    )
                    .await;
                    let _ = request_settings_persist(&state);
                }
                MissedFeedbackEnqueueResult::DroppedOutcomeCapacity => {
                    drop(settings);
                    record_feedback_terminal_drop(
                        &state,
                        "missed_opportunity",
                        "Rejected candidate: feedback outbox is full of trade/scout outcomes",
                        FeedbackTerminalDropKind::OutcomeCapacity,
                    )
                    .await;
                    crate::service_log!(
                        "WARN",
                        "Dropped missed-opportunity feedback {}: feedback outbox is full of trade/scout outcomes",
                        event_id
                    );
                }
                MissedFeedbackEnqueueResult::Duplicate | MissedFeedbackEnqueueResult::Throttled => {
                }
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

async fn receive_strategy_worker_diagnostic(
    State(state): State<AutoTraderState>,
    Json(diagnostic): Json<strategy::LiveStrategyWorkerDiagnostic>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !diagnostic.strategy.is_live_insight()
        || !strategy::is_autotrader_symbol_allowed(&diagnostic.symbol)
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    if diagnostic.contract_version != strategy::LIVE_EVALUATION_CONTRACT_VERSION
        || diagnostic.event_id.is_empty()
        || diagnostic.reason.trim().is_empty()
        || diagnostic.reason.len() > 512
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid live worker diagnostic".into(),
        ));
    }
    let now_ms = current_time_ms();
    if now_ms.saturating_sub(diagnostic.event_time_ms) > 10_000
        || diagnostic.event_time_ms > now_ms + 5_000
    {
        return Err((
            StatusCode::CONFLICT,
            "Live worker diagnostic is stale".into(),
        ));
    }
    let (enabled, profile_is_known) = {
        let settings = state.settings.lock().await;
        let enabled = settings.strategies.contains(&diagnostic.strategy);
        let profile_is_known = diagnostic.profile_id.as_ref().is_none_or(|profile_id| {
            build_live_strategy_profile_set(
                &settings,
                state.enabled.load(Ordering::Relaxed),
                diagnostic.strategy,
            )
            .profiles
            .iter()
            .any(|profile| &profile.profile_id == profile_id)
        });
        (enabled, profile_is_known)
    };
    if !enabled {
        return Ok(StatusCode::NO_CONTENT);
    }
    if !profile_is_known {
        return Err((
            StatusCode::CONFLICT,
            "Live worker diagnostic profile is no longer enabled".into(),
        ));
    }

    let key = (
        diagnostic.symbol.clone(),
        diagnostic.strategy.as_str().to_owned(),
        diagnostic.profile_id.clone().unwrap_or_default(),
    );
    let mut diagnostics = state.latest_worker_diagnostics.lock().await;
    diagnostics.retain(|_, previous| now_ms.saturating_sub(previous.event_time_ms) <= 10 * 60_000);
    if state
        .latest_worker_decisions
        .lock()
        .await
        .get(&(key.0.clone(), key.1.clone(), key.2.clone()))
        .is_some_and(|report| report.event_time_ms >= diagnostic.event_time_ms)
    {
        return Ok(StatusCode::ACCEPTED);
    }
    if diagnostics.len() >= 512 && !diagnostics.contains_key(&key) {
        if let Some(oldest) = diagnostics
            .iter()
            .min_by_key(|(_, report)| report.event_time_ms)
            .map(|(key, _)| key.clone())
        {
            diagnostics.remove(&oldest);
        }
    }
    if diagnostics
        .get(&key)
        .is_none_or(|previous| previous.event_time_ms <= diagnostic.event_time_ms)
    {
        diagnostics.insert(key, diagnostic);
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
    if settings.closed_trade_metrics_version >= 4 {
        return;
    }
    if settings.closed_trade_metrics_version < 3 {
        let count = settings.closed_trade_returns_percent.len();
        if settings.closed_trade_timestamps_ms.len() != count
            || settings.closed_trade_realized_usd.len() != count
            || settings.closed_trade_fees_usd.len() != count
        {
            settings.closed_trade_returns_percent.clear();
            settings.closed_trade_timestamps_ms.clear();
            settings.closed_trade_realized_usd.clear();
            settings.closed_trade_fees_usd.clear();
            settings.closed_trade_audit.clear();
        } else {
            let mut valid = Vec::with_capacity(count);
            for index in 0..count {
                let return_percent = settings.closed_trade_returns_percent[index];
                let realized = settings.closed_trade_realized_usd[index];
                let fees = settings.closed_trade_fees_usd[index];
                if return_percent.is_finite() && realized.is_finite() && fees.is_finite() {
                    valid.push((
                        index,
                        return_percent,
                        settings.closed_trade_timestamps_ms[index],
                        realized,
                        fees,
                    ));
                }
            }
            let prior_audit = settings.closed_trade_audit.clone();
            settings.closed_trade_returns_percent = valid.iter().map(|item| item.1).collect();
            settings.closed_trade_timestamps_ms = valid.iter().map(|item| item.2).collect();
            settings.closed_trade_realized_usd = valid.iter().map(|item| item.3).collect();
            settings.closed_trade_fees_usd = valid.iter().map(|item| item.4).collect();
            settings.closed_trade_audit = valid
                .iter()
                .map(|item| {
                    if item.1 < -100.0 {
                        ClosedTradeAuditRecord {
                            status: ClosedTradeAuditStatus::NeedsAudit,
                            reason: Some("closed_spot_return_below_minus_100_percent".into()),
                            ..ClosedTradeAuditRecord::default()
                        }
                    } else {
                        prior_audit.get(item.0).cloned().unwrap_or_default()
                    }
                })
                .collect();
        }
        settings.closed_trade_metrics_version = 3;
    }

    let count = settings.closed_trade_returns_percent.len();
    settings
        .closed_trade_audit
        .resize(count, ClosedTradeAuditRecord::default());
    settings.closed_trade_audit.truncate(count);
    for index in 0..count {
        if settings.closed_trade_audit[index].status == ClosedTradeAuditStatus::NeedsAudit {
            continue;
        }
        if settings.closed_trade_returns_percent[index] < -100.0 {
            settings.closed_trade_audit[index] = ClosedTradeAuditRecord {
                status: ClosedTradeAuditStatus::NeedsAudit,
                reason: Some("closed_spot_return_below_minus_100_percent".into()),
                ..ClosedTradeAuditRecord::default()
            };
            continue;
        }
        let timestamp = settings
            .closed_trade_timestamps_ms
            .get(index)
            .copied()
            .unwrap_or_default();
        let matching_orders = settings
            .orders
            .iter()
            .filter(|order| {
                order.purpose == "strategy-disabled"
                    && order.status == "filled"
                    && order.strategy.as_deref() == Some("unattributed")
                    && order.timestamp <= timestamp
                    && timestamp.saturating_sub(order.timestamp) <= 5_000
            })
            .collect::<Vec<_>>();
        if !matching_orders.is_empty() {
            let unique_symbol = matching_orders
                .iter()
                .map(|order| order.symbol.as_str())
                .collect::<BTreeSet<_>>();
            let unique_trade = matching_orders
                .iter()
                .filter_map(|order| order.decision_id.as_deref())
                .collect::<BTreeSet<_>>();
            settings.closed_trade_audit[index] = ClosedTradeAuditRecord {
                status: ClosedTradeAuditStatus::NeedsAudit,
                reason: Some(if matching_orders.len() == 1 {
                    "legacy_unattributed_strategy_disabled_close".into()
                } else {
                    "ambiguous_legacy_unattributed_close_match".into()
                }),
                symbol: (unique_symbol.len() == 1)
                    .then(|| (*unique_symbol.iter().next().unwrap()).to_owned()),
                trade_id: (unique_trade.len() == 1)
                    .then(|| (*unique_trade.iter().next().unwrap()).to_owned()),
            };
        }
    }
    settings.closed_trade_metrics_version = 4;
}

#[cfg(test)]
mod closed_trade_metrics_migration_tests {
    use super::*;

    #[test]
    fn legacy_impossible_return_is_retained_for_audit_without_changing_ledger_pnl() {
        let mut settings = Settings {
            closed_trade_metrics_version: 3,
            closed_trade_returns_percent: vec![-2405.0],
            closed_trade_timestamps_ms: vec![123],
            closed_trade_realized_usd: vec![-0.89],
            closed_trade_fees_usd: vec![1.99],
            daily_realized_pnl: -11.15,
            ..Settings::default()
        };
        migrate_closed_trade_metrics(&mut settings);
        assert_eq!(settings.closed_trade_returns_percent, vec![-2405.0]);
        assert_eq!(settings.closed_trade_timestamps_ms, vec![123]);
        assert_eq!(settings.closed_trade_realized_usd, vec![-0.89]);
        assert_eq!(settings.closed_trade_fees_usd, vec![1.99]);
        assert_eq!(settings.closed_trade_audit.len(), 1);
        assert_eq!(
            settings.closed_trade_audit[0].status,
            ClosedTradeAuditStatus::NeedsAudit
        );
        assert_eq!(
            settings.closed_trade_audit[0].reason.as_deref(),
            Some("closed_spot_return_below_minus_100_percent")
        );
        assert_eq!(settings.daily_realized_pnl, -11.15);
        migrate_closed_trade_metrics(&mut settings);
        assert_eq!(settings.closed_trade_metrics_version, 4);
    }

    #[test]
    fn migration_marks_legacy_unattributed_closes_without_rewriting_normal_history() {
        let order: TraderOrder = serde_json::from_value(serde_json::json!({
            "timestamp": 1_000,
            "symbol": "BNB-USDT",
            "side": "sell",
            "order_type": "Ioc",
            "purpose": "strategy-disabled",
            "strategy": "unattributed",
            "quantity": 1.0,
            "filled_quantity": 1.0,
            "average_price": 10.0,
            "status": "filled",
            "realized_pnl": -0.01
        }))
        .unwrap();
        let mut settings = Settings {
            closed_trade_metrics_version: 3,
            closed_trade_returns_percent: vec![-90.0, 0.5],
            closed_trade_timestamps_ms: vec![1_100, 10_000],
            closed_trade_realized_usd: vec![-0.9, 0.05],
            closed_trade_fees_usd: vec![0.02, 0.01],
            orders: vec![order],
            ..Settings::default()
        };

        migrate_closed_trade_metrics(&mut settings);
        let metrics = calculate_activity_learning_metrics(&settings, 0);

        assert_eq!(settings.closed_trade_returns_percent, vec![-90.0, 0.5]);
        assert_eq!(
            settings.closed_trade_audit[0].status,
            ClosedTradeAuditStatus::NeedsAudit
        );
        assert_eq!(
            settings.closed_trade_audit[0].symbol.as_deref(),
            Some("BNB-USDT")
        );
        assert_eq!(
            settings.closed_trade_audit[1].status,
            ClosedTradeAuditStatus::Eligible
        );
        assert_eq!(metrics.exit_learning_samples, 1);
        assert_eq!(metrics.closed_trade_average_return_percent, Some(0.5));
        assert_eq!(metrics.closed_trade_realized_usd, Some(0.05));
        assert_eq!(metrics.closed_trade_needs_audit_count, 1);
        assert_eq!(
            metrics.closed_trade_needs_audit_recent[0].reason,
            "legacy_unattributed_strategy_disabled_close"
        );
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
