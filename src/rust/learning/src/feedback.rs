use super::*;

pub(super) const MINIMUM_SCOUT_PARAMETER_SAMPLES: usize = 2;
pub(super) const MAXIMUM_SCOUT_PARAMETER_SAMPLES: usize = 100;
pub(super) const PARAMETER_STEP_RELATIVE_TOLERANCE: f64 = 1.0e-6;

pub(super) async fn accept_feedback(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<Json<FeedbackResult>, (StatusCode, String)> {
    if serde_json::to_vec(&event).map_or(true, |encoded| encoded.len() > 16 * 1024) {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Learning feedback event exceeds the 16 KiB limit".into(),
        ));
    }
    if let FeedbackEvent::MissedOpportunityCandidate {
        event_id,
        observed_at_ms,
        symbol,
        strategy,
        profile_id,
        parameters,
        direction,
        strength,
        score,
        learning_context,
    } = &event
    {
        if event_id.trim().is_empty()
            || event_id.len() > 256
            || symbol.trim().is_empty()
            || symbol.len() > 64
            || strategy.is_empty()
            || strategy.len() > 64
            || profile_id.trim().is_empty()
            || profile_id.len() > 128
            || parameters.len() > 64
            || parameters.keys().any(|key| key.len() > 128)
            || !strategy.starts_with("live-")
            || !matches!(direction.as_str(), "bullish" | "bearish")
            || !strength.is_finite()
            || !score.is_finite()
            || parameters.values().any(|value| !value.is_finite())
            || learning_context.len() > 256
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid missed-opportunity candidate".into(),
            ));
        }
        let mut state = service.state.lock().await;
        if has_seen_feedback_id(&state, &service.path, event_id)? {
            return Ok(Json(FeedbackResult {
                accepted: true,
                duplicate: true,
                profile: None,
            }));
        }
        const MAX_MISSED_OPPORTUNITY_JOBS: usize = 10_000;
        if state.missed_opportunity_jobs.len() >= MAX_MISSED_OPPORTUNITY_JOBS {
            let terminal = state
                .missed_opportunity_jobs
                .values()
                .find(|job| {
                    matches!(
                        job.status,
                        StrategyReplayStatus::Completed
                            | StrategyReplayStatus::Failed
                            | StrategyReplayStatus::Unsupported
                    )
                })
                .map(|job| job.event_id.clone());
            if let Some(terminal) = terminal {
                state.missed_opportunity_jobs.remove(&terminal);
            }
        }
        if state.missed_opportunity_jobs.len() >= MAX_MISSED_OPPORTUNITY_JOBS {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Learning missed-opportunity queue is full".into(),
            ));
        }
        state.missed_opportunity_jobs.insert(
            event_id.clone(),
            MissedOpportunityJob {
                event_id: event_id.clone(),
                observed_at_ms: *observed_at_ms,
                symbol: symbol.clone(),
                strategy: strategy.clone(),
                profile_id: profile_id.clone(),
                parameters: parameters.clone(),
                direction: direction.clone(),
                strength: *strength,
                score: *score,
                learning_context: learning_context.clone(),
                status: StrategyReplayStatus::Pending,
                reason: None,
                horizons: BTreeMap::new(),
                history_cursor: None,
                pages_scanned: 0,
                coverage_start_ms: None,
                coverage_through_ms: None,
                corrupt_records: 0,
                archived_days_unscanned: 0,
                archived_days_unavailable: 0,
                cadence_gap_detected: false,
                cadence_gap_at_ms: None,
                reference_price: None,
            },
        );
        finalize_feedback(&mut state, &event, event_id);
        persist(&service.path, &state)?;
        return Ok(Json(FeedbackResult {
            accepted: true,
            duplicate: false,
            profile: None,
        }));
    }
    let (event_id, symbol, strategy, parameters, profile_id) = event_parts(&event);
    validate_feedback_identity(event_id, symbol, strategy, &profile_id, parameters)?;
    validate_trade_replay_context(&event)?;
    let mut state = service.state.lock().await;
    if has_seen_feedback_id(&state, &service.path, event_id)? {
        return Ok(Json(FeedbackResult {
            accepted: true,
            duplicate: true,
            profile: None,
        }));
    }

    if matches!(&event, FeedbackEvent::TradeOutcome { .. }) && !has_replay_job_capacity(&state) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Learning replay queue is full".into(),
        ));
    }

    let profile_key = key(symbol, strategy);
    retention::ensure_active_profile_room(&state, &profile_key)?;
    if let Some(context) = event_context(&event) {
        retention::ensure_context_room(&state, &context)?;
    }
    let minimum_parameter_samples = state.minimum_live_scout_parameter_samples;
    let source_profile = StrategyProfile {
        profile_id: profile_id.clone(),
        version: 0,
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    register_feedback_variant(&mut state, &event, source_profile, parameters, &profile_id)?;
    register_scout_baseline_if_missing(&mut state, &event, symbol, strategy);
    let update = append_profile_outcome(&mut state, &event, symbol, strategy, parameters)?;
    update_active_profile(
        &mut state,
        &event,
        symbol,
        strategy,
        parameters,
        minimum_parameter_samples,
        &profile_key,
        &update,
    );
    if matches!(&event, FeedbackEvent::ScoutOutcome { .. }) {
        promote_qualified_scout_candidates(&mut state, symbol, strategy, minimum_parameter_samples);
    }
    train_threshold_parameters(&mut state, &event, symbol, strategy, &profile_key, &update);
    enqueue_strategy_replay(&mut state, &event);
    finalize_feedback(&mut state, &event, event_id);
    persist(&service.path, &state)?;
    Ok(Json(FeedbackResult {
        accepted: true,
        duplicate: false,
        profile: Some(update.profile),
    }))
}

fn has_seen_feedback_id(
    state: &LearningState,
    path: &Path,
    event_id: &str,
) -> Result<bool, (StatusCode, String)> {
    if state.seen_event_ids.contains(event_id) {
        return Ok(true);
    }
    if state.seen_event_ids_overflowed {
        return durable_event_id_exists(path, "learning_seen_event_ids", event_id)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error));
    }
    Ok(false)
}

fn validate_trade_replay_context(event: &FeedbackEvent) -> Result<(), (StatusCode, String)> {
    let FeedbackEvent::TradeOutcome {
        replay_context: Some(context),
        ..
    } = event
    else {
        return Ok(());
    };
    if context.market_symbol.trim().is_empty()
        || context.entry_timestamp_ms == 0
        || context.exit_timestamp_ms <= context.entry_timestamp_ms
        || ![
            context.entry_price,
            context.exit_price,
            context.quantity,
            context.entry_fees,
            context.exit_fees,
        ]
        .iter()
        .all(|value| value.is_finite())
        || context.entry_price <= 0.0
        || context.exit_price <= 0.0
        || context.quantity <= 0.0
        || context.entry_fees < 0.0
        || context.exit_fees < 0.0
        || !matches!(context.side.as_str(), "long" | "short")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid trade replay context".into(),
        ));
    }
    Ok(())
}

fn enqueue_strategy_replay(state: &mut LearningState, event: &FeedbackEvent) {
    let FeedbackEvent::TradeOutcome {
        event_id,
        symbol,
        strategy,
        profile_id,
        parameters,
        replay_context,
        ..
    } = event
    else {
        return;
    };
    const MAX_DURABLE_REPLAY_JOBS: usize = 10_000;
    if state.strategy_replay_jobs.contains_key(event_id) {
        return;
    }
    if state.strategy_replay_jobs.len() >= MAX_DURABLE_REPLAY_JOBS {
        let oldest_terminal = state
            .strategy_replay_jobs
            .values()
            .filter(|job| {
                matches!(
                    job.status,
                    StrategyReplayStatus::Completed
                        | StrategyReplayStatus::Failed
                        | StrategyReplayStatus::Unsupported
                )
            })
            .min_by_key(|job| (job.updated_at_ms, job.event_id.clone()))
            .map(|job| job.event_id.clone());
        if let Some(oldest) = oldest_terminal {
            state.strategy_replay_jobs.remove(&oldest);
        }
    }
    let reason = if state.strategy_replay_jobs.len() >= MAX_DURABLE_REPLAY_JOBS {
        Some("bounded replay queue is full".to_owned())
    } else if !strategy.starts_with("live-") {
        Some("daily and dated-carry strategies require exact daily/carry replay inputs".to_owned())
    } else if replay_context.is_none() {
        Some("trade feedback lacks entry/exit timestamps, prices, fees, or quantity".to_owned())
    } else if strategy == "live-trade-flow"
        && replay_context
            .as_ref()
            .is_some_and(|context| context.entry_walk_forward_evidence.is_none())
    {
        Some(
            "live trade-flow replay requires its immutable entry-time walk-forward evidence"
                .to_owned(),
        )
    } else {
        None
    };
    let now = now_ms();
    state.strategy_replay_jobs.insert(
        event_id.clone(),
        StrategyReplayJob {
            event_id: event_id.clone(),
            symbol: symbol.clone(),
            strategy: strategy.clone(),
            profile_id: profile_id.clone(),
            parameters: parameters.clone(),
            context: replay_context.clone(),
            status: if reason.is_some() {
                StrategyReplayStatus::Unsupported
            } else {
                StrategyReplayStatus::Pending
            },
            attempts: 0,
            created_at_ms: now,
            updated_at_ms: now,
            reason,
            result: None,
            history_cursor: None,
            pages_scanned: 0,
            signals_scanned: 0,
            records_scanned: 0,
            coverage_start_ms: None,
            coverage_through_ms: None,
            last_signal_timestamp_ms: None,
            cadence_gap_detected: false,
            counterfactual_entry_found: false,
            cadence_gap_at_ms: None,
            counterfactual_position_open: false,
            counterfactual_exit_timestamp_ms: None,
            counterfactual_exit_mark_price: None,
            mark_at_actual_exit: None,
            mark_at_observation_end: None,
            archived_days_unscanned: 0,
            archived_days_unavailable: 0,
            corrupt_history_records: 0,
            horizon_mark_prices: BTreeMap::new(),
            horizon_mark_timestamps: BTreeMap::new(),
            horizons_ms: Vec::new(),
        },
    );
}

fn has_replay_job_capacity(state: &LearningState) -> bool {
    const MAX_DURABLE_REPLAY_JOBS: usize = 10_000;
    state.strategy_replay_jobs.len() < MAX_DURABLE_REPLAY_JOBS
        || state.strategy_replay_jobs.values().any(|job| {
            matches!(
                job.status,
                StrategyReplayStatus::Completed
                    | StrategyReplayStatus::Failed
                    | StrategyReplayStatus::Unsupported
            )
        })
}

fn register_scout_baseline_if_missing(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
) {
    let FeedbackEvent::ScoutOutcome {
        parameters,
        baseline_parameters,
        comparison,
        specs,
        ..
    } = event
    else {
        return;
    };
    if !matches!(comparison, strategy::ScoutComparisonMetadata::Paired { .. })
        || specs.is_empty()
        || baseline_parameters.is_empty()
    {
        return;
    }

    let mut baseline = parameters.clone();
    for spec in specs {
        let Some(value) = baseline_parameters.get(&spec.key) else {
            return;
        };
        baseline.insert(spec.key.clone(), *value);
    }

    state
        .active_profiles
        .entry(key(symbol, strategy))
        .or_insert_with(|| StrategyProfile {
            profile_id: strategy::profile_id(symbol, strategy, &baseline),
            version: 0,
            symbol: symbol.to_owned(),
            strategy: strategy.to_owned(),
            parameters: baseline,
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        });
}

fn promote_qualified_scout_candidates(
    state: &mut LearningState,
    symbol: &str,
    strategy: &str,
    minimum_parameter_samples: usize,
) {
    let mut candidates = BTreeMap::new();
    for variant in state
        .variants
        .values()
        .filter(|variant| variant.profile.symbol == symbol && variant.profile.strategy == strategy)
    {
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.source == LearningOutcomeSource::Scout
                && outcome.paired_comparison
                && !outcome.baseline_parameters.is_empty()
                && !outcome.tested_parameter_keys.is_empty()
        }) {
            let specs = variant
                .specs
                .iter()
                .filter(|spec| outcome.tested_parameter_keys.contains(&spec.key))
                .cloned()
                .collect::<Vec<_>>();
            if specs.len() != outcome.tested_parameter_keys.len() {
                continue;
            }
            let baseline = outcome.baseline_parameters.clone();
            let candidate_parameters = variant.profile.parameters.clone();
            let tested_keys = outcome.tested_parameter_keys.clone();
            let Ok(signature) =
                serde_json::to_string(&(&candidate_parameters, &baseline, &tested_keys))
            else {
                continue;
            };
            candidates
                .entry(signature)
                .or_insert((candidate_parameters, baseline, specs));
        }
    }

    let profile_key = key(symbol, strategy);
    for (_, (candidate_parameters, baseline, specs)) in candidates {
        let active_matches_baseline =
            state
                .active_profiles
                .get(&profile_key)
                .is_some_and(|active| {
                    active.parameters.len() == candidate_parameters.len()
                        && candidate_parameters.iter().all(|(parameter, value)| {
                            if specs.iter().any(|spec| &spec.key == parameter) {
                                baseline.get(parameter).is_some_and(|base| {
                                    active.parameters.get(parameter) == Some(base)
                                })
                            } else {
                                active.parameters.get(parameter) == Some(value)
                            }
                        })
                });
        if !active_matches_baseline {
            continue;
        }
        let paired = paired_parameter_outcomes(
            &state.variants,
            symbol,
            strategy,
            &candidate_parameters,
            &specs,
            &baseline,
        );
        if paired.len() < minimum_parameter_samples
            || !one_sided_90_lower_bound(&paired).is_some_and(|bound| bound > 0.0)
        {
            continue;
        }
        promote_paired_parameters(
            &state.variants,
            &mut state.active_profiles,
            symbol,
            strategy,
            &specs,
            &candidate_parameters,
        );
    }
}

pub(super) fn reconcile_scout_profiles(state: &mut LearningState) {
    let mut latest_baselines = BTreeMap::<String, (u64, StrategyProfile)>::new();
    for variant in state.variants.values() {
        for outcome in variant.outcomes.iter().filter(|outcome| {
            outcome.source == LearningOutcomeSource::Scout
                && outcome.paired_comparison
                && !outcome.baseline_parameters.is_empty()
                && !outcome.tested_parameter_keys.is_empty()
        }) {
            let specs = variant
                .specs
                .iter()
                .filter(|spec| outcome.tested_parameter_keys.contains(&spec.key))
                .collect::<Vec<_>>();
            if specs.len() != outcome.tested_parameter_keys.len() {
                continue;
            }
            let mut parameters = variant.profile.parameters.clone();
            let mut valid_baseline = true;
            for spec in specs {
                let Some(value) = outcome.baseline_parameters.get(&spec.key) else {
                    valid_baseline = false;
                    break;
                };
                parameters.insert(spec.key.clone(), *value);
            }
            if !valid_baseline {
                continue;
            }
            let profile_key = key(&variant.profile.symbol, &variant.profile.strategy);
            let candidate = StrategyProfile {
                profile_id: strategy::profile_id(
                    &variant.profile.symbol,
                    &variant.profile.strategy,
                    &parameters,
                ),
                version: 0,
                symbol: variant.profile.symbol.clone(),
                strategy: variant.profile.strategy.clone(),
                parameters,
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            };
            let replace = latest_baselines
                .get(&profile_key)
                .is_none_or(|(observed_at_ms, _)| outcome.observed_at_ms > *observed_at_ms);
            if replace {
                latest_baselines.insert(profile_key, (outcome.observed_at_ms, candidate));
            }
        }
    }

    for (profile_key, (_, baseline)) in latest_baselines {
        state.active_profiles.entry(profile_key).or_insert(baseline);
    }

    let strategies = state
        .variants
        .values()
        .map(|variant| {
            (
                variant.profile.symbol.clone(),
                variant.profile.strategy.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    for (symbol, strategy) in strategies {
        promote_qualified_scout_candidates(
            state,
            &symbol,
            &strategy,
            state.minimum_live_scout_parameter_samples,
        );
    }
}

fn validate_feedback_identity(
    event_id: &str,
    symbol: &str,
    strategy: &str,
    profile_id: &str,
    parameters: &BTreeMap<String, f64>,
) -> Result<(), (StatusCode, String)> {
    if event_id.is_empty()
        || event_id.len() > 256
        || symbol.is_empty()
        || symbol.len() > 64
        || strategy.is_empty()
        || strategy.len() > 64
        || profile_id.is_empty()
        || profile_id.len() > 128
        || parameters.len() > 64
        || parameters
            .keys()
            .any(|key| key.is_empty() || key.len() > 128)
        || parameters.values().any(|value| !value.is_finite())
    {
        Err((StatusCode::BAD_REQUEST, "invalid feedback event".into()))
    } else {
        Ok(())
    }
}

fn register_feedback_variant(
    state: &mut LearningState,
    event: &FeedbackEvent,
    source_profile: StrategyProfile,
    parameters: &BTreeMap<String, f64>,
    profile_id: &str,
) -> Result<(), (StatusCode, String)> {
    let incoming_id = variant_key(&source_profile);
    retention::ensure_variant_room(state, &incoming_id)?;
    let registered = state
        .variants
        .entry(incoming_id)
        .or_insert_with(|| RegisteredProfile {
            profile: source_profile,
            specs: event_specs(event).to_vec(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: Vec::new(),
        });
    if registered
        .specs
        .iter()
        .any(|spec| !valid_spec(spec) || !parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::CONFLICT,
            "feedback does not include valid parameter bounds".into(),
        ));
    }
    if event_specs(event).iter().any(|spec| {
        !valid_spec(spec) || !parameters.contains_key(&spec.key) || !registered.specs.contains(spec)
    }) {
        return Err((
            StatusCode::CONFLICT,
            "Scout feedback references an unregistered parameter probe".into(),
        ));
    }
    if registered.profile.profile_id != profile_id || &registered.profile.parameters != parameters {
        return Err((
            StatusCode::CONFLICT,
            "feedback profile does not match registered variant".into(),
        ));
    }
    Ok(())
}

struct ProfileOutcomeUpdate {
    profile: StrategyProfile,
    samples: usize,
    positive: usize,
    realized_pnl: f64,
    registered_specs: Vec<ParameterSpec>,
    registered_profile: StrategyProfile,
}

fn append_profile_outcome(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
) -> Result<ProfileOutcomeUpdate, (StatusCode, String)> {
    let (_, _, _, _, profile_id) = event_parts(event);
    let variant = StrategyProfile {
        profile_id,
        version: 0,
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    let registered = state
        .variants
        .get_mut(&variant_key(&variant))
        .expect("feedback profile is registered before recording its outcome");
    record_learning_outcome(registered, event);
    refresh_profile_statistics(registered);
    let profile = update_profile(&registered.profile, event_specs(event), event).ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        "feedback did not match profile".into(),
    ))?;
    let update = ProfileOutcomeUpdate {
        profile,
        samples: registered.samples,
        positive: registered.positive,
        realized_pnl: registered.realized_pnl,
        registered_specs: registered.specs.clone(),
        registered_profile: registered.profile.clone(),
    };
    if retention::compact_outcome_working_set(&mut state.variants) {
        state.outcome_history_truncated = true;
        rebuild_trade_profile_metrics(state, now_ms());
    }
    Ok(update)
}

fn record_learning_outcome(registered: &mut RegisteredProfile, event: &FeedbackEvent) {
    registered.outcomes.push(learning_outcome(event));
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    registered
        .outcomes
        .retain(|outcome| outcome.observed_at_ms >= cutoff);
    retention::compact_variant_outcomes(registered, now_ms());
}

// Outcome history stores the complete event; threshold samples are added by the
// caller once the state borrow is available. This helper constructs that record.
fn learning_outcome(event: &FeedbackEvent) -> LearningOutcome {
    let (tested_parameter_keys, baseline_parameters) = match event {
        FeedbackEvent::ScoutOutcome {
            specs,
            baseline_parameters,
            ..
        } => (
            specs.iter().map(|spec| spec.key.clone()).collect(),
            baseline_parameters.clone(),
        ),
        FeedbackEvent::TradeOutcome { .. } => (Vec::new(), BTreeMap::new()),
        FeedbackEvent::MissedOpportunityCandidate { .. } => (Vec::new(), BTreeMap::new()),
    };
    LearningOutcome {
        observed_at_ms: event_observed_at(event),
        net_return_percent: event_net_return_percent(event),
        evidence_group_id: event_evidence_group_id(event),
        learning_context: match event {
            FeedbackEvent::TradeOutcome { scout: false, .. } => event_context(event)
                .and_then(|context| context.strip_suffix(":trade").map(str::to_owned)),
            _ => None,
        },
        source: match event {
            FeedbackEvent::ScoutOutcome { .. } => LearningOutcomeSource::Scout,
            FeedbackEvent::TradeOutcome { scout: true, .. } => LearningOutcomeSource::Scout,
            FeedbackEvent::TradeOutcome { scout: false, .. } => LearningOutcomeSource::Trade,
            FeedbackEvent::MissedOpportunityCandidate { .. } => LearningOutcomeSource::Trade,
        },
        realized_pnl_usd: match event {
            FeedbackEvent::TradeOutcome {
                realized_pnl_usd, ..
            } => Some(*realized_pnl_usd),
            FeedbackEvent::ScoutOutcome { .. } => None,
            FeedbackEvent::MissedOpportunityCandidate { .. } => None,
        },
        tested_parameter_keys,
        baseline_parameters,
        paired_comparison: matches!(
            event,
            FeedbackEvent::ScoutOutcome {
                comparison: ScoutComparisonMetadata::Paired { .. },
                ..
            }
        ),
    }
}

fn refresh_profile_statistics(registered: &mut RegisteredProfile) {
    // Profile gates and statistics use actual closed-trade evidence only.
    registered.samples = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade)
        .count();
    registered.positive = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade && o.net_return_percent > 0.0)
        .count();
    registered.realized_pnl = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade)
        .filter_map(|o| o.realized_pnl_usd)
        .sum();
}

fn update_active_profile(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
    minimum_parameter_samples: usize,
    profile_key: &str,
    update: &ProfileOutcomeUpdate,
) {
    let specs = match event {
        FeedbackEvent::ScoutOutcome { specs, .. } => specs.as_slice(),
        _ => &[],
    };
    let empty_baseline = BTreeMap::new();
    let baseline = match event {
        FeedbackEvent::ScoutOutcome {
            baseline_parameters,
            ..
        } => baseline_parameters,
        _ => &empty_baseline,
    };
    let active_matches_baseline = state
        .active_profiles
        .get(profile_key)
        .is_some_and(|active| {
            active.parameters.len() == parameters.len()
                && parameters.iter().all(|(key, value)| {
                    if specs.iter().any(|spec| &spec.key == key) {
                        baseline
                            .get(key)
                            .is_some_and(|base| active.parameters.get(key) == Some(base))
                    } else {
                        active.parameters.get(key) == Some(value)
                    }
                })
        });
    let paired = if specs.is_empty() || !active_matches_baseline {
        Vec::new()
    } else {
        paired_parameter_outcomes(
            &state.variants,
            symbol,
            strategy,
            parameters,
            specs,
            baseline,
        )
    };
    let promote_candidate = matches!(event, FeedbackEvent::ScoutOutcome { .. })
        && !specs.is_empty()
        && paired.len() >= minimum_parameter_samples
        && one_sided_90_lower_bound(&paired).is_some_and(|bound| bound > 0.0);
    if promote_candidate {
        promote_paired_parameters(
            &state.variants,
            &mut state.active_profiles,
            symbol,
            strategy,
            specs,
            parameters,
        );
    } else if matches!(event, FeedbackEvent::TradeOutcome { .. })
        && should_publish_trade_profile(update.samples, update.positive, update.realized_pnl)
    {
        let active = state
            .active_profiles
            .entry(profile_key.to_owned())
            .or_insert(update.profile.clone());
        // Trade feedback reports its entry profile without undoing newer Scout promotion.
        let mut promoted = active.clone();
        promoted.version = active.version.saturating_add(1);
        promoted.profile_id =
            strategy::profile_id(&promoted.symbol, &promoted.strategy, &promoted.parameters);
        promoted.learning_samples = update.samples;
        promoted.positive_samples = update.positive;
        promoted.realized_pnl = update.realized_pnl;
        *active = promoted;
    }
}

fn train_threshold_parameters(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    profile_key: &str,
    update: &ProfileOutcomeUpdate,
) {
    let FeedbackEvent::TradeOutcome {
        threshold_observations,
        net_return_percent,
        ..
    } = event
    else {
        return;
    };
    if !net_return_percent.is_finite() {
        return;
    }
    let valid_observations = threshold_observations
        .iter()
        .filter(|observation| {
            observation.parameter_key.len() <= 128
                && observation.threshold.is_finite()
                && observation.feature_value.is_finite()
                && update
                    .registered_profile
                    .parameters
                    .get(&observation.parameter_key)
                    .is_some_and(|value| (*value - observation.threshold).abs() <= 1e-8)
                && update
                    .registered_specs
                    .iter()
                    .any(|spec| spec.key == observation.parameter_key)
        })
        .collect::<Vec<_>>();
    if valid_observations.is_empty() {
        return;
    }
    for observation in valid_observations {
        state.threshold_samples.push(ThresholdTrainingSample {
            symbol: symbol.to_owned(),
            strategy: strategy.to_owned(),
            parameter_key: observation.parameter_key.clone(),
            observed_at_ms: event_observed_at(event),
            threshold: observation.threshold,
            feature_value: observation.feature_value,
            net_return_percent: *net_return_percent,
        });
    }
    if state.threshold_samples.len() > 100_000 {
        state
            .threshold_samples
            .sort_by_key(|sample| sample.observed_at_ms);
        state
            .threshold_samples
            .drain(..state.threshold_samples.len() - 100_000);
    }
    for spec in &update.registered_specs {
        let parameter_key = spec.key.as_str();
        if !state
            .threshold_samples
            .iter()
            .any(|sample| sample.strategy == strategy && sample.parameter_key == parameter_key)
        {
            continue;
        }
        let active_value = state
            .active_profiles
            .get(profile_key)
            .and_then(|active| active.parameters.get(parameter_key))
            .copied()
            .or_else(|| {
                update
                    .registered_profile
                    .parameters
                    .get(parameter_key)
                    .copied()
            });
        let Some(base) = active_value else { continue };
        let Some(tuned) = walk_forward_threshold(spec, base, strategy, &state.threshold_samples)
        else {
            continue;
        };
        if !tuning_delta_is_effective(spec.step, base, tuned) {
            continue;
        }
        let active = state
            .active_profiles
            .entry(profile_key.to_owned())
            .or_insert_with(|| update.registered_profile.clone());
        active.parameters.insert(parameter_key.to_owned(), tuned);
        active.version = active.version.saturating_add(1);
        active.profile_id =
            strategy::profile_id(&active.symbol, &active.strategy, &active.parameters);
    }
}

fn finalize_feedback(state: &mut LearningState, event: &FeedbackEvent, event_id: &str) {
    state.seen_event_ids.insert(event_id.to_owned());
    while state.seen_event_ids.len() > 100_000 {
        let Some(oldest) = state
            .seen_event_ids
            .iter()
            .find(|seen_id| seen_id.as_str() != event_id)
            .cloned()
        else {
            break;
        };
        state.seen_event_ids.remove(&oldest);
        state.seen_event_ids_overflowed = true;
    }
    state.accepted_events = state.accepted_events.saturating_add(1);
    if let Some(context) = event_context(event) {
        record_context_outcome(
            &mut state.context_learning,
            &context,
            event_observed_at(event),
            event_was_profitable(event),
            event_net_return_percent(event),
        );
    }
}

#[derive(Deserialize)]
pub(super) struct ScoutParameterSamplesRequest {
    minimum_samples: usize,
}

pub(super) async fn update_scout_parameter_samples(
    State(service): State<ServiceState>,
    Json(request): Json<ScoutParameterSamplesRequest>,
) -> Result<Json<usize>, (StatusCode, String)> {
    if !(MINIMUM_SCOUT_PARAMETER_SAMPLES..=MAXIMUM_SCOUT_PARAMETER_SAMPLES)
        .contains(&request.minimum_samples)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "minimum_samples must be between {MINIMUM_SCOUT_PARAMETER_SAMPLES} and {MAXIMUM_SCOUT_PARAMETER_SAMPLES}"
            ),
        ));
    }
    let mut state = service.state.lock().await;
    state.minimum_live_scout_parameter_samples = request.minimum_samples;
    persist(&service.path, &state)?;
    Ok(Json(state.minimum_live_scout_parameter_samples))
}

fn tuning_delta_is_effective(step: f64, base: f64, tuned: f64) -> bool {
    (tuned - base).abs() >= step * PARAMETER_STEP_RELATIVE_TOLERANCE
}

fn paired_parameter_outcomes(
    variants: &BTreeMap<String, RegisteredProfile>,
    symbol: &str,
    strategy: &str,
    candidate_parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    baseline_parameters: &BTreeMap<String, f64>,
) -> Vec<f64> {
    let mut grouped_outcomes = BTreeMap::<String, Vec<f64>>::new();
    for (profile_id, variant) in variants.iter().filter(|(_, variant)| {
        variant.profile.symbol == symbol && variant.profile.strategy == strategy
    }) {
        let matches_candidate = &variant.profile.parameters == candidate_parameters;
        if !matches_candidate {
            continue;
        }
        for (index, outcome) in variant.outcomes.iter().enumerate() {
            if outcome.source == LearningOutcomeSource::Scout
                && outcome.paired_comparison
                && outcome.baseline_parameters == *baseline_parameters
                && specs
                    .iter()
                    .all(|spec| outcome.tested_parameter_keys.contains(&spec.key))
            {
                let group = outcome
                    .evidence_group_id
                    .clone()
                    .unwrap_or_else(|| format!("{profile_id}:{index}"));
                grouped_outcomes
                    .entry(group)
                    .or_default()
                    .push(outcome.net_return_percent);
            }
        }
    }
    grouped_outcomes
        .values()
        .map(|returns| returns.iter().sum::<f64>() / returns.len() as f64)
        .collect()
}

fn promote_paired_parameters(
    variants: &BTreeMap<String, RegisteredProfile>,
    active_profiles: &mut BTreeMap<String, StrategyProfile>,
    symbol: &str,
    strategy: &str,
    specs: &[ParameterSpec],
    candidate_parameters: &BTreeMap<String, f64>,
) {
    let candidates = variants
        .values()
        .filter(|variant| {
            variant.profile.strategy == strategy
                && variant.profile.symbol == symbol
                && &variant.profile.parameters == candidate_parameters
                && variant.outcomes.iter().any(|outcome| {
                    specs
                        .iter()
                        .all(|spec| outcome.tested_parameter_keys.contains(&spec.key))
                })
        })
        .map(|variant| variant.profile.clone())
        .collect::<Vec<_>>();

    for candidate in candidates {
        let profile_key = key(&candidate.symbol, strategy);
        let active = active_profiles
            .get(&profile_key)
            .cloned()
            .unwrap_or_else(|| candidate.clone());
        let mut promoted = active.clone();
        let mut changed = false;
        for spec in specs {
            let Some(value) = candidate.parameters.get(&spec.key).copied() else {
                continue;
            };
            if promoted.parameters.get(&spec.key) != Some(&value) {
                promoted.parameters.insert(spec.key.clone(), value);
                changed = true;
            }
        }
        if changed {
            promoted.version = active.version.saturating_add(1);
            promoted.profile_id =
                strategy::profile_id(&promoted.symbol, &promoted.strategy, &promoted.parameters);
            active_profiles.insert(profile_key, promoted);
        }
    }
}

//TODO comment this - lots of random numbers!!
pub(super) fn one_sided_90_t_critical(sample_count: usize) -> Option<f64> {
    let degrees = sample_count.checked_sub(1)? as f64;
    if degrees < 1.0 {
        return None;
    }
    match degrees as usize {
        1 => return Some(3.077_683_537),
        2 => return Some(1.885_618_083),
        3 => return Some(1.637_744_354),
        4 => return Some(1.533_206_275),
        _ => {}
    }
    let z = 1.281_551_565_544_600_4;
    let z2 = z * z;
    let z3 = z2 * z;
    let z5 = z3 * z2;
    let z7 = z5 * z2;
    Some(
        z + (z3 + z) / (4.0 * degrees)
            + (5.0 * z5 + 16.0 * z3 + 3.0 * z) / (96.0 * degrees.powi(2))
            + (3.0 * z7 + 19.0 * z5 + 17.0 * z3 - 15.0 * z) / (384.0 * degrees.powi(3)),
    )
}

//TODO comment this - what is it really doing
fn one_sided_90_lower_bound(outcomes: &[f64]) -> Option<f64> {
    if outcomes.len() < 2 || outcomes.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let sample_count = outcomes.len();
    let mean = outcomes.iter().sum::<f64>() / sample_count as f64;
    let variance = outcomes
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (sample_count - 1) as f64;
    let critical = one_sided_90_t_critical(sample_count)?;
    Some(mean - critical * (variance / sample_count as f64).sqrt())
}

pub(super) fn should_publish_trade_profile(
    samples: usize,
    positive: usize,
    net_outcome: f64,
) -> bool {
    samples >= 5 && positive <= samples && net_outcome.is_finite()
}

pub(super) fn persist(path: &Path, state: &LearningState) -> Result<(), (StatusCode, String)> {
    //TODO - could we just keep the db open ?
    let connection =
        super::open_state_db(path).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    super::persist_normalized(&connection, state).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not persist learning state: {error}"),
        )
    })?;
    Ok(())
}

pub(super) fn valid_spec(spec: &ParameterSpec) -> bool {
    spec.default.is_finite()
        && spec.min.is_finite()
        && spec.max.is_finite()
        && spec.step.is_finite()
        && spec.learning_direction.is_finite()
        && spec.min <= spec.default
        && spec.default <= spec.max
        && spec.min < spec.max
        && spec.step > 0.0
}

pub(super) fn event_specs(event: &FeedbackEvent) -> &[ParameterSpec] {
    match event {
        FeedbackEvent::ScoutOutcome { specs, .. } | FeedbackEvent::TradeOutcome { specs, .. } => {
            specs
        }
        FeedbackEvent::MissedOpportunityCandidate { .. } => &[],
    }
}

//TODO rename - is positive is vague.
pub(super) fn event_was_profitable(event: &FeedbackEvent) -> bool {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        } => *net_return_percent > 0.0,
        FeedbackEvent::TradeOutcome {
            realized_pnl_usd, ..
        } => *realized_pnl_usd > 0.0,
        FeedbackEvent::MissedOpportunityCandidate { .. } => false,
    }
}

pub(super) fn event_net_return_percent(event: &FeedbackEvent) -> f64 {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        }
        | FeedbackEvent::TradeOutcome {
            net_return_percent, ..
        } => *net_return_percent,
        FeedbackEvent::MissedOpportunityCandidate { .. } => 0.0,
    }
}

fn event_evidence_group_id(event: &FeedbackEvent) -> Option<String> {
    let FeedbackEvent::ScoutOutcome {
        comparison: ScoutComparisonMetadata::Paired { group_id, .. },
        ..
    } = event
    else {
        return None;
    };
    Some(group_id.clone())
}

pub(super) fn event_context(event: &FeedbackEvent) -> Option<String> {
    match event {
        FeedbackEvent::ScoutOutcome {
            learning_context, ..
        } => learning_context.clone(),
        FeedbackEvent::TradeOutcome {
            learning_context, ..
        } => {
            let raw = learning_context.as_deref()?;
            let (scout_prefix, context) = raw
                .strip_prefix("scout:")
                .map_or(("", raw), |context| ("scout:", context));
            let context = context.split_once(":P")?.0;
            (context.split(':').count() >= 3).then(|| format!("{scout_prefix}{context}:trade"))
        }
        FeedbackEvent::MissedOpportunityCandidate { .. } => None,
    }
}

pub(super) fn record_context_outcome(
    contexts: &mut BTreeMap<String, ContextLearningStat>,
    context: &str,
    observed_at_ms: u64,
    positive: bool,
    value: f64,
) {
    if context.is_empty() || context.len() > 256 || !value.is_finite() {
        return;
    }
    let stats = contexts
        .entry(context.to_owned())
        .or_insert(ContextLearningStat {
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            updated_at_ms: observed_at_ms,
        });
    if stats.updated_at_ms != 0 {
        let elapsed = observed_at_ms.saturating_sub(stats.updated_at_ms);
        //TODO remove hardcoding
        let weight = 0.5_f64.powf(elapsed as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1_000.0));
        stats.samples = (stats.samples as f64 * weight).round() as usize;
        stats.positive = ((stats.positive as f64 * weight).round() as usize).min(stats.samples);
        stats.realized_pnl *= weight;
    }
    stats.samples = stats.samples.saturating_add(1);
    if positive {
        stats.positive = stats.positive.saturating_add(1);
    }
    stats.realized_pnl += value;
    stats.updated_at_ms = observed_at_ms;
}

/// Apply one event only to the exact profile variant that generated it.
pub fn update_profile(
    profile: &StrategyProfile,
    specs: &[ParameterSpec],
    event: &FeedbackEvent,
) -> Option<StrategyProfile> {
    let (event_profile, strategy, symbol, parameters, outcome) = match event {
        FeedbackEvent::ScoutOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            net_return_percent,
            ..
        } => (
            profile_id,
            strategy,
            symbol,
            parameters,
            *net_return_percent,
        ),
        FeedbackEvent::TradeOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            realized_pnl_usd,
            ..
        } => (profile_id, strategy, symbol, parameters, *realized_pnl_usd),
        FeedbackEvent::MissedOpportunityCandidate { .. } => return None,
    };
    if event_profile != &profile.profile_id
        || strategy != &profile.strategy
        || symbol != &profile.symbol
        || parameters != &profile.parameters
        || !outcome.is_finite()
    {
        return None;
    }
    let mut updated = profile.clone();
    // Feedback changes evidence. Paired scout promotion below chooses the
    // exact tested parameter value; outcome feedback never steps past it.
    let _ = specs;
    updated.version = updated.version.saturating_add(1);
    updated.profile_id =
        strategy::profile_id(&updated.symbol, &updated.strategy, &updated.parameters);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missed_opportunity_feedback_is_attributed_deduplicated_and_durable() {
        let path = std::env::temp_dir().join(format!(
            "learning-missed-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let service = ServiceState {
            state: Arc::new(Mutex::new(LearningState::default())),
            path: path.clone(),
        };
        let event = FeedbackEvent::MissedOpportunityCandidate {
            event_id: "missed:BTC-USDT:live-momentum:1000".into(),
            observed_at_ms: 1_000,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-1".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.002)]),
            direction: "bullish".into(),
            strength: 0.61,
            score: 0.31,
            learning_context: "trend:btc".into(),
        };
        let accepted = accept_feedback(State(service.clone()), Json(event.clone()))
            .await
            .unwrap();
        assert!(accepted.0.accepted);
        let duplicate = accept_feedback(State(service.clone()), Json(event))
            .await
            .unwrap();
        assert!(duplicate.0.duplicate);
        let state = service.state.lock().await;
        let job = state
            .missed_opportunity_jobs
            .get("missed:BTC-USDT:live-momentum:1000")
            .unwrap();
        assert_eq!(job.strategy, "live-momentum");
        assert_eq!(job.profile_id, "profile-1");
        assert_eq!(job.parameters["minimum_momentum"], 0.002);
        assert!(
            load_state(&path)
                .unwrap()
                .missed_opportunity_jobs
                .contains_key(&job.event_id)
        );
        drop(state);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn trade_feedback_creates_one_durable_replay_job_and_missing_context_is_unsupported() {
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:missing-context".into(),
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: vec![],
            replay_context: None,
        };
        let mut state = LearningState::default();
        enqueue_strategy_replay(&mut state, &event);
        enqueue_strategy_replay(&mut state, &event);
        assert_eq!(state.strategy_replay_jobs.len(), 1);
        let job = state
            .strategy_replay_jobs
            .get("trade:missing-context")
            .unwrap();
        assert_eq!(job.status, StrategyReplayStatus::Unsupported);
        assert!(job.reason.as_deref().unwrap().contains("lacks entry/exit"));
    }

    #[test]
    fn dated_trade_replay_is_unsupported_without_exact_daily_inputs() {
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:daily".into(),
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "donchian-20-btc200".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: vec![],
            replay_context: None,
        };
        let mut state = LearningState::default();
        enqueue_strategy_replay(&mut state, &event);
        let job = state.strategy_replay_jobs.get("trade:daily").unwrap();
        assert_eq!(job.status, StrategyReplayStatus::Unsupported);
        assert!(job.reason.as_deref().unwrap().contains("exact daily"));
    }

    #[test]
    fn pending_replay_job_and_cursor_state_survive_sqlite_reload() {
        let event = FeedbackEvent::TradeOutcome {
            event_id: "trade:replay".into(),
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: vec![],
            replay_context: Some(strategy::TradeReplayContext {
                market_symbol: "BTC-USDT".into(),
                entry_timestamp_ms: 1_000,
                exit_timestamp_ms: 2_000,
                side: "long".into(),
                entry_price: 10.0,
                exit_price: 11.0,
                quantity: 1.0,
                entry_fees: 0.01,
                exit_fees: 0.01,
                execution_costs_complete: false,
                entry_walk_forward_evidence: None,
            }),
        };
        let mut state = LearningState::default();
        enqueue_strategy_replay(&mut state, &event);
        let job = state.strategy_replay_jobs.get_mut("trade:replay").unwrap();
        job.history_cursor = Some("1500:42".into());
        job.counterfactual_position_open = true;
        job.counterfactual_entry_found = true;
        job.horizons_ms = vec![300_000, 1_800_000, 7_200_000, 28_800_000];
        job.counterfactual_exit_timestamp_ms = Some(50_000);
        let path =
            std::env::temp_dir().join(format!("learning-replay-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        persist(&path, &state).unwrap();
        let loaded = super::super::load_state(&path).unwrap();
        let loaded_job = loaded.strategy_replay_jobs.get("trade:replay").unwrap();
        assert_eq!(loaded_job.history_cursor.as_deref(), Some("1500:42"));
        assert!(loaded_job.counterfactual_position_open);
        assert!(loaded_job.counterfactual_entry_found);
        assert_eq!(loaded_job.counterfactual_exit_timestamp_ms, Some(50_000));
        assert_eq!(
            loaded_job.horizons_ms,
            [300_000, 1_800_000, 7_200_000, 28_800_000]
        );
        assert_eq!(loaded_job.status, StrategyReplayStatus::Pending);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn one_sided_90_student_t_bounds_match_small_and_mature_samples() {
        assert!((one_sided_90_t_critical(6).unwrap() - 1.4759).abs() < 0.002);
        assert!((one_sided_90_t_critical(20).unwrap() - 1.3277).abs() < 0.002);
        assert!((one_sided_90_t_critical(2).unwrap() - 3.0777).abs() < 0.002);
        assert!((one_sided_90_t_critical(5).unwrap() - 1.5332).abs() < 0.002);
        assert!(one_sided_90_t_critical(1).is_none());
        assert!((one_sided_90_lower_bound(&[0.02; 6]).unwrap() - 0.02).abs() < 1e-12);
    }

    fn spec() -> ParameterSpec {
        ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        }
    }

    fn scout_event() -> FeedbackEvent {
        FeedbackEvent::ScoutOutcome {
            event_id: "e".into(),
            observed_at_ms: 100,
            learning_context: Some("btc:momentum:long".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec()],
            net_return_percent: 1.0,
        }
    }

    #[test]
    fn paired_evidence_does_not_mix_baselines_or_surrounding_profiles() {
        let candidate = BTreeMap::from([("threshold".into(), 0.4), ("stop".into(), 0.5)]);
        let baseline = BTreeMap::from([("threshold".into(), 0.5)]);
        let outcome =
            |baseline: BTreeMap<String, f64>, value: f64, group: Option<&str>| LearningOutcome {
                observed_at_ms: 100,
                net_return_percent: value,
                evidence_group_id: group.map(str::to_owned),
                learning_context: None,
                source: LearningOutcomeSource::Scout,
                realized_pnl_usd: None,
                paired_comparison: true,
                tested_parameter_keys: vec!["threshold".into()],
                baseline_parameters: baseline,
            };
        let profile = |parameters: BTreeMap<String, f64>, outcomes| RegisteredProfile {
            profile: StrategyProfile {
                profile_id: "profile".into(),
                version: 0,
                symbol: "BTC-USDT".into(),
                strategy: "momentum".into(),
                parameters,
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![spec()],
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes,
        };
        let variants = BTreeMap::from([
            (
                "candidate".into(),
                profile(
                    candidate.clone(),
                    vec![
                        outcome(baseline.clone(), 0.1, Some("same-opportunity")),
                        outcome(baseline.clone(), 0.3, Some("same-opportunity")),
                        outcome(BTreeMap::from([("threshold".into(), 0.6)]), 1.0, None),
                    ],
                ),
            ),
            (
                "other".into(),
                profile(
                    BTreeMap::from([("threshold".into(), 0.4), ("stop".into(), 0.8)]),
                    vec![outcome(baseline.clone(), 2.0, None)],
                ),
            ),
        ]);
        assert_eq!(
            paired_parameter_outcomes(
                &variants,
                "BTC-USDT",
                "momentum",
                &candidate,
                &[spec()],
                &baseline
            ),
            vec![0.2]
        );
    }

    #[test]
    fn feedback_event_helpers_extract_fields_and_positive_values() {
        let scout = scout_event();
        assert_eq!(event_specs(&scout).len(), 1);
        assert!(event_was_profitable(&scout));
        assert_eq!(event_observed_at(&scout), 100);
        let mut horizon = scout_event();
        if let FeedbackEvent::ScoutOutcome { comparison, .. } = &mut horizon {
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "comparison".into(),
                horizon_seconds: None,
            };
        }
        assert_eq!(
            event_evidence_group_id(&horizon).as_deref(),
            Some("comparison")
        );
        if let FeedbackEvent::ScoutOutcome { comparison, .. } = &mut horizon {
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "comparison".into(),
                horizon_seconds: Some(300),
            };
        }
        assert_eq!(
            event_evidence_group_id(&horizon).as_deref(),
            Some("comparison")
        );
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "t".into(),
            observed_at_ms: 200,
            learning_context: Some("scout:btc:momentum:long:P2".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: true,
            realized_pnl_usd: -2.0,
            net_return_percent: 3.0,
            threshold_observations: Vec::new(),
            replay_context: None,
        };
        assert!(!event_was_profitable(&trade));
        assert_eq!(event_net_return_percent(&trade), 3.0);
        assert_eq!(
            event_context(&trade).as_deref(),
            Some("scout:btc:momentum:long:trade")
        );
    }

    #[test]
    fn tuning_delta_uses_step_relative_tolerance_boundary() {
        let step = 0.5;
        let base = 10.0;
        let tolerance = step * PARAMETER_STEP_RELATIVE_TOLERANCE;
        assert!(!tuning_delta_is_effective(
            step,
            base,
            base + tolerance * 0.5
        ));
        assert!(tuning_delta_is_effective(step, base, base + tolerance));
    }

    #[test]
    fn scout_feedback_requires_explicit_comparison_metadata() {
        let serialized = serde_json::to_value(scout_event()).unwrap();
        assert_eq!(
            serialized["comparison"]["status"],
            serde_json::json!("independent")
        );

        let mut legacy = serialized;
        legacy.as_object_mut().unwrap().remove("comparison");
        assert!(serde_json::from_value::<FeedbackEvent>(legacy).is_err());
    }

    #[test]
    fn event_context_requires_a_complete_trade_learning_context() {
        for context in [None, Some("btc:momentum"), Some("btc:momentum:long")] {
            let event = FeedbackEvent::TradeOutcome {
                event_id: "t".into(),
                observed_at_ms: 1,
                learning_context: context.map(str::to_owned),
                symbol: "BTC".into(),
                strategy: "s".into(),
                profile_id: "p".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: false,
                realized_pnl_usd: 0.0,
                net_return_percent: 0.0,
                threshold_observations: Vec::new(),
                replay_context: None,
            };
            assert!(event_context(&event).is_none());
        }
        let scout = scout_event();
        assert_eq!(event_context(&scout).as_deref(), Some("btc:momentum:long"));
    }

    #[test]
    fn scout_trade_outcomes_share_the_scout_category() {
        let mut scout_trade = match scout_event() {
            FeedbackEvent::ScoutOutcome { .. } => FeedbackEvent::TradeOutcome {
                event_id: "paper-scout".into(),
                observed_at_ms: 1,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "momentum".into(),
                profile_id: "p".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: true,
                realized_pnl_usd: 1.0,
                net_return_percent: 0.5,
                threshold_observations: vec![],
                replay_context: None,
            },
            _ => unreachable!(),
        };
        assert_eq!(
            learning_outcome(&scout_trade).source,
            LearningOutcomeSource::Scout
        );
        if let FeedbackEvent::TradeOutcome { scout, .. } = &mut scout_trade {
            *scout = false;
        }
        assert_eq!(
            learning_outcome(&scout_trade).source,
            LearningOutcomeSource::Trade
        );
    }

    #[test]
    fn parameter_specs_validate_finite_ordered_bounds_and_positive_steps() {
        assert!(valid_spec(&spec()));
        let mut invalid = spec();
        invalid.step = 0.0;
        assert!(!valid_spec(&invalid));
        let mut invalid = spec();
        invalid.default = f64::NAN;
        assert!(!valid_spec(&invalid));
        let mut invalid = spec();
        invalid.min = 0.8;
        assert!(!valid_spec(&invalid));
    }

    #[test]
    fn context_outcomes_ignore_invalid_inputs_and_decay_existing_statistics() {
        let mut contexts = BTreeMap::new();
        record_context_outcome(&mut contexts, "", 100, true, 1.0);
        record_context_outcome(&mut contexts, "valid", 100, true, f64::NAN);
        assert!(contexts.is_empty());
        record_context_outcome(&mut contexts, "valid", 100, true, 2.0);
        record_context_outcome(
            &mut contexts,
            "valid",
            100 + 7 * 24 * 60 * 60 * 1_000,
            false,
            -1.0,
        );
        let stats = &contexts["valid"];
        assert_eq!(stats.samples, 2);
        assert_eq!(stats.positive, 1);
        assert_eq!(stats.realized_pnl, 0.0);
    }

    #[test]
    fn persistence_round_trips_state_atomically() {
        let path = std::env::temp_dir().join(format!(
            "learning-persist-test-{}.sqlite",
            std::process::id()
        ));
        let mut state = LearningState {
            accepted_events: 2,
            minimum_live_scout_parameter_samples: 9,
            seen_event_ids: ["keep".to_owned(), "remove".to_owned()].into(),
            ..LearningState::default()
        };
        persist(&path, &state).unwrap();
        state.accepted_events = 1;
        state.seen_event_ids.remove("remove");
        persist(&path, &state).unwrap();
        let decoded = super::super::load_state(&path).unwrap();
        assert!(decoded.variants.is_empty());
        assert_eq!(decoded.accepted_events, 1);
        assert_eq!(decoded.minimum_live_scout_parameter_samples, 9);
        assert_eq!(
            decoded.seen_event_ids,
            ["keep".to_owned(), "remove".to_owned()].into()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn evicted_feedback_ids_are_still_deduplicated_from_sqlite() {
        let path = std::env::temp_dir().join(format!(
            "learning-feedback-dedupe-test-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let connection = open_state_db(&path).unwrap();
        connection
            .execute(
                "INSERT INTO learning_seen_event_ids (id) VALUES (?1)",
                ["evicted-trade-id"],
            )
            .unwrap();
        drop(connection);

        let mut state = LearningState::default();
        state.seen_event_ids_overflowed = true;
        assert!(has_seen_feedback_id(&state, &path, "evicted-trade-id").unwrap());
        assert!(!has_seen_feedback_id(&state, &path, "new-trade-id").unwrap());
        let _ = std::fs::remove_file(path);
    }
}
